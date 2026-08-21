#!/usr/bin/env bash
# Install a lightweight Captain Console or optional Captain Node bundle.
#
# Environment:
#   CAPTAIN_EDITION        console or node (required)
#   CAPTAIN_VERSION        exact release tag; defaults to GitHub latest
#   CAPTAIN_INSTALL_DIR    destination (default: /usr/local/bin for root,
#                          otherwise ~/.captain/bin)
#   CAPTAIN_BUNDLE_PATH    local archive instead of a network download
#   CAPTAIN_BUNDLE_SHA256  exact hash for a local archive
#   CAPTAIN_DIST_BASE_URL  controlled mirror with <version>/<archive> layout
#   CAPTAIN_GITHUB_TOKEN   optional token for a private GitHub release
#   CAPTAIN_UPDATE_PATH    0/false/no to leave shell startup files unchanged

set -euo pipefail

GITHUB_REPO="${CAPTAIN_GITHUB_REPO:-Vivien83/captain}"
GITHUB_BASE_URL="${CAPTAIN_GITHUB_BASE_URL:-https://github.com}"
DIST_BASE_URL="${CAPTAIN_DIST_BASE_URL:-}"
EDITION="${CAPTAIN_EDITION:-}"
VERSION="${CAPTAIN_VERSION:-latest}"
INSTALL_DIR="${CAPTAIN_INSTALL_DIR:-}"
INSTALL_TEMP_DIR=""

fail() {
    printf '  Error: %s\n' "$*" >&2
    exit 1
}

cleanup_temp() {
    local status=$?
    trap - EXIT
    if [ -n "${INSTALL_TEMP_DIR:-}" ]; then
        rm -rf -- "$INSTALL_TEMP_DIR" || true
        INSTALL_TEMP_DIR=""
    fi
    exit "$status"
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

detect_platform() {
    local os arch
    os=$(uname -s | tr '[:upper:]' '[:lower:]')
    arch=$(uname -m)
    case "$arch" in
        x86_64|amd64) arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *) fail "Unsupported architecture: $arch" ;;
    esac
    case "$os" in
        linux) PLATFORM="${arch}-unknown-linux-gnu" ;;
        darwin) PLATFORM="${arch}-apple-darwin" ;;
        *) fail "Unsupported operating system: $os" ;;
    esac
    OS="$os"
}

resolve_edition() {
    case "$EDITION" in
        console)
            ARCHIVE_PREFIX="captain-console"
            BINARY_NAME="captain-console"
            ;;
        node)
            ARCHIVE_PREFIX="captain-node"
            BINARY_NAME="captain-node"
            ;;
        *) fail "CAPTAIN_EDITION must be console or node" ;;
    esac
}

resolve_install_dir() {
    if [ -n "$INSTALL_DIR" ]; then
        return
    fi
    if [ "$(id -u)" -eq 0 ] && [ -d /usr/local/bin ] && [ -w /usr/local/bin ]; then
        INSTALL_DIR="/usr/local/bin"
    else
        INSTALL_DIR="$HOME/.captain/bin"
    fi
}

curl_download() {
    local url="$1" output="$2"
    if [ -n "${CAPTAIN_GITHUB_TOKEN:-}" ]; then
        curl -fL \
            -H "Authorization: Bearer $CAPTAIN_GITHUB_TOKEN" \
            -H "Accept: application/octet-stream" \
            "$url" -o "$output"
    else
        curl -fL "$url" -o "$output"
    fi
}

github_release_json() {
    local version="$1" api_url
    if [ "$version" = "latest" ]; then
        api_url="https://api.github.com/repos/$GITHUB_REPO/releases/latest"
    else
        api_url="https://api.github.com/repos/$GITHUB_REPO/releases/tags/$version"
    fi
    curl -fsSL \
        -H "Authorization: Bearer $CAPTAIN_GITHUB_TOKEN" \
        -H "Accept: application/vnd.github+json" \
        "$api_url"
}

github_asset_id() {
    local release_json="$1" asset_name="$2"
    printf '%s\n' "$release_json" | awk -v name="$asset_name" '
        /"id":/ { id=$0; gsub(/[^0-9]/, "", id); last_id=id }
        /"name":/ && index($0, "\"" name "\"") { print last_id; exit }'
}

github_download_asset() {
    local release_json="$1" asset_name="$2" output="$3" asset_id
    asset_id=$(github_asset_id "$release_json" "$asset_name")
    [ -n "$asset_id" ] || return 1
    curl -fL \
        -H "Authorization: Bearer $CAPTAIN_GITHUB_TOKEN" \
        -H "Accept: application/octet-stream" \
        "https://api.github.com/repos/$GITHUB_REPO/releases/assets/$asset_id" \
        -o "$output"
}

verify_archive() {
    local archive="$1" checksum_file="$2" expected actual listed_name
    if [ -n "${CAPTAIN_BUNDLE_SHA256:-}" ]; then
        expected="$CAPTAIN_BUNDLE_SHA256"
    else
        [ -f "$checksum_file" ] || fail "A SHA-256 sidecar is required: $checksum_file"
        expected=$(awk 'NR == 1 { print $1 }' "$checksum_file")
        listed_name=$(awk 'NR == 1 { print $2 }' "$checksum_file")
        listed_name=${listed_name#\*}
        [ "$listed_name" = "$(basename "$archive")" ] \
            || fail "Checksum sidecar names an unexpected archive: $listed_name"
    fi
    case "$expected" in
        ""|*[!0-9a-fA-F]*) fail "The expected SHA-256 is invalid" ;;
    esac
    [ "${#expected}" -eq 64 ] || fail "The expected SHA-256 is invalid"
    actual=$(sha256_file "$archive")
    [ "$(printf '%s' "$actual" | tr '[:upper:]' '[:lower:]')" = \
      "$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]')" ] \
        || fail "Bundle checksum verification failed"
}

reject_unsafe_archive_paths() {
    local archive="$1"
    if tar -tzf "$archive" | awk '
        /^\// || /(^|\/)\.\.($|\/)/ || /(^|\/)\.($|\/)/ { unsafe=1 }
        END { exit unsafe ? 0 : 1 }
    '; then
        fail "Bundle contains an unsafe archive path"
    fi
}

update_path() {
    case "${CAPTAIN_UPDATE_PATH:-1}" in
        0|false|FALSE|no|NO) return ;;
    esac
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) return ;;
    esac

    local shell_rc path_line
    case "${SHELL:-}" in
        */zsh) shell_rc="$HOME/.zshrc" ;;
        */bash)
            if [ "$(uname -s)" = "Darwin" ]; then
                shell_rc="$HOME/.bash_profile"
            else
                shell_rc="$HOME/.bashrc"
            fi
            ;;
        */fish)
            shell_rc="$HOME/.config/fish/config.fish"
            mkdir -p "$(dirname "$shell_rc")"
            ;;
        *) shell_rc="$HOME/.profile" ;;
    esac
    if [ -f "$shell_rc" ] && grep -Fq "$INSTALL_DIR" "$shell_rc" 2>/dev/null; then
        return
    fi
    case "${SHELL:-}" in
        */fish) path_line="fish_add_path \"$INSTALL_DIR\"" ;;
        *) path_line="export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
    esac
    printf '\n# Captain lightweight binaries\n%s\n' "$path_line" >>"$shell_rc" \
        || fail "Could not update PATH in $shell_rc"
    printf '  Added %s to PATH in %s\n' "$INSTALL_DIR" "$shell_rc"
}

install_binary() {
    local source="$1" destination="$INSTALL_DIR/$BINARY_NAME"
    local temporary backup="${destination}.previous" probe_output expected_output
    temporary=$(mktemp "$INSTALL_DIR/.${BINARY_NAME}.XXXXXX") \
        || fail "Could not create an atomic installation file"
    cp "$source" "$temporary" || {
        rm -f "$temporary"
        fail "Could not copy $BINARY_NAME"
    }
    chmod 755 "$temporary" || {
        rm -f "$temporary"
        fail "Could not make $BINARY_NAME executable"
    }
    if [ "$OS" = "darwin" ]; then
        command -v xattr >/dev/null 2>&1 && xattr -cr "$temporary" 2>/dev/null || true
        if ! command -v codesign >/dev/null 2>&1; then
            rm -f "$temporary"
            fail "codesign is required on macOS"
        fi
        if ! codesign --verify "$temporary" >/dev/null 2>&1; then
            if ! codesign --force --sign - "$temporary" >/dev/null 2>&1; then
                rm -f "$temporary"
                fail "Could not ad-hoc sign $BINARY_NAME"
            fi
        fi
    fi
    rm -f "$backup" || {
        rm -f "$temporary"
        fail "Could not replace the previous rollback binary"
    }
    if [ -e "$destination" ]; then
        mv "$destination" "$backup" || {
            rm -f "$temporary"
            fail "Could not preserve the previous binary"
        }
    fi
    if ! mv "$temporary" "$destination"; then
        [ ! -e "$backup" ] || mv "$backup" "$destination"
        fail "Could not install $BINARY_NAME"
    fi
    expected_output="$BINARY_NAME ${VERSION#v}"
    if ! probe_output=$("$destination" --version 2>/dev/null) \
        || [ "$probe_output" != "$expected_output" ]; then
        rm -f "$destination"
        [ ! -e "$backup" ] || mv "$backup" "$destination"
        fail "$BINARY_NAME failed its exact post-install version probe; previous binary restored"
    fi
}

main() {
    command -v curl >/dev/null 2>&1 || fail "curl is required"
    command -v tar >/dev/null 2>&1 || fail "tar is required"
    resolve_edition
    detect_platform
    resolve_install_dir

    local archive_name="$ARCHIVE_PREFIX-$PLATFORM.tar.gz"
    local archive checksum extract root binary release_json resolved_tag release_base bundle_version
    INSTALL_TEMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/captain-edition-install.XXXXXX")
    trap cleanup_temp EXIT
    archive="$INSTALL_TEMP_DIR/$archive_name"
    checksum="$archive.sha256"

    printf '\n  Captain %s installer\n' "$EDITION"
    printf '  =========================\n'
    printf '  Version:  %s\n' "$VERSION"
    printf '  Platform: %s\n\n' "$PLATFORM"

    if [ -n "${CAPTAIN_BUNDLE_PATH:-}" ]; then
        [ -f "$CAPTAIN_BUNDLE_PATH" ] || fail "CAPTAIN_BUNDLE_PATH does not exist"
        archive="$CAPTAIN_BUNDLE_PATH"
        checksum="$archive.sha256"
    elif [ -n "$DIST_BASE_URL" ]; then
        [ "$VERSION" != "latest" ] \
            || fail "CAPTAIN_VERSION is required with CAPTAIN_DIST_BASE_URL"
        curl_download "$DIST_BASE_URL/$VERSION/$archive_name" "$archive"
        curl_download "$DIST_BASE_URL/$VERSION/$archive_name.sha256" "$checksum"
    elif [ -n "${CAPTAIN_GITHUB_TOKEN:-}" ]; then
        release_json=$(github_release_json "$VERSION") \
            || fail "Could not resolve release $VERSION"
        resolved_tag=$(printf '%s\n' "$release_json" | awk -F'"' '/"tag_name":/ { print $4; exit }')
        [ -z "$resolved_tag" ] || VERSION="$resolved_tag"
        github_download_asset "$release_json" "$archive_name" "$archive" \
            || fail "Release $VERSION has no $archive_name asset"
        github_download_asset "$release_json" "$archive_name.sha256" "$checksum" \
            || fail "Release $VERSION has no checksum for $archive_name"
    else
        if [ "$VERSION" = "latest" ]; then
            release_base="$GITHUB_BASE_URL/$GITHUB_REPO/releases/latest/download"
        else
            release_base="$GITHUB_BASE_URL/$GITHUB_REPO/releases/download/$VERSION"
        fi
        curl_download "$release_base/$archive_name" "$archive"
        curl_download "$release_base/$archive_name.sha256" "$checksum"
    fi

    verify_archive "$archive" "$checksum"
    reject_unsafe_archive_paths "$archive"
    extract="$INSTALL_TEMP_DIR/extract"
    mkdir -p "$extract"
    tar -xzf "$archive" -C "$extract"
    root="$extract/$ARCHIVE_PREFIX-$PLATFORM"
    binary="$root/$BINARY_NAME"
    [ -d "$root" ] && [ ! -L "$root" ] \
        || fail "Bundle root is missing or unsafe"
    [ -f "$binary" ] && [ -x "$binary" ] && [ ! -L "$binary" ] \
        || fail "Bundle does not contain an executable $BINARY_NAME"
    [ -f "$root/VERSION" ] && [ ! -L "$root/VERSION" ] \
        || fail "Bundle VERSION marker is missing"
    bundle_version=$(tr -d '\r\n' <"$root/VERSION")
    if [ "$VERSION" = "latest" ]; then
        VERSION="$bundle_version"
    else
        [ "$bundle_version" = "$VERSION" ] \
            || fail "Bundle version does not match $VERSION"
    fi

    mkdir -p "$INSTALL_DIR"
    INSTALL_DIR=$(cd "$INSTALL_DIR" && pwd -P)
    install_binary "$binary"
    printf '%s\n' "$VERSION" >"$INSTALL_DIR/VERSION"
    chmod 644 "$INSTALL_DIR/VERSION"
    update_path

    printf '\n  Installed: %s\n' "$INSTALL_DIR/$BINARY_NAME"
    "$INSTALL_DIR/$BINARY_NAME" --version
    if [ "$EDITION" = "console" ]; then
        printf '  Next: captain-console pair --hub https://your-captain.example\n'
    else
        printf '  Next: captain-node pair --hub https://your-captain.example --workspace <path>\n'
        printf '        captain-node service install\n'
    fi
    printf '\n'
}

main "$@"
