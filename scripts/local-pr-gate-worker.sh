#!/usr/bin/env bash
# Trusted guest-side worker for Captain's disposable local PR gate.

set -euo pipefail
umask 077

REPO=""
PR_NUMBER=""
HEAD_SHA=""
BASE_SHA=""
BASE_BRANCH=""
BOOTSTRAP_ID=""
TRUSTED_MANIFEST_ID=""
PR_USER="captain-pr"
JOBS="${CAPTAIN_LOCAL_PR_JOBS:-4}"
STEP_TIMEOUT="${CAPTAIN_LOCAL_PR_STEP_TIMEOUT_SECONDS:-5400}"
LOG_LIMIT_BLOCKS="${CAPTAIN_LOCAL_PR_STEP_LOG_BLOCKS:-32768}"
LOG_TAIL_BYTES="${CAPTAIN_LOCAL_PR_LOG_TAIL_BYTES:-65536}"
WORK_ROOT="/tmp/captain-local-pr"
HEAD_DIR="$WORK_ROOT/head"
SEALED_SOURCE_STAGE="$WORK_ROOT/sealed-source-staging"
PUBLIC_EXPORT_STAGE="$WORK_ROOT/public-export-staging"
SEALED_ROOT="/var/lib/captain-pr-job"
SEALED_SOURCE="$SEALED_ROOT/source"
SEALED_PUBLIC_EXPORT="$SEALED_ROOT/public-export"
TRUSTED_ROOT="/opt/captain-pr-trusted"
TOOLCHAIN_ROOT="/opt/captain-pr-toolchain"
EXPECTED_PATH="$TOOLCHAIN_ROOT/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"

fail_infra() {
    printf 'Local PR worker infrastructure error: %s\n' "$*" >&2
    exit 30
}

fail_gate() {
    printf 'Local PR gate failed: %s\n' "$*" >&2
    exit 20
}

usage() {
    cat <<'EOF'
Usage: local-pr-gate-worker.sh --repo OWNER/REPO --pr NUMBER
       --head-sha SHA --base-sha SHA --base-branch BRANCH
       --bootstrap-id SHA256 --trusted-manifest-id SHA256
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo) REPO="${2:-}"; shift 2 ;;
        --pr) PR_NUMBER="${2:-}"; shift 2 ;;
        --head-sha) HEAD_SHA="${2:-}"; shift 2 ;;
        --base-sha) BASE_SHA="${2:-}"; shift 2 ;;
        --base-branch) BASE_BRANCH="${2:-}"; shift 2 ;;
        --bootstrap-id) BOOTSTRAP_ID="${2:-}"; shift 2 ;;
        --trusted-manifest-id) TRUSTED_MANIFEST_ID="${2:-}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) fail_infra "unknown argument: $1" ;;
    esac
done

[[ "$REPO" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] \
    || fail_infra "invalid repository"
[[ "$PR_NUMBER" =~ ^[1-9][0-9]*$ ]] || fail_infra "invalid pull request number"
[[ "$HEAD_SHA" =~ ^[0-9a-f]{40}$ ]] || fail_infra "invalid pull request SHA"
[[ "$BASE_SHA" =~ ^[0-9a-f]{40}$ ]] || fail_infra "invalid base SHA"
[[ "$BASE_BRANCH" =~ ^[A-Za-z0-9._/-]+$ ]] || fail_infra "invalid base branch"
[[ "$BOOTSTRAP_ID" =~ ^[0-9a-f]{64}$ ]] || fail_infra "invalid bootstrap identity"
[[ "$TRUSTED_MANIFEST_ID" =~ ^[0-9a-f]{64}$ ]] \
    || fail_infra "invalid trusted manifest identity"
[[ "$JOBS" =~ ^[1-9][0-9]*$ ]] || fail_infra "invalid Cargo job count"
[[ "$STEP_TIMEOUT" =~ ^[1-9][0-9]*$ ]] || fail_infra "invalid step timeout"
[[ "$LOG_LIMIT_BLOCKS" =~ ^[1-9][0-9]*$ ]] || fail_infra "invalid log limit"
[[ "$LOG_TAIL_BYTES" =~ ^[1-9][0-9]*$ ]] || fail_infra "invalid log tail limit"

if [ "$(id -un)" != "$PR_USER" ]; then
    fail_infra "worker must run as the unprivileged $PR_USER account"
fi

export PATH="$EXPECTED_PATH"
export CARGO_HOME="$HOME/.cargo-cache"
export RUSTUP_HOME="$TOOLCHAIN_ROOT/rustup"

for command in cargo cargo-audit curl findmnt git gitleaks jq node rg sha256sum sudo tar timeout; do
    command -v "$command" >/dev/null 2>&1 || fail_infra "$command is unavailable"
done

[ "$(cat /etc/captain-local-pr-base 2>/dev/null || true)" = "$BOOTSTRAP_ID" ] \
    || fail_infra "the Lima base identity does not match the trusted bootstrap"

assert_root_owned_immutable() {
    local path="$1"
    [ -e "$path" ] || fail_infra "trusted path is missing: $path"
    [ "$(stat -c '%u' "$path")" = "0" ] \
        || fail_infra "trusted path is not root-owned: $path"
    [ ! -w "$path" ] || fail_infra "trusted path is writable by $PR_USER: $path"
}

for trusted_path in \
    "$TOOLCHAIN_ROOT" \
    "$TOOLCHAIN_ROOT/cargo/bin" \
    "$TOOLCHAIN_ROOT/rustup" \
    "$TRUSTED_ROOT" \
    "$TRUSTED_ROOT/manifest.sha256" \
    "$TRUSTED_ROOT/scripts/local-pr-gate-worker.sh"; do
    assert_root_owned_immutable "$trusted_path"
done
if find "$TRUSTED_ROOT" -type l -print -quit | grep -q .; then
    fail_infra "trusted bundle contains a symlink"
fi
actual_manifest="$(sha256sum "$TRUSTED_ROOT/manifest.sha256" | cut -d ' ' -f 1)"
[ "$actual_manifest" = "$TRUSTED_MANIFEST_ID" ] \
    || fail_infra "trusted bundle manifest identity mismatch"
(cd "$TRUSTED_ROOT" && sha256sum -c manifest.sha256 >/dev/null) \
    || fail_infra "trusted bundle checksum verification failed"

for secret_name in \
    ANTHROPIC_API_KEY \
    CAPTAIN_DAEMON_API_KEY \
    CAPTAIN_MASTER_KEY \
    CODEX_API_KEY \
    GH_TOKEN \
    GITHUB_TOKEN \
    OPENAI_API_KEY \
    SSH_AUTH_SOCK; do
    if [ -n "${!secret_name:-}" ]; then
        fail_infra "secret-bearing environment variable reached the guest: $secret_name"
    fi
done

if findmnt -rn -o FSTYPE,TARGET \
    | grep -Eq '^(9p|virtiofs|fuse\.sshfs)[[:space:]]'; then
    fail_infra "a host filesystem is mounted inside the PR guest"
fi

export CARGO_BUILD_JOBS="$JOBS"
export CARGO_INCREMENTAL=0
export CARGO_NET_OFFLINE=false
export CARGO_TERM_COLOR=never
export RUST_BACKTRACE=0
export RUST_TEST_THREADS="$JOBS"
export RUSTUP_TOOLCHAIN=stable

mkdir -p "$WORK_ROOT/logs"
chmod 0700 "$WORK_ROOT" "$WORK_ROOT/logs"
ulimit -c 0
ulimit -u 4096

safe_tail() {
    local file="$1"
    tail -c "$LOG_TAIL_BYTES" "$file" \
        | LC_ALL=C tr -cd '\11\12\15\40-\176'
}

run_step() {
    local label="$1"
    local timeout_seconds="$2"
    shift 2
    local log="$WORK_ROOT/logs/$(printf '%s' "$label" | tr -c 'A-Za-z0-9._-' '_').log"
    local status

    printf '== %s\n' "$label"
    set +e
    (
        ulimit -f "$LOG_LIMIT_BLOCKS"
        timeout --signal=TERM --kill-after=30s "$timeout_seconds" "$@"
    ) >"$log" 2>&1
    status=$?
    set -e

    if [ "$status" -ne 0 ]; then
        safe_tail "$log" >&2
        printf '\nStep failed: %s (exit %s)\n' "$label" "$status" >&2
        return 1
    fi

    rm -f "$log"
    printf 'ok %s\n' "$label"
}

fetch_exact_ref() {
    local directory="$1"
    local ref="$2"
    local expected_sha="$3"
    local label="$4"

    run_step "$label repository init" 60 git init -q "$directory" \
        || fail_infra "cannot initialize $label repository"
    git -C "$directory" config core.hooksPath /dev/null
    git -C "$directory" config credential.helper ""
    git -C "$directory" remote add origin "https://github.com/$REPO.git"
    run_step "$label exact ref fetch" 300 \
        env GIT_TERMINAL_PROMPT=0 git -C "$directory" \
            fetch -q --no-tags --depth=1 origin "$ref" \
        || fail_infra "cannot fetch exact $label ref"
    run_step "$label detached checkout" 60 \
        git -C "$directory" checkout -q --detach FETCH_HEAD \
        || fail_infra "cannot check out exact $label ref"

    local actual_sha
    actual_sha="$(git -C "$directory" rev-parse HEAD)"
    [ "$actual_sha" = "$expected_sha" ] \
        || fail_infra "fetched $ref at $actual_sha instead of $expected_sha"
}

fetch_exact_ref "$HEAD_DIR" "refs/pull/$PR_NUMBER/head" "$HEAD_SHA" "pull request"

run_step "sealed source snapshot" 120 \
    bash -c '
        set -euo pipefail
        source_root="$1"
        destination="$2"
        mkdir -p "$destination"
        git -C "$source_root" ls-files -z \
            | tar -C "$source_root" --null -T - -cf - \
            | tar -xf - -C "$destination"
    ' _ "$HEAD_DIR" "$SEALED_SOURCE_STAGE" \
    || fail_infra "cannot snapshot the exact pull-request source"

run_step "trusted public export snapshot" "$STEP_TIMEOUT" \
    "$TRUSTED_ROOT/scripts/prepare-github-export.sh" \
        --yes \
        --no-git \
        --skip-audit \
        --source-root "$HEAD_DIR" \
        "$PUBLIC_EXPORT_STAGE" \
    || fail_gate "public source export failed"

cd "$WORK_ROOT"
run_step "locked dependency fetch" "$STEP_TIMEOUT" \
    cargo fetch --manifest-path "$HEAD_DIR/Cargo.toml" --locked \
    || fail_gate "locked dependency fetch failed"

run_step "RustSec vulnerability audit" "$STEP_TIMEOUT" \
    cargo audit \
        --file "$HEAD_DIR/Cargo.lock" \
        --ignore RUSTSEC-2026-0194 \
        --ignore RUSTSEC-2026-0195 \
    || fail_gate "RustSec found an unreviewed vulnerability"

run_step "seal exact trees and disable egress" 60 \
    sudo -n /usr/local/sbin/captain-pr-seal-and-lock \
        "$SEALED_SOURCE_STAGE" "$PUBLIC_EXPORT_STAGE" \
    || fail_infra "exact trees could not be sealed or egress stayed enabled"

for sealed_path in "$SEALED_ROOT" "$SEALED_SOURCE" "$SEALED_PUBLIC_EXPORT"; do
    assert_root_owned_immutable "$sealed_path"
done
[ -f "$SEALED_SOURCE/Cargo.lock" ] \
    || fail_infra "sealed source is incomplete"
rm -rf "$SEALED_SOURCE_STAGE" "$PUBLIC_EXPORT_STAGE"

run_step "tracked shell syntax" "$STEP_TIMEOUT" \
    bash -c '
        root="$1"
        while IFS= read -r -d "" file; do
            bash -n "$file"
        done < <(find "$root" -type f -name "*.sh" -print0)
    ' _ "$SEALED_SOURCE" \
    || fail_gate "a tracked shell script does not parse"

if sudo -n true >/dev/null 2>&1; then
    fail_infra "the untrusted worker retained sudo after sealing"
fi
if curl -fsS --connect-timeout 2 --max-time 4 https://github.com >/dev/null 2>&1; then
    fail_infra "outbound network remained available after dependency fetch"
fi

export CARGO_NET_OFFLINE=true
cd "$HEAD_DIR"

run_step "cargo fmt" "$STEP_TIMEOUT" \
    cargo fmt --manifest-path "$HEAD_DIR/Cargo.toml" --all -- --check \
    || fail_gate "cargo fmt failed"

run_step "cargo clippy" "$STEP_TIMEOUT" \
    cargo clippy \
        --manifest-path "$HEAD_DIR/Cargo.toml" \
        --workspace \
        --all-targets \
        --locked \
        --offline \
        -- \
        -D warnings \
    || fail_gate "cargo clippy failed"

run_step "cargo test workspace" "$STEP_TIMEOUT" \
    cargo test \
        --manifest-path "$HEAD_DIR/Cargo.toml" \
        --workspace \
        --locked \
        --offline \
        --no-fail-fast \
    || fail_gate "workspace tests failed"

run_step "trusted guarded execution audit" "$STEP_TIMEOUT" \
    "$TRUSTED_ROOT/scripts/guarded-exec-audit.sh" "$SEALED_SOURCE" \
    || fail_gate "guarded execution audit failed"

run_step "trusted public source audit" "$STEP_TIMEOUT" \
    "$TRUSTED_ROOT/scripts/public-release-audit.sh" \
        --policy-root "$TRUSTED_ROOT" \
        "$SEALED_PUBLIC_EXPORT" \
    || fail_gate "public source audit failed"

printf 'CAPTAIN_LOCAL_PR_RESULT sha=%s base=%s status=success\n' \
    "$HEAD_SHA" "$BASE_SHA"
exit 0
