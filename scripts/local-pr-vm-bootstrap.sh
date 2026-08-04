#!/usr/bin/env bash
# Provision the sealed Lima base used by Captain's local pull-request gate.

set -euo pipefail

EXPECTED_ID="${1:-}"
PR_USER="captain-pr"
TOOLCHAIN_ROOT="/opt/captain-pr-toolchain"
TOOLCHAIN_CARGO_HOME="$TOOLCHAIN_ROOT/cargo"
TOOLCHAIN_RUSTUP_HOME="$TOOLCHAIN_ROOT/rustup"
PR_CARGO_HOME="/home/$PR_USER/.cargo-cache"

fail() {
    printf 'Local PR base bootstrap failed: %s\n' "$*" >&2
    exit 1
}

[ -n "$EXPECTED_ID" ] || fail "missing bootstrap identity"
[[ "$EXPECTED_ID" =~ ^[0-9a-f]{64}$ ]] || fail "invalid bootstrap identity"

export DEBIAN_FRONTEND=noninteractive

sudo apt-get update
sudo apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    clang \
    cmake \
    curl \
    git \
    gitleaks \
    iptables \
    jq \
    libasound2-dev \
    libayatana-appindicator3-dev \
    libdbus-1-dev \
    libgtk-3-dev \
    librsvg2-dev \
    libssl-dev \
    libudev-dev \
    libwebkit2gtk-4.1-dev \
    make \
    nodejs \
    npm \
    patchelf \
    perl \
    pkg-config \
    protobuf-compiler \
    ripgrep \
    rsync \
    rustup \
    shellcheck \
    sudo

if ! id "$PR_USER" >/dev/null 2>&1; then
    sudo useradd --create-home --shell /bin/bash "$PR_USER"
fi

if id -nG "$PR_USER" | tr ' ' '\n' | grep -qxE 'sudo|wheel'; then
    fail "$PR_USER unexpectedly has broad sudo membership"
fi

sudo install -d -o root -g root -m 0755 \
    "$TOOLCHAIN_ROOT" "$TOOLCHAIN_CARGO_HOME" "$TOOLCHAIN_RUSTUP_HOME"
sudo env \
    CARGO_HOME="$TOOLCHAIN_CARGO_HOME" \
    RUSTUP_HOME="$TOOLCHAIN_RUSTUP_HOME" \
    rustup set profile minimal
sudo env \
    CARGO_HOME="$TOOLCHAIN_CARGO_HOME" \
    RUSTUP_HOME="$TOOLCHAIN_RUSTUP_HOME" \
    rustup toolchain install stable --component clippy --component rustfmt
sudo env \
    CARGO_HOME="$TOOLCHAIN_CARGO_HOME" \
    RUSTUP_HOME="$TOOLCHAIN_RUSTUP_HOME" \
    rustup default stable
sudo env \
    PATH="$TOOLCHAIN_CARGO_HOME/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
    CARGO_HOME="$TOOLCHAIN_CARGO_HOME" \
    RUSTUP_HOME="$TOOLCHAIN_RUSTUP_HOME" \
    cargo install cargo-audit --version 0.22.2 --locked
sudo chown -R root:root "$TOOLCHAIN_ROOT"
sudo chmod -R go-w "$TOOLCHAIN_ROOT"
sudo install -d -o "$PR_USER" -g "$PR_USER" -m 0700 "$PR_CARGO_HOME"

sudo tee /usr/local/sbin/captain-pr-install-trusted >/dev/null <<'EOF'
#!/bin/sh
set -eu

archive="${1:-}"
expected_manifest="${2:-}"
stage="/opt/captain-pr-trusted.next"
target="/opt/captain-pr-trusted"

[ "$archive" = "/tmp/captain-pr-trusted.tar" ] || {
    printf 'unexpected trusted bundle path\n' >&2
    exit 1
}
printf '%s' "$expected_manifest" | grep -Eq '^[0-9a-f]{64}$' || {
    printf 'invalid trusted manifest identity\n' >&2
    exit 1
}
[ -f "$archive" ] && [ ! -L "$archive" ] || {
    printf 'trusted bundle is missing or is a symlink\n' >&2
    exit 1
}
if tar -tf "$archive" | grep -Eq '(^/|(^|/)\.\.(/|$))'; then
    printf 'trusted bundle contains an unsafe path\n' >&2
    exit 1
fi

rm -rf "$stage"
install -d -o root -g root -m 0755 "$stage"
tar --no-same-owner --no-same-permissions -xf "$archive" -C "$stage"
[ -f "$stage/manifest.sha256" ] && [ ! -L "$stage/manifest.sha256" ] || {
    printf 'trusted manifest is missing\n' >&2
    exit 1
}
actual_manifest="$(sha256sum "$stage/manifest.sha256" | cut -d ' ' -f 1)"
[ "$actual_manifest" = "$expected_manifest" ] || {
    printf 'trusted manifest identity mismatch\n' >&2
    exit 1
}
if find "$stage" -type l -print -quit | grep -q .; then
    printf 'trusted bundle contains a symlink\n' >&2
    exit 1
fi
(cd "$stage" && sha256sum -c manifest.sha256)

chown -R root:root "$stage"
find "$stage" -type d -exec chmod 0555 {} +
find "$stage" -type f -exec chmod 0444 {} +
find "$stage/scripts" -type f -exec chmod 0555 {} +
rm -rf "$target"
mv "$stage" "$target"
rm -f "$archive"
EOF
sudo chmod 0755 /usr/local/sbin/captain-pr-install-trusted

sudo tee /usr/local/sbin/captain-pr-seal-and-lock >/dev/null <<'EOF'
#!/bin/sh
set -eu

[ "${SUDO_USER:-}" = "captain-pr" ] || {
    printf 'seal-and-lock may only be requested by captain-pr\n' >&2
    exit 1
}

source_stage="${1:-}"
export_stage="${2:-}"
sealed_root="/var/lib/captain-pr-job"

[ "$source_stage" = "/tmp/captain-local-pr/sealed-source-staging" ] || {
    printf 'unexpected source staging path\n' >&2
    exit 1
}
[ "$export_stage" = "/tmp/captain-local-pr/public-export-staging" ] || {
    printf 'unexpected export staging path\n' >&2
    exit 1
}
for stage in "$source_stage" "$export_stage"; do
    [ -d "$stage" ] && [ ! -L "$stage" ] || {
        printf 'staging tree is missing or is a symlink: %s\n' "$stage" >&2
        exit 1
    }
    if find "$stage" -type l -print -quit | grep -q .; then
        printf 'staging tree contains a symlink: %s\n' "$stage" >&2
        exit 1
    fi
    if find "$stage" ! -type d ! -type f -print -quit | grep -q .; then
        printf 'staging tree contains a special file: %s\n' "$stage" >&2
        exit 1
    fi
done

rm -rf "$sealed_root"
install -d -o root -g root -m 0555 \
    "$sealed_root" "$sealed_root/source" "$sealed_root/public-export"
cp -a --no-preserve=ownership "$source_stage/." "$sealed_root/source/"
cp -a --no-preserve=ownership "$export_stage/." "$sealed_root/public-export/"
diff -qr "$source_stage" "$sealed_root/source" >/dev/null
diff -qr "$export_stage" "$sealed_root/public-export" >/dev/null
chown -hR root:root "$sealed_root"
find "$sealed_root" -type d -exec chmod 0555 {} +
find "$sealed_root" -type f -exec chmod 0444 {} +

iptables -w 5 -F OUTPUT
iptables -w 5 -A OUTPUT -o lo -j ACCEPT
iptables -w 5 -A OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
iptables -w 5 -P OUTPUT DROP

ip6tables -w 5 -F OUTPUT
ip6tables -w 5 -A OUTPUT -o lo -j ACCEPT
ip6tables -w 5 -A OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
ip6tables -w 5 -P OUTPUT DROP

# This clone must never grant the untrusted worker another privileged command.
rm -f /etc/sudoers.d/captain-pr-seal-and-lock
EOF
sudo chmod 0755 /usr/local/sbin/captain-pr-seal-and-lock
printf '%s\n' \
    "$PR_USER ALL=(root) NOPASSWD: /usr/local/sbin/captain-pr-seal-and-lock /tmp/captain-local-pr/sealed-source-staging /tmp/captain-local-pr/public-export-staging" \
    | sudo tee /etc/sudoers.d/captain-pr-seal-and-lock >/dev/null
sudo chmod 0440 /etc/sudoers.d/captain-pr-seal-and-lock
sudo visudo -cf /etc/sudoers.d/captain-pr-seal-and-lock >/dev/null

sudo -u "$PR_USER" -H env \
    PATH="$TOOLCHAIN_CARGO_HOME/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
    CARGO_HOME="$PR_CARGO_HOME" \
    RUSTUP_HOME="$TOOLCHAIN_RUSTUP_HOME" \
    cargo --version >/dev/null \
    || fail "cargo is unavailable after bootstrap"
sudo -u "$PR_USER" -H env \
    PATH="$TOOLCHAIN_CARGO_HOME/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
    CARGO_HOME="$PR_CARGO_HOME" \
    RUSTUP_HOME="$TOOLCHAIN_RUSTUP_HOME" \
    cargo audit --version >/dev/null \
    || fail "cargo-audit is unavailable after bootstrap"
command -v gitleaks >/dev/null 2>&1 || fail "gitleaks is unavailable after bootstrap"
command -v iptables >/dev/null 2>&1 || fail "iptables is unavailable after bootstrap"

sudo apt-get clean
sudo rm -rf /var/lib/apt/lists/*

printf '%s\n' "$EXPECTED_ID" | sudo tee /etc/captain-local-pr-base >/dev/null
sudo chmod 0444 /etc/captain-local-pr-base

printf 'Captain local PR base ready: %s\n' "$EXPECTED_ID"
