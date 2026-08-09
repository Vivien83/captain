#!/usr/bin/env bash
# Isolated regression suite for install.sh managed VPS-domain provisioning.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
export CAPTAIN_INSTALL_LIBRARY_ONLY=1
# shellcheck source=scripts/install.sh
source "$ROOT_DIR/scripts/install.sh"

TMP="$(mktemp -d)"
cleanup_test() {
    rm -rf "$TMP"
}
trap cleanup_test EXIT

assert_eq() {
    expected=$1
    actual=$2
    label=$3
    if [ "$expected" != "$actual" ]; then
        echo "FAIL: $label: expected '$expected', got '$actual'" >&2
        exit 1
    fi
}

assert_file_eq() {
    expected=$1
    actual=$2
    label=$3
    if ! cmp -s "$expected" "$actual"; then
        echo "FAIL: $label" >&2
        diff -u "$expected" "$actual" >&2 || true
        exit 1
    fi
}

assert_eq "captain.example.com" \
    "$(normalize_vps_domain ' HTTPS://captain.example.com/ ' 2>/dev/null || true)" \
    "case-insensitive HTTPS scheme normalization"

assert_eq "captain.example.com" \
    "$(normalize_vps_domain 'Captain.Example.com///')" \
    "bare domain normalization"

for invalid in \
    'http://captain.example.com' \
    'https://captain.example.com/path' \
    'https://user@captain.example.com' \
    'https://captain.example.com:8443' \
    '127.0.0.1' \
    'localhost' \
    '*.example.com' \
    'captain_example.com'; do
    if normalize_vps_domain "$invalid" >/dev/null 2>&1; then
        echo "FAIL: unsafe domain accepted: $invalid" >&2
        exit 1
    fi
done

PROFILE=vps
CAPTAIN_PUBLIC_URL=https://captain.example.com/
CAPTAIN_DOMAIN=Captain.Example.com
prepare_vps_domain
assert_eq "captain.example.com" "$VPS_DOMAIN" "matching domain inputs"
assert_eq "https://captain.example.com" "$CAPTAIN_PUBLIC_URL" "public URL export"

if (
    VPS_DOMAIN=""
    CAPTAIN_PUBLIC_URL=https://one.example.com
    CAPTAIN_DOMAIN=two.example.com
    prepare_vps_domain
) >/dev/null 2>&1; then
    echo "FAIL: conflicting CAPTAIN_PUBLIC_URL/CAPTAIN_DOMAIN inputs were accepted" >&2
    exit 1
fi

ROOT_SOURCE="$TMP/root.source"
ROOT_ONCE="$TMP/root.once"
ROOT_TWICE="$TMP/root.twice"
printf '%s\n' 'existing.example.com {' '  respond "existing"' '}' > "$ROOT_SOURCE"
render_caddy_root_with_import "$ROOT_SOURCE" '/etc/caddy/captain.d/*.caddy' > "$ROOT_ONCE"
render_caddy_root_with_import "$ROOT_ONCE" '/etc/caddy/captain.d/*.caddy' > "$ROOT_TWICE"
assert_eq "1" \
    "$(grep -Fxc '# BEGIN CAPTAIN MANAGED IMPORT' "$ROOT_TWICE")" \
    "managed Caddy import idempotence"

FAKE_BIN="$TMP/bin"
mkdir -p "$FAKE_BIN"
cat > "$FAKE_BIN/caddy" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "version" ]; then
    echo v2.test
    exit 0
fi
if [ "${1:-}" = "validate" ]; then
    [ "${CADDY_TEST_FAIL:-0}" != "1" ]
    exit
fi
exit 0
EOF
cat > "$FAKE_BIN/systemctl" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "is-active" ]; then
    [ "${CADDY_TEST_INACTIVE:-0}" != "1" ]
    exit
fi
if [ "${1:-}" = "show" ]; then
    echo "{ path=/usr/bin/caddy ; argv[]=/usr/bin/caddy run --config $CAPTAIN_CADDYFILE ; }"
    exit 0
fi
exit 0
EOF
cat > "$FAKE_BIN/curl" <<'EOF'
#!/usr/bin/env bash
last=""
for value in "$@"; do last=$value; done
case "$last" in
    */api/health) printf '%s' '{"status":"ok","version":"0.1.0-test"}' ;;
    *) printf '%s' '<!doctype html><title>Captain</title>' ;;
esac
EOF
chmod +x "$FAKE_BIN/caddy" "$FAKE_BIN/systemctl" "$FAKE_BIN/curl"
PATH="$FAKE_BIN:$PATH"
export PATH

run_privileged() {
    "$@"
}
systemd_system_available() {
    return 0
}

CAPTAIN_HOME="$TMP/home"
CAPTAIN_CADDYFILE="$TMP/etc/caddy/Caddyfile"
CAPTAIN_CADDY_SNIPPET_DIR="$TMP/etc/caddy/captain.d"
CAPTAIN_START=1
CAPTAIN_INSTALL_PROXY=1
CAPTAIN_DOMAIN=captain.example.com
CAPTAIN_PUBLIC_URL=https://captain.example.com
VPS_DOMAIN=captain.example.com
VPS_PUBLIC_URL=https://captain.example.com
export CAPTAIN_HOME CAPTAIN_CADDYFILE CAPTAIN_CADDY_SNIPPET_DIR CAPTAIN_START
export CAPTAIN_INSTALL_PROXY CAPTAIN_DOMAIN CAPTAIN_PUBLIC_URL VPS_DOMAIN VPS_PUBLIC_URL

mkdir -p "$CAPTAIN_HOME/deploy" "$(dirname "$CAPTAIN_CADDYFILE")"
cat > "$CAPTAIN_HOME/config.toml" <<'EOF'
api_listen = "127.0.0.1:50098"

[auth]
username = "captain-admin"

[deployment]
profile = "vps"
public_url = "https://captain.example.com"
https = true
reverse_proxy = "caddy"
EOF
render_managed_caddy_fragment "$VPS_DOMAIN" 50098 > "$CAPTAIN_HOME/deploy/Caddyfile"
cat > "$CAPTAIN_HOME/initial-credentials.txt" <<'EOF'
Username: captain-admin
Password: must-not-leak
API key: must-not-leak-either
EOF

if (
    export CAPTAIN_DNS_CHECK=0
    export CADDY_TEST_INACTIVE=1
    OS=linux
    proxy_port_listeners() {
        printf '%s\n' 'LISTEN 0 128 0.0.0.0:443 0.0.0.0:*'
    }
    preflight_vps_proxy
) >/dev/null 2>&1; then
    echo "FAIL: an unrelated listener on a proxy port was accepted" >&2
    exit 1
fi

cat > "$CAPTAIN_CADDYFILE" <<'EOF'
# END CAPTAIN MANAGED IMPORT
existing.example.com {
  respond "existing"
}
# BEGIN CAPTAIN MANAGED IMPORT
EOF
cp "$CAPTAIN_CADDYFILE" "$TMP/root.malformed"
if (provision_vps_proxy) >/dev/null 2>&1; then
    echo "FAIL: reversed managed Caddy markers were accepted" >&2
    exit 1
fi
assert_file_eq "$TMP/root.malformed" "$CAPTAIN_CADDYFILE" \
    "malformed marker rejection leaves Caddy untouched"

cp "$ROOT_SOURCE" "$CAPTAIN_CADDYFILE"

provision_vps_proxy >/dev/null
grep -Fq 'reverse_proxy 127.0.0.1:50098' \
    "$CAPTAIN_CADDY_SNIPPET_DIR/captain.caddy"
provision_vps_proxy >/dev/null
assert_eq "1" \
    "$(grep -Fxc '# BEGIN CAPTAIN MANAGED IMPORT' "$CAPTAIN_CADDYFILE")" \
    "provisioning remains idempotent"

cp "$CAPTAIN_CADDYFILE" "$TMP/root.before-failure"
cp "$CAPTAIN_CADDY_SNIPPET_DIR/captain.caddy" "$TMP/fragment.before-failure"
if (CADDY_TEST_FAIL=1 provision_vps_proxy) >/dev/null 2>&1; then
    echo "FAIL: a rejected Caddy configuration was reported as successful" >&2
    exit 1
fi
assert_file_eq "$TMP/root.before-failure" "$CAPTAIN_CADDYFILE" \
    "Caddy root rollback"
assert_file_eq "$TMP/fragment.before-failure" \
    "$CAPTAIN_CADDY_SNIPPET_DIR/captain.caddy" \
    "Captain Caddy fragment rollback"

CADDY_PROVISIONED=1
verify_public_web_access >/dev/null

access_summary=$(print_web_terminal_access)
printf '%s' "$access_summary" | grep -Fq 'Browser authentication: username + password'
printf '%s' "$access_summary" | grep -Fq 'Username: captain-admin'
printf '%s' "$access_summary" | grep -Fq 'Initial password details:'
printf '%s' "$access_summary" | grep -Fq 'API bearer key: for CLI/API clients'
if printf '%s' "$access_summary" | grep -Fq 'must-not-leak'; then
    echo "FAIL: installer output disclosed initial credentials" >&2
    exit 1
fi

echo "PASS: managed VPS domain installer is validated in isolation"
