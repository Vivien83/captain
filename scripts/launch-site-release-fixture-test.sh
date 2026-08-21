#!/usr/bin/env bash
# Build and audit the launch site against the exact Alpha.15 release matrix.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
VERSION="v0.1.0-alpha.15"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/captain-launch-site-fixture.XXXXXX")"
RELEASES="$TMP_ROOT/releases"
RELEASE_DIR="$RELEASES/$VERSION"
SITE_OUT="$TMP_ROOT/captain-site-fixture"
ARTIFACTS_JSON="$TMP_ROOT/artifacts.json"

trap 'rm -rf "$TMP_ROOT"' EXIT

fail() {
  printf 'Launch site release fixture failed: %s\n' "$1" >&2
  exit 1
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{ print $1 }'
  else
    shasum -a 256 "$1" | awk '{ print $1 }'
  fi
}

for command_name in jq tar unzip zip; do
  command -v "$command_name" >/dev/null 2>&1 \
    || fail "$command_name is required"
done
if ! command -v sha256sum >/dev/null 2>&1 && ! command -v shasum >/dev/null 2>&1; then
  fail "sha256sum or shasum is required"
fi

mkdir -p "$RELEASE_DIR"
printf '%s\n' "$VERSION" >"$RELEASES/latest.txt"
printf '[]\n' >"$ARTIFACTS_JSON"

for component in full console node; do
  case "$component" in
    full) archive_prefix="captain"; binary_name="captain" ;;
    console) archive_prefix="captain-console"; binary_name="captain-console" ;;
    node) archive_prefix="captain-node"; binary_name="captain-node" ;;
  esac

  for platform in \
    aarch64-apple-darwin \
    aarch64-unknown-linux-gnu \
    x86_64-apple-darwin \
    x86_64-pc-windows-msvc \
    x86_64-unknown-linux-gnu; do
    stage="$TMP_ROOT/stage-$component-$platform"
    rm -rf "$stage"
    mkdir -p "$stage"

    if [[ "$platform" == "x86_64-pc-windows-msvc" ]]; then
      archive_name="$archive_prefix-$platform.zip"
      printf 'MZ Captain fixture\n' >"$stage/$binary_name.exe"
      printf '%s\n' "$VERSION" >"$stage/VERSION"
      printf 'Captain %s fixture\n' "$component" >"$stage/README.txt"
      (
        cd "$stage"
        zip -q "$RELEASE_DIR/$archive_name" "$binary_name.exe" VERSION README.txt
      )
    else
      archive_root="$archive_prefix-$platform"
      archive_name="$archive_root.tar.gz"
      mkdir -p "$stage/$archive_root"
      printf '#!/usr/bin/env sh\nexit 0\n' >"$stage/$archive_root/$binary_name"
      chmod +x "$stage/$archive_root/$binary_name"
      printf '%s\n' "$VERSION" >"$stage/$archive_root/VERSION"
      printf 'Captain %s fixture\n' "$component" >"$stage/$archive_root/README.txt"
      tar -czf "$RELEASE_DIR/$archive_name" -C "$stage" "$archive_root"
    fi

    hash="$(sha256_file "$RELEASE_DIR/$archive_name")"
    printf '%s  %s\n' "$hash" "$archive_name" >"$RELEASE_DIR/$archive_name.sha256"
    jq \
      --arg component "$component" \
      --arg platform "$platform" \
      --arg archive "$archive_name" \
      --arg sha256 "$hash" \
      '. + [{component: $component, platform: $platform, archive: $archive, sha256: $sha256}]' \
      "$ARTIFACTS_JSON" >"$ARTIFACTS_JSON.next"
    mv "$ARTIFACTS_JSON.next" "$ARTIFACTS_JSON"
  done
done

jq -n \
  --arg version "$VERSION" \
  --slurpfile artifacts "$ARTIFACTS_JSON" \
  '{version: $version, artifacts: $artifacts[0]}' \
  >"$RELEASE_DIR/manifest.json"

for installer in \
  install.sh install-local.sh install-git.sh install.ps1 \
  install-edition.sh install-edition.ps1; do
  cp "$ROOT_DIR/scripts/$installer" "$RELEASE_DIR/$installer"
done

CAPTAIN_SITE_OUT="$SITE_OUT" \
CAPTAIN_SITE_ACCESS_MODE=public \
CAPTAIN_SITE_PUBLIC_ORIGIN=https://captainagent.fr \
CAPTAIN_SITE_RELEASES_SOURCE="$RELEASES" \
  "$ROOT_DIR/scripts/build-launch-site.sh" >/dev/null

CAPTAIN_SITE_AUDIT_ROOT="$SITE_OUT" \
  "$ROOT_DIR/scripts/launch-site-audit.sh"

if [[ "${CAPTAIN_LAUNCH_SITE_BROWSER_SMOKE:-0}" == "1" ]]; then
  CAPTAIN_SITE_AUDIT_ROOT="$SITE_OUT" \
    node "$ROOT_DIR/scripts/launch-site-browser-smoke.mjs"
fi

# A valid checksum must not hide an archive whose embedded version is wrong.
bad_component="console"
bad_platform="aarch64-apple-darwin"
bad_archive="captain-console-$bad_platform.tar.gz"
bad_root="captain-console-$bad_platform"
bad_stage="$TMP_ROOT/stage-$bad_component-$bad_platform"
printf 'v0.1.0-invalid\n' >"$bad_stage/$bad_root/VERSION"
tar -czf "$RELEASE_DIR/$bad_archive" -C "$bad_stage" "$bad_root"
bad_hash="$(sha256_file "$RELEASE_DIR/$bad_archive")"
printf '%s  %s\n' "$bad_hash" "$bad_archive" >"$RELEASE_DIR/$bad_archive.sha256"
jq \
  --arg component "$bad_component" \
  --arg platform "$bad_platform" \
  --arg sha256 "$bad_hash" \
  '(.artifacts[] | select(.component == $component and .platform == $platform).sha256) = $sha256' \
  "$RELEASE_DIR/manifest.json" >"$RELEASE_DIR/manifest.json.next"
mv "$RELEASE_DIR/manifest.json.next" "$RELEASE_DIR/manifest.json"

if CAPTAIN_SITE_OUT="$TMP_ROOT/captain-site-invalid-payload" \
  CAPTAIN_SITE_ACCESS_MODE=public \
  CAPTAIN_SITE_PUBLIC_ORIGIN=https://captainagent.fr \
  CAPTAIN_SITE_RELEASES_SOURCE="$RELEASES" \
    "$ROOT_DIR/scripts/build-launch-site.sh" >/dev/null 2>&1; then
  fail "build accepted an archive with a mismatched embedded version"
fi

printf 'Launch site Alpha.15 fixture passed: 15 bundles and 6 installers.\n'
