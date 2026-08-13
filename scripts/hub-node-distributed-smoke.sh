#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT_DIR"

run_exact_test() {
    package=$1
    test_name=$2
    mode=${3:-normal}
    output=$(mktemp "${TMPDIR:-/tmp}/captain-distributed-smoke.XXXXXX")
    trap 'rm -f "$output"' RETURN

    args=(cargo test -p "$package" --no-default-features --lib "$test_name")
    if [[ "$mode" == "ignored" ]]; then
        args+=(-- --exact --ignored --nocapture)
    else
        args+=(-- --exact --nocapture)
    fi
    "${args[@]}" 2>&1 | tee "$output"
    if ! grep -Eq 'test result: ok\. 1 passed; 0 failed;' "$output"; then
        printf 'distributed smoke did not execute exactly one passing test: %s\n' "$test_name" >&2
        exit 1
    fi
    rm -f "$output"
    trap - RETURN
}

printf '[distributed-smoke] production transport policy\n'
run_exact_test captain-node network::tests::production_hub_origin_is_exact_https_443_without_credentials_or_path
run_exact_test captain-node network::tests::environment_proxy_precedence_is_deterministic
run_exact_test captain-node network::tests::explicit_proxy_credentials_require_a_named_resolved_secret

printf '[distributed-smoke] real fallback transports\n'
run_exact_test captain-node link::tests::link_falls_back_with_exact_hello_and_flushes_the_durable_rail ignored
run_exact_test captain-api server::hub_node_server_tests::real_websocket_upgrade_delivers_welcome_and_durable_node_ack ignored

printf '[distributed-smoke] paired Client, Hub and Node runtime\n'
run_exact_test captain-node distributed_runtime_tests::paired_client_routes_real_node_execution_across_crash_without_duplicate ignored

printf '[distributed-smoke] PASS\n'
