#!/usr/bin/env bash
# Build a controlled Captain distribution bundle.
#
# This script is for maintainers/CI. End users should run install.sh, which
# downloads the precompiled bundle produced here and never compiles Captain.
#
# Environment:
#   CAPTAIN_VERSION    — release version folder/name (default: dev timestamp)
#   CAPTAIN_DIST_DIR   — output root (default: dist/releases)
#   CAPTAIN_SKIP_BUILD — 1/true/yes to reuse a prebuilt binary
#   CAPTAIN_DIST_PLATFORM — package as this platform instead of host platform
#   CAPTAIN_DIST_COMPONENT — full (default), console, or node
#   CAPTAIN_BIN_PATH   — binary to package (default: target/release/<binary>)
#   CAPTAIN_GOOGLE_OAUTH_CLIENT_ID — optional verified Captain Desktop client

set -euo pipefail

is_yes() {
    case "${1:-}" in
        1|true|TRUE|yes|YES|y|Y) return 0 ;;
        *) return 1 ;;
    esac
}

fail() {
    echo "  Error: $*" >&2
    exit 1
}

detect_platform() {
    if [ -n "${CAPTAIN_DIST_PLATFORM:-}" ]; then
        PLATFORM="$CAPTAIN_DIST_PLATFORM"
        case "$PLATFORM" in
            x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu) OS="linux" ;;
            x86_64-apple-darwin|aarch64-apple-darwin) OS="darwin" ;;
            *) fail "Unsupported CAPTAIN_DIST_PLATFORM: $PLATFORM" ;;
        esac
        return
    fi

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
        *) fail "Unsupported packaging OS: $OS" ;;
    esac
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    else
        fail "sha256sum or shasum is required"
    fi
}

clear_macos_attrs() {
    command -v xattr >/dev/null 2>&1 || return 0
    xattr -cr "$@" 2>/dev/null || true
    for path in "$@"; do
        xattr -d com.apple.provenance "$path" 2>/dev/null || true
        xattr -d com.apple.quarantine "$path" 2>/dev/null || true
    done
}

write_manifests() {
    generated_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    platform_manifest="$VERSION_DIR/$MANIFEST_NAME"

    cat > "$platform_manifest" <<EOF
{
  "version": "$VERSION",
  "component": "$COMPONENT",
  "platform": "$PLATFORM",
  "archive": "$(basename "$ARCHIVE")",
  "sha256": "$HASH",
  "build_started_at": "$BUILD_STARTED_AT",
  "generated_at": "$generated_at",
  "source": {
    "repository": "$SOURCE_REPOSITORY",
    "revision": "$SOURCE_REVISION",
    "tree": "$SOURCE_TREE",
    "cargo_lock_sha256": "$CARGO_LOCK_SHA256",
    "dirty": $SOURCE_DIRTY
  }
}
EOF

    aggregate_manifest="$VERSION_DIR/manifest.json"
    set -- "$VERSION_DIR"/manifest-*.json
    [ -f "$1" ] || fail "No platform manifests found in $VERSION_DIR"
    jq -s \
        --arg version "$VERSION" \
        --arg generated_at "$generated_at" \
        '
          . as $manifests
          | ($manifests | map(.source) | unique) as $sources
          | if ($sources | length) != 1 then
              error("platform manifests contain mixed source provenance")
            else
              {
                version: $version,
                generated_at: $generated_at,
                source: $sources[0],
                artifacts: (
                  $manifests
                  | map({component: (.component // "full"), platform, archive, sha256})
                  | sort_by(.component, .platform)
                )
              }
            end
        ' \
        "$@" >"$aggregate_manifest"
}

create_archive() {
    if tar --help 2>/dev/null | grep -q -- "--no-mac-metadata"; then
        COPYFILE_DISABLE=1 tar --no-xattrs --no-mac-metadata --format ustar -czf "$ARCHIVE" -C "$VERSION_DIR/stage" "$ARCHIVE_ROOT"
    elif tar --help 2>/dev/null | grep -q -- "--no-xattrs"; then
        COPYFILE_DISABLE=1 tar --no-xattrs --format ustar -czf "$ARCHIVE" -C "$VERSION_DIR/stage" "$ARCHIVE_ROOT"
    else
        COPYFILE_DISABLE=1 tar --format ustar -czf "$ARCHIVE" -C "$VERSION_DIR/stage" "$ARCHIVE_ROOT"
    fi
}

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd -P)
VERSION="${CAPTAIN_VERSION:-0.1.0-dev.$(date -u +%Y%m%d%H%M%S)}"
DIST_ROOT="${CAPTAIN_DIST_DIR:-$ROOT_DIR/dist/releases}"
BUILD_STARTED_AT="${CAPTAIN_BUILD_STARTED_AT:-$(date -u +%Y-%m-%dT%H:%M:%SZ)}"
COMPONENT="${CAPTAIN_DIST_COMPONENT:-full}"
command -v jq >/dev/null 2>&1 || fail "jq is required"
SOURCE_REPOSITORY="${CAPTAIN_PROVENANCE_REPOSITORY:-https://github.com/Vivien83/captain}"
SOURCE_REVISION="$(git -C "$ROOT_DIR" rev-parse HEAD)"
SOURCE_TREE="$(git -C "$ROOT_DIR" rev-parse 'HEAD^{tree}')"
CARGO_LOCK_SHA256="$(sha256_file "$ROOT_DIR/Cargo.lock")"
SOURCE_DIRTY=false
if [ -n "$(git -C "$ROOT_DIR" status --porcelain)" ]; then
    SOURCE_DIRTY=true
fi

detect_platform

case "$COMPONENT" in
    full)
        PACKAGE_NAME="captain-cli"
        BINARY_NAME="captain"
        ARCHIVE_PREFIX="captain"
        MANIFEST_NAME="manifest-$PLATFORM.json"
        BUNDLE_DESCRIPTION="complete Captain Full distribution"
        ;;
    console)
        PACKAGE_NAME="captain-console"
        BINARY_NAME="captain-console"
        ARCHIVE_PREFIX="captain-console"
        MANIFEST_NAME="manifest-console-$PLATFORM.json"
        BUNDLE_DESCRIPTION="lightweight multi-Captain Console distribution"
        ;;
    node)
        PACKAGE_NAME="captain-node"
        BINARY_NAME="captain-node"
        ARCHIVE_PREFIX="captain-node"
        MANIFEST_NAME="manifest-node-$PLATFORM.json"
        BUNDLE_DESCRIPTION="optional outbound Captain Node distribution"
        ;;
    *) fail "Unsupported CAPTAIN_DIST_COMPONENT: $COMPONENT" ;;
esac

echo ""
echo "  Captain Release Packager"
echo "  ========================"
echo "  Version:  $VERSION"
echo "  Component: $COMPONENT"
echo "  Platform: $PLATFORM"
echo ""

if ! is_yes "${CAPTAIN_SKIP_BUILD:-}"; then
    CAPTAIN_BUILD_VERSION="$VERSION" \
        cargo build --release -p "$PACKAGE_NAME" --bin "$BINARY_NAME"
fi

BIN="${CAPTAIN_BIN_PATH:-$ROOT_DIR/target/release/$BINARY_NAME}"
[ -x "$BIN" ] || fail "Missing release binary: $BIN"

VERSION_DIR="$DIST_ROOT/$VERSION"
ARCHIVE_ROOT="$ARCHIVE_PREFIX-$PLATFORM"
STAGE="$VERSION_DIR/stage/$ARCHIVE_ROOT"
ARCHIVE="$VERSION_DIR/$ARCHIVE_ROOT.tar.gz"

rm -rf "$STAGE"
mkdir -p "$STAGE" "$VERSION_DIR"

cp "$BIN" "$STAGE/$BINARY_NAME"
chmod +x "$STAGE/$BINARY_NAME"

if [ "$COMPONENT" = "full" ] && [ -f "$ROOT_DIR/captain.toml.example" ]; then
    cp "$ROOT_DIR/captain.toml.example" "$STAGE/captain.toml.example"
fi

cat > "$STAGE/VERSION" <<EOF
$VERSION
EOF

cat > "$STAGE/README.txt" <<EOF
Captain precompiled $BUNDLE_DESCRIPTION

Version:   $VERSION
Component: $COMPONENT
Platform:  $PLATFORM

Verify $(basename "$ARCHIVE").sha256 before installing.
EOF

if [ "$COMPONENT" = "full" ]; then
    cat >> "$STAGE/README.txt" <<EOF

Local install:
  Copy install.sh, $(basename "$ARCHIVE"), and $(basename "$ARCHIVE").sha256
  into the same directory, then run:
    bash install.sh

Strict local install without network fallback:
    bash install-local.sh

GitHub install path:
    bash install-git.sh
EOF
else
    cat >> "$STAGE/README.txt" <<EOF

Local install:
  Copy install-edition.sh, $(basename "$ARCHIVE"), and
  $(basename "$ARCHIVE").sha256 into the same directory, then run:
    CAPTAIN_EDITION=$COMPONENT CAPTAIN_BUNDLE_PATH=$(basename "$ARCHIVE") bash install-edition.sh
EOF
fi

cat >> "$STAGE/README.txt" <<EOF

This bundle is produced by scripts/package-release.sh. End users should not
compile Captain during installation.
EOF

clear_macos_attrs "$STAGE"
if [ "$OS" = "darwin" ]; then
    command -v codesign >/dev/null 2>&1 \
        || fail "codesign is required for macOS release bundles"
    if ! codesign --verify "$STAGE/$BINARY_NAME" >/dev/null 2>&1; then
        codesign --force --sign - "$STAGE/$BINARY_NAME" >/dev/null 2>&1 \
            || fail "failed to ad-hoc sign $PLATFORM release binary"
    fi
    codesign --verify --verbose=2 "$STAGE/$BINARY_NAME" >/dev/null 2>&1 \
        || fail "failed to verify $PLATFORM release signature"
    clear_macos_attrs "$STAGE"
fi

create_archive
HASH=$(sha256_file "$ARCHIVE")
printf '%s  %s\n' "$HASH" "$(basename "$ARCHIVE")" > "$ARCHIVE.sha256"
printf '%s\n' "$VERSION" > "$DIST_ROOT/latest.txt"

write_manifests

cp "$ROOT_DIR/scripts/install.sh" "$VERSION_DIR/install.sh"
cp "$ROOT_DIR/scripts/install-local.sh" "$VERSION_DIR/install-local.sh"
cp "$ROOT_DIR/scripts/install-git.sh" "$VERSION_DIR/install-git.sh"
cp "$ROOT_DIR/scripts/install-edition.sh" "$VERSION_DIR/install-edition.sh"
cp "$ROOT_DIR/scripts/install-edition.ps1" "$VERSION_DIR/install-edition.ps1"
chmod +x \
    "$VERSION_DIR/install.sh" \
    "$VERSION_DIR/install-local.sh" \
    "$VERSION_DIR/install-git.sh" \
    "$VERSION_DIR/install-edition.sh"

clear_macos_attrs \
    "$VERSION_DIR" \
    "$ARCHIVE" \
    "$ARCHIVE.sha256" \
    "$VERSION_DIR/manifest.json" \
    "$VERSION_DIR/$MANIFEST_NAME" \
    "$VERSION_DIR/install.sh" \
    "$VERSION_DIR/install-local.sh" \
    "$VERSION_DIR/install-git.sh" \
    "$VERSION_DIR/install-edition.sh" \
    "$VERSION_DIR/install-edition.ps1" \
    "$DIST_ROOT/latest.txt"

rm -rf "$VERSION_DIR/stage"

echo "  Bundle:   $ARCHIVE"
echo "  Checksum: $ARCHIVE.sha256"
echo "  Latest:   $DIST_ROOT/latest.txt"
echo ""
