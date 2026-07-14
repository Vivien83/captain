#!/usr/bin/env bash
# Captain Excellence API smoke gate.
#
# Core mode is intentionally cheap: no LLM call, no Telegram/Discord delivery,
# no SSH, no TTS. Full mode enables opt-in live checks through env flags
# or explicit CLI flags so release tests do not need env-prefixed commands.

set -u

BASE="${CAPTAIN_API:-http://127.0.0.1:50051}"
MODE="${CAPTAIN_EXCELLENCE_SMOKE_MODE:-core}"
WORKDIR="${CAPTAIN_SMOKE_WORKDIR:-target/captain-excellence-smoke}"
TIMEOUT="${CAPTAIN_SMOKE_TIMEOUT:-60}"
READY_TIMEOUT="${CAPTAIN_SMOKE_READY_TIMEOUT:-20}"
CAPTAIN_SMOKE_LLM="${CAPTAIN_SMOKE_LLM:-0}"
CAPTAIN_SMOKE_TTS="${CAPTAIN_SMOKE_TTS:-0}"
CAPTAIN_SMOKE_SSH_ALIAS="${CAPTAIN_SMOKE_SSH_ALIAS:-}"
CAPTAIN_SMOKE_STRICT_RELEASE="${CAPTAIN_SMOKE_STRICT_RELEASE:-0}"
EXPECTED_CHANGELOG="${CAPTAIN_SMOKE_CHANGELOG_VERSION:-0.1.0-alpha.1}"

while [ "$#" -gt 0 ]; do
  case "$1" in
  --full)
    MODE="full"
    shift
    ;;
  --core)
    MODE="core"
    shift
    ;;
  --llm)
    MODE="full"
    CAPTAIN_SMOKE_LLM=1
    shift
    ;;
  --tts)
    MODE="full"
    CAPTAIN_SMOKE_TTS=1
    shift
    ;;
  --ssh-alias)
    MODE="full"
    if [ $# -lt 2 ] || [ -z "${2:-}" ]; then
      printf 'Missing value for --ssh-alias\n' >&2
      exit 2
    fi
    CAPTAIN_SMOKE_SSH_ALIAS="$2"
    shift 2
    ;;
  --api)
    if [ $# -lt 2 ] || [ -z "${2:-}" ]; then
      printf 'Missing value for --api\n' >&2
      exit 2
    fi
    BASE="$2"
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
  --expected-changelog)
    if [ $# -lt 2 ] || [ -z "${2:-}" ]; then
      printf 'Missing value for --expected-changelog\n' >&2
      exit 2
    fi
    EXPECTED_CHANGELOG="$2"
    shift 2
    ;;
  -h|--help)
    cat <<'EOF'
Usage: scripts/excellence-smoke.sh [--core|--full] [--llm] [--tts] [--ssh-alias name]

Environment:
  CAPTAIN_API                    API base URL (default http://127.0.0.1:50051)
  CAPTAIN_SMOKE_WORKDIR          Output dir for local artifacts
  CAPTAIN_SMOKE_TIMEOUT          curl timeout in seconds
  CAPTAIN_SMOKE_READY_TIMEOUT    Seconds to wait for /api/health
  CAPTAIN_SMOKE_CHANGELOG_VERSION Expected runtime changelog entry
  CAPTAIN_SMOKE_AGENT_ID         Agent id for optional full LLM check
  CAPTAIN_SMOKE_LLM=1            Enable live agent message in --full mode
  CAPTAIN_SMOKE_SSH_ALIAS=name   Enable ssh_health_check in --full mode
  CAPTAIN_SMOKE_TTS=1            Enable media_pipeline TTS in --full mode
  CAPTAIN_SMOKE_STRICT_RELEASE=1 Fail on leftover project runtime worker agents

Flags:
  --llm                         Enable live agent message and set --full
  --tts                         Enable media_pipeline TTS and set --full
  --ssh-alias name              Enable ssh_health_check and set --full
  --api URL                     Override API base URL
  --timeout seconds             Override curl timeout
  --ready-timeout seconds       Override health readiness timeout
  --expected-changelog version  Required runtime changelog entry
EOF
    exit 0
    ;;
  *)
    printf 'Unknown argument: %s\n' "$1" >&2
    exit 2
    ;;
  esac
done

PASS=0
FAIL=0
WARN=0
AUTH_HEADER_ARGS=()

color() {
  if [ -t 1 ]; then
    printf '\033[%sm%s\033[0m' "$1" "$2"
  else
    printf '%s' "$2"
  fi
}

ok_icon() { color "32" "✓"; }
ko_icon() { color "31" "✗"; }
warn_icon() { color "33" "!"; }
title() { printf '\n%s %s\n' "$(color 36 '==')" "$(color 1 "$1")"; }
note() { printf '   %s\n' "$*"; }

pass() {
  printf '   %s %s\n' "$(ok_icon)" "$1"
  PASS=$((PASS + 1))
}

fail() {
  printf '   %s %s\n' "$(ko_icon)" "$1"
  FAIL=$((FAIL + 1))
}

warn() {
  printf '   %s %s\n' "$(warn_icon)" "$1"
  WARN=$((WARN + 1))
}

need_cmd() {
  if command -v "$1" >/dev/null 2>&1; then
    return 0
  fi
  fail "missing required command: $1"
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

load_auth_header() {
  if [ -n "${CAPTAIN_API_KEY:-}" ]; then
    AUTH_HEADER_ARGS=(-H "Authorization: Bearer $CAPTAIN_API_KEY")
    return 0
  fi

  local cfg="${CAPTAIN_HOME:-$HOME/.captain}/config.toml"
  if [ ! -f "$cfg" ] || ! command -v openssl >/dev/null 2>&1; then
    return 0
  fi

  local api_key username password_hash ttl secret expiry payload signature token
  api_key="$(toml_top_string api_key "$cfg")"
  if [ -z "$api_key" ] && [ -f "${CAPTAIN_HOME:-$HOME/.captain}/secrets.env" ]; then
    api_key="$(awk -F= '$1=="CAPTAIN_DAEMON_API_KEY" {print $2; exit}' "${CAPTAIN_HOME:-$HOME/.captain}/secrets.env" | tr -d '\r')"
  fi
  if [ -z "$api_key" ]; then
    api_key="${CAPTAIN_DAEMON_API_KEY:-${CAPTAIN_API_KEY:-}}"
  fi
  username="$(toml_auth_string username "$cfg")"
  password_hash="$(toml_auth_string password_hash "$cfg")"
  ttl="$(toml_auth_int session_ttl_hours "$cfg")"
  ttl="${ttl:-1}"
  if [ -z "$username" ] || [ -z "$password_hash" ]; then
    return 0
  fi
  if [ -n "$api_key" ]; then
    secret="$api_key:$password_hash"
  else
    secret="$password_hash"
  fi
  expiry=$(( $(date +%s) + ttl * 3600 ))
  payload="$username:$expiry"
  signature="$(printf '%s' "$payload" | openssl dgst -sha256 -hmac "$secret" -hex | awk '{print $2}')"
  token="$(printf '%s:%s' "$payload" "$signature" | base64 | tr -d '\n')"
  AUTH_HEADER_ARGS=(-H "Authorization: Bearer $token")
}

finish() {
  printf '\n%s\n' "========================================"
  if [ "$FAIL" -eq 0 ]; then
    printf ' %s %s checks passed' "$(ok_icon)" "$PASS"
    if [ "$WARN" -gt 0 ]; then
      printf ', %s warnings' "$WARN"
    fi
    printf '.\n'
    exit 0
  fi
  printf ' %s %s failed, %s passed, %s warnings.\n' "$(ko_icon)" "$FAIL" "$PASS" "$WARN"
  exit 1
}

http_get() {
  curl -sS --max-time "$TIMEOUT" "${AUTH_HEADER_ARGS[@]}" "$1"
}

wait_for_health() {
  local elapsed=0
  local body

  while [ "$elapsed" -le "$READY_TIMEOUT" ]; do
    body=$(curl -sS --connect-timeout 1 --max-time 2 "$BASE/api/health" 2>/dev/null)
    if printf '%s' "$body" | jq -e '.status == "ok"' >/dev/null 2>&1; then
      printf '%s' "$body"
      return 0
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done

  return 1
}

mcp_call() {
  local id="$1"
  local name="$2"
  local args="$3"

  jq -nc --arg id "$id" --arg name "$name" --argjson args "$args" \
    '{jsonrpc:"2.0",id:$id,method:"tools/call",params:{name:$name,arguments:$args}}' |
    curl -sS --max-time "$TIMEOUT" \
      "${AUTH_HEADER_ARGS[@]}" \
      -H "Content-Type: application/json" \
      --data-binary @- \
      "$BASE/mcp"
}

agent_message() {
  local agent_id="$1"
  local message="$2"

  jq -nc --arg message "$message" '{message:$message}' |
    curl -sS --max-time "$TIMEOUT" \
      "${AUTH_HEADER_ARGS[@]}" \
      -H "Content-Type: application/json" \
      --data-binary @- \
      "$BASE/api/agents/$agent_id/message"
}

mcp_text() {
  jq -er '.result.content[0].text'
}

assert_jq_eq() {
  local body="$1"
  local filter="$2"
  local expected="$3"
  local label="$4"
  local got

  got=$(printf '%s' "$body" | jq -er "$filter" 2>/dev/null) || {
    fail "$label · jq failed: $filter"
    return
  }

  if [ "$got" = "$expected" ]; then
    pass "$label"
  else
    fail "$label · expected=$expected got=$got"
  fi
}

assert_jq_true() {
  local body="$1"
  local filter="$2"
  local label="$3"

  if printf '%s' "$body" | jq -e "$filter" >/dev/null 2>&1; then
    pass "$label"
  else
    fail "$label · jq false: $filter"
  fi
}

assert_file_exists() {
  local path="$1"
  local label="$2"

  if [ -s "$path" ]; then
    pass "$label"
  else
    fail "$label · missing or empty: $path"
  fi
}

need_cmd curl
need_cmd jq
load_auth_header

mkdir -p "$WORKDIR"

title "1/8 Health"
health=$(wait_for_health) || {
  fail "GET /api/health failed"
  finish
}
assert_jq_eq "$health" '.status' "ok" "daemon health is ok"
note "api=$BASE mode=$MODE"
note "expected_changelog=$EXPECTED_CHANGELOG"

title "2/8 Agents"
agents=$(http_get "$BASE/api/agents") || {
  fail "GET /api/agents failed"
  finish
}
assert_jq_true "$agents" 'type == "array" and length > 0' "agents endpoint returns at least one agent"
assert_jq_true "$agents" 'map(select(.name == "captain" and .state == "Running")) | length >= 1' "captain agent is running"
CAPTAIN_AGENT_ID="${CAPTAIN_SMOKE_AGENT_ID:-$(printf '%s' "$agents" | jq -r 'map(select(.name == "captain"))[0].id // empty')}"
project_worker_count=$(printf '%s' "$agents" | jq -r '[.[] | select((.name // "") | startswith("project-"))] | length')
if [ "$project_worker_count" -eq 0 ]; then
  pass "no leftover project runtime workers"
elif [ "$CAPTAIN_SMOKE_STRICT_RELEASE" = "1" ]; then
  fail "no leftover project runtime workers · found=$project_worker_count"
else
  warn "leftover project runtime workers detected: $project_worker_count"
fi
note "captain_agent=${CAPTAIN_AGENT_ID:-none}"

title "3/8 P0/P1 capability surface"
cap_args='{"query":"P0 P1 grouped rails web_research_batch file_inspect_batch ssh_health_check document_pipeline memory_context_batch media_pipeline channel_delivery_batch","include_schemas":false,"max_results":12}'
cap_resp=$(mcp_call "capabilities" "capability_search" "$cap_args") || {
  fail "capability_search call failed"
  finish
}
cap_text=$(printf '%s' "$cap_resp" | mcp_text 2>/dev/null) || {
  fail "capability_search returned no MCP text"
  finish
}
for tool in web_research_batch file_inspect_batch ssh_health_check document_pipeline memory_context_batch media_pipeline channel_delivery_batch; do
  if printf '%s' "$cap_text" | jq -e --arg t "$tool" '.results | map(.name) | index($t) != null' >/dev/null 2>&1; then
    pass "$tool is discoverable"
  else
    fail "$tool is discoverable"
  fi
done

title "4/8 Runtime changelog"
changelog_args=$(jq -nc --arg version "$EXPECTED_CHANGELOG" '{
  family: "runtime-changelog",
  query: $version,
  max_results: 3
}')
changelog_resp=$(mcp_call "changelog" "captain_docs" "$changelog_args") || {
  fail "captain_docs runtime-changelog call failed"
  finish
}
changelog_text=$(printf '%s' "$changelog_resp" | mcp_text 2>/dev/null) || {
  fail "captain_docs returned no MCP text"
  finish
}
if printf '%s' "$changelog_text" | grep -Fq "$EXPECTED_CHANGELOG"; then
  pass "runtime changelog exposes $EXPECTED_CHANGELOG"
else
  fail "runtime changelog exposes $EXPECTED_CHANGELOG"
fi

title "5/8 Memory strict filter"
mem_args='{"queries":["service:inventory-api"],"include_memory":true,"include_sessions":false,"include_knowledge":false,"max_results":2,"memory_max_results":2,"preview_chars":700}'
mem_resp=$(mcp_call "memory" "memory_context_batch" "$mem_args") || {
  fail "memory_context_batch call failed"
  finish
}
mem_text=$(printf '%s' "$mem_resp" | mcp_text 2>/dev/null) || {
  fail "memory_context_batch returned no MCP text"
  finish
}
assert_jq_true "$mem_text" '.results[0].memory.match_count | type == "number"' "memory result exposes match_count"
assert_jq_true "$mem_text" '.results[0].memory.filtered | type == "number"' "memory result exposes filtered count"
assert_jq_true "$mem_text" '(.results[0].memory | tostring | contains("[MemPalace]")) | not' "memory result does not expose raw MemPalace dump"
mem_matches=$(printf '%s' "$mem_text" | jq -r '.results[0].memory.match_count // 0')
mem_filtered=$(printf '%s' "$mem_text" | jq -r '.results[0].memory.filtered // 0')
note "match_count=$mem_matches filtered=$mem_filtered"

title "6/8 Web research batch"
web_args='{"queries":["example domain"],"urls":["https://example.com"],"max_results_per_query":1,"max_fetches":1,"fetch_char_limit":600}'
web_resp=$(mcp_call "web" "web_research_batch" "$web_args") || {
  fail "web_research_batch call failed"
  finish
}
web_text=$(printf '%s' "$web_resp" | mcp_text 2>/dev/null) || {
  fail "web_research_batch returned no MCP text"
  finish
}
assert_jq_true "$web_text" '.fetched[0].url == "https://example.com"' "web batch fetched requested URL"
assert_jq_true "$web_text" '.fetched[0].success == true' "web batch fetch succeeded"

title "7/8 Document pipeline"
doc_path="$(pwd)/$WORKDIR/excellence-smoke.md"
doc_args=$(jq -nc --arg path "$doc_path" '{
  document: {
    format: "markdown",
    path: $path,
    title: "Captain Excellence Smoke",
    content: "Captain generated this document through document_pipeline.\n\n| Rail | Status |\n| --- | --- |\n| document_pipeline | ok |",
    overwrite: true
  }
}')
doc_resp=$(mcp_call "document" "document_pipeline" "$doc_args") || {
  fail "document_pipeline call failed"
  finish
}
doc_text=$(printf '%s' "$doc_resp" | mcp_text 2>/dev/null) || {
  fail "document_pipeline returned no MCP text"
  finish
}
assert_jq_true "$doc_text" '.document.success == true' "document pipeline reports success"
created_doc=$(printf '%s' "$doc_text" | jq -r '.document.path // empty')
assert_file_exists "$created_doc" "document artifact exists"

title "8/8 Optional full checks"
if [ "$MODE" != "full" ]; then
  warn "full checks skipped; run with --full for opt-in LLM/SSH/TTS checks"
else
  if [ "$CAPTAIN_SMOKE_LLM" = "1" ]; then
    if [ -n "${CAPTAIN_AGENT_ID:-}" ]; then
      llm_resp=$(agent_message "$CAPTAIN_AGENT_ID" "Reponds exactement: OK_EXCELLENCE_SMOKE") || {
        fail "live agent message failed"
        finish
      }
      assert_jq_true "$llm_resp" '(.response // .message // .text // tostring) | contains("OK_EXCELLENCE_SMOKE")' "live agent response matched"
    else
      warn "LLM check requested but no captain agent id was available"
    fi
  else
    warn "LLM full check skipped; set CAPTAIN_SMOKE_LLM=1 or pass --llm"
  fi

  if [ -n "$CAPTAIN_SMOKE_SSH_ALIAS" ]; then
    ssh_args=$(jq -nc --arg key "$CAPTAIN_SMOKE_SSH_ALIAS" '{key_name:$key, include_docker:false, include_ports:false, include_logs:false, timeout_secs:30}')
    ssh_resp=$(mcp_call "ssh" "ssh_health_check" "$ssh_args") || {
      fail "ssh_health_check failed"
      finish
    }
    assert_jq_true "$ssh_resp" '.result.isError != true' "ssh health check returned success"
  else
    warn "SSH full check skipped; set CAPTAIN_SMOKE_SSH_ALIAS or pass --ssh-alias"
  fi

  if [ "$CAPTAIN_SMOKE_TTS" = "1" ]; then
    tts_args='{"items":[{"action":"tts","text":"Smoke test Captain.","format":"mp3"}],"preview_chars":1200}'
    tts_resp=$(mcp_call "tts" "media_pipeline" "$tts_args") || {
      fail "media_pipeline TTS failed"
      finish
    }
    tts_text=$(printf '%s' "$tts_resp" | mcp_text 2>/dev/null) || {
      fail "media_pipeline returned no MCP text"
      finish
    }
    assert_jq_true "$tts_text" '.success == true and .results[0].success == true' "media pipeline TTS returned success"
  else
    warn "TTS full check skipped; set CAPTAIN_SMOKE_TTS=1 or pass --tts"
  fi
fi

finish
