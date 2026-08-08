#!/usr/bin/env bash
# Apply or verify Captain's public repository discovery metadata.

set -euo pipefail

REPO="${CAPTAIN_REPO:-Vivien83/captain}"
DESCRIPTION="Captain Agent OS: self-hosted, local-first autonomous AI agent runtime in Rust with persistent memory, tools, workflows, Codex, and multi-agent orchestration."
HOMEPAGE="https://captainagent.fr/"
MODE="verify"

case "${CAPTAIN_GITHUB_DISCOVERABILITY_POLICY_TEST:-}" in
    1|true|TRUE|yes|YES|y|Y) MODE="policy-test" ;;
esac

fail() {
    printf '  Error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
Usage: scripts/github-discoverability.sh [--verify|--apply|--policy-test]

The policy keeps the public GitHub description, homepage, and topic taxonomy
aligned with Captain's actual product. It improves repository discovery but
does not claim that an external search engine will index or rank the project.
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

expected_topics_json() {
    jq -cn '[
      "agent-framework",
      "agent-os",
      "agentic-ai",
      "ai-agent",
      "autonomous-agents",
      "codex",
      "discord-bot",
      "local-first",
      "llm",
      "mcp",
      "multi-agent",
      "persistent-memory",
      "rust",
      "self-hosted",
      "telegram-bot",
      "workflow-automation"
    ] | sort'
}

verify_repository_json() {
    local expected_topics
    expected_topics="$(expected_topics_json)"
    jq -e \
        --arg description "$DESCRIPTION" \
        --arg homepage "$HOMEPAGE" \
        --argjson topics "$expected_topics" '
          .private == false
          and .visibility == "public"
          and .description == $description
          and .homepage == $homepage
          and ((.topics // []) | sort) == $topics
        ' >/dev/null
}

if [ "$MODE" = "policy-test" ]; then
    jq -cn \
        --arg description "$DESCRIPTION" \
        --arg homepage "$HOMEPAGE" \
        --argjson topics "$(expected_topics_json)" '
          {
            private: false,
            visibility: "public",
            description: $description,
            homepage: $homepage,
            topics: $topics
          }
        ' | verify_repository_json \
        || fail "local GitHub discoverability policy is inconsistent"
    printf 'GitHub discoverability policy test passed.\n'
    exit 0
fi

command -v gh >/dev/null 2>&1 || fail "gh is required"
gh auth status >/dev/null

if [ "$MODE" = "apply" ]; then
    gh api \
        --method PATCH \
        -H "Accept: application/vnd.github+json" \
        -H "X-GitHub-Api-Version: 2022-11-28" \
        "repos/$REPO" \
        -f "description=$DESCRIPTION" \
        -f "homepage=$HOMEPAGE" >/dev/null

    jq -cn --argjson names "$(expected_topics_json)" '{names: $names}' \
        | gh api \
            --method PUT \
            -H "Accept: application/vnd.github+json" \
            -H "X-GitHub-Api-Version: 2022-11-28" \
            "repos/$REPO/topics" \
            --input - >/dev/null
fi

gh api \
    -H "Accept: application/vnd.github+json" \
    -H "X-GitHub-Api-Version: 2022-11-28" \
    "repos/$REPO" \
    | verify_repository_json \
    || fail "remote repository discovery metadata does not match Captain's policy"

printf 'GitHub discoverability verified: %s\n' "$REPO"
