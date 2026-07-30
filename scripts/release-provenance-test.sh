#!/usr/bin/env bash
# Isolated contract test for release-provenance.sh.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
VERSION="v0.0.0-provenance-test"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/captain-provenance-test.XXXXXX")"
DIST_ROOT="$TMP_ROOT/releases"
VERSION_DIR="$DIST_ROOT/$VERSION"

cleanup() {
    rm -rf "$TMP_ROOT"
}
trap cleanup EXIT

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    else
        shasum -a 256 "$1" | cut -d ' ' -f 1
    fi
}

mkdir -p "$VERSION_DIR"
source_revision="$(git -C "$ROOT_DIR" rev-parse HEAD)"
source_tree="$(git -C "$ROOT_DIR" rev-parse 'HEAD^{tree}')"
lock_sha256="$(sha256_file "$ROOT_DIR/Cargo.lock")"
source_repository="https://github.com/Vivien83/captain"
source_dirty=false
if [ -n "$(git -C "$ROOT_DIR" status --porcelain)" ]; then
    source_dirty=true
fi

package_dist="$TMP_ROOT/package-release"
CAPTAIN_SKIP_BUILD=1 \
CAPTAIN_BIN_PATH=/bin/echo \
CAPTAIN_VERSION="$VERSION" \
CAPTAIN_DIST_DIR="$package_dist" \
    "$ROOT_DIR/scripts/package-release.sh" >/dev/null
jq -e \
    --arg repository "$source_repository" \
    --arg revision "$source_revision" \
    --arg tree "$source_tree" \
    --arg lock_sha256 "$lock_sha256" \
    '.source.repository == $repository
     and .source.revision == $revision
     and .source.tree == $tree
     and .source.cargo_lock_sha256 == $lock_sha256
     and (.source.dirty | type == "boolean")
     and (.artifacts | length) == 1' \
    "$package_dist/$VERSION/manifest.json" >/dev/null
jq -e '
    .build_started_at
    | type == "string"
      and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$")
' "$package_dist/$VERSION"/manifest-*.json >/dev/null

platforms=(
    aarch64-apple-darwin
    x86_64-apple-darwin
    aarch64-unknown-linux-gnu
    x86_64-unknown-linux-gnu
    x86_64-pc-windows-msvc
)

for platform in "${platforms[@]}"; do
    case "$platform" in
        *-pc-windows-msvc) archive="$VERSION_DIR/captain-$platform.zip" ;;
        *) archive="$VERSION_DIR/captain-$platform.tar.gz" ;;
    esac
    printf 'fixture %s\n' "$platform" >"$archive"
    hash="$(sha256_file "$archive")"
    printf '%s  %s\n' "$hash" "$(basename "$archive")" >"$archive.sha256"
    jq -n \
        --arg version "$VERSION" \
        --arg platform "$platform" \
        --arg archive "$(basename "$archive")" \
        --arg sha256 "$hash" \
        --arg build_started_at "2026-07-30T11:00:00Z" \
        --arg generated_at "2026-07-30T12:00:00Z" \
        --arg repository "$source_repository" \
        --arg revision "$source_revision" \
        --arg tree "$source_tree" \
        --arg cargo_lock_sha256 "$lock_sha256" \
        --argjson source_dirty "$source_dirty" \
        '{
          version:$version,
          platform:$platform,
          archive:$archive,
          sha256:$sha256,
          build_started_at:$build_started_at,
          generated_at:$generated_at,
          source:{
            repository:$repository,
            revision:$revision,
            tree:$tree,
            cargo_lock_sha256:$cargo_lock_sha256,
            dirty:$source_dirty
          }
        }' \
        >"$VERSION_DIR/manifest-$platform.json"
done

for installer in install.sh install-local.sh install-git.sh install.ps1; do
    printf 'fixture %s\n' "$installer" >"$VERSION_DIR/$installer"
done

jq -s \
    --arg version "$VERSION" \
    --arg generated_at "2026-07-30T12:00:00Z" \
    '{
      version:$version,
      generated_at:$generated_at,
      source:.[0].source,
      artifacts:(map({platform,archive,sha256})|sort_by(.platform))
    }' \
    "$VERSION_DIR"/manifest-*.json >"$VERSION_DIR/manifest.json"

CAPTAIN_VERSION="$VERSION" \
CAPTAIN_DIST_DIR="$DIST_ROOT" \
CAPTAIN_PROVENANCE_ALLOW_DIRTY=1 \
    "$ROOT_DIR/scripts/release-provenance.sh"
jq -e '
    .predicate.runDetails.metadata.startedOn == "2026-07-30T11:00:00Z"
    and .predicate.runDetails.metadata.finishedOn == "2026-07-30T12:00:00Z"
' "$VERSION_DIR/provenance.intoto.jsonl" >/dev/null
CAPTAIN_VERSION="$VERSION" \
CAPTAIN_DIST_DIR="$DIST_ROOT" \
CAPTAIN_PROVENANCE_ALLOW_DIRTY=1 \
    "$ROOT_DIR/scripts/release-provenance.sh" --verify

mixed_manifest="$VERSION_DIR/manifest-x86_64-apple-darwin.json"
cp "$mixed_manifest" "$mixed_manifest.clean"
jq '.source.revision = "0000000000000000000000000000000000000000"' \
    "$mixed_manifest.clean" >"$mixed_manifest"
if CAPTAIN_VERSION="$VERSION" \
    CAPTAIN_DIST_DIR="$DIST_ROOT" \
    CAPTAIN_PROVENANCE_ALLOW_DIRTY=1 \
        "$ROOT_DIR/scripts/release-provenance.sh" >/dev/null 2>&1; then
    printf 'mixed source revisions were accepted\n' >&2
    exit 1
fi
mv "$mixed_manifest.clean" "$mixed_manifest"

printf 'tampered\n' >>"$VERSION_DIR/captain-aarch64-apple-darwin.tar.gz"
if CAPTAIN_VERSION="$VERSION" \
    CAPTAIN_DIST_DIR="$DIST_ROOT" \
    CAPTAIN_PROVENANCE_ALLOW_DIRTY=1 \
        "$ROOT_DIR/scripts/release-provenance.sh" --verify >/dev/null 2>&1; then
    printf 'tampered release asset was accepted\n' >&2
    exit 1
fi

printf 'release provenance contract test passed\n'
