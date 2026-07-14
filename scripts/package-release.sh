#!/usr/bin/env bash
# Build a controlled Captain distribution bundle.
#
# This script is for maintainers/CI. End users should run install.sh, which
# downloads the precompiled bundle produced here and never compiles Captain.
#
# Environment:
#   CAPTAIN_VERSION    — release version folder/name (default: dev timestamp)
#   CAPTAIN_DIST_DIR   — output root (default: dist/releases)
#   CAPTAIN_SKIP_BUILD — 1/true/yes to reuse target/release/captain
#   CAPTAIN_DIST_PLATFORM — package as this platform instead of host platform
#   CAPTAIN_BIN_PATH   — binary to package (default: target/release/captain)

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
    platform_manifest="$VERSION_DIR/manifest-$PLATFORM.json"

    cat > "$platform_manifest" <<EOF
{
  "version": "$VERSION",
  "platform": "$PLATFORM",
  "archive": "$(basename "$ARCHIVE")",
  "sha256": "$HASH",
  "generated_at": "$generated_at"
}
EOF

    aggregate_manifest="$VERSION_DIR/manifest.json"
    cat > "$aggregate_manifest" <<EOF
{
  "version": "$VERSION",
  "generated_at": "$generated_at",
  "artifacts": [
EOF

    first=1
    for artifact in "$VERSION_DIR"/captain-*.tar.gz "$VERSION_DIR"/captain-*.zip; do
        [ -f "$artifact" ] || continue
        artifact_name=$(basename "$artifact")
        artifact_platform="${artifact_name#captain-}"
        case "$artifact_name" in
            *.tar.gz) artifact_platform="${artifact_platform%.tar.gz}" ;;
            *.zip) artifact_platform="${artifact_platform%.zip}" ;;
        esac
        artifact_hash=$(cut -d ' ' -f 1 < "$artifact.sha256" 2>/dev/null || sha256_file "$artifact")
        if [ "$first" = "1" ]; then
            first=0
        else
            printf ',\n' >> "$aggregate_manifest"
        fi
        printf '    {\n      "platform": "%s",\n      "archive": "%s",\n      "sha256": "%s"\n    }' \
            "$artifact_platform" \
            "$artifact_name" \
            "$artifact_hash" >> "$aggregate_manifest"
    done

    cat >> "$aggregate_manifest" <<EOF

  ]
}
EOF
}

create_archive() {
    if tar --help 2>/dev/null | grep -q -- "--no-mac-metadata"; then
        COPYFILE_DISABLE=1 tar --no-xattrs --no-mac-metadata --format ustar -czf "$ARCHIVE" -C "$VERSION_DIR/stage" "captain-$PLATFORM"
    elif tar --help 2>/dev/null | grep -q -- "--no-xattrs"; then
        COPYFILE_DISABLE=1 tar --no-xattrs --format ustar -czf "$ARCHIVE" -C "$VERSION_DIR/stage" "captain-$PLATFORM"
    else
        COPYFILE_DISABLE=1 tar --format ustar -czf "$ARCHIVE" -C "$VERSION_DIR/stage" "captain-$PLATFORM"
    fi
}

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd -P)
VERSION="${CAPTAIN_VERSION:-0.1.0-dev.$(date -u +%Y%m%d%H%M%S)}"
DIST_ROOT="${CAPTAIN_DIST_DIR:-$ROOT_DIR/dist/releases}"

detect_platform

echo ""
echo "  Captain Release Packager"
echo "  ========================"
echo "  Version:  $VERSION"
echo "  Platform: $PLATFORM"
echo ""

if ! is_yes "${CAPTAIN_SKIP_BUILD:-}"; then
    CAPTAIN_BUILD_VERSION="$VERSION" cargo build --release -p captain-cli
fi

BIN="${CAPTAIN_BIN_PATH:-$ROOT_DIR/target/release/captain}"
[ -x "$BIN" ] || fail "Missing release binary: $BIN"

VERSION_DIR="$DIST_ROOT/$VERSION"
STAGE="$VERSION_DIR/stage/captain-$PLATFORM"
ARCHIVE="$VERSION_DIR/captain-$PLATFORM.tar.gz"

rm -rf "$STAGE"
mkdir -p "$STAGE" "$VERSION_DIR"

cp "$BIN" "$STAGE/captain"
chmod +x "$STAGE/captain"

if [ -f "$ROOT_DIR/captain.toml.example" ]; then
    cp "$ROOT_DIR/captain.toml.example" "$STAGE/captain.toml.example"
fi

cat > "$STAGE/VERSION" <<EOF
$VERSION
EOF

cat > "$STAGE/README.txt" <<EOF
Captain precompiled distribution bundle

Version:  $VERSION
Platform: $PLATFORM

Local install:
  Copy install.sh, captain-$PLATFORM.tar.gz, and captain-$PLATFORM.tar.gz.sha256
  into the same directory, then run:
    bash install.sh

Strict local install without network fallback:
    bash install-local.sh

GitHub install path:
    bash install-git.sh

This bundle is produced by scripts/package-release.sh. End users should not
compile Captain during installation.
EOF

clear_macos_attrs "$STAGE"
if [ "$OS" = "darwin" ]; then
    command -v codesign >/dev/null 2>&1 \
        || fail "codesign is required for macOS release bundles"
    codesign --force --sign - "$STAGE/captain" >/dev/null 2>&1 \
        || fail "failed to ad-hoc sign $PLATFORM release binary"
    codesign --verify --verbose=2 "$STAGE/captain" >/dev/null 2>&1 \
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
chmod +x "$VERSION_DIR/install.sh" "$VERSION_DIR/install-local.sh" "$VERSION_DIR/install-git.sh"

clear_macos_attrs \
    "$VERSION_DIR" \
    "$ARCHIVE" \
    "$ARCHIVE.sha256" \
    "$VERSION_DIR/manifest.json" \
    "$VERSION_DIR/manifest-$PLATFORM.json" \
    "$VERSION_DIR/install.sh" \
    "$VERSION_DIR/install-local.sh" \
    "$VERSION_DIR/install-git.sh" \
    "$DIST_ROOT/latest.txt"

rm -rf "$VERSION_DIR/stage"

echo "  Bundle:   $ARCHIVE"
echo "  Checksum: $ARCHIVE.sha256"
echo "  Latest:   $DIST_ROOT/latest.txt"
echo ""
