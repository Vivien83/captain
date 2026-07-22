#!/usr/bin/env bash
# Build and package Captain release bundles for every supported target from
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
#   CAPTAIN_DIST_DIR         — output root (default: dist/releases)
#   CARGO_TARGET_DIR         — shared Cargo output root (default: target)
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
HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
TARGET_ROOT="${CARGO_TARGET_DIR:-target}"

fail() {
    echo "  Error: $*" >&2
    exit 1
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

# Prints only the built binary path on stdout (captured by the caller);
# progress goes to stderr, alongside cargo/cross's own output.
build_target() {
    target="$1"
    echo "" >&2
    echo "════ Building $target ════" >&2
    case "$target" in
        "$HOST_TARGET")
            CAPTAIN_BUILD_VERSION="$VERSION" cargo build --release -p captain-cli
            echo "$TARGET_ROOT/release/captain"
            ;;
        *-apple-darwin)
            rustup target add "$target" >/dev/null
            CAPTAIN_BUILD_VERSION="$VERSION" cargo build --release -p captain-cli --target "$target"
            echo "$TARGET_ROOT/$target/release/captain"
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
            CAPTAIN_BUILD_VERSION="$VERSION" \
            CARGO_TARGET_DIR="$TARGET_ROOT/cross-$target" \
            CARGO_PROFILE_RELEASE_LTO=thin \
            CARGO_PROFILE_RELEASE_CODEGEN_UNITS=8 \
                cross build --release -p captain-cli --target "$target"
            echo "$TARGET_ROOT/cross-$target/$target/release/captain"
            ;;
        *-pc-windows-msvc)
            CAPTAIN_BUILD_VERSION="$VERSION" \
            CARGO_TARGET_DIR="$TARGET_ROOT/xwin-$target" \
                cargo xwin build --release --target "$target" -p captain-cli --bin captain
            echo "$TARGET_ROOT/xwin-$target/$target/release/captain.exe"
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

verify_embedded_binary_version() {
    target="$1"
    bin_path="$2"
    expected="captain ${VERSION#v}"
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
    target="$1"
    bin_path="$2"
    dist_dir="${CAPTAIN_DIST_DIR:-dist/releases}"
    version_dir="$dist_dir/$VERSION"
    stage="$version_dir/stage-windows-$target"
    archive="$version_dir/captain-$target.zip"
    case "$archive" in
        /*) archive_abs="$archive" ;;
        *) archive_abs="$ROOT_DIR/$archive" ;;
    esac

    rm -rf "$stage"
    mkdir -p "$stage" "$version_dir"
    cp "$bin_path" "$stage/captain.exe"
    printf '%s\n' "$VERSION" > "$stage/VERSION"
    [ ! -f captain.toml.example ] || cp captain.toml.example "$stage/captain.toml.example"
    cat > "$stage/README.txt" <<EOF
Captain precompiled Windows CLI bundle

Version:  $VERSION
Platform: $target

Run captain.exe from PowerShell or install it with install.ps1 from the
GitHub Release. Verify the adjacent .sha256 file before installation.
EOF

    rm -f "$archive"
    (
        cd "$stage"
        zip -q -r "$archive_abs" .
    )
    hash="$(sha256_file "$archive")"
    printf '%s  %s\n' "$hash" "$(basename "$archive")" > "$archive.sha256"
    cat > "$version_dir/manifest-$target.json" <<EOF
{
  "version": "$VERSION",
  "platform": "$target",
  "archive": "$(basename "$archive")",
  "sha256": "$hash",
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF
    cp scripts/install.ps1 "$version_dir/install.ps1"
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
        '{version: $version, generated_at: $generated_at, artifacts: (map({platform, archive, sha256}) | sort_by(.platform))}' \
        "$@" > "$version_dir/manifest.json"
}

echo "  Captain multi-target release"
echo "  Version: $VERSION"
echo "  Targets: $TARGETS"
echo "  Target root: $TARGET_ROOT"

for target in $TARGETS; do
    bin_path="$(build_target "$target" | tail -1)"
    [ -f "$bin_path" ] || fail "Build produced no binary for $target at $bin_path"
    verify_embedded_binary_version "$target" "$bin_path"
    case "$target" in
        *-pc-windows-msvc)
            package_windows_target "$target" "$bin_path"
            ;;
        *)
            CAPTAIN_SKIP_BUILD=1 \
            CAPTAIN_BIN_PATH="$bin_path" \
            CAPTAIN_DIST_PLATFORM="$target" \
            CAPTAIN_VERSION="$VERSION" \
            CAPTAIN_DIST_DIR="${CAPTAIN_DIST_DIR:-dist/releases}" \
                bash "$ROOT_DIR/scripts/package-release.sh"
            ;;
    esac
done

DIST_DIR="${CAPTAIN_DIST_DIR:-dist/releases}"
refresh_aggregate_manifest
echo ""
echo "  Release $VERSION complete:"
ls -lh "$DIST_DIR/$VERSION/" | sed 's/^/    /'
