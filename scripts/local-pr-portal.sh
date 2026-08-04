#!/usr/bin/env bash
# Poll public pull requests and dispatch Captain's trusted local Lima gate.

set -euo pipefail
umask 077

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
CONTROLLER="${CAPTAIN_LOCAL_PR_GATE_BIN:-$ROOT_DIR/scripts/local-pr-gate.sh}"
REPO="${CAPTAIN_LOCAL_PR_REPO:-Vivien83/captain}"
TRUSTED_BRANCH="${CAPTAIN_LOCAL_PR_TRUSTED_BRANCH:-main}"
STATUS_CONTEXT="captain/local-pr-gate"
STATE_ROOT="${CAPTAIN_LOCAL_PR_STATE_DIR:-$HOME/.captain/local-pr-portal}"
PENDING_STALE_SECONDS="${CAPTAIN_LOCAL_PR_PENDING_STALE_SECONDS:-21600}"
ERROR_RETRY_SECONDS="${CAPTAIN_LOCAL_PR_ERROR_RETRY_SECONDS:-1800}"
LOG_LIMIT_BYTES="${CAPTAIN_LOCAL_PR_PORTAL_LOG_BYTES:-1048576}"
GH_BIN="${CAPTAIN_LOCAL_PR_GH_BIN:-gh}"
JQ_BIN="${CAPTAIN_LOCAL_PR_JQ_BIN:-jq}"
UNAME_BIN="${CAPTAIN_LOCAL_PR_UNAME_BIN:-uname}"
TEST_MODE="${CAPTAIN_LOCAL_PR_TEST_MODE:-0}"
MODE="once"
SERVICE_LABEL="fr.captainagent.local-pr-portal"
LOCK_DIR="$STATE_ROOT/portal.lock"
LOG_DIR="$STATE_ROOT/logs"
PORTAL_LOG="$LOG_DIR/portal.log"

CONTROLLER_FILES=(
    scripts/local-pr-portal.sh
    scripts/local-pr-gate.sh
    scripts/local-pr-gate-worker.sh
    scripts/local-pr-vm-bootstrap.sh
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
    printf 'Local PR portal failed: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
Usage:
  scripts/local-pr-portal.sh --once [--repo OWNER/REPO] [--branch NAME]
  scripts/local-pr-portal.sh --install-launchd [--repo OWNER/REPO] [--branch NAME]
  scripts/local-pr-portal.sh --uninstall-launchd

The portal runs pull requests sequentially through the disposable local Lima
gate. It starts missing checks, recovers stale pending checks after six hours,
and retries infrastructure errors after thirty minutes. GitHub Actions are not
used and no GitHub credential is copied into a guest.
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --once) MODE="once"; shift ;;
        --install-launchd) MODE="install"; shift ;;
        --uninstall-launchd) MODE="uninstall"; shift ;;
        --repo) REPO="${2:-}"; shift 2 ;;
        --branch) TRUSTED_BRANCH="${2:-}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) fail "unknown argument: $1" ;;
    esac
done

[[ "$REPO" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || fail "invalid repository"
[[ "$TRUSTED_BRANCH" =~ ^[A-Za-z0-9._/-]+$ ]] || fail "invalid trusted branch"
[[ "$PENDING_STALE_SECONDS" =~ ^[1-9][0-9]*$ ]] || fail "invalid pending timeout"
[[ "$ERROR_RETRY_SECONDS" =~ ^[1-9][0-9]*$ ]] || fail "invalid error retry delay"
[[ "$LOG_LIMIT_BYTES" =~ ^[1-9][0-9]*$ ]] || fail "invalid log limit"

if [ "$TEST_MODE" != "1" ]; then
    for test_override in \
        CAPTAIN_LOCAL_PR_GATE_BIN \
        CAPTAIN_LOCAL_PR_GH_BIN \
        CAPTAIN_LOCAL_PR_JQ_BIN \
        CAPTAIN_LOCAL_PR_UNAME_BIN; do
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

rotate_log() {
    local size=0
    local retained=$((LOG_LIMIT_BYTES / 2))
    [ -f "$PORTAL_LOG" ] || return 0
    size="$(wc -c <"$PORTAL_LOG" | tr -d ' ')"
    [[ "$size" =~ ^[0-9]+$ ]] || return 0
    if [ "$size" -gt "$LOG_LIMIT_BYTES" ]; then
        tail -c "$retained" "$PORTAL_LOG" >"$PORTAL_LOG.next"
        mv "$PORTAL_LOG.next" "$PORTAL_LOG"
        chmod 0600 "$PORTAL_LOG"
    fi
}

log() {
    local line
    line="$(printf '%s %s' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$*")"
    printf '%s\n' "$line"
    printf '%s\n' "$line" >>"$PORTAL_LOG"
}

install_launchd() {
    local install_root="$STATE_ROOT/controller"
    local launch_agents="$HOME/Library/LaunchAgents"
    local plist="$launch_agents/$SERVICE_LABEL.plist"
    local launch_domain="gui/$(id -u)"
    local path_value="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"
    local file
    local destination

    need_command "$UNAME_BIN"
    [ "$("$UNAME_BIN" -s)" = "Darwin" ] \
        || fail "launchd installation is only available on macOS"
    need_command launchctl
    "$CONTROLLER" --verify-controller --repo "$REPO" --branch "$TRUSTED_BRANCH"

    mkdir -p "$install_root" "$launch_agents"
    chmod 0700 "$install_root"
    for file in "${CONTROLLER_FILES[@]}"; do
        destination="$install_root/$file"
        mkdir -p "$(dirname "$destination")"
        install -m 0700 "$ROOT_DIR/$file" "$destination"
    done
    "$install_root/scripts/local-pr-gate.sh" \
        --verify-controller --repo "$REPO" --branch "$TRUSTED_BRANCH"

    case "$install_root$STATE_ROOT$REPO$TRUSTED_BRANCH" in
        *[\&\<\>\"\']*) fail "launchd values contain unsupported XML characters" ;;
    esac

    cat >"$plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$SERVICE_LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$install_root/scripts/local-pr-portal.sh</string>
    <string>--once</string>
    <string>--repo</string>
    <string>$REPO</string>
    <string>--branch</string>
    <string>$TRUSTED_BRANCH</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>
    <string>$path_value</string>
    <key>CAPTAIN_LOCAL_PR_STATE_DIR</key>
    <string>$STATE_ROOT</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>StartInterval</key>
  <integer>300</integer>
  <key>ProcessType</key>
  <string>Background</string>
  <key>StandardOutPath</key>
  <string>/dev/null</string>
  <key>StandardErrorPath</key>
  <string>/dev/null</string>
</dict>
</plist>
EOF
    chmod 0600 "$plist"
    plutil -lint "$plist" >/dev/null
    launchctl bootout "$launch_domain/$SERVICE_LABEL" >/dev/null 2>&1 || true
    launchctl bootstrap "$launch_domain" "$plist"
    launchctl enable "$launch_domain/$SERVICE_LABEL"
    printf 'Local PR portal installed: %s\n' "$plist"
}

uninstall_launchd() {
    local plist="$HOME/Library/LaunchAgents/$SERVICE_LABEL.plist"
    local launch_domain="gui/$(id -u)"

    need_command "$UNAME_BIN"
    [ "$("$UNAME_BIN" -s)" = "Darwin" ] \
        || fail "launchd removal is only available on macOS"
    need_command launchctl
    launchctl bootout "$launch_domain/$SERVICE_LABEL" >/dev/null 2>&1 || true
    rm -f "$plist"
    printf 'Local PR portal removed.\n'
}

case "$MODE" in
    uninstall) uninstall_launchd; exit 0 ;;
    install)
        need_command "$GH_BIN"
        need_command "$JQ_BIN"
        [ -x "$CONTROLLER" ] \
            || fail "trusted local PR controller is unavailable: $CONTROLLER"
        mkdir -p "$STATE_ROOT" "$LOG_DIR"
        chmod 0700 "$STATE_ROOT" "$LOG_DIR"
        install_launchd
        exit 0
        ;;
esac

need_command "$GH_BIN"
need_command "$JQ_BIN"
[ -x "$CONTROLLER" ] || fail "trusted local PR controller is unavailable: $CONTROLLER"
mkdir -p "$STATE_ROOT" "$LOG_DIR"
chmod 0700 "$STATE_ROOT" "$LOG_DIR"

"$GH_BIN" auth status -h github.com >/dev/null 2>&1 \
    || fail "GitHub authentication is unavailable; run 'gh auth login -h github.com'"
"$CONTROLLER" --verify-controller --repo "$REPO" --branch "$TRUSTED_BRANCH" \
    >/dev/null

acquire_portal_lock() {
    local stale_pid=""
    if ! mkdir "$LOCK_DIR" 2>/dev/null; then
        stale_pid="$(cat "$LOCK_DIR/pid" 2>/dev/null || true)"
        if [[ "$stale_pid" =~ ^[1-9][0-9]*$ ]] && kill -0 "$stale_pid" 2>/dev/null; then
            log "skip: another portal run is active (pid $stale_pid)"
            exit 0
        fi
        rm -rf "$LOCK_DIR"
        mkdir "$LOCK_DIR" 2>/dev/null || fail "cannot recover stale portal lock"
    fi
    printf '%s\n' "$$" >"$LOCK_DIR/pid"
}

cleanup() {
    local status=$?
    trap - EXIT INT TERM
    rm -rf "$LOCK_DIR"
    exit "$status"
}

acquire_portal_lock
trap cleanup EXIT INT TERM
rotate_log

api_get() {
    "$GH_BIN" api \
        -H "Accept: application/vnd.github+json" \
        -H "X-GitHub-Api-Version: 2022-11-28" \
        "$1"
}

iso_epoch() {
    local value="$1"
    if date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$value" '+%s' >/dev/null 2>&1; then
        date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$value" '+%s'
    else
        date -u -d "$value" '+%s'
    fi
}

pull_pages="$("$GH_BIN" api --paginate --slurp \
    -H "Accept: application/vnd.github+json" \
    -H "X-GitHub-Api-Version: 2022-11-28" \
    "repos/$REPO/pulls?state=open&base=$TRUSTED_BRANCH&per_page=100")" \
    || fail "cannot list open pull requests"
pulls="$(printf '%s' "$pull_pages" | "$JQ_BIN" -ce \
    'if type == "array" and (length == 0 or (.[0] | type) == "array")
     then add // []
     elif type == "array" then .
     else error("invalid pull request list")
     end
     | map(select(
         (.number | type) == "number"
         and (.head.sha | type) == "string"
         and (.head.sha | test("^[0-9a-f]{40}$"))
       ))
     | sort_by(.number)')" \
    || fail "GitHub returned an invalid pull request list"

now="$(date +%s)"
failures=0
scheduled=0

while IFS=$'\t' read -r pr_number head_sha; do
    [ -n "$pr_number" ] || continue
    status_json="$(api_get "repos/$REPO/commits/$head_sha/status")" \
        || {
            log "PR #$pr_number: cannot read commit status"
            failures=$((failures + 1))
            continue
        }
    latest="$(printf '%s' "$status_json" | "$JQ_BIN" -ce \
        --arg context "$STATUS_CONTEXT" \
        '[.statuses[]? | select(.context == $context)]
         | sort_by(.updated_at // .created_at)
         | last // {}')" \
        || {
            log "PR #$pr_number: invalid commit status response"
            failures=$((failures + 1))
            continue
        }
    state="$(printf '%s' "$latest" | "$JQ_BIN" -r '.state // ""')"
    updated_at="$(printf '%s' "$latest" | "$JQ_BIN" -r '.updated_at // .created_at // ""')"
    age=0
    if [ -n "$updated_at" ]; then
        updated_epoch="$(iso_epoch "$updated_at" 2>/dev/null || printf '0')"
        [[ "$updated_epoch" =~ ^[0-9]+$ ]] || updated_epoch=0
        age=$((now - updated_epoch))
        [ "$age" -ge 0 ] || age=0
    fi

    reason=""
    case "$state" in
        "") reason="missing status" ;;
        pending)
            [ "$age" -ge "$PENDING_STALE_SECONDS" ] \
                && reason="stale pending status" || true
            ;;
        error)
            [ "$age" -ge "$ERROR_RETRY_SECONDS" ] \
                && reason="retryable infrastructure error" || true
            ;;
        success|failure) ;;
        *) reason="unknown status '$state'" ;;
    esac

    if [ -z "$reason" ]; then
        log "PR #$pr_number @ ${head_sha:0:12}: skip ($state)"
        continue
    fi

    scheduled=$((scheduled + 1))
    run_log="$(mktemp "$LOG_DIR/.pr-$pr_number-${head_sha:0:12}.XXXXXX")"
    log "PR #$pr_number @ ${head_sha:0:12}: run ($reason)"
    set +e
    "$CONTROLLER" --pr "$pr_number" --repo "$REPO" --branch "$TRUSTED_BRANCH" \
        >"$run_log" 2>&1
    gate_status=$?
    set -e
    if [ "$gate_status" -eq 0 ]; then
        log "PR #$pr_number @ ${head_sha:0:12}: success"
    else
        log "PR #$pr_number @ ${head_sha:0:12}: gate exited $gate_status"
        tail -c 65536 "$run_log" \
            | LC_ALL=C tr -cd '\11\12\15\40-\176' >>"$PORTAL_LOG"
        failures=$((failures + 1))
    fi
    rm -f "$run_log"
done < <(printf '%s' "$pulls" | "$JQ_BIN" -r '.[] | [.number, .head.sha] | @tsv')

log "complete: $scheduled PR gate(s) scheduled, $failures failure(s)"
[ "$failures" -eq 0 ]
