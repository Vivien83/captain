#!/usr/bin/env bash
# Captain installer — works on Linux, macOS, WSL
# Usage: curl -fsSL https://raw.githubusercontent.com/Vivien83/captain/main/scripts/install.sh | bash
#
# Environment variables:
#   CAPTAIN_INSTALL_DIR  — custom install directory (default: /usr/local/bin on root VPS, otherwise ~/.captain/bin)
#   CAPTAIN_VERSION      — install a specific version tag (default: latest)
#   CAPTAIN_DIST_BASE_URL — optional controlled release mirror using dist/releases layout
#   CAPTAIN_GITHUB_REPO   — GitHub repo for release assets (default: Vivien83/captain)
#   CAPTAIN_GITHUB_TOKEN  — optional token for private GitHub release downloads
#   CAPTAIN_BUNDLE_PATH  — install from a local precompiled .tar.gz bundle
#   CAPTAIN_BUNDLE_SHA256 — expected sha256 for CAPTAIN_BUNDLE_PATH
#   CAPTAIN_INSTALL_SOURCE — auto | local | git (default: auto)
#   CAPTAIN_PROFILE      — core | vps | desktop | full-media (default: core)
#   CAPTAIN_YES          — 1/true/yes to run non-interactively where possible
#   CAPTAIN_INSTALL_DEPS — 0/false/no to skip OS package installation
#   CAPTAIN_UPDATE_PATH  — 0/false/no to skip shell rc PATH modification
#   CAPTAIN_INSTALL_SERVICE — 0/false/no to skip, 1/true/yes to force a launch service
#   CAPTAIN_START        — 1/true/yes to start the service after setup (default: yes for vps)
#   CAPTAIN_SETUP        — ask | 1 | 0 (default: ask when /dev/tty exists)
#   CAPTAIN_SETUP_QUICK  — 1/true/yes to run `captain setup --quick`
#   CAPTAIN_SETUP_ANSWERS — optional TOML answers file for `captain setup --from-env`
#   TELEGRAM_BOT_TOKEN   — auto-detected during setup: token is validated (getMe),
#                          chat_id discovered from the bot's pending messages when
#                          possible, and a confirmation message is sent
#   CAPTAIN_ADMIN_USERNAME — dashboard admin username (default: admin)
#   CAPTAIN_ADMIN_PASSWORD — dashboard admin password (generated if unset)
#   CAPTAIN_DAEMON_API_KEY — CLI/API bearer token (generated if unset)
#   CAPTAIN_PUBLIC_URL / CAPTAIN_DOMAIN — optional VPS HTTPS URL/domain
#   CAPTAIN_WEB_TERMINAL_SHELL — 1/true/yes to expose raw shell mode in /terminal
#   CAPTAIN_EMBEDDINGS_INSTALL — 0/false/no to skip native local embeddings runtime install
#   CAPTAIN_MEMPALACE_INSTALL — 0/false/no to skip managed MemPalace (default: install)
#   CAPTAIN_VOICE_INSTALL — 1/true/yes to install native STT/TTS voice pack (default: full-media only)
#   CAPTAIN_RUN_DOCTOR   — 1/true/yes to run `captain doctor --full`

set -euo pipefail

DIST_BASE_URL="${CAPTAIN_DIST_BASE_URL:-}"
GITHUB_REPO="${CAPTAIN_GITHUB_REPO:-Vivien83/captain}"
GITHUB_BASE_URL="${CAPTAIN_GITHUB_BASE_URL:-https://github.com}"
INSTALL_SOURCE="${CAPTAIN_INSTALL_SOURCE:-auto}"
INSTALL_DIR="${CAPTAIN_INSTALL_DIR:-}"
PROFILE="${CAPTAIN_PROFILE:-}"
SETUP_RAN=0
DETECTED_LOCAL_BUNDLE=0
SCRIPT_PATH="${BASH_SOURCE[0]:-$0}"
SCRIPT_DIR=""
REPO_ROOT=""
if [ -n "$SCRIPT_PATH" ] && [ "$SCRIPT_PATH" != "bash" ] && [ "$SCRIPT_PATH" != "sh" ]; then
    case "$SCRIPT_PATH" in
        */*)
            SCRIPT_DIR=$(cd "$(dirname "$SCRIPT_PATH")" 2>/dev/null && pwd -P || true)
            if [ -n "$SCRIPT_DIR" ] && [ -d "$SCRIPT_DIR/.." ]; then
                REPO_ROOT=$(cd "$SCRIPT_DIR/.." 2>/dev/null && pwd -P || true)
            fi
            ;;
    esac
fi

fail() {
    echo "  Error: $*" >&2
    exit 1
}

is_yes() {
    case "${1:-}" in
        1|true|TRUE|yes|YES|y|Y) return 0 ;;
        *) return 1 ;;
    esac
}

should_install_deps() {
    if is_yes "${CAPTAIN_INSTALL_DEPS:-1}"; then
        return 0
    fi
    return 1
}

should_update_path() {
    case "${CAPTAIN_UPDATE_PATH:-1}" in
        0|false|FALSE|no|NO|n|N) return 1 ;;
        *) return 0 ;;
    esac
}

should_install_service() {
    if [ -n "${CAPTAIN_INSTALL_SERVICE:-}" ]; then
        is_yes "$CAPTAIN_INSTALL_SERVICE" && return 0
        return 1
    fi
    [ "$PROFILE" = "vps" ] && return 0
    return 1
}

systemd_system_available() {
    command -v systemctl >/dev/null 2>&1 || return 1
    [ -d /run/systemd/system ] || return 1
    return 0
}

systemd_user_available() {
    command -v systemctl >/dev/null 2>&1 || return 1
    systemctl --user show-environment >/dev/null 2>&1 || return 1
    return 0
}

should_start_service() {
    if [ -n "${CAPTAIN_START:-}" ]; then
        is_yes "$CAPTAIN_START" && return 0
        return 1
    fi
    [ "$PROFILE" = "vps" ] && return 0
    return 1
}

should_install_proxy() {
    case "${CAPTAIN_INSTALL_PROXY:-1}" in
        0|false|FALSE|no|NO|n|N) return 1 ;;
    esac
    [ "$PROFILE" = "vps" ] || return 1
    [ -n "${CAPTAIN_PUBLIC_URL:-}${CAPTAIN_DOMAIN:-}" ] || return 1
    return 0
}

should_install_voice() {
    case "${CAPTAIN_VOICE_INSTALL:-}" in
        1|true|TRUE|yes|YES|y|Y) return 0 ;;
        0|false|FALSE|no|NO|n|N) return 1 ;;
    esac
    [ "$PROFILE" = "full-media" ] && return 0
    return 1
}

should_install_embeddings() {
    case "${CAPTAIN_EMBEDDINGS_INSTALL:-1}" in
        0|false|FALSE|no|NO|n|N) return 1 ;;
        *) return 0 ;;
    esac
}

should_install_mempalace() {
    case "${CAPTAIN_MEMPALACE_INSTALL:-1}" in
        0|false|FALSE|no|NO|n|N) return 1 ;;
        *) return 0 ;;
    esac
}

prompt_tty() {
    prompt="$1"
    if [ ! -r /dev/tty ]; then
        return 1
    fi
    printf "%s" "$prompt" > /dev/tty
    IFS= read -r CAPTAIN_TTY_ANSWER < /dev/tty || CAPTAIN_TTY_ANSWER=""
    return 0
}

run_initial_setup() {
    setup_mode="${CAPTAIN_SETUP:-ask}"

    if is_yes "${CAPTAIN_SETUP_QUICK:-}"; then
        echo ""
        echo "  Running captain setup --quick..."
        "$INSTALL_DIR/captain" setup --quick --profile "$PROFILE" --yes
        SETUP_RAN=1
        return 0
    fi

    run_setup_from_env() {
        if [ -n "${CAPTAIN_SETUP_ANSWERS:-}" ]; then
            "$INSTALL_DIR/captain" setup --from-env --profile "$PROFILE" --answers "$CAPTAIN_SETUP_ANSWERS" --yes
        else
            "$INSTALL_DIR/captain" setup --from-env --profile "$PROFILE" --yes
        fi
        SETUP_RAN=1
    }

    case "$setup_mode" in
        0|false|FALSE|no|NO|n|N)
            return 0
            ;;
        1|true|TRUE|yes|YES|y|Y)
            echo ""
            if is_yes "${CAPTAIN_YES:-}" || [ -n "${CAPTAIN_SETUP_ANSWERS:-}" ]; then
                echo "  Running non-interactive setup from environment..."
                run_setup_from_env
            elif [ -r /dev/tty ]; then
                echo "  Running guided setup..."
                "$INSTALL_DIR/captain" setup --profile "$PROFILE" < /dev/tty
                SETUP_RAN=1
            else
                echo "  Running non-interactive setup from environment..."
                run_setup_from_env
            fi
            return 0
            ;;
        ask|"")
            if is_yes "${CAPTAIN_YES:-}" || [ -n "${CAPTAIN_SETUP_ANSWERS:-}" ]; then
                echo ""
                echo "  Running non-interactive setup from environment..."
                run_setup_from_env
                return 0
            fi
            if [ ! -r /dev/tty ]; then
                if [ "$PROFILE" = "vps" ]; then
                    echo ""
                    echo "  Running unattended VPS setup from environment/defaults..."
                    run_setup_from_env
                    return 0
                fi
                echo ""
                echo "  Setup skipped: no interactive terminal available."
                echo "  Run next: captain setup"
                return 0
            fi
            echo ""
            if prompt_tty "  Configure Captain now so it is ready at first launch? [Y/n] "; then
                case "$CAPTAIN_TTY_ANSWER" in
                    n|N|no|NO)
                        echo "  Setup skipped. Run next: captain setup"
                        return 0
                        ;;
                esac
            fi
            "$INSTALL_DIR/captain" setup --profile "$PROFILE" < /dev/tty
            SETUP_RAN=1
            return 0
            ;;
        *)
            fail "Unsupported CAPTAIN_SETUP: $setup_mode (expected ask, 1, or 0)"
            ;;
    esac
}

need_cmd() {
    command -v "$1" >/dev/null 2>&1 || return 0
    return 1
}

has_shared_lib() {
    lib="$1"
    if command -v ldconfig >/dev/null 2>&1 && ldconfig -p 2>/dev/null | grep -q "$lib"; then
        return 0
    fi
    for dir in /lib /lib64 /usr/lib /usr/lib64 /lib/* /usr/lib/*; do
        if [ -e "$dir/$lib" ]; then
            return 0
        fi
    done
    return 1
}

python_for_voice() {
    if command -v python3 >/dev/null 2>&1; then
        command -v python3
        return 0
    fi
    if command -v python >/dev/null 2>&1; then
        command -v python
        return 0
    fi
    return 1
}

python_venv_ready() {
    py=$(python_for_voice 2>/dev/null || true)
    [ -n "$py" ] || return 1
    "$py" -c 'import ensurepip, venv' >/dev/null 2>&1
}

run_privileged() {
    if [ "$(id -u)" -eq 0 ]; then
        "$@"
    elif command -v sudo >/dev/null 2>&1; then
        if is_yes "${CAPTAIN_YES:-}"; then
            if ! sudo -n true 2>/dev/null; then
                fail "Root privileges are required ('$*') but non-interactive sudo has no cached credentials. Run 'sudo -v' before installing, re-run as root, or unset CAPTAIN_YES to allow an interactive sudo password prompt."
            fi
            sudo -n "$@"
        else
            sudo "$@"
        fi
    else
        fail "Root privileges are required ('$*') but sudo is not installed and this is not running as root. Install sudo, or re-run this installer as root."
    fi
}

detect_platform() {
    case "$INSTALL_SOURCE" in
        auto|local|git) ;;
        *) fail "Unsupported CAPTAIN_INSTALL_SOURCE: $INSTALL_SOURCE (expected auto, local, or git)" ;;
    esac

    OS=$(uname -s | tr '[:upper:]' '[:lower:]')
    ARCH=$(uname -m)
    case "$ARCH" in
        x86_64|amd64) ARCH="x86_64" ;;
        aarch64|arm64) ARCH="aarch64" ;;
        *) fail "Unsupported architecture: $ARCH" ;;
    esac
    case "$OS" in
        linux) PLATFORM="${ARCH}-unknown-linux-gnu" ;;
        darwin) PLATFORM="${ARCH}-apple-darwin" ;;
        mingw*|msys*|cygwin*)
            echo ""
            echo "  For Windows, use PowerShell instead:"
            echo "    irm https://captain.sh/install.ps1 | iex"
            exit 1
            ;;
        *) fail "Unsupported OS: $OS" ;;
    esac
    case "$PROFILE" in
        core|vps|desktop|full-media) ;;
        "") ;;
        *) fail "Unsupported CAPTAIN_PROFILE: $PROFILE (expected core, vps, desktop, full-media)" ;;
    esac
}

read_json_string_field() {
    file="$1"
    key="$2"
    [ -f "$file" ] || return 1
    sed -n "s/.*\"$key\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" "$file" | head -1
}

detect_adjacent_bundle() {
    [ "$INSTALL_SOURCE" != "git" ] || return 0
    [ -z "${CAPTAIN_BUNDLE_PATH:-}" ] || return 0
    [ -n "$SCRIPT_DIR" ] || return 0

    CANDIDATE="$SCRIPT_DIR/captain-$PLATFORM.tar.gz"
    if [ ! -f "$CANDIDATE" ]; then
        CANDIDATE=""
        for archive in "$SCRIPT_DIR"/captain-*.tar.gz; do
            [ -f "$archive" ] || continue
            archive_name=$(basename "$archive")
            case "$archive_name" in
                *"$PLATFORM"*) CANDIDATE="$archive"; break ;;
            esac
        done
    fi
    [ -n "$CANDIDATE" ] || return 0

    CAPTAIN_BUNDLE_PATH="$CANDIDATE"
    export CAPTAIN_BUNDLE_PATH
    DETECTED_LOCAL_BUNDLE=1

    if [ -z "${CAPTAIN_VERSION:-}" ]; then
        for manifest in "$SCRIPT_DIR/manifest-$PLATFORM.json" "$SCRIPT_DIR/manifest.json"; do
            VERSION_FROM_MANIFEST=$(read_json_string_field "$manifest" "version" || true)
            if [ -n "$VERSION_FROM_MANIFEST" ]; then
                CAPTAIN_VERSION="$VERSION_FROM_MANIFEST"
                export CAPTAIN_VERSION
                break
            fi
        done
    fi
}

resolve_profile() {
    if [ -n "$PROFILE" ]; then
        return 0
    fi

    # A root-run local Linux bundle is the common VPS path. Make that path
    # one-command while keeping curl/GitHub installs conservative by default.
    if [ "$DETECTED_LOCAL_BUNDLE" = "1" ] \
        && [ "$OS" = "linux" ] \
        && [ "$(id -u)" -eq 0 ] \
        && command -v systemctl >/dev/null 2>&1; then
        PROFILE="vps"
    else
        PROFILE="core"
    fi
}

resolve_install_dir() {
    if [ -n "$INSTALL_DIR" ]; then
        return 0
    fi
    if [ "$PROFILE" = "vps" ] && [ "$OS" = "linux" ] && [ "$(id -u)" -eq 0 ]; then
        INSTALL_DIR="/usr/local/bin"
    else
        INSTALL_DIR="$HOME/.captain/bin"
    fi
}

install_packages() {
    should_install_deps || return 0

    MISSING=""
    for cmd in curl tar; do
        if ! command -v "$cmd" >/dev/null 2>&1; then
            MISSING="$MISSING $cmd"
        fi
    done
    if [ "$PROFILE" = "full-media" ]; then
        for cmd in python3 node; do
            if ! command -v "$cmd" >/dev/null 2>&1; then
                MISSING="$MISSING $cmd"
            fi
        done
    fi
    if should_install_voice; then
        if ! command -v python3 >/dev/null 2>&1; then
            MISSING="$MISSING python3"
        fi
        if ! python_venv_ready; then
            MISSING="$MISSING python3-venv"
        fi
        if [ "$OS" = "linux" ] && ! has_shared_lib "libsndfile.so.1"; then
            MISSING="$MISSING libsndfile.so.1"
        fi
        if ! command -v ffmpeg >/dev/null 2>&1; then
            MISSING="$MISSING ffmpeg"
        fi
    fi
    if [ "$PROFILE" = "vps" ] && should_install_service && ! command -v systemctl >/dev/null 2>&1; then
        MISSING="$MISSING systemd"
    fi
    if should_install_proxy && ! command -v caddy >/dev/null 2>&1; then
        MISSING="$MISSING caddy"
    fi
    if [ "$OS" = "linux" ] && ! has_shared_lib "libssl.so.3"; then
        MISSING="$MISSING libssl.so.3"
    fi

    [ -n "$MISSING" ] || return 0

    echo "  Missing packages/commands:$MISSING"
    if [ "$OS" = "darwin" ]; then
        if ! command -v brew >/dev/null 2>&1; then
            fail "Homebrew is required to install missing packages on macOS: $MISSING"
        fi
        BREW_PKGS=""
        echo "$MISSING" | grep -q " python3" && BREW_PKGS="$BREW_PKGS python"
        echo "$MISSING" | grep -q " python3-venv" && BREW_PKGS="$BREW_PKGS python"
        echo "$MISSING" | grep -q " node" && BREW_PKGS="$BREW_PKGS node"
        echo "$MISSING" | grep -q " libsndfile.so.1" && BREW_PKGS="$BREW_PKGS libsndfile"
        [ -n "$BREW_PKGS" ] && brew install $BREW_PKGS
        return 0
    fi

    if [ "$OS" != "linux" ]; then
        return 0
    fi

    if command -v apt-get >/dev/null 2>&1; then
        PKGS="ca-certificates curl tar"
        echo "$MISSING" | grep -q " systemd" && PKGS="$PKGS systemd"
        echo "$MISSING" | grep -q " caddy" && PKGS="$PKGS caddy"
        echo "$MISSING" | grep -q " libssl.so.3" && PKGS="$PKGS libssl3 openssl"
        [ "$PROFILE" = "full-media" ] && PKGS="$PKGS python3 nodejs"
        if should_install_voice; then
            PKGS="$PKGS python3 python3-venv python3-pip libsndfile1 ffmpeg"
        fi
        run_privileged apt-get update
        run_privileged apt-get install -y $PKGS
    elif command -v dnf >/dev/null 2>&1; then
        PKGS="ca-certificates curl tar"
        echo "$MISSING" | grep -q " systemd" && PKGS="$PKGS systemd"
        echo "$MISSING" | grep -q " caddy" && PKGS="$PKGS caddy"
        echo "$MISSING" | grep -q " libssl.so.3" && PKGS="$PKGS openssl-libs"
        [ "$PROFILE" = "full-media" ] && PKGS="$PKGS python3 nodejs"
        should_install_voice && PKGS="$PKGS python3 python3-pip libsndfile"
        run_privileged dnf install -y $PKGS
    elif command -v yum >/dev/null 2>&1; then
        PKGS="ca-certificates curl tar"
        echo "$MISSING" | grep -q " systemd" && PKGS="$PKGS systemd"
        echo "$MISSING" | grep -q " caddy" && PKGS="$PKGS caddy"
        echo "$MISSING" | grep -q " libssl.so.3" && PKGS="$PKGS openssl-libs"
        [ "$PROFILE" = "full-media" ] && PKGS="$PKGS python3 nodejs"
        should_install_voice && PKGS="$PKGS python3 python3-pip libsndfile"
        run_privileged yum install -y $PKGS
    elif command -v pacman >/dev/null 2>&1; then
        PKGS="ca-certificates curl tar"
        echo "$MISSING" | grep -q " systemd" && PKGS="$PKGS systemd"
        echo "$MISSING" | grep -q " caddy" && PKGS="$PKGS caddy"
        echo "$MISSING" | grep -q " libssl.so.3" && PKGS="$PKGS openssl"
        [ "$PROFILE" = "full-media" ] && PKGS="$PKGS python nodejs"
        should_install_voice && PKGS="$PKGS python python-pip libsndfile ffmpeg"
        run_privileged pacman -Sy --needed --noconfirm $PKGS
    elif command -v zypper >/dev/null 2>&1; then
        PKGS="ca-certificates curl tar"
        echo "$MISSING" | grep -q " systemd" && PKGS="$PKGS systemd"
        echo "$MISSING" | grep -q " caddy" && PKGS="$PKGS caddy"
        echo "$MISSING" | grep -q " libssl.so.3" && PKGS="$PKGS libopenssl3"
        [ "$PROFILE" = "full-media" ] && PKGS="$PKGS python3 nodejs"
        should_install_voice && PKGS="$PKGS python3 python3-pip libsndfile1 ffmpeg"
        run_privileged zypper install -y $PKGS
    elif command -v apk >/dev/null 2>&1; then
        PKGS="ca-certificates curl tar"
        echo "$MISSING" | grep -q " caddy" && PKGS="$PKGS caddy"
        echo "$MISSING" | grep -q " libssl.so.3" && PKGS="$PKGS openssl-libs"
        [ "$PROFILE" = "full-media" ] && PKGS="$PKGS python3 nodejs"
        should_install_voice && PKGS="$PKGS python3 py3-pip py3-virtualenv libsndfile ffmpeg"
        run_privileged apk add --no-cache $PKGS
    else
        fail "No supported package manager found to install:$MISSING"
    fi
}

install_linux_service() {
    [ "$OS" = "linux" ] || return 0
    should_install_service || return 0
    if ! command -v systemctl >/dev/null 2>&1; then
        echo "  Warning: systemctl not found; skipping service install."
        return 0
    fi

    SERVICE_NAME="captain.service"
    if [ "$(id -u)" -eq 0 ]; then
        if ! systemd_system_available; then
            echo "  Warning: systemd is not active as PID 1; skipping system service install."
            return 0
        fi
        SERVICE_PATH="/etc/systemd/system/$SERVICE_NAME"
        cat > "$SERVICE_PATH" <<EOF
[Unit]
Description=Captain Agent OS daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=HOME=$HOME
Environment=CODEX_HOME=$HOME/.codex
Environment=CAPTAIN_HOME=$HOME/.captain
Environment=ORT_DYLIB_PATH=$HOME/.captain/native/onnxruntime/libonnxruntime.so
ExecStart=$INSTALL_DIR/captain start
Restart=on-failure
RestartForceExitStatus=75
RestartSec=5
WorkingDirectory=$HOME/.captain

[Install]
WantedBy=multi-user.target
EOF
        systemctl daemon-reload
        systemctl enable "$SERVICE_NAME"
        echo "  System service installed: $SERVICE_PATH"
        echo "  Start it with: systemctl start captain"
    else
        if ! systemd_user_available; then
            echo "  Warning: user systemd is not available; skipping user service install."
            return 0
        fi
        USER_SYSTEMD_DIR="$HOME/.config/systemd/user"
        mkdir -p "$USER_SYSTEMD_DIR"
        SERVICE_PATH="$USER_SYSTEMD_DIR/$SERVICE_NAME"
        cat > "$SERVICE_PATH" <<EOF
[Unit]
Description=Captain Agent OS daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=HOME=$HOME
Environment=CODEX_HOME=$HOME/.codex
Environment=CAPTAIN_HOME=$HOME/.captain
Environment=ORT_DYLIB_PATH=$HOME/.captain/native/onnxruntime/libonnxruntime.so
ExecStart=$INSTALL_DIR/captain start
Restart=on-failure
RestartForceExitStatus=75
RestartSec=5
WorkingDirectory=$HOME/.captain

[Install]
WantedBy=default.target
EOF
        systemctl --user daemon-reload || true
        systemctl --user enable "$SERVICE_NAME" || true
        if command -v loginctl >/dev/null 2>&1; then
            run_privileged loginctl enable-linger "$(id -un)" || true
        fi
        echo "  User service installed: $SERVICE_PATH"
        echo "  Start it with: systemctl --user start captain"
	    fi
}

start_linux_service() {
    [ "$OS" = "linux" ] || return 0
    should_start_service || return 0
    should_install_service || {
        echo "  Warning: CAPTAIN_START requested but no systemd service was installed."
        return 0
    }
    if [ "$(id -u)" -eq 0 ]; then
        if ! systemd_system_available; then
            echo "  Warning: systemd is not active as PID 1; cannot start service."
            return 0
        fi
        if systemctl is-active --quiet captain.service; then
            echo "  Restarting Captain service..."
            systemctl restart captain.service || fail "Failed to restart captain.service"
        else
            echo "  Starting Captain service..."
            systemctl start captain.service || fail "Failed to start captain.service"
        fi
    else
        if ! systemd_user_available; then
            echo "  Warning: user systemd is not available; cannot start user service."
            return 0
        fi
        if systemctl --user is-active --quiet captain.service; then
            echo "  Restarting Captain user service..."
            systemctl --user restart captain.service || fail "Failed to restart user captain.service"
        else
            echo "  Starting Captain user service..."
            systemctl --user start captain.service || fail "Failed to start user captain.service"
        fi
    fi

    echo "  Waiting for Captain health..."
    i=0
    while [ "$i" -lt 20 ]; do
        if "$INSTALL_DIR/captain" health >/dev/null 2>&1; then
            echo "  Captain service is healthy."
            return 0
        fi
        sleep 1
        i=$((i + 1))
    done
    fail "Captain service started but did not become healthy. Check: captain logs daemon"
}

verify_llm_ready_after_start() {
    should_start_service || return 0
    if [ "${SETUP_RAN:-0}" != "1" ] && ! is_yes "${CAPTAIN_SETUP:-1}"; then
        return 0
    fi

    STATUS_JSON=$("$INSTALL_DIR/captain" status --json 2>/dev/null || true)
    if [ -z "$STATUS_JSON" ]; then
        echo "  Warning: unable to read Captain status after service start."
        return 0
    fi

    if printf '%s' "$STATUS_JSON" | grep -q '"llm_driver_ready"[[:space:]]*:[[:space:]]*true'; then
        echo "  LLM provider verified."
        return 0
    fi

    if printf '%s' "$STATUS_JSON" | grep -q '"llm_driver_ready"[[:space:]]*:[[:space:]]*false'; then
        echo "  Captain service is reachable, but the LLM provider is not ready."
        LLM_ERROR=$(printf '%s' "$STATUS_JSON" | sed -n 's/.*"llm_driver_error"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)
        if [ -n "$LLM_ERROR" ]; then
            echo "  LLM error: $LLM_ERROR"
        fi
        echo "  Fix: relance bash install-local.sh ou execute captain setup, puis redemarre le service."
        fail "LLM provider is not ready"
    fi

    echo "  Warning: this Captain binary does not expose LLM readiness in status."
}

toml_top_string() {
    key="$1"
    file="$2"
    awk -v key="$key" '
        /^\[/ { in_top=0 }
        BEGIN { in_top=1 }
        in_top && $0 ~ "^[[:space:]]*" key "[[:space:]]*=" {
            sub(/^[^=]*=[[:space:]]*/, "")
            gsub(/^[[:space:]]*"|"[[:space:]]*$/, "")
            print
            exit
        }
    ' "$file" 2>/dev/null || true
}

toml_section_string() {
    section="$1"
    key="$2"
    file="$3"
    awk -v section="[$section]" -v key="$key" '
        $0 == section { in_section=1; next }
        /^\[/ { in_section=0 }
        in_section && $0 ~ "^[[:space:]]*" key "[[:space:]]*=" {
            sub(/^[^=]*=[[:space:]]*/, "")
            gsub(/^[[:space:]]*"|"[[:space:]]*$/, "")
            print
            exit
        }
    ' "$file" 2>/dev/null || true
}

detect_vps_public_host() {
    if [ -n "${CAPTAIN_PUBLIC_IP:-}" ]; then
        printf '%s' "$CAPTAIN_PUBLIC_IP"
        return 0
    fi
    if command -v ip >/dev/null 2>&1; then
        host=$(ip route get 1.1.1.1 2>/dev/null | sed -n 's/.* src \([^ ]*\).*/\1/p' | head -1)
        if [ -n "$host" ] && [ "$host" != "127.0.0.1" ]; then
            printf '%s' "$host"
            return 0
        fi
    fi
    if command -v hostname >/dev/null 2>&1; then
        host=$(hostname -I 2>/dev/null | awk '{print $1}')
        if [ -n "$host" ] && [ "$host" != "127.0.0.1" ]; then
            printf '%s' "$host"
            return 0
        fi
    fi
    printf '%s' '<IP_DU_VPS>'
}

print_web_terminal_access() {
    cfg="${CAPTAIN_HOME:-$HOME/.captain}/config.toml"
    [ -f "$cfg" ] || return 0

    api_listen=$(toml_top_string api_listen "$cfg")
    [ -n "$api_listen" ] || api_listen="127.0.0.1:50051"
    public_url=$(toml_section_string deployment public_url "$cfg")

    echo ""
    echo "  Web terminal:"
    if [ -n "$public_url" ]; then
        public_url="${public_url%/}"
        echo "    $public_url/terminal"
        return 0
    fi

    port="${api_listen##*:}"
    case "$api_listen" in
        0.0.0.0:*|\[::\]:*)
            host=$(detect_vps_public_host)
            echo "    http://$host:$port/terminal"
            echo "    Login: utilise l'identifiant admin configuré pendant l'installation."
            ;;
        127.0.0.1:*|localhost:*)
            echo "    http://127.0.0.1:$port/terminal"
            if [ "$PROFILE" = "vps" ]; then
                echo "    Depuis ton Mac: ssh -L $port:127.0.0.1:$port root@<IP_DU_VPS>"
            fi
            ;;
        *)
            echo "    http://$api_listen/terminal"
            ;;
    esac
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    else
        echo ""
    fi
}

verify_archive_checksum() {
    archive="$1"
    expected="${2:-}"
    checksum_file="${3:-}"

    if [ -z "$expected" ] && [ -n "$checksum_file" ] && [ -f "$checksum_file" ]; then
        expected=$(cut -d ' ' -f 1 < "$checksum_file")
    fi
    [ -n "$expected" ] || return 0

    actual=$(sha256_file "$archive")
    if [ -z "$actual" ]; then
        echo "  No sha256sum/shasum found, skipping checksum verification."
        return 0
    fi
    if [ "$expected" != "$actual" ]; then
        echo "  Checksum verification FAILED!"
        echo "    Expected: $expected"
        echo "    Got:      $actual"
        exit 1
    fi
    echo "  Checksum verified."
}

curl_download() {
    url="$1"
    output="$2"
    if [ -n "${CAPTAIN_GITHUB_TOKEN:-}" ]; then
        curl -fL \
            -H "Authorization: Bearer $CAPTAIN_GITHUB_TOKEN" \
            -H "Accept: application/octet-stream" \
            "$url" -o "$output"
    else
        curl -fL "$url" -o "$output"
    fi
}

# Private repositories reject the browser download URL
# (github.com/<repo>/releases/download/...) even with a Bearer token — the
# response is a plain 404. Assets must be fetched through the REST API:
# resolve the release, find the asset id, then GET
# api.github.com/repos/<repo>/releases/assets/<id> with Accept octet-stream.
github_api_release_json() {
    version="$1"
    if [ "$version" = "latest" ]; then
        api_url="https://api.github.com/repos/$GITHUB_REPO/releases/latest"
    else
        api_url="https://api.github.com/repos/$GITHUB_REPO/releases/tags/$version"
    fi
    curl -fsSL \
        -H "Authorization: Bearer $CAPTAIN_GITHUB_TOKEN" \
        -H "Accept: application/vnd.github+json" \
        "$api_url"
}

# Relies on GitHub's stable field order inside each asset object: the asset's
# own "id" always precedes its "name" (the uploader's nested "id" comes after
# the name), so the last id seen when the name matches is the asset id.
github_api_asset_id() {
    release_json="$1"
    asset_name="$2"
    printf '%s\n' "$release_json" | awk -v name="$asset_name" '
        /"id":/ { id=$0; gsub(/[^0-9]/, "", id); last_id=id }
        /"name":/ && index($0, "\"" name "\"") { print last_id; exit }'
}

github_api_download_asset() {
    release_json="$1"
    asset_name="$2"
    output="$3"
    asset_id=$(github_api_asset_id "$release_json" "$asset_name")
    [ -n "$asset_id" ] || return 1
    curl -fL \
        -H "Authorization: Bearer $CAPTAIN_GITHUB_TOKEN" \
        -H "Accept: application/octet-stream" \
        "https://api.github.com/repos/$GITHUB_REPO/releases/assets/$asset_id" \
        -o "$output"
}

install_captain_binary() {
    src="$1"
    dest="$INSTALL_DIR/captain"
    tmp_bin=$(mktemp "$INSTALL_DIR/.captain.XXXXXX") || fail "Could not create temporary binary in $INSTALL_DIR"

    if ! cp "$src" "$tmp_bin"; then
        rm -f "$tmp_bin" 2>/dev/null || true
        fail "Could not copy Captain binary into $INSTALL_DIR"
    fi
    if ! chmod +x "$tmp_bin"; then
        rm -f "$tmp_bin" 2>/dev/null || true
        fail "Could not make Captain binary executable"
    fi

    # Ad-hoc codesign on macOS (prevents SIGKILL on Apple Silicon).
    # Sign the temporary file before the atomic replacement so the final path
    # is never left with a half-written or unsigned binary.
    if [ "$OS" = "darwin" ]; then
        if command -v xattr &>/dev/null; then
            xattr -cr "$tmp_bin" 2>/dev/null || true
        fi
        if command -v codesign &>/dev/null; then
            if ! codesign --force --sign - "$tmp_bin"; then
                echo ""
                echo "  Warning: ad-hoc code signing failed."
                echo "  On Apple Silicon, the binary may be killed (SIGKILL) by Gatekeeper."
                echo "  Try manually: xattr -cr $dest && codesign --force --sign - $dest"
                echo ""
            fi
        fi
    fi

    # Atomic replacement avoids Linux ETXTBSY when an existing Captain binary
    # is still mapped by a running systemd service during reinstall.
    if ! mv -f "$tmp_bin" "$dest"; then
        rm -f "$tmp_bin" 2>/dev/null || true
        fail "Could not install Captain binary at $dest"
    fi
}

install_version_marker() {
    version="$1"
    [ -n "$version" ] || return 0
    home_dir="${CAPTAIN_HOME:-$HOME/.captain}"
    mkdir -p "$home_dir" 2>/dev/null || return 0
    if printf '%s\n' "$version" > "$home_dir/VERSION" 2>/dev/null; then
        chmod 600 "$home_dir/VERSION" 2>/dev/null || true
    fi
}

install() {
    detect_platform
    detect_adjacent_bundle
    if [ "$INSTALL_SOURCE" = "local" ] && [ -z "${CAPTAIN_BUNDLE_PATH:-}" ]; then
        fail "Local install requested, but no captain-$PLATFORM.tar.gz bundle was found next to install.sh. Place both files in the same directory or set CAPTAIN_BUNDLE_PATH."
    fi
    resolve_profile
    resolve_install_dir

    echo ""
    echo "  Captain Installer"
    echo "  =================="
    echo "  Profile: $PROFILE"
    if [ "$DETECTED_LOCAL_BUNDLE" = "1" ]; then
        echo "  Source:  local bundle ($(basename "$CAPTAIN_BUNDLE_PATH"))"
    elif [ "$INSTALL_SOURCE" = "git" ]; then
        echo "  Source:  GitHub Releases"
    fi
    echo ""

    install_packages

    VERSION="${CAPTAIN_VERSION:-}"
    if [ -n "${CAPTAIN_BUNDLE_PATH:-}" ]; then
        ARCHIVE="$CAPTAIN_BUNDLE_PATH"
        [ -f "$ARCHIVE" ] || fail "CAPTAIN_BUNDLE_PATH does not exist: $ARCHIVE"
        [ -n "$VERSION" ] || VERSION="local-bundle"
        echo "  Installing Captain $VERSION for $PLATFORM from local bundle..."
    else
        if [ -n "$DIST_BASE_URL" ]; then
            if [ -n "$VERSION" ]; then
                echo "  Using specified version: $VERSION"
            else
                echo "  Fetching latest Captain distribution metadata..."
                VERSION=$(curl -fsSL "$DIST_BASE_URL/latest.txt" | tr -d '[:space:]')
            fi
            if [ -z "$VERSION" ]; then
                fail "Could not determine latest version from $DIST_BASE_URL/latest.txt. Set CAPTAIN_VERSION or CAPTAIN_BUNDLE_PATH."
            fi
            URL="$DIST_BASE_URL/$VERSION/captain-$PLATFORM.tar.gz"
            CHECKSUM_URL="$URL.sha256"
            echo "  Installing Captain $VERSION for $PLATFORM from controlled mirror..."
        elif [ -n "$VERSION" ]; then
            URL="$GITHUB_BASE_URL/$GITHUB_REPO/releases/download/$VERSION/captain-$PLATFORM.tar.gz"
            CHECKSUM_URL="$URL.sha256"
            echo "  Installing Captain $VERSION for $PLATFORM from GitHub Releases..."
        else
            VERSION="latest"
            URL="$GITHUB_BASE_URL/$GITHUB_REPO/releases/latest/download/captain-$PLATFORM.tar.gz"
            CHECKSUM_URL="$URL.sha256"
            echo "  Installing latest Captain for $PLATFORM from GitHub Releases..."
        fi
    fi

    mkdir -p "$INSTALL_DIR"
    INSTALL_DIR=$(cd "$INSTALL_DIR" && pwd -P)

    # Download to temp
    TMPDIR=$(mktemp -d)
    DOWNLOADED_ARCHIVE="$TMPDIR/captain.tar.gz"
    CHECKSUM_FILE="$TMPDIR/checksum.sha256"

    cleanup() { rm -rf "$TMPDIR"; }
    trap cleanup EXIT

    if [ -z "${CAPTAIN_BUNDLE_PATH:-}" ]; then
        if [ -z "$DIST_BASE_URL" ] && [ -n "${CAPTAIN_GITHUB_TOKEN:-}" ]; then
            # Token present: assume the repo may be private and go through
            # the API (works for public repos too).
            RELEASE_JSON=$(github_api_release_json "$VERSION") \
                || fail "Could not resolve release '$VERSION' from the GitHub API for $GITHUB_REPO. Check CAPTAIN_GITHUB_TOKEN and that a release exists."
            RESOLVED_TAG=$(printf '%s\n' "$RELEASE_JSON" | awk -F'"' '/"tag_name":/ { print $4; exit }')
            [ -z "$RESOLVED_TAG" ] || VERSION="$RESOLVED_TAG"
            echo "  Resolved release: $VERSION"
            if ! github_api_download_asset "$RELEASE_JSON" "captain-$PLATFORM.tar.gz" "$DOWNLOADED_ARCHIVE"; then
                fail "Release $VERSION has no asset captain-$PLATFORM.tar.gz (or the download failed). The controlled Captain distribution may not contain this platform yet."
            fi
            ARCHIVE="$DOWNLOADED_ARCHIVE"
            if github_api_download_asset "$RELEASE_JSON" "captain-$PLATFORM.tar.gz.sha256" "$CHECKSUM_FILE" 2>/dev/null; then
                verify_archive_checksum "$ARCHIVE" "" "$CHECKSUM_FILE"
            else
                echo "  Warning: checksum asset not available for captain-$PLATFORM.tar.gz"
            fi
        else
            if ! curl_download "$URL" "$DOWNLOADED_ARCHIVE" 2>/dev/null; then
                if [ -z "$DIST_BASE_URL" ] && [ -z "${CAPTAIN_GITHUB_TOKEN:-}" ]; then
                    fail "Download failed from $URL. If $GITHUB_REPO is a private repository: export CAPTAIN_GITHUB_TOKEN=<token with read access to releases> and retry."
                fi
                fail "Download failed from $URL. The controlled Captain distribution may not contain this platform yet."
            fi
            ARCHIVE="$DOWNLOADED_ARCHIVE"
            if curl_download "$CHECKSUM_URL" "$CHECKSUM_FILE" 2>/dev/null; then
                verify_archive_checksum "$ARCHIVE" "" "$CHECKSUM_FILE"
            else
                echo "  Warning: checksum file not available at $CHECKSUM_URL"
            fi
        fi
    else
        if [ -z "${CAPTAIN_BUNDLE_SHA256:-}" ] && [ ! -f "$ARCHIVE.sha256" ]; then
            echo "  Warning: checksum file not available at $ARCHIVE.sha256"
        fi
        verify_archive_checksum "$ARCHIVE" "${CAPTAIN_BUNDLE_SHA256:-}" "$ARCHIVE.sha256"
    fi

    # Extract into a temp directory first. Release archives may contain either
    # `captain` at the root or a nested directory; the installer contract is
    # still strict: a working CLI must end up at $INSTALL_DIR/captain.
    EXTRACT_DIR="$TMPDIR/extract"
    mkdir -p "$EXTRACT_DIR"
    tar xzf "$ARCHIVE" -C "$EXTRACT_DIR"

    CAPTAIN_BIN="$EXTRACT_DIR/captain"
    if [ ! -f "$CAPTAIN_BIN" ]; then
        CAPTAIN_BIN=$(find "$EXTRACT_DIR" -type f -name captain 2>/dev/null | head -1 || true)
    fi
    if [ -z "${CAPTAIN_BIN:-}" ] || [ ! -f "$CAPTAIN_BIN" ]; then
        fail "Release archive does not contain a captain CLI binary."
    fi
    BUNDLE_VERSION_FILE="$(dirname "$CAPTAIN_BIN")/VERSION"
    if [ -f "$BUNDLE_VERSION_FILE" ]; then
        BUNDLE_VERSION=$(tr -d '\r\n' < "$BUNDLE_VERSION_FILE" 2>/dev/null || true)
        if [ -n "$BUNDLE_VERSION" ]; then
            VERSION="$BUNDLE_VERSION"
        fi
    fi

    install_captain_binary "$CAPTAIN_BIN"
    install_version_marker "$VERSION"

    # Add to PATH — detect the user's login shell
    USER_SHELL="${SHELL:-}"
    # Fallback: check /etc/passwd if $SHELL is unset (e.g. minimal containers)
    if [ -z "$USER_SHELL" ] && command -v getent &>/dev/null; then
        USER_SHELL=$(getent passwd "$(id -un)" 2>/dev/null | cut -d: -f7)
    fi
    if [ -z "$USER_SHELL" ] && [ -f /etc/passwd ]; then
        USER_SHELL=$(grep "^$(id -un):" /etc/passwd 2>/dev/null | cut -d: -f7)
    fi

    SHELL_RC=""
    case "$USER_SHELL" in
        */zsh)  SHELL_RC="$HOME/.zshrc" ;;
        */bash) SHELL_RC="$HOME/.bashrc" ;;
        */fish) SHELL_RC="$HOME/.config/fish/config.fish" ;;
    esac
    # Also check for config files if shell detection failed.
    # Check bash/zsh first (more common defaults), fish last — avoids
    # writing to config.fish for users who merely have Fish installed.
    if [ -z "$SHELL_RC" ]; then
        if [ -f "$HOME/.bashrc" ]; then
            SHELL_RC="$HOME/.bashrc"
        elif [ -f "$HOME/.zshrc" ]; then
            SHELL_RC="$HOME/.zshrc"
        elif [ -f "$HOME/.config/fish/config.fish" ]; then
            SHELL_RC="$HOME/.config/fish/config.fish"
        fi
    fi

    if should_update_path && [ -n "$SHELL_RC" ] && ! grep -Fq "$INSTALL_DIR" "$SHELL_RC" 2>/dev/null; then
        # Determine syntax from the TARGET FILE, not $USER_SHELL — this
        # prevents Bash syntax from ever being written to config.fish even
        # when shell detection mis-identifies the user's shell.
        PATH_LINE=""
        mkdir -p "$(dirname "$SHELL_RC")" 2>/dev/null || true
        case "$SHELL_RC" in
            */config.fish)
                PATH_LINE="fish_add_path \"$INSTALL_DIR\""
                ;;
            *)
                PATH_LINE="export PATH=\"$INSTALL_DIR:\$PATH\""
                ;;
        esac
        if printf '%s\n' "$PATH_LINE" >> "$SHELL_RC" 2>/dev/null; then
            echo "  Added $INSTALL_DIR to PATH in $SHELL_RC"
        else
            echo "  Warning: could not update PATH in $SHELL_RC"
            echo "  Add manually: $PATH_LINE"
        fi
    fi

    # Verify installation. The CLI is a hard requirement for every Captain
    # profile, including future Docker/VPS installs.
    if [ ! -x "$INSTALL_DIR/captain" ]; then
        fail "Captain CLI was not installed as an executable at $INSTALL_DIR/captain."
    fi
    if ! "$INSTALL_DIR/captain" --version >/dev/null 2>&1; then
        fail "Captain CLI is present but failed to run: $INSTALL_DIR/captain --version"
    fi

    export PATH="$INSTALL_DIR:$PATH"
    hash -r 2>/dev/null || true
    RESOLVED_CAPTAIN=$(command -v captain 2>/dev/null || true)
    if [ -z "$RESOLVED_CAPTAIN" ]; then
        fail "Captain CLI was installed but is not resolvable on PATH."
    fi

    INSTALLED_VERSION=$("$INSTALL_DIR/captain" --version 2>/dev/null || echo "$VERSION")
    echo ""
    echo "  Captain CLI verified: $INSTALL_DIR/captain"
    echo "  Captain installed successfully! ($INSTALLED_VERSION)"

    install_linux_service

    run_initial_setup

    if should_install_mempalace; then
        echo ""
        echo "  Installing managed MemPalace memory runtime (no API key or system Python)..."
        if ! "$INSTALL_DIR/captain" memory install; then
            fail "Managed MemPalace installation failed. Captain keeps local durable memory, but the selected semantic backend is not production-ready. Retry: captain memory install --force"
        fi
        if ! "$INSTALL_DIR/captain" memory doctor --json >/dev/null; then
            fail "Managed MemPalace installed but failed its live semantic probe. Retry: captain memory install --force"
        fi
        echo "  Managed MemPalace runtime checked."
    else
        echo ""
        echo "  Warning: managed MemPalace installation explicitly skipped."
        echo "  Captain will retain durable local memory but report the semantic backend as degraded."
    fi

    if should_install_embeddings; then
        echo ""
        echo "  Installing native embeddings runtime (ONNX Runtime, no API key)..."
        if "$INSTALL_DIR/captain" embeddings install --best-effort; then
            echo "  Native embeddings runtime checked."
        else
            echo "  Warning: native embeddings runtime is incomplete. Run: captain embeddings install"
        fi
    fi

    if should_install_voice; then
        echo ""
        echo "  Installing native voice pack (STT/TTS, no API key)..."
        if "$INSTALL_DIR/captain" voice install --best-effort; then
            echo "  Native voice pack checked."
        else
            echo "  Warning: native voice pack is incomplete. Run: captain voice install"
        fi
    fi

    start_linux_service
    verify_llm_ready_after_start

    if is_yes "${CAPTAIN_RUN_DOCTOR:-}"; then
        echo ""
        echo "  Running captain doctor --full..."
        "$INSTALL_DIR/captain" doctor --full
    fi

    echo ""
    echo "  Get started:"
    if [ "$SETUP_RAN" = "1" ]; then
        if should_start_service; then
            echo "    captain status"
            echo "    captain chat"
        else
            echo "    captain start"
            echo "    captain chat"
        fi
    else
        echo "    captain setup"
        echo ""
        echo "  The setup wizard will guide you through provider selection"
        echo "  and configuration."
    fi
    print_web_terminal_access
    echo ""
}

install
