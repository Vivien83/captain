#!/usr/bin/env bash
# Reproducible full-daemon durability smoke.
#
# Starts Captain in an isolated home, commits memory/project/config state,
# sends SIGKILL (no graceful shutdown), restarts the same home, and verifies
# every committed value plus SQLite integrity.

set -u

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STAMP="$(date +%Y%m%d-%H%M%S)-$$"
WORKDIR="${CAPTAIN_DURABILITY_SMOKE_WORKDIR:-$ROOT_DIR/target/persistence-power-loss-smoke/$STAMP}"
HOME_DIR="$WORKDIR/home"
CONFIG="$HOME_DIR/config.toml"
PORT="${CAPTAIN_DURABILITY_SMOKE_PORT:-50461}"
BASE="http://127.0.0.1:$PORT"
READY_TIMEOUT="${CAPTAIN_DURABILITY_SMOKE_READY_TIMEOUT:-45}"
BOOTSTRAP_READY_TIMEOUT="${CAPTAIN_DURABILITY_SMOKE_BOOTSTRAP_TIMEOUT:-300}"
TIMEOUT="${CAPTAIN_DURABILITY_SMOKE_TIMEOUT:-30}"
CAPTAIN_BIN="${CAPTAIN_DURABILITY_SMOKE_BIN:-}"
MARKER="power-loss-$STAMP"
PROJECT_SLUG="durability-$STAMP"
PID=""
GENERATION=0
PASS=0

note() { printf '   %s\n' "$*"; }
pass() {
  printf '   ok %s\n' "$1"
  PASS=$((PASS + 1))
}
fail() {
  printf '   FAIL %s\n' "$1" >&2
  if [ -f "$WORKDIR/daemon-$GENERATION.log" ]; then
    printf '\n--- daemon log tail ---\n' >&2
    tail -80 "$WORKDIR/daemon-$GENERATION.log" >&2 || true
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
    wait "$PID" >/dev/null 2>&1 || true
  fi
  PID=""
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

resolve_captain_bin() {
  if [ -n "$CAPTAIN_BIN" ]; then
    [ -x "$CAPTAIN_BIN" ] || fail "Captain binary is not executable: $CAPTAIN_BIN"
    return
  fi
  note "building the current worktree for the durability smoke"
  (cd "$ROOT_DIR" && cargo build -p captain-cli) || fail "cargo build -p captain-cli failed"
  CAPTAIN_BIN="$ROOT_DIR/target/debug/captain"
  [ -x "$CAPTAIN_BIN" ] || fail "current Captain build is missing"
}

write_config() {
  mkdir -p "$HOME_DIR/data" "$HOME_DIR/agents"
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
model = "gpt-5.6-sol"
api_key_env = ""

[assistant]
onboarding_completed = true

[approval]
require_approval = []
EOF
}

wait_for_health() {
  local limit="$1"
  local elapsed=0
  local body
  while [ "$elapsed" -le "$limit" ]; do
    body="$(curl -sS --connect-timeout 1 --max-time 2 "$BASE/api/health" 2>/dev/null || true)"
    if printf '%s' "$body" | jq -e '.status == "ok"' >/dev/null 2>&1; then
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
  GENERATION=$((GENERATION + 1))
  CAPTAIN_HOME="$HOME_DIR" "$CAPTAIN_BIN" start --config "$CONFIG" --yolo \
    >"$WORKDIR/daemon-$GENERATION.log" 2>&1 &
  PID="$!"
  local ready_limit="$READY_TIMEOUT"
  if [ "$GENERATION" -eq 1 ]; then
    ready_limit="$BOOTSTRAP_READY_TIMEOUT"
  fi
  wait_for_health "$ready_limit" ||
    fail "daemon generation $GENERATION did not become healthy within ${ready_limit}s"
  pass "daemon generation $GENERATION healthy"
}

sigkill_daemon() {
  [ -n "$PID" ] || fail "daemon pid missing"
  kill -KILL "$PID" >/dev/null 2>&1 || fail "SIGKILL failed"
  wait "$PID" >/dev/null 2>&1 || true
  if kill -0 "$PID" >/dev/null 2>&1; then
    fail "daemon remained alive after SIGKILL"
  fi
  PID=""
  pass "daemon stopped by SIGKILL without graceful shutdown"
}

http_get() {
  curl -sS --max-time "$TIMEOUT" "$BASE$1"
}

http_json() {
  local method="$1"
  local path="$2"
  local body="$3"
  printf '%s' "$body" |
    curl -sS --max-time "$TIMEOUT" -X "$method" \
      -H "Content-Type: application/json" --data-binary @- "$BASE$path"
}

assert_json() {
  local file="$1"
  local filter="$2"
  local label="$3"
  jq -e "$filter" "$file" >/dev/null 2>&1 || fail "$label"
  pass "$label"
}

run_smoke() {
  require_cmd curl
  require_cmd jq
  require_cmd sqlite3
  require_cmd tail
  resolve_captain_bin
  mkdir -p "$WORKDIR"
  write_config

  note "workdir=$WORKDIR"
  note "captain_bin=$CAPTAIN_BIN"
  start_daemon

  http_json PUT "/api/memory/agents/captain/kv/power_loss_marker" \
    "$(jq -nc --arg marker "$MARKER" '{value:$marker}')" \
    >"$WORKDIR/memory-set.json" || fail "memory write request failed"
  assert_json "$WORKDIR/memory-set.json" '.status == "stored"' "memory commit acknowledged"

  http_json POST "/api/projects" \
    "$(jq -nc --arg slug "$PROJECT_SLUG" --arg marker "$MARKER" '{name:"Durability Smoke",slug:$slug,goal:("Retain " + $marker + " after SIGKILL") }')" \
    >"$WORKDIR/project-create.json" || fail "project create request failed"
  assert_json "$WORKDIR/project-create.json" '.slug == "'"$PROJECT_SLUG"'"' "project commit acknowledged"

  http_json POST "/api/config/set" '{"path":"language","value":"fr"}' \
    >"$WORKDIR/config-set.json" || fail "config write request failed"
  assert_json "$WORKDIR/config-set.json" '.status != "error"' "config commit acknowledged"

  http_get "/api/memory/agents/captain/kv/power_loss_marker" \
    >"$WORKDIR/memory-before.json" || fail "memory pre-crash read failed"
  assert_json "$WORKDIR/memory-before.json" '.value == "'"$MARKER"'"' "memory readable before crash"

  sigkill_daemon
  start_daemon

  http_get "/api/memory/agents/captain/kv/power_loss_marker" \
    >"$WORKDIR/memory-after.json" || fail "memory post-crash read failed"
  assert_json "$WORKDIR/memory-after.json" '.value == "'"$MARKER"'"' "memory survives SIGKILL"

  http_get "/api/projects/$PROJECT_SLUG" >"$WORKDIR/project-after.json" ||
    fail "project post-crash read failed"
  assert_json "$WORKDIR/project-after.json" '.slug == "'"$PROJECT_SLUG"'"' "project survives SIGKILL"

  grep -F 'language = "fr"' "$CONFIG" >/dev/null 2>&1 ||
    fail "durable config value missing after restart"
  pass "config survives SIGKILL and remains parseable"

  integrity="$(sqlite3 "$HOME_DIR/data/captain.db" 'PRAGMA integrity_check;' 2>/dev/null || true)"
  [ "$integrity" = "ok" ] || fail "SQLite integrity_check returned: ${integrity:-empty}"
  pass "SQLite integrity_check is ok after abrupt restart"

  http_get "/api/status" >"$WORKDIR/status-after.json" || fail "status read failed"
  assert_json "$WORKDIR/status-after.json" '.status == "ok" or .runtime.status == "ok" or .version != null' "restarted daemon remains operational"

  printf '\nPersistence power-loss smoke passed: %s checks. Artifacts: %s\n' "$PASS" "$WORKDIR"
}

trap cleanup EXIT INT TERM
run_smoke
