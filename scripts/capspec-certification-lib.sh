#!/usr/bin/env bash

# Shared process-level helpers for scripts/capspec-real-certification.sh.
# The caller owns cleanup through an EXIT trap and provides BASE, WORKDIR,
# TIMEOUT, READY_TIMEOUT, and CAPTAIN_AGENT_ID.

CAPSPEC_CERT_PASS=0
HTTP_STATUS=""

note() { printf '   %s\n' "$*"; }

pass() {
  printf '   ok %s\n' "$*"
  CAPSPEC_CERT_PASS=$((CAPSPEC_CERT_PASS + 1))
}

fail() {
  printf '   FAIL %s\n' "$*" >&2
  return 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

http_request() {
  local method="$1"
  local path="$2"
  local payload_file="$3"
  local output_file="$4"
  local url="$BASE$path"
  if [[ -n "$payload_file" ]]; then
    HTTP_STATUS="$(curl -sS --max-time "$TIMEOUT" -o "$output_file" -w '%{http_code}' \
      -X "$method" -H "X-API-Key: $CERT_API_KEY" -H 'Content-Type: application/json' \
      --data-binary @"$payload_file" "$url")"
  else
    HTTP_STATUS="$(curl -sS --max-time "$TIMEOUT" -o "$output_file" -w '%{http_code}' \
      -X "$method" -H "X-API-Key: $CERT_API_KEY" "$url")"
  fi
}

assert_status() {
  local expected="$1"
  local label="$2"
  local body_file="${3:-}"
  if [[ "$HTTP_STATUS" == "$expected" ]]; then
    pass "$label"
    return 0
  fi
  if [[ -n "$body_file" && -f "$body_file" ]]; then
    note "response: $(tr '\n' ' ' <"$body_file" | cut -c 1-700)"
  fi
  fail "$label returned HTTP ${HTTP_STATUS:-none}, expected $expected"
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

wait_for_health() {
  local output_file="$1"
  local elapsed=0
  while [[ "$elapsed" -le "$READY_TIMEOUT" ]]; do
    if curl -sS --connect-timeout 1 --max-time 2 "$BASE/api/health" >"$output_file" 2>/dev/null \
      && jq -e '.status == "ok"' "$output_file" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  return 1
}

wait_for_fixture() {
  local fixture_base="$1"
  local output_file="$2"
  local elapsed=0
  while [[ "$elapsed" -le "$READY_TIMEOUT" ]]; do
    if curl -sS --connect-timeout 1 --max-time 2 "$fixture_base/cert/health" >"$output_file" 2>/dev/null \
      && jq -e '.status == "ok"' "$output_file" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  return 1
}

wait_for_api_jq() {
  local path="$1"
  local filter="$2"
  local output_file="$3"
  local limit="${4:-$READY_TIMEOUT}"
  local elapsed=0
  while [[ "$elapsed" -le "$limit" ]]; do
    if curl -sS --max-time 3 -H "X-API-Key: $CERT_API_KEY" "$BASE$path" \
      >"$output_file" 2>/dev/null \
      && jq -e "$filter" "$output_file" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  return 1
}

capability_payload() {
  local source_file="$1"
  local scope="$2"
  local workspace="$3"
  local output_file="$4"
  if [[ -n "$workspace" ]]; then
    jq -n --arg scope "$scope" --arg workspace "$workspace" \
      --rawfile source "$source_file" \
      '{scope:$scope,workspace:$workspace,source:$source}' >"$output_file"
  else
    jq -n --arg scope "$scope" --rawfile source "$source_file" \
      '{scope:$scope,source:$source}' >"$output_file"
  fi
}

install_capability() {
  local source_file="$1"
  local scope="$2"
  local workspace="$3"
  local artifact_stem="$4"
  local payload="$WORKDIR/${artifact_stem}-install-request.json"
  local response="$WORKDIR/${artifact_stem}-install-response.json"
  capability_payload "$source_file" "$scope" "$workspace" "$payload"
  http_request POST "/api/capabilities/native/install" "$payload" "$response"
  assert_status 200 "install $artifact_stem" "$response"
}

decide_pending_capability() {
  local install_response="$1"
  local decision="$2"
  local scope="$3"
  local workspace="$4"
  local artifact_stem="$5"
  local name hash payload response
  name="$(jq -r '.name' "$install_response")"
  hash="$(jq -r '.pending_hash' "$install_response")"
  [[ -n "$name" && "$name" != "null" ]] || fail "missing capability name in $install_response"
  [[ -n "$hash" && "$hash" != "null" ]] || fail "missing pending hash for $name"
  payload="$WORKDIR/${artifact_stem}-${decision}-request.json"
  response="$WORKDIR/${artifact_stem}-${decision}-response.json"
  if [[ -n "$workspace" ]]; then
    jq -n --arg decision "$decision" --arg expected_hash "$hash" --arg scope "$scope" \
      --arg workspace "$workspace" \
      '{decision:$decision,expected_hash:$expected_hash,scope:$scope,workspace:$workspace}' >"$payload"
  else
    jq -n --arg decision "$decision" --arg expected_hash "$hash" --arg scope "$scope" \
      '{decision:$decision,expected_hash:$expected_hash,scope:$scope}' >"$payload"
  fi
  http_request POST "/api/capabilities/native/$name/decision" "$payload" "$response"
  assert_status 200 "$decision exact pending hash for $name" "$response"
}

run_agent_scenario() {
  local scenario="$1"
  local message="$2"
  local output_file="$3"
  local payload="$WORKDIR/message-${scenario}-request.json"
  jq -n --arg message "[CAPSPEC-CERT:${scenario}] $message" '{message:$message}' >"$payload"
  http_request POST "/api/agents/$CAPTAIN_AGENT_ID/message" "$payload" "$output_file"
  assert_status 200 "agent scenario $scenario completed" "$output_file"
}

capture_tool_transcript() {
  local label="$1"
  local marker="$2"
  local success_message="$3"
  local destination="$WORKDIR/session-evidence/$label"
  local elapsed=0
  mkdir -p "$destination"
  while [[ "$elapsed" -le 10 ]]; do
    if [[ -d "$WORKSPACE/sessions" ]]; then
      cp "$WORKSPACE"/sessions/*.jsonl "$destination/" 2>/dev/null || true
    fi
    if grep -R -F "$marker" "$destination" >/dev/null 2>&1; then
      pass "$success_message"
      return
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  fail "$label tool output is absent from its immediate session evidence"
}

wait_for_latest_run_status() {
  local capability_name="$1"
  local expected_status="$2"
  local output_file="$3"
  local limit="${4:-$READY_TIMEOUT}"
  local elapsed=0
  while [[ "$elapsed" -le "$limit" ]]; do
    if curl -sS --max-time 3 -H "X-API-Key: $CERT_API_KEY" \
      "$BASE/api/capabilities/native/runs?limit=500" >"$output_file" 2>/dev/null \
      && jq -e --arg name "$capability_name" --arg status "$expected_status" \
        '[.runs[] | select(.capability_name == $name)][0].status == $status' \
        "$output_file" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  return 1
}

extract_latest_run() {
  local runs_file="$1"
  local capability_name="$2"
  local output_file="$3"
  jq --arg name "$capability_name" \
    '[.runs[] | select(.capability_name == $name)][0] // error("run not found")' \
    "$runs_file" >"$output_file"
}
