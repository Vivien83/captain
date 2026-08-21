#!/usr/bin/env bash
# Build and package Captain Full, Console, and Node bundles for every supported target from
# a single Apple Silicon Mac — GitHub Actions minutes are metered, so the
# whole release pipeline must be runnable locally.
#
# Targets:
#   aarch64-apple-darwin      native cargo build
#   x86_64-apple-darwin       cargo build --target (Apple toolchain cross-arch)
#   x86_64-unknown-linux-gnu  cross (Docker required)
#   aarch64-unknown-linux-gnu cross (Docker required)
#   x86_64-pc-windows-msvc    cargo-xwin (Microsoft CRT/SDK fetched locally)
#
# All targets build with DEFAULT features on purpose: local-embeddings must
# stay in (ort-load-dynamic does not link ONNX at build time), otherwise the
# distributed binaries lose the local embeddings / Tool RAG fallback.
#
# Environment:
#   CAPTAIN_VERSION          — release version (default: 0.1.0-dev.<timestamp>)
#   CAPTAIN_RELEASE_TARGETS  — space-separated subset of targets to build
#   CAPTAIN_RELEASE_COMPONENTS — subset of: full console node
#   CAPTAIN_DIST_DIR         — output root (default: dist/releases)
#   CARGO_TARGET_DIR         — shared Cargo output root (default: target)
#   CAPTAIN_GOOGLE_OAUTH_CLIENT_ID — optional verified Captain Desktop client
#
# Output: dist/releases/$VERSION/captain-<target>.tar.gz (+ .sha256,
# manifests, install scripts) and dist/releases/latest.txt — the exact layout
# scripts/install.sh consumes via CAPTAIN_DIST_BASE_URL, and the archive
# names GitHub Releases will need later.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$ROOT_DIR"

VERSION="${CAPTAIN_VERSION:-0.1.0-dev.$(date +%Y%m%d%H%M%S)}"
TARGETS="${CAPTAIN_RELEASE_TARGETS:-aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-pc-windows-msvc}"
COMPONENTS="${CAPTAIN_RELEASE_COMPONENTS:-full console node}"
HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
TARGET_ROOT="${CARGO_TARGET_DIR:-target}"

fail() {
    echo "  Error: $*" >&2
    exit 1
}

release_host_checkpoint() {
    local label="$1"
    local available_kib
    local min_free_gib
    local min_free_kib
    available_kib="$(df -Pk "$ROOT_DIR" | awk 'END {print $4}')"
    min_free_gib="${CAPTAIN_RELEASE_MIN_FREE_GIB:-15}"
    min_free_kib=$((min_free_gib * 1024 * 1024))
    printf '  Host checkpoint (%s): %s GiB free; %s\n' \
        "$label" "$((available_kib / 1024 / 1024))" "$(uptime | sed 's/^[[:space:]]*//')" >&2
    [ "$available_kib" -gt "$min_free_kib" ] \
        || fail "release build requires more than ${min_free_gib} GiB free"
}

needs_docker=0
needs_xwin=0
for target in $TARGETS; do
    case "$target" in
        *-unknown-linux-gnu) needs_docker=1 ;;
        *-pc-windows-msvc) needs_xwin=1 ;;
    esac
done
if [ "$needs_docker" = "1" ]; then
    command -v cross >/dev/null 2>&1 || fail "cross is not installed (cargo install cross --locked)"
    if ! docker info >/dev/null 2>&1; then
        if [ "$(uname -s)" = "Darwin" ] && [ -d "/Applications/Docker.app" ]; then
            echo "  Starting Docker Desktop for cross builds..."
            if ! open -a "Docker Desktop" >/dev/null 2>&1; then
                open -a Docker >/dev/null 2>&1 \
                    || fail "Docker Desktop is installed but its application cannot be opened"
            fi
            for _ in $(seq 1 60); do
                docker info >/dev/null 2>&1 && break
                sleep 3
            done
        fi
        docker info >/dev/null 2>&1 || fail "Docker is required for Linux cross builds"
    fi
fi
if [ "$needs_xwin" = "1" ]; then
    if [ "$(uname -s)" = "Darwin" ]; then
        for llvm_bin in /opt/homebrew/opt/llvm/bin /usr/local/opt/llvm/bin; do
            if [ -d "$llvm_bin" ]; then
                PATH="$llvm_bin:$PATH"
                export PATH
                break
            fi
        done
    fi
    command -v cargo-xwin >/dev/null 2>&1 || fail "cargo-xwin is not installed (cargo install cargo-xwin --locked)"
    command -v llvm-ar >/dev/null 2>&1 || fail "LLVM is required by cargo-xwin (brew install llvm on macOS)"
    command -v nasm >/dev/null 2>&1 || fail "NASM is required by Windows native dependencies (brew install nasm on macOS)"
    command -v zip >/dev/null 2>&1 || fail "zip is required for the Windows bundle"
fi
command -v jq >/dev/null 2>&1 || fail "jq is required for the aggregate release manifest"

BUILD_TARGET_BIN=""
PACKAGE_NAME=""
BINARY_NAME=""
ARCHIVE_PREFIX=""
MANIFEST_NAME=""

select_component() {
    component="$1"
    case "$component" in
        full)
            PACKAGE_NAME="captain-cli"
            BINARY_NAME="captain"
            ARCHIVE_PREFIX="captain"
            MANIFEST_NAME="manifest"
            ;;
        console)
            PACKAGE_NAME="captain-console"
            BINARY_NAME="captain-console"
            ARCHIVE_PREFIX="captain-console"
            MANIFEST_NAME="manifest-console"
            ;;
        node)
            PACKAGE_NAME="captain-node"
            BINARY_NAME="captain-node"
            ARCHIVE_PREFIX="captain-node"
            MANIFEST_NAME="manifest-node"
            ;;
        *) fail "Unsupported release component: $component" ;;
    esac
}

# Stores the exact output path in BUILD_TARGET_BIN. Keep the build in the
# current shell: command substitutions can suppress `set -e` inside functions
# and must never allow a stale binary to survive a failed compiler invocation.
build_target() {
    target="$1"
    component="$2"
    select_component "$component"
    BUILD_TARGET_BIN=""
    echo "" >&2
    echo "════ Building $component / $target ════" >&2
    case "$target" in
        "$HOST_TARGET")
            BUILD_TARGET_BIN="$TARGET_ROOT/release/$BINARY_NAME"
            rm -f "$BUILD_TARGET_BIN"
            CAPTAIN_BUILD_VERSION="$VERSION" \
                cargo build --release -p "$PACKAGE_NAME" --bin "$BINARY_NAME" \
                || fail "release build failed for $target ($component)"
            ;;
        *-apple-darwin)
            rustup target add "$target" >/dev/null
            BUILD_TARGET_BIN="$TARGET_ROOT/$target/release/$BINARY_NAME"
            rm -f "$BUILD_TARGET_BIN"
            CAPTAIN_BUILD_VERSION="$VERSION" \
                cargo build --release -p "$PACKAGE_NAME" --bin "$BINARY_NAME" --target "$target" \
                || fail "release build failed for $target ($component)"
            ;;
        *-unknown-linux-gnu)
            # Thin LTO for cross builds: the workspace release profile
            # (fat LTO + codegen-units=1) OOM-kills the linker inside
            # Docker Desktop's memory-capped VM on the aarch64 target.
            #
            # Isolated CARGO_TARGET_DIR per target: cross compiles build
            # scripts for the container platform into target/release/build,
            # the same directory the native macOS cache uses — sharing it
            # cross-contaminates both caches (host build scripts linked
            # against the container's glibc, and vice versa).
            BUILD_TARGET_BIN="$TARGET_ROOT/cross-$target/$target/release/$BINARY_NAME"
            rm -f "$BUILD_TARGET_BIN"
            CAPTAIN_BUILD_VERSION="$VERSION" \
            CARGO_TARGET_DIR="$TARGET_ROOT/cross-$target" \
            CARGO_PROFILE_RELEASE_LTO=thin \
            CARGO_PROFILE_RELEASE_CODEGEN_UNITS=8 \
                cross build --release -p "$PACKAGE_NAME" --bin "$BINARY_NAME" --target "$target" \
                || fail "release build failed for $target ($component)"
            ;;
        *-pc-windows-msvc)
            BUILD_TARGET_BIN="$TARGET_ROOT/xwin-$target/$target/release/$BINARY_NAME.exe"
            rm -f "$BUILD_TARGET_BIN"
            RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-D warnings" \
            CAPTAIN_BUILD_VERSION="$VERSION" \
            CARGO_TARGET_DIR="$TARGET_ROOT/xwin-$target" \
                cargo xwin build --release --target "$target" -p "$PACKAGE_NAME" --bin "$BINARY_NAME" \
                || fail "release build failed for $target ($component)"
            ;;
        *)
            fail "Unsupported target: $target"
            ;;
    esac
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    else
        shasum -a 256 "$1" | cut -d ' ' -f 1
    fi
}

SOURCE_REPOSITORY="${CAPTAIN_PROVENANCE_REPOSITORY:-https://github.com/Vivien83/captain}"
SOURCE_REVISION="$(git rev-parse HEAD)"
SOURCE_TREE="$(git rev-parse 'HEAD^{tree}')"
CARGO_LOCK_SHA256="$(sha256_file "$ROOT_DIR/Cargo.lock")"
SOURCE_DIRTY=false
if [ -n "$(git status --porcelain)" ]; then
    SOURCE_DIRTY=true
fi

verify_embedded_binary_version() {
    target="$1"
    bin_path="$2"
    expected="$BINARY_NAME ${VERSION#v}"
    case "$bin_path" in
        /*) bin_abs="$bin_path" ;;
        *) bin_abs="$ROOT_DIR/$bin_path" ;;
    esac

    case "$target" in
        *-apple-darwin)
            actual="$("$bin_abs" --version)" \
                || fail "cannot execute macOS release binary for $target"
            ;;
        x86_64-unknown-linux-gnu)
            actual="$(docker run --rm --platform linux/amd64 \
                -v "$bin_abs:/captain:ro" \
                --entrypoint /captain \
                ghcr.io/cross-rs/x86_64-unknown-linux-gnu:0.2.5 \
                --version)" \
                || fail "cannot execute Linux release binary for $target"
            ;;
        aarch64-unknown-linux-gnu)
            actual="$(docker run --rm --platform linux/amd64 \
                -v "$bin_abs:/captain:ro" \
                --entrypoint /usr/bin/qemu-aarch64-static \
                ghcr.io/cross-rs/aarch64-unknown-linux-gnu:0.2.5 \
                -L /usr/aarch64-linux-gnu /captain --version)" \
                || fail "cannot execute Linux release binary for $target"
            ;;
        *-pc-windows-msvc)
            # The macOS local-release host cannot execute PE binaries. The
            # publisher still verifies PE magic, checksums, and manifests.
            return 0
            ;;
    esac

    [ "$actual" = "$expected" ] \
        || fail "embedded binary version mismatch for $target: expected '$expected', got '$actual'"
    echo "  Embedded version: $actual" >&2
}

package_windows_target() {
    local target="$1"
    local bin_path="$2"
    local build_started_at="$3"
    local component="$4"
    local dist_dir="${CAPTAIN_DIST_DIR:-dist/releases}"
    local version_dir="$dist_dir/$VERSION"
    local stage="$version_dir/stage-windows-$component-$target"
    local archive="$version_dir/$ARCHIVE_PREFIX-$target.zip"
    local archive_abs
    case "$archive" in
        /*) archive_abs="$archive" ;;
        *) archive_abs="$ROOT_DIR/$archive" ;;
    esac

    rm -rf "$stage"
    mkdir -p "$stage" "$version_dir"
    cp "$bin_path" "$stage/$BINARY_NAME.exe"
    printf '%s\n' "$VERSION" > "$stage/VERSION"
    if [ "$component" = "full" ] && [ -f captain.toml.example ]; then
        cp captain.toml.example "$stage/captain.toml.example"
    fi
    cat > "$stage/README.txt" <<EOF
Captain precompiled Windows $component bundle

Version:   $VERSION
Component: $component
Platform:  $target

Run $BINARY_NAME.exe from PowerShell. Install Full with install.ps1, or install
Console/Node with install-edition.ps1. Verify the adjacent .sha256 file first.
EOF

    rm -f "$archive"
    (
        cd "$stage"
        zip -q -r "$archive_abs" .
    )
    hash="$(sha256_file "$archive")"
    printf '%s  %s\n' "$hash" "$(basename "$archive")" > "$archive.sha256"
    cat > "$version_dir/$MANIFEST_NAME-$target.json" <<EOF
{
  "version": "$VERSION",
  "component": "$component",
  "platform": "$target",
  "archive": "$(basename "$archive")",
  "sha256": "$hash",
  "build_started_at": "$build_started_at",
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "source": {
    "repository": "$SOURCE_REPOSITORY",
    "revision": "$SOURCE_REVISION",
    "tree": "$SOURCE_TREE",
    "cargo_lock_sha256": "$CARGO_LOCK_SHA256",
    "dirty": $SOURCE_DIRTY
  }
}
EOF
    cp scripts/install.ps1 "$version_dir/install.ps1"
    cp scripts/install-edition.ps1 "$version_dir/install-edition.ps1"
    rm -rf "$stage"

    echo "  Bundle:   $archive"
    echo "  Checksum: $archive.sha256"
}

refresh_aggregate_manifest() {
    dist_dir="${CAPTAIN_DIST_DIR:-dist/releases}"
    version_dir="$dist_dir/$VERSION"
    set -- "$version_dir"/manifest-*.json
    [ -f "$1" ] || fail "No platform manifests found in $version_dir"
    jq -s \
        --arg version "$VERSION" \
        --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
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
        "$@" > "$version_dir/manifest.json"
}

echo "  Captain multi-target release"
echo "  Version: $VERSION"
echo "  Targets: $TARGETS"
echo "  Components: $COMPONENTS"
echo "  Target root: $TARGET_ROOT"

for target in $TARGETS; do
    for component in $COMPONENTS; do
        release_host_checkpoint "before $component / $target"
        build_started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        build_target "$target" "$component"
        bin_path="$BUILD_TARGET_BIN"
        [ -n "$bin_path" ] || fail "Build produced no output path for $component / $target"
        [ -f "$bin_path" ] || fail "Build produced no binary for $component / $target at $bin_path"
        verify_embedded_binary_version "$target" "$bin_path"
        case "$target" in
            *-pc-windows-msvc)
                package_windows_target "$target" "$bin_path" "$build_started_at" "$component"
                ;;
            *)
                CAPTAIN_SKIP_BUILD=1 \
                CAPTAIN_BIN_PATH="$bin_path" \
                CAPTAIN_BUILD_STARTED_AT="$build_started_at" \
                CAPTAIN_DIST_COMPONENT="$component" \
                CAPTAIN_DIST_PLATFORM="$target" \
                CAPTAIN_VERSION="$VERSION" \
                CAPTAIN_DIST_DIR="${CAPTAIN_DIST_DIR:-dist/releases}" \
                    bash "$ROOT_DIR/scripts/package-release.sh"
                ;;
        esac
        release_host_checkpoint "after $component / $target"
    done
done

DIST_DIR="${CAPTAIN_DIST_DIR:-dist/releases}"
refresh_aggregate_manifest
echo ""
echo "  Release $VERSION complete:"
ls -lh "$DIST_DIR/$VERSION/" | sed 's/^/    /'
