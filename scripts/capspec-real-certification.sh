#!/usr/bin/env bash
# Reproducible process-level certification for Captain Forge / CapSpec.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STAMP="$(date +%Y%m%d-%H%M%S)-$$"
WORKDIR="${CAPSPEC_CERT_WORKDIR:-$ROOT_DIR/target/capspec-real-certification/$STAMP}"
CAPTAIN_BIN="${CAPSPEC_CERT_BIN:-}"
PORT="${CAPSPEC_CERT_PORT:-50481}"
FIXTURE_PORT="${CAPSPEC_CERT_FIXTURE_PORT:-50482}"
SECONDARY_PORT="${CAPSPEC_CERT_SECONDARY_PORT:-50483}"
TIMEOUT="${CAPSPEC_CERT_TIMEOUT:-90}"
READY_TIMEOUT="${CAPSPEC_CERT_READY_TIMEOUT:-180}"
BASE="http://127.0.0.1:$PORT"
FIXTURE_BASE="http://127.0.0.1:$FIXTURE_PORT"
HOME_DIR="$WORKDIR/captain-home"
CONFIG="$HOME_DIR/config.toml"
WORKSPACE="$HOME_DIR/workspaces/captain"
PROJECT_ROOT="$WORKSPACE/cert-project"
FIXTURE_DIR="$ROOT_DIR/tests/fixtures/capspec-certification"
CAPTAIN_AGENT_ID=""
DAEMON_PID=""
FIXTURE_PID=""
SECONDARY_PID=""
DAEMON_GENERATION=0
CERT_SECRET="captain-cert-secret-DO-NOT-LEAK"
CERT_API_KEY="capspec-certification-api-key"
CONTROL_USERNAME="certifier"
CONTROL_PASSWORD="capspec-control-password"

# shellcheck source=scripts/capspec-certification-lib.sh
source "$ROOT_DIR/scripts/capspec-certification-lib.sh"
# shellcheck source=scripts/capspec-certification-lifecycle.sh
source "$ROOT_DIR/scripts/capspec-certification-lifecycle.sh"

usage() {
  cat <<'USAGE'
Usage: scripts/capspec-real-certification.sh [options]

Options:
  --bin path          Captain binary (defaults to target/debug/captain, building it when absent).
  --workdir path      Persistent evidence and isolated home directory.
  --port port         Primary isolated Captain API port.
  --fixture-port port Local deterministic OpenAI/Telegram fixture port.
  --secondary-port p  Fresh-home portability daemon port.
  --timeout seconds   Per-request timeout.
  --ready-timeout s   Boot and asynchronous-state timeout.
  -h, --help          Show this help.

The smoke uses public example.com for one allowed HTTP proof and binds three
loopback ports. It never reads or modifies the user's personal CAPTAIN_HOME,
daemon, Docker state, releases, or external disks.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bin) CAPTAIN_BIN="$2"; shift 2 ;;
    --workdir) WORKDIR="$2"; shift 2 ;;
    --port) PORT="$2"; shift 2 ;;
    --fixture-port) FIXTURE_PORT="$2"; shift 2 ;;
    --secondary-port) SECONDARY_PORT="$2"; shift 2 ;;
    --timeout) TIMEOUT="$2"; shift 2 ;;
    --ready-timeout) READY_TIMEOUT="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) printf 'unknown argument: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
done

BASE="http://127.0.0.1:$PORT"
FIXTURE_BASE="http://127.0.0.1:$FIXTURE_PORT"
HOME_DIR="$WORKDIR/captain-home"
CONFIG="$HOME_DIR/config.toml"
WORKSPACE="$HOME_DIR/workspaces/captain"
PROJECT_ROOT="$WORKSPACE/cert-project"

cleanup() {
  for pid in "${SECONDARY_PID:-}" "${DAEMON_PID:-}" "${FIXTURE_PID:-}"; do
    [[ -n "$pid" ]] || continue
    if kill -0 "$pid" >/dev/null 2>&1; then
      kill "$pid" >/dev/null 2>&1 || true
      sleep 0.5
      kill -0 "$pid" >/dev/null 2>&1 && kill -KILL "$pid" >/dev/null 2>&1 || true
      wait "$pid" >/dev/null 2>&1 || true
    fi
  done
}
trap cleanup EXIT INT TERM

resolve_captain_bin() {
  if [[ -n "$CAPTAIN_BIN" ]]; then
    [[ -x "$CAPTAIN_BIN" ]] || fail "Captain binary is not executable: $CAPTAIN_BIN"
    CAPTAIN_BIN="$(cd "$(dirname "$CAPTAIN_BIN")" && pwd)/$(basename "$CAPTAIN_BIN")"
    return
  fi
  if [[ -x "$ROOT_DIR/target/debug/captain" ]]; then
    CAPTAIN_BIN="$ROOT_DIR/target/debug/captain"
    return
  fi
  note "building captain-cli for the real certification"
  (cd "$ROOT_DIR" && cargo build -p captain-cli)
  CAPTAIN_BIN="$ROOT_DIR/target/debug/captain"
}

write_config() {
  local home="$1"
  local port="$2"
  local memory_backend="$3"
  local telegram="$4"
  local config="$home/config.toml"
  local password_hash
  password_hash="$(printf '%s' "$CONTROL_PASSWORD" | shasum -a 256 | cut -d ' ' -f 1)"
  mkdir -p "$home/data" "$home/capabilities" "$home/workspaces/captain"
  cat >"$config" <<EOF
home_dir = "$home"
data_dir = "$home/data"
log_level = "info"
api_listen = "127.0.0.1:$port"
network_enabled = true
api_key = "$CERT_API_KEY"
language = "en"

[default_model]
provider = "openai"
model = "captain-capspec-certifier"
api_key_env = "OPENAI_API_KEY"
base_url = "$FIXTURE_BASE/v1"

[memory]
backend = "$memory_backend"
embedding_provider = "local"

[learning]
enabled = false

[checkpoints]
enabled = false

[skills]
enabled = false

[assistant]
onboarding_completed = true

[approval]
require_approval = []

[auth]
enabled = true
username = "$CONTROL_USERNAME"
password_hash = "$password_hash"
session_ttl_hours = 1
EOF
  if [[ "$telegram" == "true" ]]; then
    cat >>"$config" <<EOF

[channels.telegram]
bot_token_env = "TELEGRAM_BOT_TOKEN"
allowed_users = ["4242"]
default_agent = "captain"
default_chat_id = "4242"
poll_interval_secs = 1
api_url = "$FIXTURE_BASE/telegram"
streaming = false
EOF
  fi
}

write_real_repo() {
  local workspace="$1"
  mkdir -p "$workspace/cert-repo/src" "$workspace/cert-work" "$workspace/cert-project"
  cat >"$workspace/cert-repo/Cargo.toml" <<'EOF'
[package]
name = "captain-capspec-cert-repo"
version = "0.1.0"
edition = "2021"
[lib]
path = "src/lib.rs"
[workspace]
EOF
  cat >"$workspace/cert-repo/README.md" <<'EOF'
# CapSpec Certification Repository

This small real Rust repository verifies project inspection and test execution.
EOF
  cat >"$workspace/cert-repo/src/lib.rs" <<'EOF'
pub fn invoice_total(lines: &[(u64, u64)]) -> u64 {
    lines.iter().map(|(quantity, cents)| quantity * cents).sum()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn totals_realistic_invoice_lines() {
        assert_eq!(invoice_total(&[(2, 1_250), (1, 499)]), 2_999);
    }
}
EOF
  printf 'workspace traversal sentinel\n' >"$workspace/escape.txt"
}

start_fixture() {
  CAPSPEC_CERT_FIXTURE_PORT="$FIXTURE_PORT" \
  CAPSPEC_CERT_ARTIFACTS="$WORKDIR/fixture" \
  CAPSPEC_CERT_REPO_ROOT="$WORKSPACE/cert-repo" \
  CAPSPEC_CERT_WORK_ROOT="$WORKSPACE/cert-work" \
  CAPSPEC_CERT_PROJECT_ROOT="$PROJECT_ROOT" \
    node "$ROOT_DIR/scripts/capspec-certification-fixture.mjs" \
      >"$WORKDIR/fixture.log" 2>&1 &
  FIXTURE_PID=$!
  wait_for_fixture "$FIXTURE_BASE" "$WORKDIR/fixture-health.json" \
    || fail "certification fixture did not become healthy"
  pass "deterministic OpenAI/Telegram fixture is healthy"
}

start_primary_daemon() {
  DAEMON_GENERATION=$((DAEMON_GENERATION + 1))
  CAPTAIN_HOME="$HOME_DIR" OPENAI_API_KEY="certification-local-only" \
  TELEGRAM_BOT_TOKEN="certification-telegram-token" \
    "$CAPTAIN_BIN" --config "$CONFIG" start --yolo \
      >>"$WORKDIR/daemon-$DAEMON_GENERATION.log" 2>&1 &
  DAEMON_PID=$!
  wait_for_health "$WORKDIR/health-$DAEMON_GENERATION.json" \
    || fail "Captain daemon generation $DAEMON_GENERATION did not become healthy"
  pass "Captain daemon generation $DAEMON_GENERATION is healthy"
}

load_captain_agent_id() {
  http_request GET "/api/agents" "" "$WORKDIR/agents.json"
  assert_status 200 "list isolated agents" "$WORKDIR/agents.json"
  CAPTAIN_AGENT_ID="$(jq -r '.[] | select(.name == "captain") | .id' "$WORKDIR/agents.json")"
  [[ -n "$CAPTAIN_AGENT_ID" && "$CAPTAIN_AGENT_ID" != "null" ]] || fail "captain agent missing"
  [[ "$(jq 'length' "$WORKDIR/agents.json")" == "1" ]] || fail "fresh certification home created extra agents"
  pass "fresh home contains only the captain agent"
}

approve_initial_capabilities() {
  http_request GET "/api/capabilities/native?scope=global" "" "$WORKDIR/initial-capabilities.json"
  assert_status 200 "list discovered certification capabilities" "$WORKDIR/initial-capabilities.json"
  while IFS= read -r row; do
    local name
    name="$(printf '%s' "$row" | jq -r '.name')"
    printf '%s\n' "$row" >"$WORKDIR/initial-$name-pending.json"
    decide_pending_capability "$WORKDIR/initial-$name-pending.json" approve global "" "initial-$name"
  done < <(jq -c '.capabilities[] | select(.human_action_required)' "$WORKDIR/initial-capabilities.json")
  wait_for_api_jq "/api/capabilities/native?scope=global" \
    '[.capabilities[] | select(.name | startswith("cert-"))] as $items | ($items | length) == 10 and all($items[]; .ready)' \
    "$WORKDIR/initial-capabilities-ready.json" || fail "initial capabilities did not all become ready"
  pass "ten dropped sources are discovered and ready after exact approvals"
}

certify_agent_run() {
  local scenario="$1"
  local capability="$2"
  local expected_status="$3"
  local expected_error="$4"
  local response="$WORKDIR/scenario-$scenario-response.json"
  local runs="$WORKDIR/scenario-$scenario-runs.json"
  local run="$WORKDIR/scenario-$scenario-run.json"
  local node_identity_filter
  if [[ "$expected_error" == "true" ]]; then
    node_identity_filter='all(.nodes[]; .tool_use_id == null and .attempts == 0 and .started_at == null)'
  else
    node_identity_filter='all(.nodes[]; (.tool_use_id | type) == "string")'
  fi
  run_agent_scenario "$scenario" "Execute the named native certification capability." "$response"
  assert_jq "$response" 'any(.tool_calls[]; .name == "capability_search" and (.is_error | not))' \
    "$scenario uses native capability discovery"
  assert_jq "$response" \
    "any(.tool_calls[]; .name == \"cap_cert_${capability//-/_}\" and .is_error == $expected_error)" \
    "$scenario dispatches the expected native tool"
  wait_for_latest_run_status "cert-$capability" "$expected_status" "$runs" 120 \
    || fail "$scenario did not reach $expected_status"
  extract_latest_run "$runs" "cert-$capability" "$run"
  assert_jq "$run" \
    ".status == \"$expected_status\" and (.source_hash | length) == 64 and (.nodes | length) > 0 and $node_identity_filter" \
    "$scenario persists hash, nodes, dispatch evidence, and terminal state"
  http_request GET "/api/capabilities/native/cert-$capability?scope=global" "" \
    "$WORKDIR/scenario-$scenario-capability.json"
  assert_status 200 "inspect source hash for $scenario" "$WORKDIR/scenario-$scenario-capability.json"
  local active_hash run_hash
  active_hash="$(jq -r '.active_hash' "$WORKDIR/scenario-$scenario-capability.json")"
  run_hash="$(jq -r '.source_hash' "$run")"
  [[ "$active_hash" == "$run_hash" ]] || fail "$scenario run is not pinned to its active source hash"
  pass "$scenario run is pinned to the exact active source"
}

certify_nonzero_command_failure() {
  local manifest="$WORKSPACE/cert-repo/Cargo.toml"
  local valid_manifest="$WORKDIR/cert-repo-valid-Cargo.toml"
  cp "$manifest" "$valid_manifest"
  sed '/^\[workspace\]$/d' "$valid_manifest" >"$manifest"

  run_agent_scenario cargo "Reject a real non-zero package command." \
    "$WORKDIR/scenario-cargo-nonzero-response.json"
  assert_jq "$WORKDIR/scenario-cargo-nonzero-response.json" \
    'any(.tool_calls[]; .name == "cap_cert_cargo" and .is_error == true)' \
    "non-zero cargo result is returned as a CapSpec tool error"
  wait_for_latest_run_status cert-cargo failed \
    "$WORKDIR/scenario-cargo-nonzero-runs.json" 120 \
    || fail "non-zero cargo command did not fail its durable run"
  extract_latest_run "$WORKDIR/scenario-cargo-nonzero-runs.json" cert-cargo \
    "$WORKDIR/scenario-cargo-nonzero-run.json"
  assert_jq "$WORKDIR/scenario-cargo-nonzero-run.json" \
    '.status == "failed" and .nodes[0].status == "failed" and .nodes[0].attempts == 1 and (.nodes[0].tool_use_id | type) == "string"' \
    "non-zero cargo exit is durably classified as a failed dispatched node"
  capture_tool_transcript cargo-nonzero 'Exit code: 101' \
    "non-zero cargo transcript preserves the real exit evidence"

  cp "$valid_manifest" "$manifest"
}

certify_basic_matrix() {
  certify_agent_run parallel parallel succeeded false
  assert_jq "$WORKDIR/scenario-parallel-run.json" \
    '.nodes[1].started_at < .nodes[0].finished_at and .nodes[0].started_at < .nodes[1].finished_at' \
    "independent repository reads overlap in real time"
  certify_agent_run transform transform succeeded false
  capture_tool_transcript transform '2999' \
    "structured invoice result is present in the real tool transcript"
  certify_agent_run write write succeeded false
  grep -Fx 'customer=Acme' "$WORKSPACE/cert-work/customer-status.txt" >/dev/null
  grep -Fx 'status=verified' "$WORKSPACE/cert-work/customer-status.txt" >/dev/null
  pass "controlled write is observable after its dependency barrier"
  certify_nonzero_command_failure
  certify_agent_run cargo cargo succeeded false
  capture_tool_transcript cargo 'test result: ok' \
    "real cargo test result is present in the tool transcript"
  certify_agent_run http-allowed http-allowed succeeded false
  certify_agent_run http-denied http-denied failed true
  certify_agent_run memory memory succeeded false
  certify_agent_run traversal traversal failed true
  certify_agent_run secret secret succeeded false
}

certify_cli_surface() {
  local before
  http_request GET "/api/capabilities/native/runs?limit=500" "" "$WORKDIR/cli-runs-before.json"
  before="$(jq -r '[.runs[] | select(.capability_name == "cert-parallel")][0].run_id' "$WORKDIR/cli-runs-before.json")"
  CAPTAIN_HOME="$HOME_DIR" OPENAI_API_KEY="certification-local-only" \
    "$CAPTAIN_BIN" --config "$CONFIG" message --json captain \
      '[CAPSPEC-CERT:api] Invoke the native repository inspection capability.' \
      >"$WORKDIR/cli-message.json"
  wait_for_api_jq "/api/capabilities/native/runs?limit=500" \
    "any(.runs[]; .capability_name == \"cert-parallel\" and .run_id != \"$before\" and .status == \"succeeded\")" \
    "$WORKDIR/cli-runs-after.json" || fail "CLI message did not produce a CapSpec run"
  pass "CLI one-shot message invokes a native capability"
}

certify_tui_and_control() {
  CAPTAIN_HOME="$HOME_DIR" OPENAI_API_KEY="certification-local-only" \
    CAPSPEC_CERT_API_KEY="$CERT_API_KEY" \
    python3 "$ROOT_DIR/scripts/capspec-tui-certification.py" \
      --bin "$CAPTAIN_BIN" --config "$CONFIG" --base "$BASE" \
      --artifacts "$WORKDIR/tui" --timeout 90 >"$WORKDIR/tui.stdout"
  assert_jq "$WORKDIR/tui/tui-summary.json" \
    '.status == "passed" and .native_frame_observed == true and (.run_id | length) > 0' \
    "real TUI invokes and renders native capabilities"
  CAPSPEC_CERT_BASE="$BASE" CAPSPEC_CERT_CONTROL_ARTIFACTS="$WORKDIR/control" \
    CAPSPEC_CERT_CONTROL_USERNAME="$CONTROL_USERNAME" \
    CAPSPEC_CERT_CONTROL_PASSWORD="$CONTROL_PASSWORD" \
    node "$ROOT_DIR/scripts/capspec-control-certification.mjs" >"$WORKDIR/control.stdout"
  assert_jq "$WORKDIR/control/control-summary.json" \
    '.status == "passed" and (.results | length) == 2' \
    "real Control renders capabilities and runs on desktop and Fold6"
}

certify_fresh_home_portability() {
  local saved_base="$BASE"
  local saved_agent="$CAPTAIN_AGENT_ID"
  local secondary_home="$WORKDIR/portable-home"
  local secondary_workspace="$secondary_home/workspaces/captain"
  write_config "$secondary_home" "$SECONDARY_PORT" graph false
  write_real_repo "$secondary_workspace"
  cp "$FIXTURE_DIR/cert-parallel.captain" "$secondary_home/capabilities/cert-parallel.captain"
  CAPTAIN_HOME="$secondary_home" OPENAI_API_KEY="certification-local-only" \
    "$CAPTAIN_BIN" --config "$secondary_home/config.toml" start --yolo \
      >"$WORKDIR/portable-daemon.log" 2>&1 &
  SECONDARY_PID=$!
  BASE="http://127.0.0.1:$SECONDARY_PORT"
  wait_for_health "$WORKDIR/portable-health.json" || fail "portable home daemon did not boot"
  http_request GET "/api/capabilities/native?scope=global" "" "$WORKDIR/portable-capabilities.json"
  assert_status 200 "inspect copied capability in fresh home" "$WORKDIR/portable-capabilities.json"
  assert_jq "$WORKDIR/portable-capabilities.json" \
    '(.count == 1) and .capabilities[0].name == "cert-parallel" and .capabilities[0].ready == true' \
    "one copied readable file activates in a fresh home"
  http_request GET "/api/agents" "" "$WORKDIR/portable-agents.json"
  CAPTAIN_AGENT_ID="$(jq -r '.[] | select(.name == "captain") | .id' "$WORKDIR/portable-agents.json")"
  run_agent_scenario api \
    "{\"root\":\"$secondary_workspace/cert-repo\"}" "$WORKDIR/portable-message.json"
  wait_for_latest_run_status cert-parallel succeeded "$WORKDIR/portable-runs.json" 60 \
    || fail "copied capability did not execute in fresh home"
  pass "copied capability executes through a fresh runtime database"
  kill "$SECONDARY_PID" >/dev/null 2>&1 || true
  wait "$SECONDARY_PID" >/dev/null 2>&1 || true
  SECONDARY_PID=""
  BASE="$saved_base"
  CAPTAIN_AGENT_ID="$saved_agent"
}

capture_final_evidence() {
  mkdir -p "$WORKDIR/evidence/sessions"
  http_request GET "/api/capabilities/native/runs?limit=500" "" "$WORKDIR/evidence/runs.json"
  assert_status 200 "capture final durable runs" "$WORKDIR/evidence/runs.json"
  http_request GET "/api/status" "" "$WORKDIR/evidence/status.json"
  assert_status 200 "capture final runtime status" "$WORKDIR/evidence/status.json"
  http_request GET "/api/sessions" "" "$WORKDIR/evidence/sessions.json"
  assert_status 200 "capture cross-surface sessions" "$WORKDIR/evidence/sessions.json"
  http_request GET "/api/audit/recent?limit=500" "" "$WORKDIR/evidence/audit-recent.json"
  assert_status 200 "capture security audit" "$WORKDIR/evidence/audit-recent.json"
  http_request GET "/api/audit/verify" "" "$WORKDIR/evidence/audit-verify.json"
  assert_status 200 "verify security audit chain" "$WORKDIR/evidence/audit-verify.json"
  cp "$WORKDIR/fixture/fixture-state.json" "$WORKDIR/evidence/fixture-state.json"
  if [[ -d "$WORKDIR/session-evidence" ]]; then
    cp -R "$WORKDIR/session-evidence" "$WORKDIR/evidence/"
  fi
  if [[ -d "$WORKSPACE/sessions" ]]; then
    cp "$WORKSPACE"/sessions/*.jsonl "$WORKDIR/evidence/sessions/" 2>/dev/null || true
  fi
  sqlite3 "$HOME_DIR/data/capabilities.db" 'PRAGMA integrity_check;' >"$WORKDIR/evidence/capabilities-integrity.txt"
  sqlite3 "$HOME_DIR/data/captain.db" 'PRAGMA integrity_check;' >"$WORKDIR/evidence/captain-integrity.txt"
  [[ "$(cat "$WORKDIR/evidence/capabilities-integrity.txt")" == "ok" ]] || fail "capabilities DB integrity failed"
  [[ "$(cat "$WORKDIR/evidence/captain-integrity.txt")" == "ok" ]] || fail "captain DB integrity failed"
  pass "both SQLite databases pass integrity_check"
  grep -R -F "$CERT_SECRET" "$WORKDIR/evidence" >/dev/null 2>&1 \
    && fail "raw certification secret leaked into public evidence"
  pass "raw secret is absent from API, run, session, and audit evidence"
  grep -R -F '2999' "$WORKDIR/evidence/session-evidence/transform" >/dev/null \
    || fail "structured invoice transform output is absent from session evidence"
  grep -R -F 'test result: ok' "$WORKDIR/evidence/session-evidence/cargo" >/dev/null \
    || fail "real cargo test result is absent from session evidence"

  jq -n --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg branch "$(git -C "$ROOT_DIR" branch --show-current)" \
    --arg commit "$(git -C "$ROOT_DIR" rev-parse HEAD)" \
    --argjson checks "$CAPSPEC_CERT_PASS" \
    --slurpfile runs "$WORKDIR/evidence/runs.json" \
    '{status:"passed",generated_at:$generated_at,branch:$branch,commit:$commit,checks:$checks,run_count:$runs[0].count,runs:$runs[0].runs}' \
    >"$WORKDIR/evidence/certificate.json"
  cat >"$WORKDIR/REPORT.md" <<EOF
# CapSpec Real Certification

- Status: passed
- Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)
- Branch: $(git -C "$ROOT_DIR" branch --show-current)
- Commit: $(git -C "$ROOT_DIR" rev-parse HEAD)
- Checks: $CAPSPEC_CERT_PASS
- Durable runs: $(jq '.count' "$WORKDIR/evidence/runs.json")

The evidence directory contains source hashes, run and node identities, tool-use
IDs, session tool transcripts, Telegram protocol calls, TUI and Control proofs,
audit-chain verification, and SQLite integrity results. Runtime payloads remain
absent from the public run API by design.
EOF
  find "$WORKDIR/evidence" -type f ! -name SHA256SUMS -print0 \
    | sort -z | xargs -0 shasum -a 256 >"$WORKDIR/evidence/SHA256SUMS"
}

main() {
  require_cmd cargo
  require_cmd curl
  require_cmd jq
  require_cmd node
  require_cmd python3
  require_cmd sqlite3
  require_cmd shasum
  resolve_captain_bin
  mkdir -p "$WORKDIR/sources" "$HOME_DIR/capabilities"
  note "workdir=$WORKDIR"
  note "captain_bin=$CAPTAIN_BIN"
  write_config "$HOME_DIR" "$PORT" mempalace true
  write_real_repo "$WORKSPACE"
  cp "$FIXTURE_DIR"/*.captain "$HOME_DIR/capabilities/"
  printf 'CAPSPEC_CERT_SECRET=%s\n' "$CERT_SECRET" >"$HOME_DIR/secrets.env"
  chmod 600 "$HOME_DIR/secrets.env"

  start_fixture
  start_primary_daemon
  load_captain_agent_id
  approve_initial_capabilities
  certify_basic_matrix
  certify_permission_refusal
  certify_hot_reload
  certify_project_scope
  certify_cli_surface
  certify_tui_and_control
  certify_telegram_surface
  certify_fresh_home_portability
  certify_sigkill_recovery
  capture_final_evidence
  printf '\nCapSpec real certification passed: %s checks. Artifacts: %s\n' \
    "$CAPSPEC_CERT_PASS" "$WORKDIR"
}

main
