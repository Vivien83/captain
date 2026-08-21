#!/usr/bin/env bash
# Isolated contract test for the distinct Full, Console, and Node bundles.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/captain-lightweight-release.XXXXXX")"
INSTALL_ROOT="$TMP_ROOT"
DIST_ROOT="$TMP_ROOT/releases"
VERSION="v0.0.0-lightweight-release-test"

cleanup() {
    rm -rf "$TMP_ROOT"
    if [ "$INSTALL_ROOT" != "$TMP_ROOT" ]; then
        rm -rf "$INSTALL_ROOT"
    fi
}
trap cleanup EXIT

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    else
        shasum -a 256 "$1" | cut -d ' ' -f 1
    fi
}

case "$(uname -m)" in
    arm64|aarch64) arch="aarch64" ;;
    x86_64|amd64) arch="x86_64" ;;
    *) printf 'unsupported test architecture\n' >&2; exit 1 ;;
esac
case "$(uname -s)" in
    Darwin)
        platform="${arch}-apple-darwin"
        INSTALL_ROOT="$(mktemp -d "$HOME/.captain-lightweight-release.XXXXXX")"
        ;;
    Linux) platform="${arch}-unknown-linux-gnu" ;;
    *) printf 'unsupported test platform\n' >&2; exit 1 ;;
esac

CAPTAIN_BUILD_VERSION="$VERSION" cargo build --quiet -p captain-console --bin captain-console
CAPTAIN_BUILD_VERSION="$VERSION" cargo build --quiet -p captain-node --bin captain-node

for component in full console node; do
    case "$component" in
        full) binary_path="$ROOT_DIR/target/debug/captain-console" ;;
        console) binary_path="$ROOT_DIR/target/debug/captain-console" ;;
        node) binary_path="$ROOT_DIR/target/debug/captain-node" ;;
    esac
    CAPTAIN_SKIP_BUILD=1 \
    CAPTAIN_BIN_PATH="$binary_path" \
    CAPTAIN_DIST_COMPONENT="$component" \
    CAPTAIN_DIST_PLATFORM="$platform" \
    CAPTAIN_VERSION="$VERSION" \
    CAPTAIN_DIST_DIR="$DIST_ROOT" \
        "$ROOT_DIR/scripts/package-release.sh" >/dev/null
done

version_dir="$DIST_ROOT/$VERSION"
jq -e \
    --arg platform "$platform" \
    '(.artifacts | length) == 3
     and ([.artifacts[].component] | sort) == ["console", "full", "node"]
     and all(.artifacts[]; .platform == $platform)' \
    "$version_dir/manifest.json" >/dev/null

for component in console node; do
    archive="$version_dir/captain-$component-$platform.tar.gz"
    binary="captain-$component"
    root="captain-$component-$platform"
    [ -f "$archive" ] || exit 1
    tar -tzf "$archive" | grep -Fx "$root/$binary" >/dev/null
    tar -tzf "$archive" | grep -Fx "$root/VERSION" >/dev/null
    if tar -tzf "$archive" | grep -Fq 'captain.toml.example'; then
        printf '%s bundle contains Full configuration\n' "$component" >&2
        exit 1
    fi

    home="$INSTALL_ROOT/home-$component"
    destination="$INSTALL_ROOT/bin-$component"
    mkdir -p "$home"
    HOME="$home" \
    CAPTAIN_UPDATE_PATH=0 \
    CAPTAIN_EDITION="$component" \
    CAPTAIN_VERSION="$VERSION" \
    CAPTAIN_INSTALL_DIR="$destination" \
    CAPTAIN_BUNDLE_PATH="$archive" \
        "$ROOT_DIR/scripts/install-edition.sh" >/dev/null
    [ -x "$destination/$binary" ] || exit 1
    [ "$(find "$home" -mindepth 1 -print -quit)" = "" ] \
        || { printf '%s installer created runtime state\n' "$component" >&2; exit 1; }
done

tampered="$TMP_ROOT/tampered.tar.gz"
cp "$version_dir/captain-console-$platform.tar.gz" "$tampered"
printf 'tamper\n' >>"$tampered"
original_hash="$(awk 'NR == 1 { print $1 }' \
    "$version_dir/captain-console-$platform.tar.gz.sha256")"
printf '%s  %s\n' "$original_hash" "$(basename "$tampered")" >"$tampered.sha256"
if HOME="$TMP_ROOT/tampered-home" \
    CAPTAIN_UPDATE_PATH=0 \
    CAPTAIN_EDITION=console \
    CAPTAIN_VERSION="$VERSION" \
    CAPTAIN_INSTALL_DIR="$TMP_ROOT/tampered-bin" \
    CAPTAIN_BUNDLE_PATH="$tampered" \
        "$ROOT_DIR/scripts/install-edition.sh" >/dev/null 2>&1; then
    printf 'lightweight installer accepted a tampered archive\n' >&2
    exit 1
fi

console_destination="$INSTALL_ROOT/bin-console/captain-console"
console_before="$(sha256_file "$console_destination")"
bad_dist="$TMP_ROOT/bad-releases"
CAPTAIN_SKIP_BUILD=1 \
CAPTAIN_BIN_PATH="$ROOT_DIR/target/debug/captain-node" \
CAPTAIN_DIST_COMPONENT=console \
CAPTAIN_DIST_PLATFORM="$platform" \
CAPTAIN_VERSION="$VERSION" \
CAPTAIN_DIST_DIR="$bad_dist" \
    "$ROOT_DIR/scripts/package-release.sh" >/dev/null
if HOME="$TMP_ROOT/rollback-home" \
    CAPTAIN_UPDATE_PATH=0 \
    CAPTAIN_EDITION=console \
    CAPTAIN_VERSION="$VERSION" \
    CAPTAIN_INSTALL_DIR="$INSTALL_ROOT/bin-console" \
    CAPTAIN_BUNDLE_PATH="$bad_dist/$VERSION/captain-console-$platform.tar.gz" \
        "$ROOT_DIR/scripts/install-edition.sh" >/dev/null 2>&1; then
    printf 'lightweight installer accepted a binary with the wrong version\n' >&2
    exit 1
fi
console_after="$(sha256_file "$console_destination")"
[ "$console_after" = "$console_before" ] \
    || { printf 'lightweight installer did not restore the previous binary\n' >&2; exit 1; }

printf 'lightweight release contract test passed\n'
