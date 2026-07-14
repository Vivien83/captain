#!/usr/bin/env bash
# Publish a complete Captain release from a maintainer workstation.
#
# This path intentionally does not depend on GitHub Actions. It validates the
# five local CLI bundles, pushes a multi-architecture image to GHCR, creates
# and pushes the release tag, then creates or refreshes the GitHub Release.
#
# Environment:
#   CAPTAIN_VERSION             Release tag (defaults to dist/releases/latest.txt)
#   CAPTAIN_IMAGE               GHCR image (derived from the GitHub repo owner)
#   CAPTAIN_REPO                owner/repo (derived with gh repo view)
#   CAPTAIN_FASTEMBED_CACHE     Optional source FastEmbed cache directory
#   CAPTAIN_SKIP_DOCKER_PUSH    1 to reuse an already-published image
#   CAPTAIN_VALIDATE_ONLY       1 to validate assets without network or Git writes
#   CAPTAIN_RELEASE_POLICY_TEST 1 to test version/channel policy and exit

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

release_channel_for_version() {
    local normalized="${1#v}"
    case "$normalized" in
        *-alpha|*-alpha.*) printf 'alpha\n' ;;
        *-beta|*-beta.*) printf 'beta\n' ;;
        *-rc|*-rc.*) printf 'rc\n' ;;
        *-*) printf 'edge\n' ;;
        *) printf 'latest\n' ;;
    esac
}

is_prerelease_version() {
    case "${1#v}" in
        *-*) return 0 ;;
        *) return 1 ;;
    esac
}

need_cmd() {
    command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

if is_yes "${CAPTAIN_RELEASE_POLICY_TEST:-}"; then
    [ "$(release_channel_for_version v0.1.0-alpha.1)" = "alpha" ] || exit 1
    [ "$(release_channel_for_version v0.1.0-beta.2)" = "beta" ] || exit 1
    [ "$(release_channel_for_version v0.1.0-rc.3)" = "rc" ] || exit 1
    [ "$(release_channel_for_version v0.1.0-dev.1)" = "edge" ] || exit 1
    [ "$(release_channel_for_version v0.1.0)" = "latest" ] || exit 1
    is_prerelease_version v0.1.0-alpha.1 || exit 1
    if is_prerelease_version v0.1.0; then
        exit 1
    fi
    printf 'Captain release channel policy passed.\n'
    exit 0
fi

need_cmd git
need_cmd jq
need_cmd tar
need_cmd unzip

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    else
        fail "sha256sum or shasum is required"
    fi
}

VERSION="${CAPTAIN_VERSION:-$(cat dist/releases/latest.txt 2>/dev/null || true)}"
[ -n "$VERSION" ] || fail "CAPTAIN_VERSION is empty and dist/releases/latest.txt is missing"
case "$VERSION" in
    v*) ;;
    *) fail "release version must be a Git tag beginning with v (got: $VERSION)" ;;
esac
RELEASE_CHANNEL="$(release_channel_for_version "$VERSION")"
IS_PRERELEASE=0
if is_prerelease_version "$VERSION"; then
    IS_PRERELEASE=1
fi
RELEASE_NOTES_FILE="$ROOT_DIR/docs/releases/$VERSION.md"

VERSION_DIR="$ROOT_DIR/dist/releases/$VERSION"
[ -d "$VERSION_DIR" ] || fail "release directory not found: $VERSION_DIR"

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
    expected_hash="$(cut -d ' ' -f 1 < "$checksum")"
    actual_hash="$(sha256_file "$archive")"
    [ "$actual_hash" = "$expected_hash" ] || fail "checksum mismatch: $archive"
    jq -e \
        --arg version "$VERSION" \
        --arg platform "$platform" \
        --arg archive "$(basename "$archive")" \
        --arg sha256 "$expected_hash" \
        '.version == $version and .platform == $platform and .archive == $archive and .sha256 == $sha256' \
        "$manifest" >/dev/null || fail "platform manifest mismatch: $manifest"
    case "$platform" in
        *-pc-windows-msvc)
            unzip -Z1 "$archive" | grep -Fx 'captain.exe' >/dev/null || fail "Windows bundle has no captain.exe"
            unzip -Z1 "$archive" | grep -Fx 'VERSION' >/dev/null || fail "Windows bundle has no VERSION"
            archive_version="$(unzip -p "$archive" VERSION)"
            binary_magic="$(unzip -p "$archive" captain.exe | dd bs=2 count=1 2>/dev/null || true)"
            [ "$binary_magic" = "MZ" ] || fail "Windows bundle does not contain a PE executable"
            ;;
        *)
            archive_root="captain-$platform"
            tar -tzf "$archive" | grep -Fx "$archive_root/captain" >/dev/null || fail "Unix bundle has no captain binary: $platform"
            tar -tzf "$archive" | grep -Fx "$archive_root/VERSION" >/dev/null || fail "Unix bundle has no VERSION: $platform"
            archive_version="$(tar -xOzf "$archive" "$archive_root/VERSION")"
            ;;
    esac
    [ "$archive_version" = "$VERSION" ] || fail "embedded bundle version mismatch: $platform"
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
    '.version == $version and
     ([.artifacts[].platform] | sort) == [
       "aarch64-apple-darwin",
       "aarch64-unknown-linux-gnu",
       "x86_64-apple-darwin",
       "x86_64-pc-windows-msvc",
       "x86_64-unknown-linux-gnu"
     ]' \
    "$aggregate" >/dev/null || fail "aggregate manifest does not describe five unique platforms"
assets+=("$aggregate")

for platform in "${platforms[@]}"; do
    manifest="$VERSION_DIR/manifest-$platform.json"
    expected_hash="$(jq -r '.sha256' "$manifest")"
    aggregate_hash="$(jq -r --arg platform "$platform" '.artifacts[] | select(.platform == $platform) | .sha256' "$aggregate")"
    [ "$aggregate_hash" = "$expected_hash" ] || fail "aggregate checksum mismatch for $platform"
done

printf '  Captain local release assets\n'
printf '  Version: %s\n' "$VERSION"
printf '  Channel: %s (%s)\n' "$RELEASE_CHANNEL" "$([ "$IS_PRERELEASE" = "1" ] && printf prerelease || printf stable)"
printf '  Assets:  %s validated\n' "${#assets[@]}"

if is_yes "${CAPTAIN_VALIDATE_ONLY:-}"; then
    printf '\nCaptain %s assets are ready for publication.\n' "$VERSION"
    exit 0
fi

need_cmd gh
need_cmd docker

REPO="${CAPTAIN_REPO:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}"
OWNER="${REPO%%/*}"
OWNER_LOWER="$(printf '%s' "$OWNER" | tr '[:upper:]' '[:lower:]')"
IMAGE="${CAPTAIN_IMAGE:-ghcr.io/$OWNER_LOWER/captain-agent-os}"
BRANCH="$(git branch --show-current)"
[ -n "$BRANCH" ] || fail "detached HEAD is not a valid release source"

if [ -n "$(git status --porcelain)" ]; then
    fail "worktree must be clean before local publication"
fi

printf '\n== deterministic embedding cache\n'
"$ROOT_DIR/scripts/prepare-docker-embedding-cache.sh"

printf '  Captain local publisher\n'
printf '  Version: %s\n' "$VERSION"
printf '  Repo:    %s\n' "$REPO"
printf '  Image:   %s\n' "$IMAGE"
printf '  Channel: %s\n' "$RELEASE_CHANNEL"
printf '  Assets:  %s\n' "${#assets[@]}"

gh auth status >/dev/null
docker info >/dev/null

printf '\n== Git branch\n'
git push origin "$BRANCH"

if ! is_yes "${CAPTAIN_SKIP_DOCKER_PUSH:-}"; then
    printf '\n== GHCR login\n'
    gh auth token | docker login ghcr.io -u "$OWNER" --password-stdin

    printf '\n== multi-architecture image build and push\n'
    docker buildx inspect --bootstrap >/dev/null
    docker buildx build \
        --file Dockerfile.release \
        --platform linux/amd64,linux/arm64 \
        --build-arg "CAPTAIN_RELEASE_VERSION=$VERSION" \
        --label "org.opencontainers.image.source=https://github.com/$REPO" \
        --label "org.opencontainers.image.revision=$(git rev-parse HEAD)" \
        --label "org.opencontainers.image.version=$VERSION" \
        --tag "$IMAGE:$VERSION" \
        --tag "$IMAGE:$RELEASE_CHANNEL" \
        --push \
        .
fi

printf '\n== Git tag\n'
head_commit="$(git rev-parse HEAD)"
if git rev-parse -q --verify "refs/tags/$VERSION" >/dev/null; then
    tag_commit="$(git rev-list -n 1 "$VERSION")"
    [ "$tag_commit" = "$head_commit" ] || fail "existing tag $VERSION does not point to HEAD"
else
    git tag -a "$VERSION" -m "Captain $VERSION"
fi
git push origin "refs/tags/$VERSION"

printf '\n== GitHub Release\n'
if gh release view "$VERSION" --repo "$REPO" >/dev/null 2>&1; then
    gh release upload "$VERSION" "${assets[@]}" --clobber --repo "$REPO"
    edit_args=(--repo "$REPO" --title "Captain ${VERSION#v}")
    if [ -f "$RELEASE_NOTES_FILE" ]; then
        edit_args+=(--notes-file "$RELEASE_NOTES_FILE")
    fi
    if [ "$IS_PRERELEASE" = "1" ]; then
        edit_args+=(--prerelease)
    else
        edit_args+=(--prerelease=false --latest)
    fi
    gh release edit "$VERSION" "${edit_args[@]}"
else
    create_args=(--repo "$REPO" --verify-tag --title "Captain ${VERSION#v}")
    if [ -f "$RELEASE_NOTES_FILE" ]; then
        create_args+=(--notes-file "$RELEASE_NOTES_FILE")
    else
        create_args+=(--generate-notes)
    fi
    if [ "$IS_PRERELEASE" = "1" ]; then
        create_args+=(--prerelease)
    else
        create_args+=(--latest)
    fi
    gh release create "$VERSION" "${assets[@]}" "${create_args[@]}"
fi

printf '\n== remote verification\n'
release_json="$(gh release view "$VERSION" --repo "$REPO" --json assets,isDraft,isPrerelease,tagName)"
release_asset_count="$(jq -r '.assets | length' <<<"$release_json")"
[ "$release_asset_count" -eq "${#assets[@]}" ] || fail "GitHub Release has $release_asset_count assets; expected ${#assets[@]}"
jq -e --arg tag "$VERSION" --argjson prerelease "$IS_PRERELEASE" '
    .tagName == $tag
    and .isDraft == false
    and .isPrerelease == ($prerelease == 1)
' <<<"$release_json" >/dev/null || fail "GitHub Release state does not match the local release policy"
for image_ref in "$IMAGE:$VERSION" "$IMAGE:$RELEASE_CHANNEL"; do
    docker buildx imagetools inspect "$image_ref" --raw | jq -e '
        [.manifests[]?.platform | select(.os == "linux") | .architecture] as $architectures
        | ($architectures | index("amd64")) != null
          and ($architectures | index("arm64")) != null
    ' >/dev/null || fail "remote image is missing linux/amd64 or linux/arm64: $image_ref"
done

printf '\nCaptain %s published locally: %s assets and GHCR multi-arch image (%s channel).\n' \
    "$VERSION" "$release_asset_count" "$RELEASE_CHANNEL"
