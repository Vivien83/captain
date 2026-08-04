#!/usr/bin/env bash
# Lightweight real-VM smoke for the mountless local PR execution boundary.

set -euo pipefail
umask 077

INSTANCE="captain-pr-isolation-smoke-$$"
MIN_FREE_GIB="${CAPTAIN_LOCAL_PR_SMOKE_MIN_FREE_GIB:-8}"
VM_DISK_GIB="${CAPTAIN_LOCAL_PR_SMOKE_DISK_GIB:-6}"
LIMA_BIN="${CAPTAIN_LOCAL_PR_LIMACTL_BIN:-limactl}"

fail() {
    printf 'Local PR Lima smoke failed: %s\n' "$*" >&2
    exit 1
}

cleanup() {
    local status=$?
    trap - EXIT INT TERM
    "$LIMA_BIN" delete -f "$INSTANCE" >/dev/null 2>&1 || true
    exit "$status"
}
trap cleanup EXIT INT TERM

command -v "$LIMA_BIN" >/dev/null 2>&1 || fail "limactl is required"
[[ "$MIN_FREE_GIB" =~ ^[0-9]+$ ]] || fail "invalid free-space floor"
[[ "$VM_DISK_GIB" =~ ^[1-9][0-9]*$ ]] || fail "invalid smoke disk size"

available_kib="$(df -Pk "${TMPDIR:-/tmp}" | awk 'NR == 2 {print $4}')"
[[ "$available_kib" =~ ^[0-9]+$ ]] || fail "cannot determine host free space"
required_kib=$((MIN_FREE_GIB * 1024 * 1024))
[ "$available_kib" -ge "$required_kib" ] \
    || fail "at least $MIN_FREE_GIB GiB free is required"

"$LIMA_BIN" start \
    --tty=false \
    --name="$INSTANCE" \
    --plain \
    --mount-none \
    --containerd=none \
    --cpus=1 \
    --memory=1 \
    --disk="$VM_DISK_GIB" \
    --set='.ssh.forwardAgent=false | .ssh.forwardX11=false' \
    template:ubuntu-24.04 >/dev/null

"$LIMA_BIN" shell --tty=false "$INSTANCE" bash -ec '
    if findmnt -rn -o FSTYPE,TARGET \
        | grep -Eq "^(9p|virtiofs|fuse\.sshfs)[[:space:]]"; then
        printf "host filesystem mount detected\n" >&2
        exit 1
    fi
    [ -z "${SSH_AUTH_SOCK:-}" ] || {
        printf "forwarded SSH agent detected\n" >&2
        exit 1
    }
    if systemctl is-active --quiet containerd 2>/dev/null; then
        printf "containerd unexpectedly active\n" >&2
        exit 1
    fi
    printf "guest=%s mounts=isolated ssh_agent=absent containerd=inactive\n" \
        "$(uname -m)"
'

printf 'Local PR Lima isolation smoke passed.\n'
