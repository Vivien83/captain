#!/usr/bin/env bash
# Captain release readiness gate.
#
# This script is intentionally stricter than the normal smoke check. It is for
# maintainers before tagging or publishing a release candidate.

set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd -P)
EXPECTED_CHANGELOG="${CAPTAIN_RELEASE_CHANGELOG_VERSION:-0.1.0-alpha.5}"
CARGO_PROFILE="${CAPTAIN_RELEASE_CARGO_PROFILE:-release}"
ALLOW_DIRTY=0
RUN_TESTS=1
RUN_SMOKE=1
RUN_PACKAGE=0
SMOKE_MODE="core"
PUBLIC_EXPORT_TMP=""
SMOKE_TMP=""
SMOKE_PID=""

cleanup() {
    stop_candidate_smoke_daemon
    if [ -n "$PUBLIC_EXPORT_TMP" ] && [ -d "$PUBLIC_EXPORT_TMP" ]; then
        rm -rf -- "$PUBLIC_EXPORT_TMP"
    fi
}

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
  CAPTAIN_RELEASE_CARGO_PROFILE      Cargo test profile: release (default) or dev
  CAPTAIN_RELEASE_SMOKE_PORT         Optional isolated candidate smoke port
  CAPTAIN_RELEASE_SMOKE_READY_TIMEOUT Candidate startup timeout (default: 180)
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

run_cargo_test() {
    if [ "$CARGO_PROFILE" = "release" ]; then
        run env CAPTAIN_BUILD_VERSION="$EXPECTED_CHANGELOG" cargo test --release "$@"
    else
        run env CAPTAIN_BUILD_VERSION="$EXPECTED_CHANGELOG" cargo test "$@"
    fi
}

release_target_root() {
    local configured="${CARGO_TARGET_DIR:-target}"
    case "$configured" in
        /*) printf '%s\n' "$configured" ;;
        *) printf '%s/%s\n' "$ROOT_DIR" "$configured" ;;
    esac
}

release_candidate_bin() {
    printf '%s/release/captain\n' "$(release_target_root)"
}

collect_process_descendants() {
    local parent_pid="$1"
    local child_pid
    while IFS= read -r child_pid; do
        [ -n "$child_pid" ] || continue
        collect_process_descendants "$child_pid"
        printf '%s\n' "$child_pid"
    done < <(pgrep -P "$parent_pid" 2>/dev/null || true)
}

signal_process_list() {
    local signal="$1"
    local pids="$2"
    local pid
    for pid in $pids; do
        if kill -0 "$pid" >/dev/null 2>&1; then
            kill -"$signal" "$pid" >/dev/null 2>&1 || true
        fi
    done
}

process_list_has_live_pid() {
    local pids="$1"
    local pid
    for pid in $pids; do
        if kill -0 "$pid" >/dev/null 2>&1; then
            return 0
        fi
    done
    return 1
}

stop_candidate_smoke_daemon() {
    local descendants=""
    local late_descendants=""
    if [ -n "${SMOKE_PID:-}" ]; then
        # Snapshot and terminate descendants before their parent can reparent
        # them. MemPalace bootstrap may have uv/Python workers below Captain.
        descendants="$(collect_process_descendants "$SMOKE_PID")"
        signal_process_list TERM "$descendants"
        late_descendants="$(collect_process_descendants "$SMOKE_PID")"
        if [ -n "$late_descendants" ]; then
            descendants="$descendants $late_descendants"
            signal_process_list TERM "$late_descendants"
        fi
        if kill -0 "$SMOKE_PID" >/dev/null 2>&1; then
            kill -TERM "$SMOKE_PID" >/dev/null 2>&1 || true
        fi
        for _ in $(seq 1 20); do
            if ! kill -0 "$SMOKE_PID" >/dev/null 2>&1 \
                && ! process_list_has_live_pid "$descendants"; then
                break
            fi
            sleep 0.2
        done
        signal_process_list KILL "$descendants"
        if kill -0 "$SMOKE_PID" >/dev/null 2>&1; then
            kill -KILL "$SMOKE_PID" >/dev/null 2>&1 || true
        fi
        wait "$SMOKE_PID" 2>/dev/null || true
    fi
    SMOKE_PID=""
    if [ -n "${SMOKE_TMP:-}" ] && [ -d "$SMOKE_TMP" ]; then
        rm -rf -- "$SMOKE_TMP"
    fi
    SMOKE_TMP=""
}

# Argument-only exits happen before this function is defined and allocate no
# temporary state. Register cleanup only once every cleanup dependency exists.
trap cleanup EXIT

start_candidate_smoke_daemon() {
    local candidate_bin="$1"
    local port="$2"
    local home_dir="$SMOKE_TMP/home"
    local config="$home_dir/config.toml"
    local log="$SMOKE_TMP/daemon.log"

    if curl -sS --connect-timeout 1 --max-time 2 \
        "http://127.0.0.1:$port/api/health" >/dev/null 2>&1; then
        fail "isolated candidate smoke port is already in use: $port"
    fi

    mkdir -p "$home_dir/data"
    cat >"$config" <<EOF
home_dir = "$home_dir"
data_dir = "$home_dir/data"
log_level = "info"
api_listen = "127.0.0.1:$port"
network_enabled = false
api_key = ""
mode = "stable"
language = "en"

[default_model]
provider = "codex"
model = "gpt-5.5"
api_key_env = ""

[memory]
backend = "mempalace"

[approval]
require_approval = []
EOF

    CAPTAIN_MEMPALACE_INSTALL=1 CAPTAIN_HOME="$home_dir" \
        "$candidate_bin" start --config "$config" --yolo >"$log" 2>&1 &
    SMOKE_PID="$!"
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
    run_cargo_test -p captain-runtime
    run_cargo_test -p captain-kernel
    run_cargo_test -p captain-api
    run_cargo_test -p captain-cli
    run env CAPTAIN_BUILD_VERSION="$EXPECTED_CHANGELOG" cargo build --release -p captain-cli
}

run_smoke() {
    step "isolated candidate daemon smoke"
    local candidate_bin
    local actual_version
    local health
    local expected_version="captain $EXPECTED_CHANGELOG"
    local port="${CAPTAIN_RELEASE_SMOKE_PORT:-$((52000 + ($$ % 1000)))}"
    local ready_timeout="${CAPTAIN_RELEASE_SMOKE_READY_TIMEOUT:-180}"
    local base="http://127.0.0.1:$port"

    candidate_bin="$(release_candidate_bin)"
    [ -x "$candidate_bin" ] || fail "release candidate is not executable: $candidate_bin"
    actual_version="$("$candidate_bin" --version)" \
        || fail "release candidate version command failed: $candidate_bin"
    [ "$actual_version" = "$expected_version" ] \
        || fail "release candidate version mismatch: expected '$expected_version', got '$actual_version'"

    SMOKE_TMP=$(mktemp -d "${TMPDIR:-/tmp}/captain-release-smoke.XXXXXX")
    start_candidate_smoke_daemon "$candidate_bin" "$port"

    if ! CAPTAIN_API="$base" \
        CAPTAIN_HOME="$SMOKE_TMP/home" \
        CAPTAIN_SMOKE_WORKDIR="$SMOKE_TMP/artifacts" \
        CAPTAIN_SMOKE_READY_TIMEOUT="$ready_timeout" \
        CAPTAIN_SMOKE_STRICT_RELEASE=1 \
        CAPTAIN_SMOKE_CHANGELOG_VERSION="$EXPECTED_CHANGELOG" \
            "$ROOT_DIR/scripts/excellence-smoke.sh" "--$SMOKE_MODE" \
                --expected-changelog "$EXPECTED_CHANGELOG"; then
        tail -80 "$SMOKE_TMP/daemon.log" >&2 || true
        fail "isolated release candidate smoke failed"
    fi

    health="$(curl -sS --max-time 10 "$base/api/health")" \
        || fail "isolated release candidate health could not be read"
    printf '%s' "$health" | jq -e --arg version "$EXPECTED_CHANGELOG" \
        '.status == "ok" and .version == $version' >/dev/null \
        || fail "isolated release candidate health version mismatch"
    echo "candidate version verified: $EXPECTED_CHANGELOG"
    stop_candidate_smoke_daemon
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
need_cmd curl
need_cmd jq
need_cmd pgrep

case "$CARGO_PROFILE" in
    dev|release) ;;
    *) fail "CAPTAIN_RELEASE_CARGO_PROFILE must be dev or release" ;;
esac

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
