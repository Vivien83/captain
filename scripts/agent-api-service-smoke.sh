#!/usr/bin/env bash
# Reproducible e2e smoke for Captain's per-agent API surface.
#
# This starts an isolated Captain daemon, creates a deterministic Python agent,
# rotates ingress auth, configures a signed callback, calls the external ingress
# hook, and verifies callback signature + agent API audit trail.

set -u

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STAMP="$(date +%Y%m%d-%H%M%S)"
WORKDIR="${CAPTAIN_AGENT_API_SMOKE_WORKDIR:-$ROOT_DIR/target/agent-api-service-smoke/$STAMP}"
CAPTAIN_BIN="${CAPTAIN_AGENT_API_SMOKE_BIN:-}"
CAPTAIN_PORT="${CAPTAIN_AGENT_API_SMOKE_PORT:-50431}"
CALLBACK_PORT="${CAPTAIN_AGENT_API_SMOKE_CALLBACK_PORT:-50432}"
PROVIDER="${CAPTAIN_AGENT_API_SMOKE_PROVIDER:-codex}"
MODEL="${CAPTAIN_AGENT_API_SMOKE_MODEL:-gpt-5.5}"
API_KEY_ENV="${CAPTAIN_AGENT_API_SMOKE_API_KEY_ENV:-}"
TIMEOUT="${CAPTAIN_AGENT_API_SMOKE_TIMEOUT:-60}"
READY_TIMEOUT="${CAPTAIN_AGENT_API_SMOKE_READY_TIMEOUT:-35}"
KEEP_DAEMONS=0
REQUEST_ID="v6-agent-api-$STAMP-$$"
CALLBACK_SECRET="cap_v6_callback_secret_${STAMP}_$$"
BASE="http://127.0.0.1:$CAPTAIN_PORT"
CALLBACK_URL="http://127.0.0.1:$CALLBACK_PORT/captain-agent-api"
ACTIVE_PIDS=""
TMP_SECRET_FILES=""

usage() {
  cat <<'USAGE'
Usage: scripts/agent-api-service-smoke.sh [options]

Options:
  --captain-bin path       Captain V2 binary.
  --workdir path           Artifact directory.
  --captain-port port      Isolated Captain API port.
  --callback-port port     Local external-service callback port.
  --provider name          default_model.provider for isolated config.
  --model name             default_model.model for isolated config.
  --api-key-env name       default_model.api_key_env for isolated config.
  --timeout seconds        Per-request timeout.
  --ready-timeout seconds  Daemon health timeout.
  --keep-daemons           Leave processes running for manual inspection.
  -h, --help               Show this help.

Environment mirrors these flags with CAPTAIN_AGENT_API_SMOKE_* variables.

The smoke sets CAPTAIN_AGENT_API_ALLOW_LOCAL_CALLBACKS=1 only for the isolated
daemon process so a local callback can prove signed egress without weakening
production webhook defaults.
USAGE
}

note() { printf '   %s\n' "$*"; }
pass() { printf '   ok %s\n' "$*"; }

fail() {
  printf '   FAIL %s\n' "$*" >&2
  cleanup
  exit 1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --captain-bin)
      [ $# -ge 2 ] && [ -n "${2:-}" ] || fail "missing value for --captain-bin"
      CAPTAIN_BIN="$2"
      shift 2
      ;;
    --workdir)
      [ $# -ge 2 ] && [ -n "${2:-}" ] || fail "missing value for --workdir"
      WORKDIR="$2"
      shift 2
      ;;
    --captain-port)
      [ $# -ge 2 ] && [ -n "${2:-}" ] || fail "missing value for --captain-port"
      CAPTAIN_PORT="$2"
      BASE="http://127.0.0.1:$CAPTAIN_PORT"
      shift 2
      ;;
    --callback-port)
      [ $# -ge 2 ] && [ -n "${2:-}" ] || fail "missing value for --callback-port"
      CALLBACK_PORT="$2"
      CALLBACK_URL="http://127.0.0.1:$CALLBACK_PORT/captain-agent-api"
      shift 2
      ;;
    --provider)
      [ $# -ge 2 ] && [ -n "${2:-}" ] || fail "missing value for --provider"
      PROVIDER="$2"
      shift 2
      ;;
    --model)
      [ $# -ge 2 ] && [ -n "${2:-}" ] || fail "missing value for --model"
      MODEL="$2"
      shift 2
      ;;
    --api-key-env)
      [ $# -ge 2 ] && [ -n "${2:-}" ] || fail "missing value for --api-key-env"
      API_KEY_ENV="$2"
      shift 2
      ;;
    --timeout)
      [ $# -ge 2 ] && [ -n "${2:-}" ] || fail "missing value for --timeout"
      TIMEOUT="$2"
      shift 2
      ;;
    --ready-timeout)
      [ $# -ge 2 ] && [ -n "${2:-}" ] || fail "missing value for --ready-timeout"
      READY_TIMEOUT="$2"
      shift 2
      ;;
    --keep-daemons)
      KEEP_DAEMONS=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

cleanup() {
  for file in $TMP_SECRET_FILES; do
    rm -f "$file" >/dev/null 2>&1 || true
  done
  if [ "$KEEP_DAEMONS" = "1" ]; then
    return
  fi
  for pid in $ACTIVE_PIDS; do
    if kill -0 "$pid" >/dev/null 2>&1; then
      kill "$pid" >/dev/null 2>&1 || true
      for _ in 1 2 3 4 5; do
        kill -0 "$pid" >/dev/null 2>&1 || break
        sleep 0.2
      done
      if kill -0 "$pid" >/dev/null 2>&1; then
        kill -KILL "$pid" >/dev/null 2>&1 || true
      fi
    fi
  done
}

trap cleanup EXIT INT TERM

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

resolve_captain_bin() {
  if [ -n "$CAPTAIN_BIN" ]; then
    [ -x "$CAPTAIN_BIN" ] || fail "Captain binary is not executable: $CAPTAIN_BIN"
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
  CAPTAIN_BIN="$(command -v captain || true)"
  [ -n "$CAPTAIN_BIN" ] && [ -x "$CAPTAIN_BIN" ] || fail "no Captain binary found"
}

redact_file() {
  local file="$1"
  [ -f "$file" ] || return 0
  command -v perl >/dev/null 2>&1 || return 0
  perl -0pi -e 's/(Authorization:\s*Bearer\s+)[A-Za-z0-9._:\-+\/=]+/${1}[REDACTED]/g; s/("?(?:token|callback_secret|password)"?\s*[:=]\s*")([^"]+)(")/${1}[REDACTED]${3}/gi; s/\bcap_at_[A-Za-z0-9_]+\b/[REDACTED]/g; s/\bcap_v6_callback_secret_[A-Za-z0-9_-]+\b/[REDACTED]/g' "$file" 2>/dev/null || true
}

json_bool() {
  if [ "$1" = "1" ] || [ "$1" = "true" ]; then
    printf 'true'
  else
    printf 'false'
  fi
}

write_config() {
  local home_dir="$1"
  local config="$2"
  mkdir -p "$home_dir" "$home_dir/data" "$home_dir/agents"
  cat >"$config" <<EOF
home_dir = "$home_dir"
data_dir = "$home_dir/data"
log_level = "info"
api_listen = "127.0.0.1:$CAPTAIN_PORT"
network_enabled = false
api_key = ""
language = "en"

[default_model]
provider = "$PROVIDER"
model = "$MODEL"
api_key_env = "$API_KEY_ENV"

[assistant]
onboarding_completed = true

[approval]
require_approval = []
EOF
}

write_python_agent() {
  local path="$1"
  cat >"$path" <<'PY'
#!/usr/bin/env python3
import json
import sys

line = sys.stdin.readline()
payload = json.loads(line or "{}")
message = payload.get("message", "")
agent_id = payload.get("agent_id", "")
text = "AGENT-AS-SERVICE-OK agent=%s message=%s" % (agent_id[:8], message[:120])
print(json.dumps({"type": "response", "text": text}, ensure_ascii=True), flush=True)
PY
  chmod +x "$path"
}

write_callback_server() {
  local path="$1"
  cat >"$path" <<'PY'
#!/usr/bin/env python3
import argparse
import hashlib
import hmac
import http.server
import json
import pathlib
import time

parser = argparse.ArgumentParser()
parser.add_argument("--port", type=int, required=True)
parser.add_argument("--secret", required=True)
parser.add_argument("--out", required=True)
parser.add_argument("--ready", required=True)
args = parser.parse_args()

out_path = pathlib.Path(args.out)
ready_path = pathlib.Path(args.ready)
out_path.parent.mkdir(parents=True, exist_ok=True)

class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("content-length", "0"))
        body = self.rfile.read(length)
        expected = "sha256=" + hmac.new(
            args.secret.encode("utf-8"), body, hashlib.sha256
        ).hexdigest()
        provided = self.headers.get("x-captain-signature", "")
        try:
            payload = json.loads(body.decode("utf-8"))
        except Exception as exc:
            payload = {"decode_error": str(exc)}
        row = {
            "received_at": time.time(),
            "path": self.path,
            "event": self.headers.get("x-captain-event"),
            "agent_id": self.headers.get("x-captain-agent-id"),
            "signature_header": provided,
            "signature_valid": hmac.compare_digest(provided, expected),
            "body": payload,
        }
        with out_path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(row, ensure_ascii=True, sort_keys=True) + "\n")
        self.send_response(204)
        self.end_headers()

    def log_message(self, fmt, *values):
        return

server = http.server.ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
ready_path.write_text("ready\n", encoding="utf-8")
server.serve_forever()
PY
}

wait_for_health() {
  local output="$1"
  local elapsed=0
  local body
  while [ "$elapsed" -le "$READY_TIMEOUT" ]; do
    body="$(curl -sS --connect-timeout 1 --max-time 2 "$BASE/api/health" 2>/dev/null || true)"
    if printf '%s' "$body" | jq -e '.status == "ok"' >/dev/null 2>&1; then
      printf '%s\n' "$body" >"$output"
      redact_file "$output"
      return 0
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  printf '%s\n' "$body" >"$output"
  redact_file "$output"
  return 1
}

http_get_json() {
  local url="$1"
  local out="$2"
  curl -sS --max-time "$TIMEOUT" -o "$out" -w '%{http_code}' "$url"
}

http_post_json() {
  local url="$1"
  local payload="$2"
  local out="$3"
  shift 3
  curl -sS --max-time "$TIMEOUT" -o "$out" -w '%{http_code}' \
    -X POST "$url" \
    -H 'Content-Type: application/json' \
    "$@" \
    --data-binary @"$payload"
}

assert_status() {
  local status="$1"
  local expected="$2"
  local label="$3"
  local body="${4:-}"
  if [ "$status" = "$expected" ]; then
    pass "$label"
    return
  fi
  if [ -n "$body" ] && [ -f "$body" ]; then
    redact_file "$body"
    note "response body: $(tr '\n' ' ' <"$body" | cut -c 1-500)"
  fi
  fail "$label returned HTTP $status, expected $expected"
}

assert_jq() {
  local file="$1"
  local filter="$2"
  local label="$3"
  if jq -e "$filter" "$file" >/dev/null 2>&1; then
    pass "$label"
  else
    note "failed jq filter: $filter"
    fail "$label"
  fi
}

assert_agent_api_audit() {
  local file="$1"
  local request_id="$2"
  local label="$3"
  if jq -e --arg request_id "$request_id" '
    [.items[] | select(.detail.request_id == $request_id)] as $items
    | any($items[]; .detail.direction == "ingress" and .detail.phase == "accepted")
      and any($items[]; .detail.direction == "ingress" and .detail.phase == "completed")
      and any($items[]; .detail.direction == "egress" and .detail.phase == "callback" and (.outcome | contains("delivered")))
  ' "$file" >/dev/null 2>&1; then
    pass "$label"
  else
    fail "$label"
  fi
}

wait_for_callbacks() {
  local expected="$1"
  local file="$2"
  local elapsed=0
  local count=0
  while [ "$elapsed" -le "$READY_TIMEOUT" ]; do
    if [ -f "$file" ]; then
      count="$(wc -l <"$file" | tr -d ' ')"
      if [ "$count" -ge "$expected" ]; then
        return 0
      fi
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  return 1
}

write_secret_payload() {
  local file="$1"
  local callback_url="$2"
  python3 - "$file" "$callback_url" "$CALLBACK_SECRET" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
payload = {
    "callback_url": sys.argv[2],
    "callback_secret": sys.argv[3],
    "generate_secret": False,
}
path.write_text(json.dumps(payload), encoding="utf-8")
PY
  TMP_SECRET_FILES="$TMP_SECRET_FILES $file"
}

write_report() {
  local report="$1"
  local summary="$2"
  python3 - "$report" "$summary" <<'PY'
import json
import pathlib
import sys

report = pathlib.Path(sys.argv[1])
summary = json.loads(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"))
lines = [
    "# Agent API Service Smoke",
    "",
    f"- status: {summary['status']}",
    f"- agent_id: {summary['agent_id']}",
    f"- request_id: {summary['request_id']}",
    f"- callback_events: {summary['callback_events']}",
    f"- signed_callbacks_valid: {summary['signed_callbacks_valid']}",
    f"- audit_verified: {summary['audit_verified']}",
    f"- queue_empty: {summary['queue_empty']}",
    "",
    "Artifacts:",
    "- spawn.json",
    "- manifest.json",
    "- token-rotate.redacted.json",
    "- callback-config.json",
    "- callback-test.json",
    "- ingress-response.json",
    "- callback-events.jsonl",
    "- audit-events.json",
    "- egress-queue.json",
]
report.write_text("\n".join(lines) + "\n", encoding="utf-8")
PY
}

main() {
  case "$WORKDIR" in
    /*) ;;
    *) WORKDIR="$ROOT_DIR/$WORKDIR" ;;
  esac
  mkdir -p "$WORKDIR"
  require_cmd curl
  require_cmd jq
  require_cmd python3
  resolve_captain_bin

  local home_dir="$WORKDIR/captain-home"
  local config="$WORKDIR/config.toml"
  local callback_server="$WORKDIR/callback_server.py"
  local callback_events="$WORKDIR/callback-events.jsonl"
  local callback_ready="$WORKDIR/callback-ready"
  local daemon_log="$WORKDIR/captain-daemon.log"
  local callback_log="$WORKDIR/callback-server.log"
  local python_agent="$home_dir/agents/v6_service_agent.py"
  local manifest_file="$WORKDIR/agent-manifest.toml"
  local payload_file="$WORKDIR/spawn-payload.json"
  local token_raw="$WORKDIR/.token-rotate.raw.json"
  local callback_payload="$WORKDIR/.callback-config.raw.json"
  local status agent_id token status_code

  note "artifact directory: $WORKDIR"
  write_config "$home_dir" "$config"
  write_python_agent "$python_agent"
  write_callback_server "$callback_server"

  if curl -sS --connect-timeout 1 --max-time 2 "$BASE/api/health" >/dev/null 2>&1; then
    fail "Captain port $CAPTAIN_PORT already responds; choose --captain-port"
  fi
  if curl -sS --connect-timeout 1 --max-time 2 "$CALLBACK_URL" >/dev/null 2>&1; then
    fail "callback port $CALLBACK_PORT already responds; choose --callback-port"
  fi

  python3 "$callback_server" \
    --port "$CALLBACK_PORT" \
    --secret "$CALLBACK_SECRET" \
    --out "$callback_events" \
    --ready "$callback_ready" >"$callback_log" 2>&1 &
  ACTIVE_PIDS="$ACTIVE_PIDS $!"
  for _ in $(seq 1 "$READY_TIMEOUT"); do
    [ -f "$callback_ready" ] && break
    sleep 1
  done
  [ -f "$callback_ready" ] || fail "callback server did not become ready"
  pass "external callback service is listening"

  : >"$daemon_log"
  CAPTAIN_HOME="$home_dir" \
  CAPTAIN_AGENT_API_ALLOW_LOCAL_CALLBACKS=1 \
    "$CAPTAIN_BIN" --config "$config" start --yolo >>"$daemon_log" 2>&1 &
  ACTIVE_PIDS="$ACTIVE_PIDS $!"
  wait_for_health "$WORKDIR/health.json" || fail "Captain daemon did not become healthy"
  pass "isolated Captain daemon is healthy"

  cat >"$manifest_file" <<EOF
name = "v6-agent-as-service"
version = "0.1.0"
description = "Deterministic service agent for Captain V6 agent API smoke"
author = "captain-v6-smoke"
module = "python:agents/v6_service_agent.py"
generate_identity_files = false

[model]
provider = "static"
model = "python-agent"
max_tokens = 256
temperature = 0.0
system_prompt = "Return deterministic smoke responses."

[resources]
max_cpu_time_ms = 10000
max_memory_bytes = 134217728

[capabilities]
network = []
tools = []
memory_read = []
memory_write = []
agent_spawn = false
agent_message = []
shell = []
ofp_discover = false
ofp_connect = []

[metadata]
smoke = "agent-api-service"
EOF
  jq -Rs '{manifest_toml: .}' "$manifest_file" >"$payload_file"
  status="$(http_post_json "$BASE/api/agents" "$payload_file" "$WORKDIR/spawn.json")"
  assert_status "$status" "201" "agent creation returns dedicated API surface" "$WORKDIR/spawn.json"
  agent_id="$(jq -r '.agent_id // empty' "$WORKDIR/spawn.json")"
  [ -n "$agent_id" ] || fail "spawn response did not include agent_id"

  status="$(http_post_json "$BASE/api/agents/$agent_id/api/token/rotate" /dev/null "$token_raw")"
  assert_status "$status" "200" "ingress token rotation succeeds" "$token_raw"
  token="$(jq -r '.rotation.token // empty' "$token_raw")"
  [ "${#token}" -ge 32 ] || fail "rotated token is missing or too short"
  jq '.rotation.token = "[REDACTED]"' "$token_raw" >"$WORKDIR/token-rotate.redacted.json"
  rm -f "$token_raw"

  write_secret_payload "$callback_payload" "$CALLBACK_URL"
  status="$(http_post_json "$BASE/api/agents/$agent_id/api/egress/configure" "$callback_payload" "$WORKDIR/callback-config.json")"
  assert_status "$status" "200" "callback egress configuration accepts local smoke callback" "$WORKDIR/callback-config.json"
  redact_file "$WORKDIR/callback-config.json"
  rm -f "$callback_payload"

  status="$(http_get_json "$BASE/api/agents/$agent_id/api/manifest" "$WORKDIR/manifest.json")"
  assert_status "$status" "200" "manifest endpoint returns integration contract" "$WORKDIR/manifest.json"
  assert_jq "$WORKDIR/manifest.json" '.kind == "captain.agent_api.manifest" and .readiness.state == "ready" and .ingress.auth.scheme == "bearer" and .egress.signature.algorithm == "hmac-sha256"' "manifest is ready and describes signed ingress/egress"

  jq -n \
    --arg request_id "$REQUEST_ID-test" \
    --arg message "V6 callback diagnostic" \
    '{request_id:$request_id,message:$message,metadata:{source:"agent-api-service-smoke",kind:"test"}}' \
    >"$WORKDIR/callback-test-payload.json"
  status="$(http_post_json "$BASE/api/agents/$agent_id/api/egress/test" "$WORKDIR/callback-test-payload.json" "$WORKDIR/callback-test.json")"
  assert_status "$status" "200" "diagnostic callback is delivered" "$WORKDIR/callback-test.json"
  assert_jq "$WORKDIR/callback-test.json" '.status == "delivered" and .event == "agent_api.test" and .delivery.delivered == true' "diagnostic callback response proves delivery"

  jq -n \
    --arg request_id "$REQUEST_ID-main" \
    '{request_id:$request_id,message:"V6-INBOUND-CALL please answer deterministically",sender_id:"external-service:v6",sender_name:"V6 External Service",metadata:{source:"agent-api-service-smoke",correlation:"v6-main"}}' \
    >"$WORKDIR/ingress-payload.json"
  status="$(http_post_json "$BASE/hooks/agents/$agent_id/ingress" "$WORKDIR/ingress-payload.json" "$WORKDIR/ingress-response.json" -H "Authorization: Bearer $token")"
  assert_status "$status" "200" "external service ingress call completes" "$WORKDIR/ingress-response.json"
  assert_jq "$WORKDIR/ingress-response.json" '.status == "completed" and (.response | contains("AGENT-AS-SERVICE-OK")) and .egress.delivered == true and .metadata_received == true' "ingress response includes deterministic agent output and signed egress delivery"

  wait_for_callbacks 2 "$callback_events" || fail "expected two signed callbacks"
  pass "external service captured two callback outputs"
  assert_jq "$callback_events" 'select(.event == "agent_api.test" and .signature_valid == true)' "test callback signature is valid"
  assert_jq "$callback_events" 'select(.event == "agent_api.completed" and .signature_valid == true and .body.request_id != null and (.body.response | contains("AGENT-AS-SERVICE-OK")))' "completed callback signature and payload are valid"

  status="$(http_get_json "$BASE/api/agents/$agent_id/api/events?n=50" "$WORKDIR/audit-events.json")"
  assert_status "$status" "200" "agent API audit endpoint is readable" "$WORKDIR/audit-events.json"
  assert_agent_api_audit "$WORKDIR/audit-events.json" "$REQUEST_ID-main" "audit trail records ingress accepted/completed and delivered egress"

  status="$(http_get_json "$BASE/api/agents/$agent_id/api/egress" "$WORKDIR/egress-queue.json")"
  assert_status "$status" "200" "egress queue endpoint is readable" "$WORKDIR/egress-queue.json"
  assert_jq "$WORKDIR/egress-queue.json" '.pending == 0 and .dead_letters == 0' "egress queue has no pending or dead-lettered callbacks"

  local callback_count signed_ok audit_ok queue_empty
  callback_count="$(wc -l <"$callback_events" | tr -d ' ')"
  signed_ok="$(jq -s 'all(.[]; .signature_valid == true)' "$callback_events")"
  audit_ok="$(jq --arg request_id "$REQUEST_ID-main" '[.items[] | select(.detail.request_id == $request_id)] as $items | (any($items[]; .detail.phase == "accepted") and any($items[]; .detail.phase == "completed") and any($items[]; .detail.direction == "egress" and .detail.phase == "callback"))' "$WORKDIR/audit-events.json")"
  queue_empty="$(jq '.pending == 0 and .dead_letters == 0' "$WORKDIR/egress-queue.json")"
  jq -n \
    --arg status "passed" \
    --arg agent_id "$agent_id" \
    --arg request_id "$REQUEST_ID-main" \
    --argjson callback_events "$callback_count" \
    --argjson signed_callbacks_valid "$signed_ok" \
    --argjson audit_verified "$audit_ok" \
    --argjson queue_empty "$queue_empty" \
    '{
      status:$status,
      agent_id:$agent_id,
      request_id:$request_id,
      callback_events:$callback_events,
      signed_callbacks_valid:$signed_callbacks_valid,
      audit_verified:$audit_verified,
      queue_empty:$queue_empty
    }' >"$WORKDIR/summary.json"
  write_report "$WORKDIR/report.md" "$WORKDIR/summary.json"
  redact_file "$daemon_log"
  redact_file "$callback_log"

  printf '\nAgent API service smoke passed. Artifacts: %s\n' "$WORKDIR"
}

main "$@"
