#!/usr/bin/env bash
# Reproducible real user-flow smoke for Captain's active surfaces.
#
# This smoke targets the already-running daemon by default. It verifies the
# operator path a user actually touches: CLI/status, TUI, authenticated web
# terminal/chat, Projects, Learning, Capabilities/Status, and one live channel.

set -u

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASE="${CAPTAIN_API:-http://127.0.0.1:50051}"
WORKDIR="${CAPTAIN_USER_SMOKE_WORKDIR:-$ROOT_DIR/target/user-flow-smoke}"
TIMEOUT="${CAPTAIN_USER_SMOKE_TIMEOUT:-45}"
READY_TIMEOUT="${CAPTAIN_USER_SMOKE_READY_TIMEOUT:-20}"
CHANNEL="${CAPTAIN_USER_SMOKE_CHANNEL:-telegram}"
RUN_TUI="${CAPTAIN_USER_SMOKE_TUI:-1}"
ARCHIVE_PROJECT="${CAPTAIN_USER_SMOKE_ARCHIVE_PROJECT:-1}"
PROJECT_SLUG="${CAPTAIN_USER_SMOKE_PROJECT_SLUG:-user-smoke-$(date +%Y%m%d-%H%M%S)-$$}"
CAPTAIN_BIN="${CAPTAIN_BIN:-}"

PASS=0
FAIL=0
WARN=0
AUTH_HEADER_ARGS=()
PROJECT_ID=""
SESSION_TOKEN=""
SESSION_TOKEN_SOURCE=""
DAEMON_API_KEY=""
WEB_USERNAME=""
WEB_PASSWORD=""
TMP_FILES=()

while [ "$#" -gt 0 ]; do
  case "$1" in
  --api)
    if [ $# -lt 2 ] || [ -z "${2:-}" ]; then
      printf 'Missing value for --api\n' >&2
      exit 2
    fi
    BASE="$2"
    shift 2
    ;;
  --workdir)
    if [ $# -lt 2 ] || [ -z "${2:-}" ]; then
      printf 'Missing value for --workdir\n' >&2
      exit 2
    fi
    WORKDIR="$2"
    shift 2
    ;;
  --timeout)
    if [ $# -lt 2 ] || [ -z "${2:-}" ]; then
      printf 'Missing value for --timeout\n' >&2
      exit 2
    fi
    TIMEOUT="$2"
    shift 2
    ;;
  --ready-timeout)
    if [ $# -lt 2 ] || [ -z "${2:-}" ]; then
      printf 'Missing value for --ready-timeout\n' >&2
      exit 2
    fi
    READY_TIMEOUT="$2"
    shift 2
    ;;
  --channel)
    if [ $# -lt 2 ] || [ -z "${2:-}" ]; then
      printf 'Missing value for --channel\n' >&2
      exit 2
    fi
    CHANNEL="$2"
    shift 2
    ;;
  --skip-tui)
    RUN_TUI=0
    shift
    ;;
  --keep-project)
    ARCHIVE_PROJECT=0
    shift
    ;;
  -h|--help)
    cat <<'EOF'
Usage: scripts/user-flow-smoke.sh [--api URL] [--channel telegram|discord|none]

Environment:
  CAPTAIN_API                       API base URL (default http://127.0.0.1:50051)
  CAPTAIN_USER_SMOKE_WORKDIR        Output dir for redacted artifacts
  CAPTAIN_USER_SMOKE_TIMEOUT        curl/WS timeout seconds
  CAPTAIN_USER_SMOKE_READY_TIMEOUT  health readiness timeout seconds
  CAPTAIN_USER_SMOKE_CHANNEL        live channel to test (default telegram)
  CAPTAIN_USER_SMOKE_TUI=0          skip scripts/tui-smoke.sh
  CAPTAIN_USER_SMOKE_ARCHIVE_PROJECT=0 keep the created smoke project
  CAPTAIN_BIN                       captain binary override for CLI checks

The default channel check sends one real test message through the configured
channel. Use --channel none only for dry rehearsals; it does not close the P0
real-user smoke requirement.
EOF
    exit 0
    ;;
  *)
    printf 'Unknown argument: %s\n' "$1" >&2
    exit 2
    ;;
  esac
done

cleanup() {
  for file in "${TMP_FILES[@]:-}"; do
    rm -f "$file" >/dev/null 2>&1 || true
  done
}
trap cleanup EXIT

new_tmp() {
  local tmp
  tmp="$(mktemp "${TMPDIR:-/tmp}/captain-user-smoke.XXXXXX")" || exit 1
  TMP_FILES+=("$tmp")
  printf '%s' "$tmp"
}

ok_icon() { printf 'ok'; }
warn_icon() { printf 'warn'; }
ko_icon() { printf 'FAIL'; }

title() { printf '\n== %s\n' "$1"; }
note() { printf '   %s\n' "$*"; }

pass() {
  printf '   %s %s\n' "$(ok_icon)" "$1"
  PASS=$((PASS + 1))
}

warn() {
  printf '   %s %s\n' "$(warn_icon)" "$1"
  WARN=$((WARN + 1))
}

fail() {
  printf '   %s %s\n' "$(ko_icon)" "$1" >&2
  FAIL=$((FAIL + 1))
}

finish() {
  mkdir -p "$WORKDIR"
  jq -n \
    --arg api "$BASE" \
    --arg channel "$CHANNEL" \
    --arg project_slug "$PROJECT_SLUG" \
    --arg auth_source "${SESSION_TOKEN_SOURCE:-none}" \
    --argjson passed "$PASS" \
    --argjson failed "$FAIL" \
    --argjson warnings "$WARN" \
    '{
      api: $api,
      channel: $channel,
      project_slug: $project_slug,
      auth_source: $auth_source,
      passed: $passed,
      failed: $failed,
      warnings: $warnings
    }' >"$WORKDIR/summary.json" 2>/dev/null || true

  printf '\n========================================\n'
  if [ "$FAIL" -eq 0 ]; then
    printf 'User-flow smoke passed: %s checks' "$PASS"
    if [ "$WARN" -gt 0 ]; then
      printf ', %s warnings' "$WARN"
    fi
    printf '. Artifacts: %s\n' "$WORKDIR"
    exit 0
  fi
  printf 'User-flow smoke failed: %s failed, %s passed, %s warnings. Artifacts: %s\n' \
    "$FAIL" "$PASS" "$WARN" "$WORKDIR"
  exit 1
}

need_cmd() {
  if command -v "$1" >/dev/null 2>&1; then
    return 0
  fi
  fail "missing required command: $1"
  finish
}

resolve_captain_bin() {
  if [ -n "$CAPTAIN_BIN" ]; then
    [ -x "$CAPTAIN_BIN" ] || {
      fail "CAPTAIN_BIN is not executable: $CAPTAIN_BIN"
      finish
    }
    return
  fi
  if command -v captain >/dev/null 2>&1; then
    CAPTAIN_BIN="$(command -v captain)"
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
  fail "captain binary not found"
  finish
}

toml_top_string() {
  local key="$1"
  local path="$2"
  awk -v key="$key" '
    /^\[/ { in_section=1 }
    !in_section && $1 == key {
      sub(/^[^=]*=[[:space:]]*"/, "")
      sub(/".*$/, "")
      print
      exit
    }
  ' "$path"
}

toml_auth_string() {
  local key="$1"
  local path="$2"
  awk -v key="$key" '
    /^\[auth\]/ { in_auth=1; next }
    /^\[/ { in_auth=0 }
    in_auth && $1 == key {
      sub(/^[^=]*=[[:space:]]*"/, "")
      sub(/".*$/, "")
      print
      exit
    }
  ' "$path"
}

toml_auth_int() {
  local key="$1"
  local path="$2"
  awk -v key="$key" '
    /^\[auth\]/ { in_auth=1; next }
    /^\[/ { in_auth=0 }
    in_auth && $1 == key {
      sub(/^[^=]*=[[:space:]]*/, "")
      sub(/[[:space:]]*#.*/, "")
      print
      exit
    }
  ' "$path"
}

initial_password() {
  local path="${CAPTAIN_HOME:-$HOME/.captain}/initial-credentials.txt"
  if [ ! -f "$path" ]; then
    return 0
  fi
  awk '
    /^Password: / {
      value=$0
      sub(/^Password:[[:space:]]*/, "", value)
      if (value !~ /^provided during setup/) {
        print value
      }
      exit
    }
  ' "$path"
}

load_auth_material() {
  local home="${CAPTAIN_HOME:-$HOME/.captain}"
  local cfg="$home/config.toml"
  local ttl secret expiry payload signature

  if [ ! -f "$cfg" ]; then
    warn "config not found at $cfg; protected web/API checks may fail"
    return 0
  fi

  DAEMON_API_KEY="$(toml_top_string api_key "$cfg")"
  if [ -z "$DAEMON_API_KEY" ] && [ -f "$home/secrets.env" ]; then
    DAEMON_API_KEY="$(awk -F= '$1=="CAPTAIN_DAEMON_API_KEY" {print $2; exit}' "$home/secrets.env" | tr -d '\r')"
  fi
  if [ -z "$DAEMON_API_KEY" ]; then
    DAEMON_API_KEY="${CAPTAIN_DAEMON_API_KEY:-${CAPTAIN_API_KEY:-}}"
  fi

  WEB_USERNAME="$(toml_auth_string username "$cfg")"
  WEB_PASSWORD="$(initial_password)"
  if [ -n "$WEB_USERNAME" ] && [ -n "$WEB_PASSWORD" ]; then
    try_web_login
  fi

  if [ -n "$SESSION_TOKEN" ]; then
    AUTH_HEADER_ARGS=(-H "Authorization: Bearer $SESSION_TOKEN" -H "Cookie: captain_session=$SESSION_TOKEN")
    return 0
  fi

  local password_hash
  password_hash="$(toml_auth_string password_hash "$cfg")"
  ttl="$(toml_auth_int session_ttl_hours "$cfg")"
  ttl="${ttl:-1}"
  if [ -n "$WEB_USERNAME" ] && [ -n "$password_hash" ] && command -v openssl >/dev/null 2>&1; then
    if [ -n "$DAEMON_API_KEY" ]; then
      secret="$DAEMON_API_KEY:$password_hash"
    else
      secret="$password_hash"
    fi
    expiry=$(( $(date +%s) + ttl * 3600 ))
    payload="$WEB_USERNAME:$expiry"
    signature="$(printf '%s' "$payload" | openssl dgst -sha256 -hmac "$secret" -hex | awk '{print $2}')"
    SESSION_TOKEN="$(printf '%s:%s' "$payload" "$signature" | base64 | tr -d '\n')"
    SESSION_TOKEN_SOURCE="generated-session"
    AUTH_HEADER_ARGS=(-H "Authorization: Bearer $SESSION_TOKEN" -H "Cookie: captain_session=$SESSION_TOKEN")
    return 0
  fi

  if [ -n "$DAEMON_API_KEY" ]; then
    SESSION_TOKEN_SOURCE="api-key"
    AUTH_HEADER_ARGS=(-H "Authorization: Bearer $DAEMON_API_KEY")
  fi
}

try_web_login() {
  local raw headers code token
  raw="$(new_tmp)"
  headers="$(new_tmp)"
  code="$(jq -nc --arg username "$WEB_USERNAME" --arg password "$WEB_PASSWORD" \
    '{username:$username,password:$password}' |
    curl -sS --connect-timeout 2 --max-time "$TIMEOUT" \
      -D "$headers" \
      -o "$raw" \
      -w "%{http_code}" \
      -H "Content-Type: application/json" \
      --data-binary @- \
      "$BASE/api/auth/login" 2>/dev/null || true)"
  if [ "$code" = "200" ]; then
    token="$(jq -r '.token // empty' "$raw" 2>/dev/null || true)"
    if [ -n "$token" ]; then
      SESSION_TOKEN="$token"
      SESSION_TOKEN_SOURCE="web-login"
      jq '{status, username, token_present: ((.token // "") | length > 0)}' \
        "$raw" >"$WORKDIR/auth-login-redacted.json" 2>/dev/null || true
      return 0
    fi
  fi
  warn "web login using initial credentials did not return a usable session token"
}

wait_for_health() {
  local elapsed=0
  local raw
  raw="$(new_tmp)"
  while [ "$elapsed" -le "$READY_TIMEOUT" ]; do
    if curl -sS --connect-timeout 1 --max-time 2 "$BASE/api/health" >"$raw" 2>/dev/null &&
      jq -e '.status == "ok"' "$raw" >/dev/null 2>&1; then
      jq '{status, version}' "$raw" >"$WORKDIR/health.json"
      return 0
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  return 1
}

http_get_json() {
  local path="$1"
  local out="$2"
  local label="$3"
  local raw code
  raw="$(new_tmp)"
  code="$(curl -sS --connect-timeout 2 --max-time "$TIMEOUT" \
    "${AUTH_HEADER_ARGS[@]}" \
    -o "$raw" \
    -w "%{http_code}" \
    "$BASE$path" 2>/dev/null || true)"
  if [ "$code" -lt 200 ] || [ "$code" -ge 300 ]; then
    fail "$label - HTTP $code"
    return 1
  fi
  if ! jq -e . "$raw" >"$out" 2>/dev/null; then
    fail "$label - invalid JSON"
    return 1
  fi
  pass "$label"
}

http_post_json() {
  local path="$1"
  local body="$2"
  local out="$3"
  local label="$4"
  local raw code
  raw="$(new_tmp)"
  code="$(printf '%s' "$body" |
    curl -sS --connect-timeout 2 --max-time "$TIMEOUT" \
      "${AUTH_HEADER_ARGS[@]}" \
      -H "Content-Type: application/json" \
      -o "$raw" \
      -w "%{http_code}" \
      --data-binary @- \
      "$BASE$path" 2>/dev/null || true)"
  if [ "$code" -lt 200 ] || [ "$code" -ge 300 ]; then
    fail "$label - HTTP $code"
    return 1
  fi
  if ! jq -e . "$raw" >"$out" 2>/dev/null; then
    fail "$label - invalid JSON"
    return 1
  fi
  pass "$label"
}

assert_jq() {
  local file="$1"
  local filter="$2"
  local label="$3"
  if jq -e "$filter" "$file" >/dev/null 2>&1; then
    pass "$label"
  else
    fail "$label"
  fi
}

assert_jq_arg() {
  local file="$1"
  local arg="$2"
  local value="$3"
  local filter="$4"
  local label="$5"
  if jq -e --arg "$arg" "$value" "$filter" "$file" >/dev/null 2>&1; then
    pass "$label"
  else
    fail "$label"
  fi
}

fetch_page() {
  local path="$1"
  local marker="$2"
  local out="$3"
  local raw code bytes
  raw="$(new_tmp)"
  code="$(curl -sS --connect-timeout 2 --max-time "$TIMEOUT" \
    "${AUTH_HEADER_ARGS[@]}" \
    -o "$raw" \
    -w "%{http_code}" \
    "$BASE$path" 2>/dev/null || true)"
  if [ "$code" -lt 200 ] || [ "$code" -ge 300 ]; then
    fail "$path page served - HTTP $code"
    return 1
  fi
  if ! grep -Fq "$marker" "$raw"; then
    fail "$path page contains marker"
    return 1
  fi
  bytes="$(wc -c <"$raw" | tr -d ' ')"
  jq -n --arg path "$path" --arg marker "$marker" --argjson bytes "$bytes" \
    '{path:$path, marker:$marker, bytes:$bytes}' >"$out"
  pass "$path page served"
}

mcp_call() {
  local id="$1"
  local name="$2"
  local args="$3"
  local out="$4"
  jq -nc --arg id "$id" --arg name "$name" --argjson args "$args" \
    '{jsonrpc:"2.0",id:$id,method:"tools/call",params:{name:$name,arguments:$args}}' |
    curl -sS --connect-timeout 2 --max-time "$TIMEOUT" \
      "${AUTH_HEADER_ARGS[@]}" \
      -H "Content-Type: application/json" \
      --data-binary @- \
      "$BASE/mcp" >"$out"
}

terminal_ws_url() {
  local ws_base="$BASE"
  case "$ws_base" in
  http://*) ws_base="ws://${ws_base#http://}" ;;
  https://*) ws_base="wss://${ws_base#https://}" ;;
  esac
  printf '%s/api/sessions/user-smoke-%s/terminal?rows=20&cols=80' "$ws_base" "$$"
}

run_terminal_ws_smoke() {
  local ws_url
  ws_url="$(terminal_ws_url)"
  if [ -z "$SESSION_TOKEN" ] && [ -n "$DAEMON_API_KEY" ]; then
    ws_url="$ws_url&token=$DAEMON_API_KEY"
  fi
  if [ -z "$SESSION_TOKEN" ] && [ -z "$DAEMON_API_KEY" ]; then
    fail "terminal websocket auth material available"
    return 1
  fi

  python3 - "$ws_url" "${SESSION_TOKEN:-}" "$WORKDIR/terminal-ws.json" "$BASE" <<'PY'
import asyncio
import json
import sys
import time

import websockets

url, token, out_path, origin = sys.argv[1:5]

async def ws_connect(headers):
    try:
        return await websockets.connect(
            url,
            additional_headers=headers,
            origin=origin,
            open_timeout=8,
            close_timeout=2,
        )
    except TypeError:
        return await websockets.connect(
            url,
            extra_headers=headers,
            origin=origin,
            open_timeout=8,
            close_timeout=2,
        )

async def main():
    headers = []
    if token:
        headers.append(("Cookie", f"captain_session={token}"))
    output = []
    errors = []
    exit_code = None
    sent_exit = False
    started = time.monotonic()
    ws = await ws_connect(headers)
    try:
        await ws.send(json.dumps({"type": "resize", "rows": 20, "cols": 80}))
        while time.monotonic() - started < 14:
            if not sent_exit and time.monotonic() - started > 1.0:
                await ws.send(json.dumps({"type": "input", "data": "/exit\n"}))
                sent_exit = True
            try:
                msg = await asyncio.wait_for(ws.recv(), timeout=0.8)
            except asyncio.TimeoutError:
                continue
            if isinstance(msg, bytes):
                chunk = msg.decode("utf-8", errors="replace")
                output.append(chunk)
                continue
            try:
                payload = json.loads(msg)
            except Exception:
                output.append(str(msg))
                continue
            typ = payload.get("type")
            if typ == "output":
                output.append(str(payload.get("data", "")))
            elif typ == "error":
                errors.append(str(payload.get("message", "")))
                break
            elif typ == "exit":
                exit_code = payload.get("code")
                break
        try:
            await ws.send(json.dumps({"type": "terminate"}))
        except Exception:
            pass
    finally:
        await ws.close()
    text = "".join(output)
    result = {
        "output_bytes": len(text.encode("utf-8")),
        "saw_captain": "Captain" in text or "captain" in text,
        "sent_exit": sent_exit,
        "exit_code": exit_code,
        "error": errors[0] if errors else None,
        "preview": text[-700:],
    }
    with open(out_path, "w", encoding="utf-8") as handle:
        json.dump(result, handle, ensure_ascii=True, indent=2)
    if errors or result["output_bytes"] <= 0:
        raise SystemExit(1)

asyncio.run(main())
PY
  if [ "$?" -eq 0 ]; then
    pass "web terminal websocket starts chat and returns output"
  else
    fail "web terminal websocket starts chat and returns output"
  fi
}

run_cli_checks() {
  title "1/8 CLI and TUI"
  "$CAPTAIN_BIN" --version >"$WORKDIR/cli-version.txt" 2>&1 &&
    pass "captain --version" || fail "captain --version"
  "$CAPTAIN_BIN" status --verbose >"$WORKDIR/cli-status.txt" 2>&1 &&
    grep -Fq "Status:" "$WORKDIR/cli-status.txt" &&
    grep -Fq "Channels:" "$WORKDIR/cli-status.txt" &&
    pass "captain status --verbose" || fail "captain status --verbose"

  if [ "$RUN_TUI" = "1" ]; then
    CAPTAIN_BIN="$CAPTAIN_BIN" "$ROOT_DIR/scripts/tui-smoke.sh" --timeout 5 >"$WORKDIR/tui-smoke.txt" 2>&1 &&
      pass "TUI smoke" || fail "TUI smoke"
  else
    warn "TUI smoke skipped"
  fi
}

run_web_checks() {
  title "2/8 Web terminal/chat"
  if [ "$SESSION_TOKEN_SOURCE" = "web-login" ]; then
    pass "web auth login returned a session"
  elif [ -n "$SESSION_TOKEN" ]; then
    warn "web auth login not used; generated session token from local config"
  elif [ -n "$DAEMON_API_KEY" ]; then
    warn "web auth login not used; falling back to daemon API key"
  else
    fail "web auth material available"
  fi

  fetch_page "/terminal" "Captain Terminal" "$WORKDIR/page-terminal.json"
  fetch_page "/embed" "Captain Embed" "$WORKDIR/page-embed-chat.json"

  if http_get_json "/api/auth/check" "$WORKDIR/auth-check.json" "auth check"; then
    assert_jq "$WORKDIR/auth-check.json" '.authenticated == true' "auth check is authenticated"
  fi

  if http_get_json "/api/terminal/sessions" "$WORKDIR/terminal-sessions.json" "terminal session list"; then
    assert_jq "$WORKDIR/terminal-sessions.json" '.sessions | type == "array"' "terminal sessions shape"
  fi
  run_terminal_ws_smoke
}

run_status_capabilities_checks() {
  local raw cap_raw cap_text
  title "3/8 Capabilities and Status"
  raw="$(new_tmp)"
  if http_get_json "/api/status" "$raw" "status endpoint"; then
    jq '{
      status,
      version,
      agent_count,
      auth_mode,
      channels: {
        active: .channels.active,
        configured_count: .channels.configured_count,
        ready: .channels.ready,
        ready_count: .channels.ready_count,
        total: .channels.total,
        inbound_queue: {
          bridge_running: .channels.inbound_queue.bridge_running,
          pending_messages: .channels.inbound_queue.pending_messages,
          dead_letter_messages: .channels.inbound_queue.dead_letter_messages
        }
      },
      consciousness: {
        state: .consciousness.state,
        confidence: .consciousness.confidence,
        signals: .consciousness.signals,
        operator_actions: .consciousness.operator_actions
      },
      workload: .workload,
      runtime_health: {
        state: .runtime_health.state,
        issue_count: .runtime_health.issue_count
      }
    }' "$raw" >"$WORKDIR/status-summary.json"
    assert_jq "$raw" '.status == "running"' "status reports running"
    assert_jq "$raw" '.channels.ready_count >= 1' "status reports at least one ready channel"
    assert_jq "$raw" '(.consciousness.state | type) == "string"' "status exposes consciousness awareness"
  fi

  cap_raw="$(new_tmp)"
  if mcp_call "user-smoke-capabilities" "capability_search" \
    '{"query":"web terminal projects learning status channel delivery", "include_schemas":false, "max_results":12}' \
    "$cap_raw"; then
    cap_text="$(new_tmp)"
    if jq -er '.result.content[0].text' "$cap_raw" >"$cap_text" 2>/dev/null &&
      jq -e '.results | type == "array" and length > 0' "$cap_text" >/dev/null 2>&1; then
      jq '{results: [.results[] | {name, family, score}]}' "$cap_text" \
        >"$WORKDIR/capability-search-summary.json"
      pass "capability_search returns active results"
    else
      fail "capability_search returns active results"
    fi
  else
    fail "capability_search call"
  fi
}

run_projects_checks() {
  local raw body
  title "4/8 Projects"
  raw="$(new_tmp)"
  body="$(jq -nc --arg slug "$PROJECT_SLUG" '{
    name: "User Flow Smoke",
    slug: $slug,
    goal: "Verify Captain active user surfaces: CLI, TUI, web, Projects, Learning, Capabilities, Status and channels."
  }')"
  if http_post_json "/api/projects" "$body" "$raw" "project created"; then
    jq '{id, slug, name, status, goal}' "$raw" >"$WORKDIR/project-create-summary.json"
    PROJECT_ID="$(jq -r '.id // .project.id // empty' "$raw")"
    assert_jq_arg "$raw" slug "$PROJECT_SLUG" '(.slug // .project.slug // "") == $slug' "created project slug matches"
  fi

  raw="$(new_tmp)"
  if http_get_json "/api/projects?include_archived=true" "$raw" "project list"; then
    jq --arg slug "$PROJECT_SLUG" '{
      total: (if type == "array" then length else (.projects // [] | length) end),
      smoke_present: (((if type == "array" then . else (.projects // []) end) | map(.slug)) | index($slug) != null)
    }' "$raw" >"$WORKDIR/project-list-summary.json"
    assert_jq_arg "$raw" slug "$PROJECT_SLUG" '(((if type == "array" then . else (.projects // []) end) | map(.slug)) | index($slug) != null)' "project list includes smoke project"
  fi

  raw="$(new_tmp)"
  if http_get_json "/api/projects/$PROJECT_SLUG/runtime?events=20" "$raw" "project runtime view"; then
    jq '{
      project: {
        slug: (.project.slug // .slug // null),
        status: (.project.status // .status // null)
      },
      runtime: {
        status: (.runtime.status // null),
        operator_state: (.operator_status.state // .runtime.operator_status.state // null)
      },
      event_count: ((.runtime.timeline // .transcript.events // .events.items // []) | length)
    }' "$raw" >"$WORKDIR/project-runtime-summary.json"
    assert_jq_arg "$raw" slug "$PROJECT_SLUG" '(.project.slug // .slug // "") == $slug' "project runtime resolves smoke project"
  fi

  if [ "$ARCHIVE_PROJECT" = "1" ]; then
    raw="$(new_tmp)"
    local archive_key="${PROJECT_ID:-$PROJECT_SLUG}"
    if http_post_json "/api/projects/$archive_key/archive" '{}' "$raw" "project archived"; then
      jq '{slug:(.slug // .project.slug // null), status:(.status // .project.status // null)}' \
        "$raw" >"$WORKDIR/project-archive-summary.json"
    fi
  else
    warn "smoke project kept: $PROJECT_SLUG"
  fi
}

learning_count_filter() {
  cat <<'EOF'
if type == "array" then length
elif (.items | type) == "array" then (.items | length)
elif (.committed | type) == "array" then (.committed | length)
elif (.review | type) == "array" then (.review | length)
elif (.pending | type) == "array" then (.pending | length)
else -1
end
EOF
}

run_learning_checks() {
  local raw committed_count review_count
  title "5/8 Learning"
  raw="$(new_tmp)"
  if http_get_json "/api/learning/committed?limit=20" "$raw" "learning committed"; then
    committed_count="$(jq "$(learning_count_filter)" "$raw")"
    jq --argjson count "$committed_count" '{committed_count:$count}' \
      >"$WORKDIR/learning-committed-summary.json"
    [ "$committed_count" -ge 0 ] && pass "learning committed shape" || fail "learning committed shape"
  fi

  raw="$(new_tmp)"
  if http_get_json "/api/learning/review?limit=20" "$raw" "learning review"; then
    review_count="$(jq "$(learning_count_filter)" "$raw")"
    jq --argjson count "$review_count" '{review_count:$count}' \
      >"$WORKDIR/learning-review-summary.json"
    [ "$review_count" -ge 0 ] && pass "learning review shape" || fail "learning review shape"
  fi

  raw="$(new_tmp)"
  if http_get_json "/api/learning/metrics" "$raw" "learning metrics"; then
    jq . "$raw" >"$WORKDIR/learning-metrics.json"
    assert_jq "$raw" 'type == "object"' "learning metrics shape"
  fi

  fetch_page "/learning" "Captain Learning" "$WORKDIR/page-learning.json"
}

run_channel_checks() {
  local raw ready_count
  title "6/8 Channels"
  raw="$(new_tmp)"
  if http_get_json "/api/channels" "$raw" "channels endpoint"; then
    jq '{
      active_only,
      total,
      configured_count,
      ready: [.channels[] | select(.ready == true) | .name],
      frozen_channels,
      channels: [.channels[] | {
        name,
        configured,
        ready,
        security_state,
        missing_required_fields
      }]
    }' "$raw" >"$WORKDIR/channels-summary.json"
    assert_jq "$raw" '.total == 4' "channels surface has four active channels"
    assert_jq "$raw" '[.channels[] | select(.ready == true)] | length >= 1' "at least one channel ready"
  fi

  if [ "$CHANNEL" = "none" ]; then
    warn "live channel delivery skipped"
    return
  fi

  ready_count="$(jq -r --arg channel "$CHANNEL" '[.channels[] | select(.name == $channel and .ready == true)] | length' "$raw" 2>/dev/null || printf '0')"
  if [ "$ready_count" -lt 1 ]; then
    fail "selected channel is ready: $CHANNEL"
    return
  fi
  "$CAPTAIN_BIN" channel test "$CHANNEL" >"$WORKDIR/channel-test-$CHANNEL.txt" 2>&1 &&
    grep -Fq "Test message sent to $CHANNEL" "$WORKDIR/channel-test-$CHANNEL.txt" &&
    pass "live channel test sent through $CHANNEL" || fail "live channel test sent through $CHANNEL"
}

run_web_surface_pages() {
  title "7/8 Web Projects/Status pages"
  fetch_page "/projects" "Captain Projects" "$WORKDIR/page-projects.json"
  fetch_page "/system" "Captain System" "$WORKDIR/page-system-status.json"
}

run_final_probe() {
  title "8/8 Final health"
  if wait_for_health; then
    pass "daemon remains healthy after user-flow smoke"
  else
    fail "daemon remains healthy after user-flow smoke"
  fi
}

run_smoke() {
  mkdir -p "$WORKDIR"
  need_cmd curl
  need_cmd jq
  need_cmd grep
  need_cmd python3
  need_cmd openssl
  resolve_captain_bin

  note "api=$BASE"
  note "workdir=$WORKDIR"
  note "captain_bin=$CAPTAIN_BIN"
  if [ "$CHANNEL" != "none" ]; then
    note "live channel test will send one message through: $CHANNEL"
  fi

  wait_for_health || {
    fail "daemon health is ok"
    finish
  }
  pass "daemon health is ok"
  load_auth_material

  run_cli_checks
  run_web_checks
  run_status_capabilities_checks
  run_projects_checks
  run_learning_checks
  run_channel_checks
  run_web_surface_pages
  run_final_probe
  finish
}

run_smoke
