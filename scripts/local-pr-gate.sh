#!/usr/bin/env bash
# Run a pull request in a disposable Lima VM and publish an exact-SHA status.

set -euo pipefail
umask 077

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
WORKER="$ROOT_DIR/scripts/local-pr-gate-worker.sh"
BOOTSTRAP="$ROOT_DIR/scripts/local-pr-vm-bootstrap.sh"
REPO="${CAPTAIN_LOCAL_PR_REPO:-Vivien83/captain}"
TRUSTED_BRANCH="${CAPTAIN_LOCAL_PR_TRUSTED_BRANCH:-main}"
STATUS_CONTEXT="captain/local-pr-gate"
PR_NUMBER=""
MODE="run"
REFRESH_BASE=0
TEST_MODE="${CAPTAIN_LOCAL_PR_TEST_MODE:-0}"
STATE_ROOT="${CAPTAIN_LOCAL_PR_STATE_DIR:-$HOME/.captain/local-pr-portal}"
MIN_FREE_GIB="${CAPTAIN_LOCAL_PR_MIN_FREE_GIB:-35}"
VM_CPUS="${CAPTAIN_LOCAL_PR_VM_CPUS:-4}"
VM_MEMORY_GIB="${CAPTAIN_LOCAL_PR_VM_MEMORY_GIB:-8}"
VM_DISK_GIB="${CAPTAIN_LOCAL_PR_VM_DISK_GIB:-32}"
GATE_TIMEOUT_SECONDS="${CAPTAIN_LOCAL_PR_GATE_TIMEOUT_SECONDS:-14400}"
MAX_RESULT_BYTES="${CAPTAIN_LOCAL_PR_MAX_RESULT_BYTES:-262144}"
GH_BIN="${CAPTAIN_LOCAL_PR_GH_BIN:-gh}"
GIT_BIN="${CAPTAIN_LOCAL_PR_GIT_BIN:-git}"
LIMA_BIN="${CAPTAIN_LOCAL_PR_LIMACTL_BIN:-limactl}"
JQ_BIN="${CAPTAIN_LOCAL_PR_JQ_BIN:-jq}"
INSTANCE=""
RAW_LOG=""
TRUSTED_BUNDLE_DIR=""
TRUSTED_BUNDLE_ARCHIVE=""
TRUSTED_MANIFEST_ID=""
LOCK_DIR=""
LOCK_OWNER=""
HEAD_SHA=""
PENDING_PUBLISHED=0
FINAL_STATUS_PUBLISHED=0

TRUSTED_CONTROLLER_FILES=(
    scripts/local-pr-gate.sh
    scripts/local-pr-gate-worker.sh
    scripts/local-pr-vm-bootstrap.sh
    scripts/local-pr-portal.sh
    scripts/guarded-exec-audit.sh
    scripts/prepare-github-export.sh
    scripts/public-release-audit.sh
    scripts/publish-release-local.sh
    scripts/github-discoverability.sh
    scripts/public-boundary-guard.sh
    scripts/check-markdown-links.mjs
    .gitleaks.toml
)

TRUSTED_GUEST_FILES=(
    scripts/local-pr-gate-worker.sh
    scripts/guarded-exec-audit.sh
    scripts/prepare-github-export.sh
    scripts/public-release-audit.sh
    scripts/publish-release-local.sh
    scripts/github-discoverability.sh
    scripts/public-boundary-guard.sh
    scripts/check-markdown-links.mjs
    .gitleaks.toml
)

fail() {
    printf 'Local PR gate failed: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
Usage:
  scripts/local-pr-gate.sh --pr NUMBER [--repo OWNER/REPO] [--branch NAME]
  scripts/local-pr-gate.sh --verify-controller [--repo OWNER/REPO]
  scripts/local-pr-gate.sh --refresh-base --pr NUMBER

The controller resolves the exact pull-request head through GitHub, runs it in
a disposable plain-mode Lima VM, re-reads the head SHA, and only then publishes
the final captain/local-pr-gate status. GitHub Actions are not used.

Environment:
  CAPTAIN_LOCAL_PR_MIN_FREE_GIB          Host free-space floor (default: 35)
  CAPTAIN_LOCAL_PR_VM_CPUS               Guest CPUs (default: 4)
  CAPTAIN_LOCAL_PR_VM_MEMORY_GIB         Guest memory GiB (default: 8)
  CAPTAIN_LOCAL_PR_VM_DISK_GIB           Guest disk GiB (default: 32)
  CAPTAIN_LOCAL_PR_GATE_TIMEOUT_SECONDS  Whole worker deadline (default: 14400)
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --pr) PR_NUMBER="${2:-}"; shift 2 ;;
        --repo) REPO="${2:-}"; shift 2 ;;
        --branch) TRUSTED_BRANCH="${2:-}"; shift 2 ;;
        --refresh-base) REFRESH_BASE=1; shift ;;
        --verify-controller) MODE="verify-controller"; shift ;;
        -h|--help) usage; exit 0 ;;
        *) fail "unknown argument: $1" ;;
    esac
done

[[ "$REPO" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || fail "invalid repository"
[[ "$TRUSTED_BRANCH" =~ ^[A-Za-z0-9._/-]+$ ]] || fail "invalid trusted branch"
[[ "$MIN_FREE_GIB" =~ ^[0-9]+$ ]] || fail "invalid free-space floor"
[[ "$VM_CPUS" =~ ^[1-9][0-9]*$ ]] || fail "invalid VM CPU count"
[[ "$VM_MEMORY_GIB" =~ ^[1-9][0-9]*$ ]] || fail "invalid VM memory"
[[ "$VM_DISK_GIB" =~ ^[1-9][0-9]*$ ]] || fail "invalid VM disk"
[[ "$GATE_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]] || fail "invalid gate timeout"
[[ "$MAX_RESULT_BYTES" =~ ^[1-9][0-9]*$ ]] || fail "invalid result log cap"

if [ "$MODE" = "run" ]; then
    [[ "$PR_NUMBER" =~ ^[1-9][0-9]*$ ]] || fail "a positive --pr number is required"
fi

if [ "$TEST_MODE" != "1" ]; then
    for test_override in \
        CAPTAIN_LOCAL_PR_GH_BIN \
        CAPTAIN_LOCAL_PR_GIT_BIN \
        CAPTAIN_LOCAL_PR_JQ_BIN \
        CAPTAIN_LOCAL_PR_LIMACTL_BIN; do
        [ -z "${!test_override:-}" ] || fail "$test_override is test-only"
    done
fi

need_command() {
    local command="$1"
    case "$command" in
        */*) [ -x "$command" ] || fail "required executable is unavailable: $command" ;;
        *) command -v "$command" >/dev/null 2>&1 || fail "$command is required" ;;
    esac
}

need_command "$GH_BIN"
need_command "$GIT_BIN"
need_command "$JQ_BIN"
need_command "$LIMA_BIN"
need_command install
need_command tar
[ -f "$WORKER" ] || fail "trusted worker is missing"
[ -f "$BOOTSTRAP" ] || fail "trusted VM bootstrap is missing"

"$GH_BIN" auth status -h github.com >/dev/null 2>&1 \
    || fail "GitHub authentication is unavailable; run 'gh auth login -h github.com'"

api_get() {
    "$GH_BIN" api \
        -H "Accept: application/vnd.github+json" \
        -H "X-GitHub-Api-Version: 2022-11-28" \
        "$1"
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    else
        shasum -a 256 "$1" | cut -d ' ' -f 1
    fi
}

verify_remote_file() {
    local relative="$1"
    local local_path="${2:-$ROOT_DIR/$relative}"
    local metadata remote_blob local_blob

    metadata="$(api_get "repos/$REPO/contents/$relative?ref=$BASE_SHA")" \
        || fail "cannot read protected controller file: $relative"
    remote_blob="$(printf '%s' "$metadata" | "$JQ_BIN" -er \
        'select(.type == "file") | .sha | select(test("^[0-9a-f]{40}$"))')" \
        || fail "invalid protected controller metadata: $relative"
    local_blob="$("$GIT_BIN" hash-object "$local_path")"
    [ "$local_blob" = "$remote_blob" ] \
        || fail "$relative does not match protected $TRUSTED_BRANCH"
}

base_metadata="$(api_get "repos/$REPO/commits/$TRUSTED_BRANCH")" \
    || fail "cannot resolve protected branch $TRUSTED_BRANCH"
BASE_SHA="$(printf '%s' "$base_metadata" | "$JQ_BIN" -er \
    '.sha | select(test("^[0-9a-f]{40}$"))')" \
    || fail "protected branch returned an invalid SHA"

if [ "$TEST_MODE" != "1" ]; then
    for trusted_file in "${TRUSTED_CONTROLLER_FILES[@]}"; do
        verify_remote_file "$trusted_file"
    done
fi

if [ "$MODE" = "verify-controller" ]; then
    printf 'Local PR controller verified against %s@%s\n' "$REPO" "$BASE_SHA"
    exit 0
fi

mkdir -p "$STATE_ROOT/locks" "$STATE_ROOT/results"
chmod 0700 "$STATE_ROOT" "$STATE_ROOT/locks" "$STATE_ROOT/results"
find "$STATE_ROOT/results" -type f -mtime +30 -delete 2>/dev/null || true
repo_key="${REPO//\//-}"
LOCK_DIR="$STATE_ROOT/locks/$repo_key-pr-$PR_NUMBER"

acquire_lock() {
    local stale_pid=""
    local stale_instance=""

    if ! mkdir "$LOCK_DIR" 2>/dev/null; then
        if [ -f "$LOCK_DIR/owner" ]; then
            stale_pid="$(sed -n 's/^pid=//p' "$LOCK_DIR/owner" | head -n 1)"
            stale_instance="$(sed -n 's/^instance=//p' "$LOCK_DIR/owner" | head -n 1)"
        fi
        if [[ "$stale_pid" =~ ^[1-9][0-9]*$ ]] && kill -0 "$stale_pid" 2>/dev/null; then
            fail "pull request $PR_NUMBER already has a local gate in progress"
        fi
        if [[ "$stale_instance" =~ ^captain-pr-${PR_NUMBER}-[0-9a-f]{10}-[0-9]+$ ]]; then
            "$LIMA_BIN" delete -f "$stale_instance" >/dev/null 2>&1 || true
        fi
        rm -rf "$LOCK_DIR"
        mkdir "$LOCK_DIR" 2>/dev/null \
            || fail "cannot recover stale lock for pull request $PR_NUMBER"
    fi

    LOCK_OWNER="$LOCK_DIR/owner"
    printf 'pid=%s\ninstance=\n' "$$" >"$LOCK_OWNER"
    chmod 0600 "$LOCK_OWNER"
}

acquire_lock

cleanup() {
    local status=$?
    local current_json=""
    local current_sha=""
    trap - EXIT INT TERM
    set +e
    if [ -n "$INSTANCE" ]; then
        "$LIMA_BIN" delete -f "$INSTANCE" >/dev/null 2>&1 || true
    fi
    [ -z "$RAW_LOG" ] || rm -f "$RAW_LOG"
    [ -z "$TRUSTED_BUNDLE_ARCHIVE" ] || rm -f "$TRUSTED_BUNDLE_ARCHIVE"
    [ -z "$TRUSTED_BUNDLE_DIR" ] || rm -rf "$TRUSTED_BUNDLE_DIR"
    if [ "$status" -ne 0 ] \
        && [ "$PENDING_PUBLISHED" = "1" ] \
        && [ "$FINAL_STATUS_PUBLISHED" = "0" ] \
        && type fetch_pr >/dev/null 2>&1 \
        && type validate_pr_json >/dev/null 2>&1 \
        && type post_status >/dev/null 2>&1; then
        current_json="$(fetch_pr 2>/dev/null || true)"
        if [ -n "$current_json" ] && validate_pr_json "$current_json"; then
            current_sha="$(printf '%s' "$current_json" | "$JQ_BIN" -r '.head.sha')"
            if [ "$current_sha" = "$HEAD_SHA" ]; then
                post_status "$HEAD_SHA" error \
                    "Local gate infrastructure stopped for ${HEAD_SHA:0:12}" \
                    >/dev/null 2>&1 || true
            fi
        fi
    fi
    [ -z "$LOCK_DIR" ] || rm -rf "$LOCK_DIR"
    exit "$status"
}
trap cleanup EXIT INT TERM

available_kib="$(df -Pk "$STATE_ROOT" | awk 'NR == 2 {print $4}')"
[[ "$available_kib" =~ ^[0-9]+$ ]] || fail "cannot determine host free space"
required_kib=$((MIN_FREE_GIB * 1024 * 1024))
if [ "$available_kib" -lt "$required_kib" ]; then
    fail "only $((available_kib / 1024 / 1024)) GiB free; $MIN_FREE_GIB GiB required before creating a PR VM"
fi

prepare_trusted_bundle() {
    local relative
    local destination
    local digest
    local manifest

    TRUSTED_BUNDLE_DIR="$(mktemp -d "$STATE_ROOT/results/.trusted-$PR_NUMBER-${HEAD_SHA:0:12}.XXXXXX")"
    TRUSTED_BUNDLE_ARCHIVE="$(mktemp "$STATE_ROOT/results/.trusted-$PR_NUMBER-${HEAD_SHA:0:12}.XXXXXX")"
    manifest="$TRUSTED_BUNDLE_DIR/manifest.sha256"
    : >"$manifest"

    for relative in "${TRUSTED_GUEST_FILES[@]}"; do
        destination="$TRUSTED_BUNDLE_DIR/$relative"
        mkdir -p "$(dirname "$destination")"
        install -m 0600 "$ROOT_DIR/$relative" "$destination"
        if [ "$TEST_MODE" != "1" ]; then
            verify_remote_file "$relative" "$destination"
        fi
        digest="$(sha256_file "$destination")"
        printf '%s  %s\n' "$digest" "$relative" >>"$manifest"
    done

    TRUSTED_MANIFEST_ID="$(sha256_file "$manifest")"
    tar -C "$TRUSTED_BUNDLE_DIR" -cf "$TRUSTED_BUNDLE_ARCHIVE" .
}

BOOTSTRAP_ID="$(sha256_file "$BOOTSTRAP")"
BASE_INSTANCE="captain-pr-base-v1"

instance_exists() {
    [ "$("$LIMA_BIN" list "$1" --format '{{.Name}}' 2>/dev/null || true)" = "$1" ]
}

refresh_base() {
    if instance_exists "$BASE_INSTANCE"; then
        "$LIMA_BIN" unprotect "$BASE_INSTANCE" >/dev/null 2>&1 || true
        "$LIMA_BIN" delete -f "$BASE_INSTANCE" >/dev/null
    fi
}

ensure_base() {
    local marker=""
    local created=0

    if [ "$REFRESH_BASE" = "1" ]; then
        refresh_base
    fi

    if instance_exists "$BASE_INSTANCE"; then
        "$LIMA_BIN" start --tty=false "$BASE_INSTANCE" >/dev/null
        marker="$("$LIMA_BIN" shell --tty=false "$BASE_INSTANCE" \
            cat /etc/captain-local-pr-base 2>/dev/null || true)"
        "$LIMA_BIN" stop "$BASE_INSTANCE" >/dev/null
        if [ "$marker" = "$BOOTSTRAP_ID" ]; then
            return
        fi
        refresh_base
    fi

    created=1
    if ! "$LIMA_BIN" start \
        --tty=false \
        --name="$BASE_INSTANCE" \
        --plain \
        --mount-none \
        --containerd=none \
        --cpus="$VM_CPUS" \
        --memory="$VM_MEMORY_GIB" \
        --disk="$VM_DISK_GIB" \
        --set='.ssh.forwardAgent=false | .ssh.forwardX11=false' \
        template:ubuntu-24.04; then
        "$LIMA_BIN" delete -f "$BASE_INSTANCE" >/dev/null 2>&1 || true
        fail "could not create the plain Lima base"
    fi

    "$LIMA_BIN" copy --backend=scp "$BOOTSTRAP" \
        "$BASE_INSTANCE:/tmp/captain-local-pr-vm-bootstrap.sh"
    if ! "$LIMA_BIN" shell --tty=false "$BASE_INSTANCE" \
        bash /tmp/captain-local-pr-vm-bootstrap.sh "$BOOTSTRAP_ID"; then
        [ "$created" = "0" ] || "$LIMA_BIN" delete -f "$BASE_INSTANCE" >/dev/null 2>&1 || true
        fail "Lima base provisioning failed"
    fi
    marker="$("$LIMA_BIN" shell --tty=false "$BASE_INSTANCE" \
        cat /etc/captain-local-pr-base)"
    [ "$marker" = "$BOOTSTRAP_ID" ] || fail "Lima base marker verification failed"
    "$LIMA_BIN" stop "$BASE_INSTANCE" >/dev/null
    "$LIMA_BIN" protect "$BASE_INSTANCE" >/dev/null
}

fetch_pr() {
    api_get "repos/$REPO/pulls/$PR_NUMBER"
}

validate_pr_json() {
    local json="$1"
    printf '%s' "$json" | "$JQ_BIN" -e \
        --arg repo "$REPO" \
        --arg branch "$TRUSTED_BRANCH" \
        '.state == "open"
         and .base.repo.full_name == $repo
         and .base.ref == $branch
         and (.head.sha | test("^[0-9a-f]{40}$"))' >/dev/null
}

post_status() {
    local sha="$1"
    local state="$2"
    local description="$3"
    local payload
    payload="$("$JQ_BIN" -cn \
        --arg state "$state" \
        --arg context "$STATUS_CONTEXT" \
        --arg description "$description" \
        '{state:$state,context:$context,description:$description}')" \
        || fail "cannot build GitHub status payload"
    printf '%s' "$payload" | "$GH_BIN" api \
        --method POST \
        -H "Accept: application/vnd.github+json" \
        -H "X-GitHub-Api-Version: 2022-11-28" \
        "repos/$REPO/statuses/$sha" \
        --input - >/dev/null
}

initial_pr="$(fetch_pr)" || fail "cannot resolve pull request $PR_NUMBER"
validate_pr_json "$initial_pr" \
    || fail "pull request is closed, targets another branch, or has invalid metadata"
HEAD_SHA="$(printf '%s' "$initial_pr" | "$JQ_BIN" -r '.head.sha')"

ensure_base

fresh_pr="$(fetch_pr)" || fail "cannot re-read pull request before execution"
validate_pr_json "$fresh_pr" || fail "pull request changed before execution"
fresh_sha="$(printf '%s' "$fresh_pr" | "$JQ_BIN" -r '.head.sha')"
[ "$fresh_sha" = "$HEAD_SHA" ] || fail "pull request head changed before execution; rerun"

prepare_trusted_bundle

post_status "$HEAD_SHA" pending "Local isolated gate is running for ${HEAD_SHA:0:12}"
PENDING_PUBLISHED=1

INSTANCE="captain-pr-${PR_NUMBER}-${HEAD_SHA:0:10}-$$"
printf 'pid=%s\ninstance=%s\n' "$$" "$INSTANCE" >"$LOCK_OWNER"
"$LIMA_BIN" clone "$BASE_INSTANCE" "$INSTANCE" \
    --start \
    --cpus="$VM_CPUS" \
    --memory="$VM_MEMORY_GIB" \
    --disk="$VM_DISK_GIB" >/dev/null
"$LIMA_BIN" copy --backend=scp "$TRUSTED_BUNDLE_ARCHIVE" \
    "$INSTANCE:/tmp/captain-pr-trusted.tar"
"$LIMA_BIN" shell --tty=false "$INSTANCE" \
    sudo /usr/local/sbin/captain-pr-install-trusted \
        /tmp/captain-pr-trusted.tar "$TRUSTED_MANIFEST_ID"

RAW_LOG="$(mktemp "$STATE_ROOT/results/.raw-$PR_NUMBER-${HEAD_SHA:0:12}.XXXXXX")"
set +e
"$LIMA_BIN" shell --tty=false "$INSTANCE" \
    sudo -u captain-pr -H /usr/bin/env -i \
        HOME=/home/captain-pr \
        USER=captain-pr \
        LOGNAME=captain-pr \
        LANG=C.UTF-8 \
        LC_ALL=C.UTF-8 \
        PATH=/opt/captain-pr-toolchain/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
        CARGO_HOME=/home/captain-pr/.cargo-cache \
        RUSTUP_HOME=/opt/captain-pr-toolchain/rustup \
        /usr/bin/bash /opt/captain-pr-trusted/scripts/local-pr-gate-worker.sh \
        --repo "$REPO" \
        --pr "$PR_NUMBER" \
        --head-sha "$HEAD_SHA" \
        --base-sha "$BASE_SHA" \
        --base-branch "$TRUSTED_BRANCH" \
        --bootstrap-id "$BOOTSTRAP_ID" \
        --trusted-manifest-id "$TRUSTED_MANIFEST_ID" \
        >"$RAW_LOG" 2>&1 &
worker_pid=$!
set -e

started_at="$(date +%s)"
timed_out=0
while kill -0 "$worker_pid" 2>/dev/null; do
    now="$(date +%s)"
    if [ $((now - started_at)) -ge "$GATE_TIMEOUT_SECONDS" ]; then
        timed_out=1
        kill -TERM "$worker_pid" >/dev/null 2>&1 || true
        sleep 5
        kill -KILL "$worker_pid" >/dev/null 2>&1 || true
        break
    fi
    sleep 5
done

set +e
wait "$worker_pid"
worker_status=$?
set -e
if [ "$timed_out" = "1" ]; then
    worker_status=124
fi

RESULT_LOG="$STATE_ROOT/results/pr-$PR_NUMBER-$HEAD_SHA.log"
tail -c "$MAX_RESULT_BYTES" "$RAW_LOG" \
    | LC_ALL=C tr -cd '\11\12\15\40-\176' >"$RESULT_LOG"
chmod 0600 "$RESULT_LOG"
cat "$RESULT_LOG"

current_pr="$(fetch_pr)" || fail "cannot re-read pull request after execution; no final status was published"
if ! validate_pr_json "$current_pr"; then
    printf 'Pull request is no longer eligible; no final status was published.\n' >&2
    exit 3
fi
current_sha="$(printf '%s' "$current_pr" | "$JQ_BIN" -r '.head.sha')"
if [ "$current_sha" != "$HEAD_SHA" ]; then
    printf 'Pull request advanced from %s to %s; no final status was published.\n' \
        "$HEAD_SHA" "$current_sha" >&2
    exit 3
fi

case "$worker_status" in
    0)
        post_status "$HEAD_SHA" success "Local isolated gate passed for ${HEAD_SHA:0:12}"
        FINAL_STATUS_PUBLISHED=1
        printf 'Local PR gate passed: %s#%s @ %s\n' "$REPO" "$PR_NUMBER" "$HEAD_SHA"
        ;;
    20)
        post_status "$HEAD_SHA" failure "Local isolated gate failed for ${HEAD_SHA:0:12}"
        FINAL_STATUS_PUBLISHED=1
        fail "pull request checks failed; see $RESULT_LOG"
        ;;
    124)
        post_status "$HEAD_SHA" error "Local isolated gate timed out for ${HEAD_SHA:0:12}"
        FINAL_STATUS_PUBLISHED=1
        fail "pull request gate exceeded ${GATE_TIMEOUT_SECONDS}s; see $RESULT_LOG"
        ;;
    *)
        post_status "$HEAD_SHA" error "Local gate infrastructure failed for ${HEAD_SHA:0:12}"
        FINAL_STATUS_PUBLISHED=1
        fail "local gate infrastructure failed with exit $worker_status; see $RESULT_LOG"
        ;;
esac
