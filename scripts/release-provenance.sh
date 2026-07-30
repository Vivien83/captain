#!/usr/bin/env bash
# Generate or verify a deterministic SLSA v1 provenance statement for the
# complete set of Captain host release assets.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$ROOT_DIR"

fail() {
    printf '  Error: %s\n' "$*" >&2
    exit 1
}

is_yes() {
    case "${1:-}" in
        1|true|TRUE|yes|YES|y|Y) return 0 ;;
        *) return 1 ;;
    esac
}

need_cmd() {
    command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
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

usage() {
    cat <<'EOF'
Usage: scripts/release-provenance.sh [--verify]

Environment:
  CAPTAIN_VERSION                    Release tag (or dist/releases/latest.txt)
  CAPTAIN_DIST_DIR                   Release output root (default: dist/releases)
  CAPTAIN_PROVENANCE_REPOSITORY      Canonical public source URI
  CAPTAIN_PROVENANCE_ALLOW_DIRTY     Test-only opt-out from the clean-tree guard
EOF
}

MODE="generate"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --verify) MODE="verify" ;;
        -h|--help)
            usage
            exit 0
            ;;
        *) fail "unknown argument: $1" ;;
    esac
    shift
done

need_cmd git
need_cmd jq
need_cmd rustc
need_cmd cargo

VERSION="${CAPTAIN_VERSION:-$(cat dist/releases/latest.txt 2>/dev/null || true)}"
[ -n "$VERSION" ] || fail "CAPTAIN_VERSION is empty and dist/releases/latest.txt is missing"
case "$VERSION" in
    v*) ;;
    *) fail "release version must be a Git tag beginning with v (got: $VERSION)" ;;
esac

DIST_ROOT="${CAPTAIN_DIST_DIR:-$ROOT_DIR/dist/releases}"
case "$DIST_ROOT" in
    /*) ;;
    *) DIST_ROOT="$ROOT_DIR/$DIST_ROOT" ;;
esac
VERSION_DIR="$DIST_ROOT/$VERSION"
[ -d "$VERSION_DIR" ] || fail "release directory not found: $VERSION_DIR"

SOURCE_DIRTY=false
if [ -n "$(git status --porcelain)" ]; then
    if ! is_yes "${CAPTAIN_PROVENANCE_ALLOW_DIRTY:-}"; then
        fail "source worktree must be clean before generating or verifying release provenance"
    fi
    SOURCE_DIRTY=true
fi

SOURCE_REPOSITORY="${CAPTAIN_PROVENANCE_REPOSITORY:-https://github.com/Vivien83/captain}"
case "$SOURCE_REPOSITORY" in
    https://github.com/*) ;;
    *) fail "CAPTAIN_PROVENANCE_REPOSITORY must be a canonical HTTPS GitHub URI" ;;
esac
SOURCE_REVISION="$(git rev-parse HEAD)"
SOURCE_TREE="$(git rev-parse 'HEAD^{tree}')"
LOCK_SHA256="$(sha256_file "$ROOT_DIR/Cargo.lock")"
PROVENANCE="$VERSION_DIR/provenance.intoto.jsonl"
PROVENANCE_CHECKSUM="$PROVENANCE.sha256"

platforms=(
    aarch64-apple-darwin
    x86_64-apple-darwin
    aarch64-unknown-linux-gnu
    x86_64-unknown-linux-gnu
    x86_64-pc-windows-msvc
)

assets=()
for platform in "${platforms[@]}"; do
    case "$platform" in
        *-pc-windows-msvc) archive="$VERSION_DIR/captain-$platform.zip" ;;
        *) archive="$VERSION_DIR/captain-$platform.tar.gz" ;;
    esac
    checksum="$archive.sha256"
    manifest="$VERSION_DIR/manifest-$platform.json"
    [ -f "$archive" ] || fail "missing archive: $archive"
    [ -f "$checksum" ] || fail "missing checksum: $checksum"
    [ -f "$manifest" ] || fail "missing platform manifest: $manifest"

    actual_hash="$(sha256_file "$archive")"
    expected_hash="$(cut -d ' ' -f 1 <"$checksum")"
    [ "$actual_hash" = "$expected_hash" ] || fail "checksum mismatch: $archive"
    jq -e \
        --arg version "$VERSION" \
        --arg platform "$platform" \
        --arg archive "$(basename "$archive")" \
        --arg sha256 "$actual_hash" \
        --arg source_repository "$SOURCE_REPOSITORY" \
        --arg source_revision "$SOURCE_REVISION" \
        --arg source_tree "$SOURCE_TREE" \
        --arg lock_sha256 "$LOCK_SHA256" \
        --argjson source_dirty "$SOURCE_DIRTY" \
        '.version == $version
         and .platform == $platform
         and .archive == $archive
         and .sha256 == $sha256
         and (.build_started_at
              | type == "string"
                and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"))
         and (.generated_at
              | type == "string"
                and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"))
         and .build_started_at <= .generated_at
         and .source == {
           repository: $source_repository,
           revision: $source_revision,
           tree: $source_tree,
           cargo_lock_sha256: $lock_sha256,
           dirty: $source_dirty
         }' \
        "$manifest" >/dev/null || fail "platform manifest mismatch: $manifest"
    assets+=("$archive" "$checksum" "$manifest")
done

for installer in install.sh install-local.sh install-git.sh install.ps1; do
    path="$VERSION_DIR/$installer"
    [ -f "$path" ] || fail "missing installer: $path"
    assets+=("$path")
done

aggregate="$VERSION_DIR/manifest.json"
[ -f "$aggregate" ] || fail "missing aggregate manifest: $aggregate"
jq -e \
    --arg version "$VERSION" \
    --arg source_repository "$SOURCE_REPOSITORY" \
    --arg source_revision "$SOURCE_REVISION" \
    --arg source_tree "$SOURCE_TREE" \
    --arg lock_sha256 "$LOCK_SHA256" \
    --argjson source_dirty "$SOURCE_DIRTY" \
    '.version == $version
     and ([.artifacts[].platform] | sort) == [
       "aarch64-apple-darwin",
       "aarch64-unknown-linux-gnu",
       "x86_64-apple-darwin",
       "x86_64-pc-windows-msvc",
       "x86_64-unknown-linux-gnu"
     ]
     and .source == {
       repository: $source_repository,
       revision: $source_revision,
       tree: $source_tree,
       cargo_lock_sha256: $lock_sha256,
       dirty: $source_dirty
     }' \
    "$aggregate" >/dev/null || fail "aggregate manifest does not describe five unique platforms"
assets+=("$aggregate")

subjects_json="$(
    for asset in "${assets[@]}"; do
        jq -cn \
            --arg name "$(basename "$asset")" \
            --arg sha256 "$(sha256_file "$asset")" \
            '{name: $name, digest: {sha256: $sha256}}'
    done | jq -sc 'sort_by(.name)'
)"

started_on="$(
    for platform in "${platforms[@]}"; do
        jq -r '.build_started_at' "$VERSION_DIR/manifest-$platform.json"
    done | LC_ALL=C sort | head -n 1
)"
finished_on="$(
    for platform in "${platforms[@]}"; do
        jq -r '.generated_at' "$VERSION_DIR/manifest-$platform.json"
    done | LC_ALL=C sort | tail -n 1
)"
aggregate_sha256="$(sha256_file "$aggregate")"
invocation_id="$(
    printf '%s\n' "$VERSION" "$SOURCE_REVISION" "$SOURCE_TREE" "$LOCK_SHA256" "$aggregate_sha256" \
        | if command -v sha256sum >/dev/null 2>&1; then
              sha256sum | cut -d ' ' -f 1
          else
              shasum -a 256 | cut -d ' ' -f 1
          fi
)"

verify_statement() {
    [ -f "$PROVENANCE" ] || fail "missing provenance statement: $PROVENANCE"
    [ -f "$PROVENANCE_CHECKSUM" ] || fail "missing provenance checksum: $PROVENANCE_CHECKSUM"
    expected_provenance_hash="$(cut -d ' ' -f 1 <"$PROVENANCE_CHECKSUM")"
    actual_provenance_hash="$(sha256_file "$PROVENANCE")"
    [ "$actual_provenance_hash" = "$expected_provenance_hash" ] \
        || fail "provenance checksum mismatch"

    jq -e \
        --arg version "$VERSION" \
        --arg source_uri "git+$SOURCE_REPOSITORY" \
        --arg source_revision "$SOURCE_REVISION" \
        --arg source_tree "$SOURCE_TREE" \
        --arg lock_sha256 "$LOCK_SHA256" \
        --arg invocation_id "urn:sha256:$invocation_id" \
        --arg started_on "$started_on" \
        --arg finished_on "$finished_on" \
        --argjson source_dirty "$SOURCE_DIRTY" \
        --argjson subjects "$subjects_json" \
        '._type == "https://in-toto.io/Statement/v1"
         and .predicateType == "https://slsa.dev/provenance/v1"
         and .subject == $subjects
         and .predicate.buildDefinition.buildType
             == "https://github.com/Vivien83/captain/blob/main/docs/release-provenance.md"
         and .predicate.buildDefinition.externalParameters.version == $version
         and .predicate.buildDefinition.externalParameters.targets == [
           "aarch64-apple-darwin",
           "x86_64-apple-darwin",
           "aarch64-unknown-linux-gnu",
           "x86_64-unknown-linux-gnu",
           "x86_64-pc-windows-msvc"
         ]
         and .predicate.buildDefinition.resolvedDependencies == [
           {
             "uri": $source_uri,
             "digest": {"gitCommit": $source_revision, "gitTree": $source_tree}
           },
           {
             "uri": "file:Cargo.lock",
             "digest": {"sha256": $lock_sha256}
           }
         ]
         and .predicate.runDetails.metadata.invocationId == $invocation_id
         and .predicate.runDetails.metadata.startedOn == $started_on
         and .predicate.runDetails.metadata.finishedOn == $finished_on
         and .predicate.buildDefinition.internalParameters.sourceDirty == $source_dirty' \
        "$PROVENANCE" >/dev/null || fail "release provenance statement does not match source and assets"
}

if [ "$MODE" = "verify" ]; then
    verify_statement
    printf 'release provenance verified: %s subjects, source %s\n' \
        "${#assets[@]}" "$SOURCE_REVISION"
    exit 0
fi

rustc_version="$(rustc --version)"
cargo_version="$(cargo --version)"
builder_platform="$(uname -srm)"
tmp_provenance="$(mktemp "$VERSION_DIR/.provenance.intoto.jsonl.XXXXXX")"
cleanup() {
    rm -f "$tmp_provenance"
}
trap cleanup EXIT

jq -cn \
    --argjson subjects "$subjects_json" \
    --arg version "$VERSION" \
    --arg source_uri "git+$SOURCE_REPOSITORY" \
    --arg source_revision "$SOURCE_REVISION" \
    --arg source_tree "$SOURCE_TREE" \
    --arg lock_sha256 "$LOCK_SHA256" \
    --arg rustc "$rustc_version" \
    --arg cargo "$cargo_version" \
    --arg builder_platform "$builder_platform" \
    --arg builder_id "$SOURCE_REPOSITORY#local-maintainer-workstation" \
    --arg invocation_id "urn:sha256:$invocation_id" \
    --arg started_on "$started_on" \
    --arg finished_on "$finished_on" \
    --argjson source_dirty "$SOURCE_DIRTY" \
    '{
      "_type": "https://in-toto.io/Statement/v1",
      "subject": $subjects,
      "predicateType": "https://slsa.dev/provenance/v1",
      "predicate": {
        "buildDefinition": {
          "buildType": "https://github.com/Vivien83/captain/blob/main/docs/release-provenance.md",
          "externalParameters": {
            "version": $version,
            "cargoProfile": "release",
            "targets": [
              "aarch64-apple-darwin",
              "x86_64-apple-darwin",
              "aarch64-unknown-linux-gnu",
              "x86_64-unknown-linux-gnu",
              "x86_64-pc-windows-msvc"
            ],
            "scripts": [
              "scripts/release-all.sh",
              "scripts/package-release.sh"
            ]
          },
          "internalParameters": {
            "builderPlatform": $builder_platform,
            "rustc": $rustc,
            "cargo": $cargo,
            "sourceDirty": $source_dirty,
            "parallelTargetBuilds": false
          },
          "resolvedDependencies": [
            {
              "uri": $source_uri,
              "digest": {
                "gitCommit": $source_revision,
                "gitTree": $source_tree
              }
            },
            {
              "uri": "file:Cargo.lock",
              "digest": {"sha256": $lock_sha256}
            }
          ]
        },
        "runDetails": {
          "builder": {"id": $builder_id},
          "metadata": {
            "invocationId": $invocation_id,
            "startedOn": $started_on,
            "finishedOn": $finished_on
          }
        }
      }
    }' >"$tmp_provenance"

mv "$tmp_provenance" "$PROVENANCE"
provenance_hash="$(sha256_file "$PROVENANCE")"
printf '%s  %s\n' "$provenance_hash" "$(basename "$PROVENANCE")" >"$PROVENANCE_CHECKSUM"
verify_statement
printf 'release provenance generated: %s subjects, source %s\n' \
    "${#assets[@]}" "$SOURCE_REVISION"
