#!/usr/bin/env bash
# Isolated fail-closed contract test for release-all.sh.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/captain-release-all-test.XXXXXX")"
MOCK_BIN="$TMP_ROOT/bin"
TARGET_ROOT="$TMP_ROOT/target"
DIST_ROOT="$TMP_ROOT/dist"
TARGET="x86_64-pc-windows-msvc"
VERSION="0.0.0-release-failure-test"
STALE_BIN="$TARGET_ROOT/xwin-$TARGET/$TARGET/release/captain.exe"
OUTPUT="$TMP_ROOT/output.log"

cleanup() {
    rm -rf "$TMP_ROOT"
}
trap cleanup EXIT

mkdir -p "$MOCK_BIN" "$(dirname "$STALE_BIN")"
printf 'stale binary\n' >"$STALE_BIN"

for command in cargo-xwin llvm-ar nasm; do
    printf '#!/bin/sh\nexit 0\n' >"$MOCK_BIN/$command"
    chmod +x "$MOCK_BIN/$command"
done
printf '#!/bin/sh\nexit 42\n' >"$MOCK_BIN/cargo"
chmod +x "$MOCK_BIN/cargo"

if PATH="$MOCK_BIN:$PATH" \
    CAPTAIN_VERSION="$VERSION" \
    CAPTAIN_RELEASE_TARGETS="$TARGET" \
    CAPTAIN_DIST_DIR="$DIST_ROOT" \
    CARGO_TARGET_DIR="$TARGET_ROOT" \
        "$ROOT_DIR/scripts/release-all.sh" >"$OUTPUT" 2>&1; then
    printf 'release-all accepted a failed Windows compiler invocation\n' >&2
    exit 1
fi

if ! rg -Fq "release build failed for $TARGET" "$OUTPUT"; then
    printf 'release-all did not report the failed Windows build\n' >&2
    exit 1
fi
if [ -e "$STALE_BIN" ]; then
    printf 'release-all retained a stale Windows binary after build failure\n' >&2
    exit 1
fi
if [ -e "$DIST_ROOT/$VERSION/captain-$TARGET.zip" ]; then
    printf 'release-all packaged a stale Windows binary after build failure\n' >&2
    exit 1
fi

printf 'release-all fail-closed contract test passed\n'
