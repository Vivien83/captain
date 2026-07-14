#!/usr/bin/env bash
# v3.9 live E2E smoke test for the computer panel.
#
# Verifies end-to-end that every feature shipped in v3.9 + v3.10 (caches
# excluded) is reachable from a running daemon:
#   1. API health  ...............  GET /api/health
#   2. Session events (v3.9f) ....  GET /api/sessions/{id}/events
#   3. SSE persistence (v3.9f) ...  POST /api/agents/{id}/message/stream
#                                   then re-query /events and assert count grew
#   4. Hard interrupt (v3.9g) ....  kick a long SSE, POST /interrupt,
#                                   assert stream_cancelled == true
#   5. WebSocket terminal (v3.9b) .  GET ws://.../api/sessions/{id}/terminal
#                                   (negotiation only; upgrade is optional)
#
# Exit 0 on full pass, non-zero on first failure. Output is compact so
# the run reads well in CI logs.

set -u

BASE="${CAPTAIN_API:-http://127.0.0.1:50051}"
AGENT="${CAPTAIN_AGENT_ID:-}"
PASS=0
FAIL=0

color() { if [ -t 1 ]; then printf '\033[%sm%s\033[0m' "$1" "$2"; else printf '%s' "$2"; fi }
ok()    { color "32" "✓"; }
ko()    { color "31" "✗"; }
title() { printf '\n%s %s\n' "$(color 36 '==')" "$(color 1 "$1")"; }
note()  { printf '   %s\n' "$*"; }

assert_eq() {
  local got="$1" want="$2" label="$3"
  if [ "$got" = "$want" ]; then
    echo "   $(ok) ${label}"
    PASS=$((PASS + 1))
  else
    echo "   $(ko) ${label} · expected=${want} got=${got}"
    FAIL=$((FAIL + 1))
  fi
}

assert_gt() {
  local got="$1" want="$2" label="$3"
  if [ "$got" -gt "$want" ] 2>/dev/null; then
    echo "   $(ok) ${label} (${got} > ${want})"
    PASS=$((PASS + 1))
  else
    echo "   $(ko) ${label} · expected > ${want} got=${got}"
    FAIL=$((FAIL + 1))
  fi
}

assert_json_field() {
  local body="$1" key="$2" want="$3" label="$4"
  local got
  got=$(python3 -c "import sys,json; d=json.loads(sys.argv[1]); print(d.get('${key}'))" "$body" 2>/dev/null)
  assert_eq "$got" "$want" "$label"
}

# ----------------------------------------------------------------------------
title "1/5  Health"
code=$(curl -s -o /tmp/e2e-health.json -w '%{http_code}' "$BASE/api/health")
assert_eq "$code" "200" "GET /api/health → 200"

# Pick an agent if none given.
if [ -z "$AGENT" ]; then
  AGENT=$(curl -s "$BASE/api/agents" | python3 -c "import sys,json; a=json.load(sys.stdin); print(a[0]['id'] if a else '')" 2>/dev/null)
fi
if [ -z "$AGENT" ]; then
  echo "   $(ko) no agent found — create one first"
  FAIL=$((FAIL + 1))
  exit 1
fi
note "using agent $AGENT"

# ----------------------------------------------------------------------------
title "2/5  Session events endpoint (v3.9f)"
body=$(curl -s "$BASE/api/sessions/$AGENT/events?limit=1")
echo "$body" > /tmp/e2e-events-before.json
count_before=$(python3 -c "import sys,json; d=json.load(open('/tmp/e2e-events-before.json')); print(d.get('count', 0))")
assert_json_field "$body" "session_id" "$AGENT" "events response echoes session_id"
note "events before: $count_before"

# ----------------------------------------------------------------------------
title "3/5  SSE stream persists events (v3.9f)"
curl -s -m 30 -N -X POST "$BASE/api/agents/$AGENT/message/stream" \
  -H "Content-Type: application/json" \
  -d '{"message":"dis bonjour en 3 mots"}' > /tmp/e2e-sse.log 2>&1
sse_events=$(grep -c '^event:' /tmp/e2e-sse.log || echo 0)
assert_gt "$sse_events" "0" "SSE produced at least 1 event"

sleep 1
body_after=$(curl -s "$BASE/api/sessions/$AGENT/events?limit=100")
count_after=$(python3 -c "import sys,json; d=json.loads(sys.argv[1]); print(d.get('count', 0))" "$body_after")
note "events after: $count_after"
assert_gt "$count_after" "$count_before" "sessions_events count grew after SSE"

# ----------------------------------------------------------------------------
title "4/5  Hard interrupt (v3.9g)"
curl -s -m 60 -N -X POST "$BASE/api/agents/$AGENT/message/stream" \
  -H "Content-Type: application/json" \
  -d '{"message":"ecris 100 haikus en francais"}' > /tmp/e2e-interrupt.log 2>&1 &
CURL_PID=$!
sleep 2
interrupt_body=$(curl -s -X POST "$BASE/api/agents/$AGENT/interrupt")
assert_json_field "$interrupt_body" "status" "interrupted" "interrupt status"
assert_json_field "$interrupt_body" "stream_cancelled" "True" "stream_cancelled == true"
wait "$CURL_PID" 2>/dev/null

# ----------------------------------------------------------------------------
title "5/5  WebSocket terminal route exists (v3.9b)"
# Do the HTTP GET without Upgrade headers — the server should answer with
# 400/426 "missing WebSocket upgrade headers", not a 404. Anything other
# than 404 means the route is registered.
ws_probe=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/api/sessions/e2e-probe/terminal")
case "$ws_probe" in
  200|400|426|101)
    echo "   $(ok) terminal WS route reachable (HTTP $ws_probe)"
    PASS=$((PASS + 1))
    ;;
  404)
    echo "   $(ko) terminal WS route is 404 — not registered"
    FAIL=$((FAIL + 1))
    ;;
  *)
    echo "   $(ok) terminal WS route returned $ws_probe (non-404 → wired)"
    PASS=$((PASS + 1))
    ;;
esac

# ----------------------------------------------------------------------------
printf '\n%s\n' "========================================"
if [ "$FAIL" -eq 0 ]; then
  echo " $(ok) All $PASS checks passed."
  exit 0
else
  echo " $(ko) $FAIL failed, $PASS passed."
  exit 1
fi
