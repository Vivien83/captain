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
case "$WORKDIR" in
  /*) ;;
  *) WORKDIR="$ROOT_DIR/$WORKDIR" ;;
esac
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
SESSION_LABEL="Durability $STAMP"
TOOL_RUN_ID="toolrun-power-loss-$STAMP"
TOOL_RUN_DIGEST="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
TOOL_RUN_SECRET="synthetic-power-loss-secret-never-retain"
TOOL_RUN_EVIDENCE="useful partial evidence retained after abrupt stop"
PID=""
GENERATION=0
PASS=0
AGENT_ID=""
SESSION_ID=""
AUDIT_TIP=""
AUDIT_EPOCH=""
AUDIT_ENTRIES=0
API_KEY=""

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
  API_KEY="$(openssl rand -hex 32)" || fail "isolated API key generation failed"
  [ "${#API_KEY}" -eq 64 ] || fail "isolated API key has an invalid length"
  (umask 077; printf 'CAPTAIN_DAEMON_API_KEY=%s\n' "$API_KEY" >"$HOME_DIR/secrets.env") ||
    fail "isolated API key write failed"
  chmod 600 "$HOME_DIR/secrets.env" || fail "isolated API key permissions failed"
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
  curl -sS --max-time "$TIMEOUT" \
    -H "Accept: application/json" \
    -H "Authorization: Bearer $API_KEY" \
    "$BASE$1"
}

http_json() {
  local method="$1"
  local path="$2"
  local body="$3"
  printf '%s' "$body" |
    curl -sS --max-time "$TIMEOUT" -X "$method" \
      -H "Accept: application/json" \
      -H "Authorization: Bearer $API_KEY" \
      -H "Content-Type: application/json" --data-binary @- "$BASE$path"
}

assert_json() {
  local file="$1"
  local filter="$2"
  local label="$3"
  jq -e "$filter" "$file" >/dev/null 2>&1 || fail "$label"
  pass "$label"
}

seed_inflight_tool_run() {
  local started_at_unix_ms
  started_at_unix_ms="$(($(date +%s) * 1000))"
  mkdir -p "$HOME_DIR/data/tool-runs" || fail "tool-run output directory create failed"
  chmod 700 "$HOME_DIR/data/tool-runs" || fail "tool-run output directory permissions failed"
  (umask 077; printf 'password=%s\n%s\n' \
    "$TOOL_RUN_SECRET" "$TOOL_RUN_EVIDENCE" >"$HOME_DIR/data/tool-runs/$TOOL_RUN_ID.part") ||
    fail "in-flight tool-run capture write failed"

  sqlite3 "$HOME_DIR/data/captain.db" \
    "PRAGMA busy_timeout=5000;
     INSERT INTO detached_tool_runs
       (run_id, tool_name, status, caller_agent_id, origin_tool_use_id,
        started_at, finished_at, is_error, result, result_truncated,
        detached, input_sha256)
     VALUES
       ('$TOOL_RUN_ID', 'shell_exec', 'running', '$AGENT_ID',
        'durability-smoke', $started_at_unix_ms, NULL, NULL, NULL, 0, 1,
        '$TOOL_RUN_DIGEST');" >/dev/null || fail "in-flight tool-run ledger seed failed"

  [ "$(sqlite3 "$HOME_DIR/data/captain.db" \
    "SELECT count(*) FROM detached_tool_runs WHERE run_id = '$TOOL_RUN_ID' AND status = 'running';")" = "1" ] ||
    fail "in-flight tool-run checkpoint is not durable"
  pass "in-flight tool run and partial evidence committed before crash"
}

run_smoke() {
  require_cmd curl
  require_cmd jq
  require_cmd openssl
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

  http_get "/api/agents" >"$WORKDIR/agents-before.json" ||
    fail "agent inventory read failed"
  assert_json "$WORKDIR/agents-before.json" \
    'type == "array" and (map(select(.name == "captain" and .state == "Running")) | length == 1)' \
    "captain agent available before crash"
  AGENT_ID="$(jq -r 'map(select(.name == "captain"))[0].id // empty' "$WORKDIR/agents-before.json")"
  [ -n "$AGENT_ID" ] || fail "captain agent id missing"

  http_json POST "/api/agents/$AGENT_ID/sessions" \
    "$(jq -nc --arg label "$SESSION_LABEL" '{label:$label,activate:false}')" \
    >"$WORKDIR/session-create.json" || fail "detached session create request failed"
  assert_json "$WORKDIR/session-create.json" \
    '.active == false and .session_id != null and .label == "'"$SESSION_LABEL"'"' \
    "detached cross-surface session committed"
  SESSION_ID="$(jq -r '.session_id // empty' "$WORKDIR/session-create.json")"
  printf '%s' "$SESSION_ID" | grep -Eq '^[0-9a-fA-F-]{36}$' ||
    fail "detached session id is invalid"

  seed_inflight_tool_run

  http_get "/api/audit/verify" >"$WORKDIR/audit-before.json" ||
    fail "pre-crash audit verification failed"
  assert_json "$WORKDIR/audit-before.json" \
    '.valid == true and .status == "healthy" and .entries >= 1 and .active_epoch_valid == true' \
    "audit chain committed before crash"
  AUDIT_TIP="$(jq -r '.tip_hash // empty' "$WORKDIR/audit-before.json")"
  AUDIT_EPOCH="$(jq -r '.active_epoch // empty' "$WORKDIR/audit-before.json")"
  AUDIT_ENTRIES="$(jq -r '.entries // 0' "$WORKDIR/audit-before.json")"
  [ -n "$AUDIT_TIP" ] && [ -n "$AUDIT_EPOCH" ] || fail "audit checkpoint is incomplete"

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

  http_get "/api/sessions/$SESSION_ID" >"$WORKDIR/session-after.json" ||
    fail "session post-crash read failed"
  assert_json "$WORKDIR/session-after.json" \
    '.session_id == "'"$SESSION_ID"'" and .label == "'"$SESSION_LABEL"'"' \
    "detached session survives SIGKILL"

  http_json POST "/api/agents/$AGENT_ID/sessions/$SESSION_ID/switch" '{}' \
    >"$WORKDIR/session-switch.json" || fail "session restore request failed"
  assert_json "$WORKDIR/session-switch.json" '.status == "ok"' \
    "recovered session can be activated"
  http_get "/api/agents/$AGENT_ID/session" >"$WORKDIR/session-active.json" ||
    fail "active session read failed"
  assert_json "$WORKDIR/session-active.json" \
    '.session_id == "'"$SESSION_ID"'" and .label == "'"$SESSION_LABEL"'"' \
    "activated session is shared by the daemon"

  http_get "/api/tool-runs/$TOOL_RUN_ID" >"$WORKDIR/tool-run-after.json" ||
    fail "interrupted tool-run read failed"
  assert_json "$WORKDIR/tool-run-after.json" \
    '.run.run_id == "'"$TOOL_RUN_ID"'" and .run.status == "interrupted" and .run.cancellable == false and .run.output_available == true and .run.output_redacted == true and .run.input_sha256 == "'"$TOOL_RUN_DIGEST"'" and (.run | has("result") | not) and (.run | has("input") | not) and (.run | has("file_name") | not)' \
    "in-flight tool run becomes selectively projected interrupted evidence"

  http_get "/api/tool-runs/$TOOL_RUN_ID/tail?max_lines=20" \
    >"$WORKDIR/tool-run-tail.json" || fail "interrupted tool-run tail read failed"
  assert_json "$WORKDIR/tool-run-tail.json" \
    '.tail.run_id == "'"$TOOL_RUN_ID"'" and .tail.status == "interrupted" and .tail.sanitized == true and .tail.content_withheld == false and (.tail.content | contains("password=[REDACTED]")) and (.tail.content | contains("'"$TOOL_RUN_EVIDENCE"'"))' \
    "partial tool-run evidence is recovered and redacted after SIGKILL"
  if grep -F "$TOOL_RUN_SECRET" "$WORKDIR/tool-run-after.json" \
    "$WORKDIR/tool-run-tail.json" >/dev/null 2>&1; then
    fail "synthetic tool-run secret escaped the operator projection"
  fi
  pass "tool-run operator projection contains no synthetic secret"
  [ -f "$HOME_DIR/data/tool-runs/$TOOL_RUN_ID.log" ] &&
    [ ! -e "$HOME_DIR/data/tool-runs/$TOOL_RUN_ID.part" ] ||
    fail "interrupted tool-run capture was not finalized atomically"
  pass "interrupted tool-run capture is finalized without stale partial file"

  http_json POST "/api/tool-runs/$TOOL_RUN_ID/cancel" '{}' \
    >"$WORKDIR/tool-run-cancel.json" || fail "interrupted tool-run cancellation probe failed"
  assert_json "$WORKDIR/tool-run-cancel.json" '.error == "tool_run_not_active"' \
    "interrupted tool run cannot be ambiguously cancelled"
  http_get "/api/tool-runs?status=interrupted&limit=200" \
    >"$WORKDIR/tool-runs-interrupted.json" || fail "interrupted tool-run inventory failed"
  assert_json "$WORKDIR/tool-runs-interrupted.json" \
    '[.items[].run_id] | index("'"$TOOL_RUN_ID"'") != null' \
    "interrupted tool run remains discoverable after restart"

  http_get "/api/audit/verify" >"$WORKDIR/audit-after.json" ||
    fail "post-crash audit verification failed"
  assert_json "$WORKDIR/audit-after.json" \
    '.valid == true and .status == "healthy" and .active_epoch_valid == true and .active_epoch == '"$AUDIT_EPOCH"' and .entries >= '"$AUDIT_ENTRIES" \
    "audit chain remains valid in the same epoch"
  http_get "/api/audit/recent?n=1000" >"$WORKDIR/audit-recent.json" ||
    fail "post-crash audit history read failed"
  assert_json "$WORKDIR/audit-recent.json" \
    '(.tip_hash == "'"$AUDIT_TIP"'") or ([.entries[].hash] | index("'"$AUDIT_TIP"'") != null)' \
    "pre-crash audit tip remains in the recovered chain"

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
