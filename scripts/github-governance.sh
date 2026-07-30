#!/usr/bin/env bash
# Apply or verify Captain's no-hosted-CI branch protection policy.

set -euo pipefail

REPO="${CAPTAIN_REPO:-Vivien83/captain}"
BRANCH="${CAPTAIN_BRANCH:-main}"
MODE="verify"
case "${CAPTAIN_GITHUB_GOVERNANCE_POLICY_TEST:-}" in
    1|true|TRUE|yes|YES|y|Y) MODE="policy-test" ;;
esac

fail() {
    printf '  Error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
Usage: scripts/github-governance.sh [--verify|--apply|--policy-test]

The policy requires reviewed pull requests for non-admin contributors, linear
history, resolved conversations, and blocks force-pushes/deletion. It leaves
required status checks empty because Captain's mandatory release gate runs
locally and GitHub Actions are manual-only.
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --verify) MODE="verify" ;;
        --apply) MODE="apply" ;;
        --policy-test) MODE="policy-test" ;;
        -h|--help)
            usage
            exit 0
            ;;
        *) fail "unknown argument: $1" ;;
    esac
    shift
done

command -v jq >/dev/null 2>&1 || fail "jq is required"

policy_json() {
    jq -cn '{
      required_status_checks: null,
      enforce_admins: false,
      required_pull_request_reviews: {
        dismiss_stale_reviews: true,
        require_code_owner_reviews: false,
        required_approving_review_count: 1,
        require_last_push_approval: true
      },
      restrictions: null,
      required_linear_history: true,
      allow_force_pushes: false,
      allow_deletions: false,
      block_creations: false,
      required_conversation_resolution: true,
      lock_branch: false
    }'
}

verify_policy_json() {
    jq -e '
      .required_status_checks == null
      and .enforce_admins == false
      and .required_pull_request_reviews.required_approving_review_count == 1
      and .required_pull_request_reviews.dismiss_stale_reviews == true
      and .required_pull_request_reviews.require_last_push_approval == true
      and (.required_pull_request_reviews | has("bypass_pull_request_allowances") | not)
      and .required_linear_history == true
      and .allow_force_pushes == false
      and .allow_deletions == false
      and .required_conversation_resolution == true
    ' >/dev/null
}

verify_remote_response() {
    jq -e '
      .required_status_checks == null
      and .enforce_admins.enabled == false
      and .required_pull_request_reviews.required_approving_review_count == 1
      and .required_pull_request_reviews.dismiss_stale_reviews == true
      and .required_pull_request_reviews.require_last_push_approval == true
      and .required_linear_history.enabled == true
      and .allow_force_pushes.enabled == false
      and .allow_deletions.enabled == false
      and .required_conversation_resolution.enabled == true
    ' >/dev/null
}

if [ "$MODE" = "policy-test" ]; then
    policy_json | verify_policy_json || fail "local GitHub governance policy is inconsistent"
    printf 'GitHub governance policy test passed.\n'
    exit 0
fi

command -v gh >/dev/null 2>&1 || fail "gh is required"
gh auth status >/dev/null

if [ "$MODE" = "apply" ]; then
    policy_json | gh api \
        --method PUT \
        -H "Accept: application/vnd.github+json" \
        -H "X-GitHub-Api-Version: 2022-11-28" \
        "repos/$REPO/branches/$BRANCH/protection" \
        --input - >/dev/null
fi

gh api \
    -H "Accept: application/vnd.github+json" \
    -H "X-GitHub-Api-Version: 2022-11-28" \
    "repos/$REPO/branches/$BRANCH/protection" \
    | verify_remote_response \
    || fail "remote branch protection does not match Captain's local-only CI policy"

printf 'GitHub governance verified: %s branch %s\n' "$REPO" "$BRANCH"
