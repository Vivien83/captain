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

    args=(cargo test -p "$package" --lib "$test_name")
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
run_exact_test captain-node link::tests::link_falls_back_through_explicit_proxy_with_exact_durable_hello ignored
run_exact_test captain-api server::hub_node_server_tests::real_websocket_upgrade_delivers_welcome_and_durable_node_ack ignored

printf '[distributed-smoke] authority isolation and immediate revocation\n'
run_exact_test captain-api server::hub_pairing_server_tests::paired_client_access_is_scoped_role_checked_and_immediately_revocable
run_exact_test captain-api server::hub_pairing_server_tests::paired_client_access_token_is_bound_to_one_hub_instance
run_exact_test captain-node pairing::tests::durable_identity_cannot_be_reused_against_another_hub ignored

printf '[distributed-smoke] local workspace authority\n'
run_exact_test captain-node execution_policy::tests::workspace_and_family_grants_are_both_required
run_exact_test captain-node-tools node_tool_runtime::tests::file_tools_reject_traversal_and_redact_local_root

printf '[distributed-smoke] paired Client, Hub and Node runtime\n'
run_exact_test captain-node distributed_runtime_tests::paired_client_routes_real_node_execution_across_crash_without_duplicate ignored

printf '[distributed-smoke] PASS\n'
