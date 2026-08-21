#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT_DIR"

run_exact_test() {
    package=$1
    test_target=$2
    test_name=$3
    output=$(mktemp "${TMPDIR:-/tmp}/captain-alpha15-smoke.XXXXXX")
    trap 'rm -f "$output"' RETURN

    cargo test -p "$package" --test "$test_target" "$test_name" \
        -- --exact --nocapture 2>&1 | tee "$output"
    if ! grep -Eq 'test result: ok\. 1 passed; 0 failed;' "$output"; then
        printf 'Alpha.15 smoke did not execute exactly one passing test: %s\n' "$test_name" >&2
        exit 1
    fi
    rm -f "$output"
    trap - RETURN
}

printf '[alpha15-smoke] lightweight dependency boundaries\n'
scripts/node-lightweight-audit.sh

printf '[alpha15-smoke] standalone Node creates no Full state\n'
scripts/node-standalone-smoke.sh

printf '[alpha15-smoke] standalone Console creates no Full state\n'
run_exact_test \
    captain-console \
    console_cli \
    fresh_console_inventory_never_creates_full_runtime_state

printf '[alpha15-smoke] distributed authority and recovery\n'
scripts/hub-node-distributed-smoke.sh

printf '[alpha15-smoke] Windows service API\n'
scripts/node-windows-service-check.sh

printf '[alpha15-smoke] PASS\n'
