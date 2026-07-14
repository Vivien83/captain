#!/usr/bin/env bash
# Captain release readiness gate.
#
# This script is intentionally stricter than the normal smoke check. It is for
# maintainers before tagging or publishing a release candidate.

set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd -P)
EXPECTED_CHANGELOG="${CAPTAIN_RELEASE_CHANGELOG_VERSION:-0.1.0-alpha.1}"
ALLOW_DIRTY=0
RUN_TESTS=1
RUN_SMOKE=1
RUN_PACKAGE=0
SMOKE_MODE="core"
PUBLIC_EXPORT_TMP=""

cleanup() {
    if [ -n "$PUBLIC_EXPORT_TMP" ] && [ -d "$PUBLIC_EXPORT_TMP" ]; then
        rm -rf -- "$PUBLIC_EXPORT_TMP"
    fi
}
trap cleanup EXIT

usage() {
    cat <<EOF
Usage: scripts/release-readiness.sh [options]

Options:
  --allow-dirty          Do not fail when the git worktree has changes
  --skip-tests           Skip cargo fmt/test/build gates
  --skip-smoke           Skip live daemon API smoke
  --full-smoke           Run excellence smoke in --full mode
  --package              Build the release archive after all gates pass
  --expected-changelog V Require changelog entry V
  -h, --help             Show this help

Environment:
  CAPTAIN_RELEASE_CHANGELOG_VERSION  Expected runtime changelog entry
  CAPTAIN_VERSION                    Version used by package-release.sh
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --allow-dirty)
            ALLOW_DIRTY=1
            shift
            ;;
        --skip-tests)
            RUN_TESTS=0
            shift
            ;;
        --skip-smoke)
            RUN_SMOKE=0
            shift
            ;;
        --full-smoke)
            SMOKE_MODE="full"
            shift
            ;;
        --package)
            RUN_PACKAGE=1
            shift
            ;;
        --expected-changelog)
            if [ $# -lt 2 ] || [ -z "${2:-}" ]; then
                echo "Missing value for --expected-changelog" >&2
                exit 2
            fi
            EXPECTED_CHANGELOG="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

step() {
    printf '\n== %s\n' "$*"
}

fail() {
    echo "Release readiness failed: $*" >&2
    exit 1
}

need_cmd() {
    command -v "$1" >/dev/null 2>&1 || fail "$1 is required"
}

run() {
    step "$*"
    "$@"
}

check_worktree() {
    step "worktree"
    if [ "$ALLOW_DIRTY" = "1" ]; then
        echo "dirty worktree allowed for this run"
        return
    fi
    if [ -n "$(git -C "$ROOT_DIR" status --porcelain)" ]; then
        git -C "$ROOT_DIR" status --short
        fail "git worktree must be clean before publication; use --allow-dirty only for local rehearsal"
    fi
    echo "clean"
}

check_changelog() {
    step "runtime changelog"
    if grep -Fq "$EXPECTED_CHANGELOG" "$ROOT_DIR/docs/captain-tools/runtime-changelog.md"; then
        echo "found $EXPECTED_CHANGELOG"
    else
        fail "missing runtime changelog entry: $EXPECTED_CHANGELOG"
    fi
}

secret_scan() {
    step "secret scan"
    if ! command -v rg >/dev/null 2>&1; then
        echo "rg not found; secret scan skipped"
        return
    fi
    matches=$(rg -n --hidden \
        --glob '!Cargo.lock' \
        --glob '!target/**' \
        --glob '!dist/releases/**' \
        --glob '!dist/**/*.tar.gz' \
        --glob '!.git/**' \
        --glob '!.code-review-graph/**' \
        --glob '!.fastembed_cache/**' \
        --glob '!**/node_modules/**' \
        --glob '!**/.venv/**' \
        --glob '!**/*.png' \
        --glob '!**/*.jpg' \
        --glob '!**/*.jpeg' \
        --glob '!**/*.svg' \
        --glob '!**/*.woff2' \
        -i '(sk-ant-|sk-or-|AIza[0-9A-Za-z_-]{20,}|xox[baprs]-[0-9A-Za-z-]{10,}|gh[pousr]_[0-9A-Za-z_]{20,}|-----BEGIN (RSA |OPENSSH |EC |DSA )?PRIVATE KEY-----)' \
        "$ROOT_DIR" || true)
    matches=$(printf '%s\n' "$matches" | rg -v -i '(\.\.\.|placeholder|starts with|fake|test|example|dummy|quoted|slack|xxx|aaaa|abcdefgh|abcdefghijklmnopqrstuvwxyz|Regex::new|scan_for_secrets|PRIVATE KEY-----\\n|pii_filter\.rs|memory_policy\.rs|channel_bridge\.rs|config\.rs:.*xoxb|channel_frozen\.rs:.*xoxb|prepare-github-export\.sh|release-readiness\.sh)' || true)
    if [ -n "$matches" ]; then
        printf '%s\n' "$matches"
        fail "possible secret found"
    fi
    direct_config_matches=$(rg -n --hidden \
        --glob '!target/**' \
        --glob '!dist/releases/**' \
        --glob '!dist/**/*.tar.gz' \
        --glob '!.git/**' \
        --glob '!.code-review-graph/**' \
        --glob '!.fastembed_cache/**' \
        --glob '!**/node_modules/**' \
        --glob '!**/.venv/**' \
        --glob '*.toml' \
        --glob '*.toml.example' \
        -i "^[[:space:]]*(api_key|shared_secret|password_hash)[[:space:]]*=[[:space:]]*(\"[^\"]*[[:graph:]][^\"]*\"|'[^']*[[:graph:]][^']*'|[^\"'[:space:]#][^#]*)" \
        "$ROOT_DIR" || true)
    if [ -n "$direct_config_matches" ]; then
        printf '%s\n' "$direct_config_matches"
        fail "direct secret assignment in release TOML"
    fi
    echo "ok"
}

docs_audit() {
    run "$ROOT_DIR/scripts/docs-global-audit.sh"
    run "$ROOT_DIR/scripts/docs-release-audit.sh"
}

release_workflow_audit() {
    run "$ROOT_DIR/scripts/release-workflow-audit.sh"
}

dependency_audit() {
    run "$ROOT_DIR/scripts/dependency-audit.sh"
}

public_source_audit() {
    PUBLIC_EXPORT_TMP=$(mktemp -d "${TMPDIR:-/tmp}/captain-public-export.XXXXXX")
    args=(--yes --no-git "$PUBLIC_EXPORT_TMP/source")
    if [ "$ALLOW_DIRTY" = "1" ]; then
        args=(--yes --allow-dirty --no-git "$PUBLIC_EXPORT_TMP/source")
    fi
    run "$ROOT_DIR/scripts/prepare-github-export.sh" "${args[@]}"
    rm -rf -- "$PUBLIC_EXPORT_TMP"
    PUBLIC_EXPORT_TMP=""
}

run_tests() {
    run cargo fmt -- --check
    run "$ROOT_DIR/scripts/captain-graph-bindings-check.sh"
    run cargo test -p captain-runtime
    run cargo test -p captain-kernel
    run cargo test -p captain-api
    run cargo test -p captain-cli
    run env CAPTAIN_BUILD_VERSION="$EXPECTED_CHANGELOG" cargo build --release -p captain-cli
}

run_smoke() {
    step "live daemon smoke"
    if ! command -v captain >/dev/null 2>&1; then
        fail "captain command is not available"
    fi
    if ! captain status >/dev/null 2>&1; then
        fail "captain daemon is not running; start it before release smoke"
    fi
    CAPTAIN_SMOKE_STRICT_RELEASE=1 \
    CAPTAIN_SMOKE_CHANGELOG_VERSION="$EXPECTED_CHANGELOG" \
        "$ROOT_DIR/scripts/excellence-smoke.sh" "--$SMOKE_MODE" --expected-changelog "$EXPECTED_CHANGELOG"
}

package_release() {
    step "release package"
    CAPTAIN_VERSION="${CAPTAIN_VERSION:-$EXPECTED_CHANGELOG}" "$ROOT_DIR/scripts/package-release.sh"
}

cd "$ROOT_DIR"
need_cmd git
need_cmd cargo
need_cmd grep
need_cmd cargo-audit
need_cmd mktemp

check_worktree
check_changelog
secret_scan
docs_audit
release_workflow_audit
dependency_audit
public_source_audit

if [ "$RUN_TESTS" = "1" ]; then
    run_tests
else
    step "tests"
    echo "skipped"
fi

if [ "$RUN_SMOKE" = "1" ]; then
    run_smoke
else
    step "live daemon smoke"
    echo "skipped"
fi

if [ "$RUN_PACKAGE" = "1" ]; then
    package_release
fi

step "result"
echo "Captain release readiness passed for $EXPECTED_CHANGELOG"
