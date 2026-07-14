#!/usr/bin/env bash
# Reproducible project runtime replay/restart smoke.
#
# Starts an isolated Captain daemon, creates a project, starts its runtime,
# simulates a daemon crash, restarts the daemon, resumes the runtime, then
# verifies operator-safe API/CLI replay and timeline views.

set -u

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKDIR="${CAPTAIN_PROJECT_SMOKE_WORKDIR:-$ROOT_DIR/target/project-runtime-restart-smoke}"
PORT="${CAPTAIN_PROJECT_SMOKE_PORT:-50371}"
BASE="http://127.0.0.1:$PORT"
TIMEOUT="${CAPTAIN_PROJECT_SMOKE_TIMEOUT:-45}"
READY_TIMEOUT="${CAPTAIN_PROJECT_SMOKE_READY_TIMEOUT:-25}"
SETTLE_SECS="${CAPTAIN_PROJECT_SMOKE_SETTLE_SECS:-6}"
PROJECT_SLUG="${CAPTAIN_PROJECT_SMOKE_SLUG:-restart-smoke-$$}"
CAPTAIN_BIN="${CAPTAIN_BIN:-}"
HOME_DIR="$WORKDIR/home"
CONFIG="$HOME_DIR/config.toml"
LOG="$WORKDIR/daemon.log"
PID=""
PASS=0

note() { printf '   %s\n' "$*"; }
pass() {
  printf '   ok %s\n' "$1"
  PASS=$((PASS + 1))
}
fail() {
  printf '   FAIL %s\n' "$1" >&2
  if [ -f "$LOG" ]; then
    printf '\n--- daemon log tail ---\n' >&2
    tail -80 "$LOG" >&2 || true
  fi
  cleanup
  exit 1
}

cleanup() {
  if [ -n "${PID:-}" ] && kill -0 "$PID" >/dev/null 2>&1; then
    kill "$PID" >/dev/null 2>&1 || true
    sleep 1
    if kill -0 "$PID" >/dev/null 2>&1; then
      kill -KILL "$PID" >/dev/null 2>&1 || true
    fi
  fi
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

resolve_captain_bin() {
  if [ -n "$CAPTAIN_BIN" ]; then
    [ -x "$CAPTAIN_BIN" ] || fail "CAPTAIN_BIN is not executable: $CAPTAIN_BIN"
    return
  fi
  if [ -x "$ROOT_DIR/target/release/captain" ]; then
    CAPTAIN_BIN="$ROOT_DIR/target/release/captain"
    return
  fi
  if [ -x "$ROOT_DIR/target/debug/captain" ]; then
    CAPTAIN_BIN="$ROOT_DIR/target/debug/captain"
    return
  fi
  note "building captain CLI because no local binary exists"
  (cd "$ROOT_DIR" && cargo build -p captain-cli) || fail "cargo build -p captain-cli failed"
  CAPTAIN_BIN="$ROOT_DIR/target/debug/captain"
}

write_config() {
  mkdir -p "$HOME_DIR" "$HOME_DIR/data" "$WORKDIR"
  cat >"$CONFIG" <<EOF
home_dir = "$HOME_DIR"
data_dir = "$HOME_DIR/data"
log_level = "info"
api_listen = "127.0.0.1:$PORT"
network_enabled = false
api_key = ""
language = "en"

[default_model]
provider = "codex"
model = "gpt-5.5"
api_key_env = ""

[approval]
require_approval = []
EOF
}

wait_for_health() {
  local elapsed=0
  local body
  while [ "$elapsed" -le "$READY_TIMEOUT" ]; do
    body=$(curl -sS --connect-timeout 1 --max-time 2 "$BASE/api/health" 2>/dev/null || true)
    if printf '%s' "$body" | jq -e '.status == "ok"' >/dev/null 2>&1; then
      printf '%s' "$body" >"$WORKDIR/health.json"
      return 0
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  return 1
}

start_daemon() {
  if curl -sS --connect-timeout 1 --max-time 2 "$BASE/api/health" >/dev/null 2>&1; then
    fail "port $PORT already serves an HTTP endpoint"
  fi
  : >"$LOG"
  CAPTAIN_HOME="$HOME_DIR" "$CAPTAIN_BIN" start --config "$CONFIG" --yolo >>"$LOG" 2>&1 &
  PID="$!"
  wait_for_health || fail "daemon did not become healthy on $BASE"
  pass "daemon healthy on $BASE"
}

crash_daemon() {
  [ -n "$PID" ] || fail "daemon pid missing"
  kill "$PID" >/dev/null 2>&1 || true
  for _ in $(seq 1 20); do
    if ! kill -0 "$PID" >/dev/null 2>&1; then
      PID=""
      pass "test daemon interrupted"
      return
    fi
    sleep 0.2
  done
  kill -KILL "$PID" >/dev/null 2>&1 || true
  PID=""
  pass "test daemon force-interrupted"
}

http_get() {
  local path="$1"
  curl -sS --max-time "$TIMEOUT" "$BASE$path"
}

http_post_json() {
  local path="$1"
  local body="$2"
  printf '%s' "$body" |
    curl -sS --max-time "$TIMEOUT" -H "Content-Type: application/json" --data-binary @- "$BASE$path"
}

assert_jq() {
  local file="$1"
  local filter="$2"
  local label="$3"
  jq -e "$filter" "$file" >/dev/null 2>&1 || fail "$label"
  pass "$label"
}

event_filter() {
  local kind="$1"
  printf '((.transcript.events // []) + (.runtime.timeline // []) | map(.kind) | index("%s") != null)' "$kind"
}

assert_no_blocked_runtime() {
  local file="$1"
  local label="$2"
  assert_jq "$file" '
    def replay_events:
      if (.events | type) == "object" then (.events.items // [])
      elif (.events | type) == "array" then .events
      else []
      end;
    (((.transcript.events // []) + (.runtime.timeline // []) + replay_events)
      | map(.kind)
      | index("orchestrator.blocked") == null)
    and ((.runtime.status // "") != "blocked")
    and ((.state // "") != "blocked")
    and ((.operator_status.state // "") != "blocked")
  ' "$label"
}

capture_timeline_follow() {
  CAPTAIN_HOME="$HOME_DIR" "$CAPTAIN_BIN" project timeline "$PROJECT_SLUG" --limit 50 --follow \
    >"$WORKDIR/cli-timeline-follow.txt" 2>&1 &
  local follow_pid="$!"
  sleep 3
  kill "$follow_pid" >/dev/null 2>&1 || true
  wait "$follow_pid" >/dev/null 2>&1 || true
  grep -F "orchestrator.resume_after_restart" "$WORKDIR/cli-timeline-follow.txt" >/dev/null 2>&1 ||
    fail "CLI timeline follow did not show resume event"
  pass "CLI timeline follow shows resume event"
}

run_smoke() {
  require_cmd curl
  require_cmd grep
  require_cmd jq
  require_cmd tail
  resolve_captain_bin
  write_config

  note "workdir=$WORKDIR"
  note "captain_bin=$CAPTAIN_BIN"

  start_daemon

  http_post_json "/api/projects" "$(jq -nc --arg slug "$PROJECT_SLUG" '{
    name: "Restart Smoke",
    slug: $slug,
    goal: "Prove project runtime restart replay without raw payload leaks."
  }')" >"$WORKDIR/project-create.json" || fail "create project request failed"
  assert_jq "$WORKDIR/project-create.json" '.slug == "'"$PROJECT_SLUG"'"' "project created"

  http_post_json "/api/projects/$PROJECT_SLUG/runtime/start" '{}' >"$WORKDIR/runtime-start-1.json" ||
    fail "first runtime start request failed"
  assert_jq "$WORKDIR/runtime-start-1.json" "$(event_filter project.started)" "first start records project.started"

  http_get "/api/projects/$PROJECT_SLUG/runtime?events=80" >"$WORKDIR/runtime-before-restart.json" ||
    fail "runtime fetch before restart failed"
  assert_jq "$WORKDIR/runtime-before-restart.json" "$(event_filter project.started)" "runtime before restart has timeline"

  crash_daemon
  start_daemon

  http_post_json "/api/projects/$PROJECT_SLUG/runtime/start" '{}' >"$WORKDIR/runtime-start-2.json" ||
    fail "second runtime start request failed"
  assert_jq "$WORKDIR/runtime-start-2.json" "$(event_filter orchestrator.resume_after_restart)" "restart records resume_after_restart"
  assert_jq "$WORKDIR/runtime-start-2.json" '.operator_status.declared_active == true' "runtime remains declared active"
  assert_no_blocked_runtime "$WORKDIR/runtime-start-2.json" "restart response has no blocked runtime"

  sleep "$SETTLE_SECS"
  http_get "/api/projects/$PROJECT_SLUG/runtime?events=80" >"$WORKDIR/runtime-after-restart.json" ||
    fail "runtime fetch after restart failed"
  assert_jq "$WORKDIR/runtime-after-restart.json" "$(event_filter worker.recovered)" "restart records recovered workers"
  assert_no_blocked_runtime "$WORKDIR/runtime-after-restart.json" "runtime after restart has no blocked event"
  assert_jq "$WORKDIR/runtime-after-restart.json" '(.operator_status.running_in_process == true) or (.runtime.status == "done")' "runtime is running in-process after restart"

  CAPTAIN_HOME="$HOME_DIR" "$CAPTAIN_BIN" project replay "$PROJECT_SLUG" --events 80 --workers 8 --json \
    >"$WORKDIR/cli-replay.json" || fail "captain project replay failed"
  assert_jq "$WORKDIR/cli-replay.json" '.events.items | map(.kind) | index("orchestrator.resume_after_restart") != null' "CLI replay sees resume event"
  assert_no_blocked_runtime "$WORKDIR/cli-replay.json" "CLI replay has no blocked runtime"
  assert_jq "$WORKDIR/cli-replay.json" '(.state // "") != "blocked"' "CLI replay state is not blocked"

  CAPTAIN_HOME="$HOME_DIR" "$CAPTAIN_BIN" project timeline "$PROJECT_SLUG" --limit 50 --json \
    >"$WORKDIR/cli-timeline.json" || fail "captain project timeline failed"
  assert_jq "$WORKDIR/cli-timeline.json" '.events | map(.kind) | index("orchestrator.resume_after_restart") != null' "CLI timeline sees resume event"
  assert_no_blocked_runtime "$WORKDIR/cli-timeline.json" "CLI timeline has no blocked runtime"
  capture_timeline_follow

  assert_jq "$WORKDIR/runtime-after-restart.json" 'tostring | contains("\"data\":{\"secret\"") | not' "runtime response omits obvious raw secret payloads"

  printf '\nProject runtime restart smoke passed: %s checks. Artifacts: %s\n' "$PASS" "$WORKDIR"
}

trap cleanup EXIT
run_smoke
