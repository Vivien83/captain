#!/usr/bin/env bash
# Clean-install smoke for the product installer.
#
# This does not touch the real ~/.captain. It installs a precompiled local
# bundle into a temporary CAPTAIN_HOME, runs full non-interactive setup from
# env vars, and verifies the resulting CLI/config/profile artifacts.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

detect_platform() {
    local os arch
    os="$(uname -s | tr '[:upper:]' '[:lower:]')"
    arch="$(uname -m)"
    case "$arch" in
        x86_64|amd64) arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *) echo "unsupported arch: $arch" >&2; exit 2 ;;
    esac
    case "$os" in
        linux) echo "${arch}-unknown-linux-gnu" ;;
        darwin) echo "${arch}-apple-darwin" ;;
        *) echo "unsupported os: $os" >&2; exit 2 ;;
    esac
}

PLATFORM="$(detect_platform)"
VERSION="${CAPTAIN_SMOKE_VERSION:-$(cat "$ROOT_DIR/dist/releases/latest.txt" 2>/dev/null || true)}"
if [ -z "$VERSION" ]; then
    echo "No dist/releases/latest.txt found. Run scripts/package-release.sh first, or set CAPTAIN_BUNDLE_PATH." >&2
    exit 2
fi

BUNDLE="${CAPTAIN_BUNDLE_PATH:-$ROOT_DIR/dist/releases/$VERSION/captain-$PLATFORM.tar.gz}"
if [ ! -f "$BUNDLE" ]; then
    echo "Bundle not found: $BUNDLE" >&2
    echo "Run scripts/package-release.sh first, or set CAPTAIN_BUNDLE_PATH." >&2
    exit 2
fi

TMP="${CAPTAIN_INSTALL_SMOKE_TMP:-$(mktemp -d)}"
cleanup() {
    if [ -z "${CAPTAIN_INSTALL_SMOKE_KEEP:-}" ]; then
        rm -rf "$TMP"
    else
        echo "Keeping smoke dir: $TMP"
    fi
}
trap cleanup EXIT

INSTALL_SCRIPT="$ROOT_DIR/scripts/install.sh"
if [ "${CAPTAIN_INSTALL_SMOKE_LOCAL_AUTO:-1}" != "0" ]; then
    LOCAL_RELEASE="$TMP/release"
    mkdir -p "$LOCAL_RELEASE"
    cp "$BUNDLE" "$LOCAL_RELEASE/$(basename "$BUNDLE")"
    [ ! -f "$BUNDLE.sha256" ] || cp "$BUNDLE.sha256" "$LOCAL_RELEASE/$(basename "$BUNDLE").sha256"
    [ ! -f "$ROOT_DIR/dist/releases/$VERSION/manifest-$PLATFORM.json" ] || cp "$ROOT_DIR/dist/releases/$VERSION/manifest-$PLATFORM.json" "$LOCAL_RELEASE/manifest-$PLATFORM.json"
    [ ! -f "$ROOT_DIR/dist/releases/$VERSION/manifest.json" ] || cp "$ROOT_DIR/dist/releases/$VERSION/manifest.json" "$LOCAL_RELEASE/manifest.json"
    cp "$ROOT_DIR/scripts/install.sh" "$LOCAL_RELEASE/install.sh"
    cp "$ROOT_DIR/scripts/install-local.sh" "$LOCAL_RELEASE/install-local.sh"
    cp "$ROOT_DIR/scripts/install-git.sh" "$LOCAL_RELEASE/install-git.sh"
    INSTALL_SCRIPT="$LOCAL_RELEASE/install.sh"
    unset CAPTAIN_BUNDLE_PATH
    unset CAPTAIN_VERSION
    export CAPTAIN_INSTALL_SOURCE=local
else
    export CAPTAIN_BUNDLE_PATH="$BUNDLE"
    export CAPTAIN_VERSION="$VERSION"
fi

export CAPTAIN_HOME="$TMP/home"
export CAPTAIN_INSTALL_DIR="$TMP/bin"
export CAPTAIN_PROFILE="${CAPTAIN_PROFILE:-vps}"
export CAPTAIN_INSTALL_DEPS=0
export CAPTAIN_UPDATE_PATH=0
export CAPTAIN_INSTALL_SERVICE=0
export CAPTAIN_START=0
export CAPTAIN_SETUP=1
export CAPTAIN_YES=1
export CAPTAIN_PROVIDER="${CAPTAIN_PROVIDER:-groq}"
export CAPTAIN_MODEL="${CAPTAIN_MODEL:-llama-3.3-70b-versatile}"
export CAPTAIN_API_KEY_ENV="${CAPTAIN_API_KEY_ENV:-GROQ_API_KEY}"
export CAPTAIN_API_KEY="${CAPTAIN_API_KEY:-gsk_FAKE_INSTALL_SMOKE_KEY_LONG_ENOUGH}"
export CAPTAIN_ASSISTANT_NAME="${CAPTAIN_ASSISTANT_NAME:-Captain}"
export CAPTAIN_USER_NAME="${CAPTAIN_USER_NAME:-Install Smoke}"
export CAPTAIN_LANGUAGE="${CAPTAIN_LANGUAGE:-en}"
export CAPTAIN_TIMEZONE="${CAPTAIN_TIMEZONE:-UTC}"
export CAPTAIN_ASSISTANT_STYLE="${CAPTAIN_ASSISTANT_STYLE:-concise}"
export CAPTAIN_VOICE_PREFERENCE="${CAPTAIN_VOICE_PREFERENCE:-OpenAI Nova}"
export CAPTAIN_TTS_PROVIDER="${CAPTAIN_TTS_PROVIDER:-openai}"
export CAPTAIN_TTS_API_KEY="${CAPTAIN_TTS_API_KEY:-sk-FAKE_INSTALL_SMOKE_TTS_KEY_LONG_ENOUGH}"
export CAPTAIN_ADMIN_USERNAME="${CAPTAIN_ADMIN_USERNAME:-smoke-admin}"
export CAPTAIN_PUBLIC_URL="${CAPTAIN_PUBLIC_URL:-https://captain-smoke.example.com}"
export CAPTAIN_RUN_DOCTOR=0

bash "$INSTALL_SCRIPT"

"$CAPTAIN_INSTALL_DIR/captain" --version >/dev/null
"$CAPTAIN_INSTALL_DIR/captain" memory status --json > "$TMP/mempalace-status.json"
"$CAPTAIN_INSTALL_DIR/captain" memory doctor --json > "$TMP/mempalace-doctor.json"
test -s "$CAPTAIN_HOME/config.toml"
test -s "$CAPTAIN_HOME/USER.md"
grep -Fq '"ready": true' "$TMP/mempalace-status.json"
grep -Fq '"ok": true' "$TMP/mempalace-doctor.json"
MEMPALACE_BIN=$(sed -n 's/^[[:space:]]*"mempalace_binary": "\([^"]*\)",*$/\1/p' "$TMP/mempalace-status.json")
MEMPALACE_MCP_BIN=$(sed -n 's/^[[:space:]]*"mcp_binary": "\([^"]*\)",*$/\1/p' "$TMP/mempalace-status.json")
MEMPALACE_PYTHON_BIN=$(sed -n 's/^[[:space:]]*"python_binary": "\([^"]*\)",*$/\1/p' "$TMP/mempalace-status.json")
test -n "$MEMPALACE_BIN"
test -n "$MEMPALACE_MCP_BIN"
test -n "$MEMPALACE_PYTHON_BIN"
test -x "$MEMPALACE_BIN"
test -x "$MEMPALACE_MCP_BIN"
test -x "$MEMPALACE_PYTHON_BIN"
"$MEMPALACE_PYTHON_BIN" --version > "$TMP/mempalace-python-version.txt"
grep -Fxq 'Python 3.13.14' "$TMP/mempalace-python-version.txt"
test -s "$CAPTAIN_HOME/native/mempalace/install.json"
test -s "$CAPTAIN_HOME/integrations.toml"
grep -Fq 'id = "mempalace"' "$CAPTAIN_HOME/integrations.toml"

grep -Fq 'provider = "groq"' "$CAPTAIN_HOME/config.toml"
grep -Fq 'model = "llama-3.3-70b-versatile"' "$CAPTAIN_HOME/config.toml"
grep -Fq 'onboarding_completed = true' "$CAPTAIN_HOME/config.toml"
grep -Fq 'provider = "openai"' "$CAPTAIN_HOME/config.toml"
grep -Fq 'voice = "nova"' "$CAPTAIN_HOME/config.toml"
grep -Fq 'api_key = ""' "$CAPTAIN_HOME/config.toml"
grep -Fq 'CAPTAIN_DAEMON_API_KEY=captain_api_' "$CAPTAIN_HOME/secrets.env"
grep -Fq 'enabled = true' "$CAPTAIN_HOME/config.toml"
grep -Fq 'username = "smoke-admin"' "$CAPTAIN_HOME/config.toml"
grep -Fq '[web_terminal]' "$CAPTAIN_HOME/config.toml"
grep -Fq 'default_mode = "captain"' "$CAPTAIN_HOME/config.toml"
grep -Fq '[deployment]' "$CAPTAIN_HOME/config.toml"
grep -Fq 'public_url = "https://captain-smoke.example.com"' "$CAPTAIN_HOME/config.toml"
test -s "$CAPTAIN_HOME/initial-credentials.txt"
test -s "$CAPTAIN_HOME/deploy/Caddyfile"
grep -Fq 'First interview status: completed during setup' "$CAPTAIN_HOME/USER.md"

echo "PASS: clean install smoke completed in $TMP"
