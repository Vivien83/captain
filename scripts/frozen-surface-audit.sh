#!/usr/bin/env bash
# Verify frozen/non-core surfaces are not promoted by active UX or prompts.

set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd -P)
PASS=0
FAIL=0

pass() {
  PASS=$((PASS + 1))
  printf '   ok %s\n' "$1"
}

fail() {
  FAIL=$((FAIL + 1))
  printf '   FAIL %s\n' "$1" >&2
}

finish() {
  printf '\n========================================\n'
  if [[ "$FAIL" -eq 0 ]]; then
    printf 'Frozen surface audit passed: %s checks.\n' "$PASS"
    exit 0
  fi
  printf 'Frozen surface audit failed: %s failed, %s passed.\n' "$FAIL" "$PASS" >&2
  exit 1
}

run_check() {
  local label="$1"
  shift
  printf '\n== %s\n' "$label"
  printf '+'
  printf ' %q' "$@"
  printf '\n'
  if "$@"; then
    pass "$label"
  else
    fail "$label"
  fi
}

assert_no_active_match() {
  local label="$1"
  local pattern="$2"
  shift 2
  local matches
  matches=$(
    rg -n -i "$pattern" "$@" |
      rg -v -i '(assert|frozen|test_|tests|category_tabs|names\.contains|FROZEN_CHANNELS)' || true
  )
  if [[ -n "$matches" ]]; then
    fail "$label"
    printf '%s\n' "$matches" >&2
  else
    pass "$label"
  fi
}

cd "$ROOT_DIR"
trap finish EXIT

printf '== Frozen surface audit\n'
printf '   root=%s\n' "$ROOT_DIR"

assert_no_active_match \
  "TUI channel screen exposes only active channels" \
  'slack|whatsapp|matrix|teams|mattermost|irc' \
  crates/captain-cli/src/tui/screens/channels.rs \
  crates/captain-cli/src/tui/screens/channels_draw.rs

assert_no_active_match \
  "runtime prompt does not specialize frozen messaging channels" \
  'Slack mrkdwn|Matrix supports|Teams-flavored|WhatsApp markdown|IRC formatting' \
  crates/captain-runtime/src/prompt_builder*.rs

run_check "runtime surface gates hide frozen tools" \
  cargo test -p captain-runtime surface_gates

run_check "tool discovery hides frozen builtin surfaces" \
  cargo test -p captain-runtime tool_search_hides_frozen_surfaces_by_default

run_check "capability search hides frozen builtin surfaces" \
  cargo test -p captain-runtime capability_search_hides_frozen_builtin_surfaces_by_default

run_check "captain docs live contracts hide frozen surfaces" \
  cargo test -p captain-runtime live_tool_contracts_hide_frozen_surfaces

run_check "agent loop discovery does not rehydrate frozen tools" \
  cargo test -p captain-runtime discovery_does_not_rehydrate_frozen_builtin_surfaces

run_check "active channel registry keeps frozen channels out" \
  cargo test -p captain-api frozen_channels_are_known_but_not_active

run_check "whatsapp QR routes stay frozen" \
  cargo test -p captain-api whatsapp_routes

run_check "TUI channel tests keep frozen groups closed" \
  cargo test -p captain-cli channels

run_check "TUI welcome summary filters frozen channel config" \
  cargo test -p captain-cli chat_welcome_summary

run_check "release-facing docs do not revive frozen surfaces" \
  scripts/docs-release-audit.sh
