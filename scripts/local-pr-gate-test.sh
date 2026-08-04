#!/usr/bin/env bash
# Contract tests for the host controller and automatic local PR portal.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/captain-local-pr-test.XXXXXX")"
BASE_SHA="1111111111111111111111111111111111111111"
HEAD_SHA="2222222222222222222222222222222222222222"
NEXT_SHA="3333333333333333333333333333333333333333"

cleanup() {
    rm -rf "$TMP_ROOT"
}
trap cleanup EXIT INT TERM

fail() {
    printf 'Local PR gate contract test failed: %s\n' "$*" >&2
    exit 1
}

assert_contains() {
    local file="$1"
    local expected="$2"
    grep -Fq -- "$expected" "$file" \
        || fail "$file does not contain: $expected"
}

assert_not_contains() {
    local file="$1"
    local unexpected="$2"
    if grep -Fq -- "$unexpected" "$file"; then
        fail "$file unexpectedly contains: $unexpected"
    fi
}

MOCK_BIN="$TMP_ROOT/bin"
mkdir -p "$MOCK_BIN"

cat >"$MOCK_BIN/gh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if [ "${1:-}" = "auth" ]; then
    [ "${MOCK_AUTH_OK:-1}" = "1" ]
    exit
fi

[ "${1:-}" = "api" ] || exit 90
shift
method="GET"
endpoint=""
paginate=0
slurp=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        -H) shift 2 ;;
        --method) method="$2"; shift 2 ;;
        --input) shift 2 ;;
        --paginate) paginate=1; shift ;;
        --slurp) slurp=1; shift ;;
        *) endpoint="$1"; shift ;;
    esac
done

if [ "$method" = "POST" ]; then
    payload="$(cat)"
    state="$(printf '%s' "$payload" | jq -r '.state')"
    context="$(printf '%s' "$payload" | jq -r '.context')"
    printf '%s %s %s\n' "$endpoint" "$state" "$context" >>"$MOCK_STATE/statuses"
    printf '{}\n'
    exit
fi

case "$endpoint" in
    repos/*/commits/main)
        printf '{"sha":"%s"}\n' "$MOCK_BASE_SHA"
        ;;
    repos/*/pulls/[0-9]*)
        calls_file="$MOCK_STATE/pull_calls"
        calls="$(cat "$calls_file" 2>/dev/null || printf '0')"
        calls=$((calls + 1))
        printf '%s\n' "$calls" >"$calls_file"
        sha="$MOCK_HEAD_SHA"
        if [ "${MOCK_CHANGE_ON_PULL_CALL:-0}" -gt 0 ] \
            && [ "$calls" -ge "$MOCK_CHANGE_ON_PULL_CALL" ]; then
            sha="$MOCK_NEXT_SHA"
        fi
        printf '{"state":"open","base":{"repo":{"full_name":"Vivien83/captain"},"ref":"main"},"head":{"sha":"%s"}}\n' "$sha"
        ;;
    repos/*/pulls\?*)
        page='[{"number":1,"head":{"sha":"'"$MOCK_HEAD_SHA"'"}}]'
        if [ "$paginate" = "1" ] && [ "$slurp" = "1" ]; then
            printf '[%s]\n' "$page"
        else
            printf '%s\n' "$page"
        fi
        ;;
    repos/*/commits/*/status)
        state="${MOCK_COMMIT_STATUS:-}"
        updated="${MOCK_STATUS_UPDATED_AT:-2020-01-01T00:00:00Z}"
        if [ -z "$state" ]; then
            printf '{"statuses":[]}\n'
        else
            printf '{"statuses":[{"context":"captain/local-pr-gate","state":"%s","updated_at":"%s"}]}\n' \
                "$state" "$updated"
        fi
        ;;
    *)
        printf 'unexpected mock gh endpoint: %s\n' "$endpoint" >&2
        exit 91
        ;;
esac
EOF

cat >"$MOCK_BIN/limactl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

printf '%s\n' "$*" >>"$MOCK_STATE/lima-calls"
command_name="${1:-}"
shift || true

case "$command_name" in
    list)
        instance="${1:-}"
        [ -f "$MOCK_STATE/instance-$instance" ] && printf '%s\n' "$instance"
        ;;
    start)
        name=""
        for arg in "$@"; do
            case "$arg" in --name=*) name="${arg#--name=}" ;; esac
        done
        if [ -n "$name" ]; then
            touch "$MOCK_STATE/instance-$name"
        fi
        ;;
    shell)
        instance=""
        for arg in "$@"; do
            case "$arg" in
                --*) ;;
                *) instance="$arg"; break ;;
            esac
        done
        if printf '%s\n' "$*" | grep -Fq 'cat /etc/captain-local-pr-base'; then
            printf '%s\n' "$MOCK_BOOTSTRAP_ID"
        elif printf '%s\n' "$*" | grep -Fq 'local-pr-gate-worker.sh'; then
            printf 'mock worker result\n'
            exit "${MOCK_WORKER_STATUS:-0}"
        fi
        ;;
    clone)
        [ "${MOCK_CLONE_FAIL:-0}" != "1" ] || exit 92
        source_instance="${1:-}"
        target_instance="${2:-}"
        [ -f "$MOCK_STATE/instance-$source_instance" ] || exit 93
        touch "$MOCK_STATE/instance-$target_instance"
        ;;
    delete)
        target=""
        for arg in "$@"; do
            case "$arg" in --*) ;; *) target="$arg" ;; esac
        done
        rm -f "$MOCK_STATE/instance-$target"
        ;;
    copy|stop|protect|unprotect) ;;
    *) exit 94 ;;
esac
EOF

chmod 0700 "$MOCK_BIN/gh" "$MOCK_BIN/limactl"

cat >"$MOCK_BIN/uname" <<'EOF'
#!/usr/bin/env sh
printf 'Darwin\n'
EOF
cat >"$MOCK_BIN/launchctl" <<'EOF'
#!/usr/bin/env sh
exit 0
EOF
chmod 0700 "$MOCK_BIN/uname" "$MOCK_BIN/launchctl"

bootstrap_id() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$ROOT_DIR/scripts/local-pr-vm-bootstrap.sh" | cut -d ' ' -f 1
    else
        shasum -a 256 "$ROOT_DIR/scripts/local-pr-vm-bootstrap.sh" | cut -d ' ' -f 1
    fi
}

run_controller_case() {
    local name="$1"
    shift
    local state="$TMP_ROOT/$name"
    mkdir -p "$state"
    touch "$state/statuses" "$state/lima-calls"

    env \
        CAPTAIN_LOCAL_PR_TEST_MODE=1 \
        CAPTAIN_LOCAL_PR_GH_BIN="$MOCK_BIN/gh" \
        CAPTAIN_LOCAL_PR_LIMACTL_BIN="$MOCK_BIN/limactl" \
        CAPTAIN_LOCAL_PR_STATE_DIR="$state/controller" \
        CAPTAIN_LOCAL_PR_MIN_FREE_GIB=0 \
        MOCK_STATE="$state" \
        MOCK_BASE_SHA="$BASE_SHA" \
        MOCK_HEAD_SHA="$HEAD_SHA" \
        MOCK_NEXT_SHA="$NEXT_SHA" \
        MOCK_BOOTSTRAP_ID="$(bootstrap_id)" \
        "$@" \
        "$ROOT_DIR/scripts/local-pr-gate.sh" \
            --pr 1 \
            --repo Vivien83/captain \
            --branch main \
            >"$state/stdout" 2>"$state/stderr"
}

run_controller_case success
assert_contains "$TMP_ROOT/success/statuses" " pending captain/local-pr-gate"
assert_contains "$TMP_ROOT/success/statuses" " success captain/local-pr-gate"
assert_contains "$TMP_ROOT/success/lima-calls" "captain-pr-install-trusted"
assert_contains "$TMP_ROOT/success/lima-calls" "/usr/bin/env -i"
assert_contains "$TMP_ROOT/success/lima-calls" "/opt/captain-pr-trusted/scripts/local-pr-gate-worker.sh"
assert_not_contains "$TMP_ROOT/success/lima-calls" "bash /tmp/captain-local-pr-gate-worker.sh"

if run_controller_case gate-failure MOCK_WORKER_STATUS=20; then
    fail "worker gate failure was accepted"
fi
assert_contains "$TMP_ROOT/gate-failure/statuses" " failure captain/local-pr-gate"
assert_not_contains "$TMP_ROOT/gate-failure/statuses" " success captain/local-pr-gate"

if run_controller_case clone-failure MOCK_CLONE_FAIL=1; then
    fail "clone infrastructure failure was accepted"
fi
assert_contains "$TMP_ROOT/clone-failure/statuses" " pending captain/local-pr-gate"
assert_contains "$TMP_ROOT/clone-failure/statuses" " error captain/local-pr-gate"

if run_controller_case moved-head MOCK_CHANGE_ON_PULL_CALL=3; then
    fail "a moved pull-request head was accepted"
fi
assert_contains "$TMP_ROOT/moved-head/statuses" " pending captain/local-pr-gate"
assert_not_contains "$TMP_ROOT/moved-head/statuses" " success captain/local-pr-gate"
assert_not_contains "$TMP_ROOT/moved-head/statuses" " error captain/local-pr-gate"

stale_state="$TMP_ROOT/stale-lock"
mkdir -p "$stale_state/controller/locks/Vivien83-captain-pr-1"
printf 'pid=999999\ninstance=captain-pr-1-2222222222-999999\n' \
    >"$stale_state/controller/locks/Vivien83-captain-pr-1/owner"
touch "$stale_state/statuses" "$stale_state/lima-calls"
env \
    CAPTAIN_LOCAL_PR_TEST_MODE=1 \
    CAPTAIN_LOCAL_PR_GH_BIN="$MOCK_BIN/gh" \
    CAPTAIN_LOCAL_PR_LIMACTL_BIN="$MOCK_BIN/limactl" \
    CAPTAIN_LOCAL_PR_STATE_DIR="$stale_state/controller" \
    CAPTAIN_LOCAL_PR_MIN_FREE_GIB=0 \
    MOCK_STATE="$stale_state" \
    MOCK_BASE_SHA="$BASE_SHA" \
    MOCK_HEAD_SHA="$HEAD_SHA" \
    MOCK_NEXT_SHA="$NEXT_SHA" \
    MOCK_BOOTSTRAP_ID="$(bootstrap_id)" \
    "$ROOT_DIR/scripts/local-pr-gate.sh" --pr 1 >/dev/null
assert_contains "$stale_state/lima-calls" "delete -f captain-pr-1-2222222222-999999"

auth_state="$TMP_ROOT/auth-failure"
mkdir -p "$auth_state"
touch "$auth_state/statuses"
if env \
    CAPTAIN_LOCAL_PR_TEST_MODE=1 \
    CAPTAIN_LOCAL_PR_GH_BIN="$MOCK_BIN/gh" \
    CAPTAIN_LOCAL_PR_LIMACTL_BIN="$MOCK_BIN/limactl" \
    CAPTAIN_LOCAL_PR_STATE_DIR="$auth_state/controller" \
    MOCK_STATE="$auth_state" \
    MOCK_AUTH_OK=0 \
    "$ROOT_DIR/scripts/local-pr-gate.sh" --pr 1 \
    >"$auth_state/stdout" 2>"$auth_state/stderr"; then
    fail "missing GitHub authentication was accepted"
fi
assert_contains "$auth_state/stderr" "GitHub authentication is unavailable"
[ ! -s "$auth_state/statuses" ] || fail "auth failure published a commit status"

uninstall_home="$TMP_ROOT/uninstall-home"
mkdir -p "$uninstall_home/Library/LaunchAgents"
touch "$uninstall_home/Library/LaunchAgents/fr.captainagent.local-pr-portal.plist"
env \
    HOME="$uninstall_home" \
    PATH="$MOCK_BIN:/usr/bin:/bin" \
    CAPTAIN_LOCAL_PR_TEST_MODE=1 \
    CAPTAIN_LOCAL_PR_GATE_BIN="$TMP_ROOT/missing-controller" \
    CAPTAIN_LOCAL_PR_GH_BIN="$TMP_ROOT/missing-gh" \
    CAPTAIN_LOCAL_PR_JQ_BIN="$TMP_ROOT/missing-jq" \
    CAPTAIN_LOCAL_PR_UNAME_BIN="$MOCK_BIN/uname" \
    "$ROOT_DIR/scripts/local-pr-portal.sh" --uninstall-launchd >/dev/null
[ ! -e "$uninstall_home/Library/LaunchAgents/fr.captainagent.local-pr-portal.plist" ] \
    || fail "portal uninstall left its launchd property list behind"

cat >"$MOCK_BIN/controller" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [ "${1:-}" = "--verify-controller" ]; then
    exit 0
fi
printf '%s\n' "$*" >>"$MOCK_STATE/controller-runs"
exit "${MOCK_CONTROLLER_STATUS:-0}"
EOF
chmod 0700 "$MOCK_BIN/controller"

run_portal_case() {
    local name="$1"
    local commit_status="$2"
    local updated_at="$3"
    local expected_runs="$4"
    local state="$TMP_ROOT/portal-$name"
    mkdir -p "$state"
    touch "$state/statuses" "$state/controller-runs"

    set +e
    env \
        CAPTAIN_LOCAL_PR_TEST_MODE=1 \
        CAPTAIN_LOCAL_PR_GATE_BIN="$MOCK_BIN/controller" \
        CAPTAIN_LOCAL_PR_GH_BIN="$MOCK_BIN/gh" \
        CAPTAIN_LOCAL_PR_STATE_DIR="$state/portal" \
        MOCK_STATE="$state" \
        MOCK_BASE_SHA="$BASE_SHA" \
        MOCK_HEAD_SHA="$HEAD_SHA" \
        MOCK_NEXT_SHA="$NEXT_SHA" \
        MOCK_COMMIT_STATUS="$commit_status" \
        MOCK_STATUS_UPDATED_AT="$updated_at" \
        "$ROOT_DIR/scripts/local-pr-portal.sh" --once \
        >"$state/stdout" 2>"$state/stderr"
    portal_status=$?
    set -e
    [ "$portal_status" -eq 0 ] || fail "portal case $name failed"

    actual_runs="$(wc -l <"$state/controller-runs" | tr -d ' ')"
    [ "$actual_runs" = "$expected_runs" ] \
        || fail "portal case $name ran $actual_runs controller(s), expected $expected_runs"
}

run_portal_case missing "" "2020-01-01T00:00:00Z" 1
run_portal_case success success "2020-01-01T00:00:00Z" 0
run_portal_case failure failure "2020-01-01T00:00:00Z" 0
run_portal_case stale-pending pending "2020-01-01T00:00:00Z" 1
run_portal_case old-error error "2020-01-01T00:00:00Z" 1
run_portal_case fresh-pending pending "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" 0

assert_contains "$ROOT_DIR/scripts/local-pr-gate-worker.sh" 'TRUSTED_ROOT="/opt/captain-pr-trusted"'
assert_contains "$ROOT_DIR/scripts/local-pr-gate-worker.sh" 'SEALED_ROOT="/var/lib/captain-pr-job"'
assert_contains "$ROOT_DIR/scripts/local-pr-gate-worker.sh" '"$TRUSTED_ROOT/scripts/public-release-audit.sh"'
assert_not_contains "$ROOT_DIR/scripts/local-pr-gate-worker.sh" 'rsync -a "$TRUSTED_DIR/scripts/"'
assert_contains "$ROOT_DIR/scripts/local-pr-vm-bootstrap.sh" '/usr/local/sbin/captain-pr-seal-and-lock'
assert_contains "$ROOT_DIR/scripts/local-pr-vm-bootstrap.sh" 'TOOLCHAIN_ROOT="/opt/captain-pr-toolchain"'
assert_contains "$ROOT_DIR/scripts/local-pr-portal.sh" 'scripts/public-release-audit.sh'
assert_contains "$ROOT_DIR/scripts/local-pr-gate.sh" 'scripts/github-discoverability.sh'
assert_contains "$ROOT_DIR/scripts/local-pr-portal.sh" 'scripts/github-discoverability.sh'

extract_embedded_helper() {
    local marker="$1"
    local output="$2"
    awk -v marker="$marker" '
        $0 == marker { capture = 1; next }
        capture && $0 == "EOF" { exit }
        capture { print }
    ' "$ROOT_DIR/scripts/local-pr-vm-bootstrap.sh" >"$output"
    [ -s "$output" ] || fail "embedded helper was not found: $marker"
    sh -n "$output" || fail "embedded helper does not parse: $marker"
}

extract_embedded_helper "sudo tee /usr/local/sbin/captain-pr-install-trusted >/dev/null <<'EOF'" "$TMP_ROOT/captain-pr-install-trusted"
extract_embedded_helper "sudo tee /usr/local/sbin/captain-pr-seal-and-lock >/dev/null <<'EOF'" "$TMP_ROOT/captain-pr-seal-and-lock"

fixture="$TMP_ROOT/export-source"
fixture_export="$TMP_ROOT/export-result"
mkdir -p "$fixture/docs"
printf 'fixture\n' >"$fixture/README.md"
printf 'kept\n' >"$fixture/docs/keep.md"
git -C "$fixture" init -q
git -C "$fixture" add README.md docs/keep.md
git -C "$fixture" \
    -c user.name='Captain Gate Test' \
    -c user.email='captain-gate@example.invalid' \
    commit -qm fixture
"$ROOT_DIR/scripts/prepare-github-export.sh" \
    --yes \
    --no-git \
    --skip-audit \
    --source-root "$fixture" \
    "$fixture_export" >/dev/null
[ -f "$fixture_export/docs/keep.md" ] \
    || fail "trusted source-root export omitted a tracked fixture"
if "$ROOT_DIR/scripts/prepare-github-export.sh" \
    --yes \
    --skip-audit \
    --source-root "$fixture" \
    "$TMP_ROOT/unsafe-export" >/dev/null 2>&1; then
    fail "--skip-audit was accepted without --no-git"
fi

"$ROOT_DIR/scripts/guarded-exec-audit.sh" "$ROOT_DIR" >/dev/null

printf 'Local PR gate contract tests passed.\n'
