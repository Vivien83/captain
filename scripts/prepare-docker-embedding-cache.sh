#!/usr/bin/env bash
# Stage the exact FastEmbed model snapshot used by release images.
#
# The generated cache lives under ignored dist/docker/. It is a Docker build
# input only: it is never committed and never added to GitHub Release assets.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
SOURCE_CACHE="${CAPTAIN_FASTEMBED_CACHE:-${CAPTAIN_HOME:-$HOME/.captain}/.fastembed_cache}"
DESTINATION="$ROOT_DIR/dist/docker/fastembed-cache"
MODEL_CACHE_DIR="models--Qdrant--all-MiniLM-L6-v2-onnx"
REVISION="5f1b8cd78bc4fb444dd171e59b18f3a3af89a079"

files=(
    config.json
    model.onnx
    special_tokens_map.json
    tokenizer.json
    tokenizer_config.json
)
blobs=(
    56c8c186de9040d4fea8daac2ca110f9d412bf04
    bbd7b466f6d58e646fdc2bd5fd67b2f5e93c0b687011bd4548c420f7bd46f0c5
    9bbecc17cabbcbd3112c14d6982b51403b264bfa
    c17ed520ed8438736732a54957a69306b8822215
    61e23f16c75ff9995b1d2f251d720c6146d21338
)
hashes=(
    1b4d8e2a3988377ed8b519a31d8d31025a25f1c5f8606998e8014111438efcd7
    bbd7b466f6d58e646fdc2bd5fd67b2f5e93c0b687011bd4548c420f7bd46f0c5
    5d5b662e421ea9fac075174bb0688ee0d9431699900b90662acd44b2a350503a
    da0e79933b9ed51798a3ae27893d3c5fa4a201126cef75586296df9b4d2c62a0
    bd2e06a5b20fd1b13ca988bedc8763d332d242381b4fbc98f8fead4524158f79
)
sizes=(650 90387630 695 711661 1433)

fail() {
    printf '  Error: %s\n' "$*" >&2
    exit 1
}

if [ "${#files[@]}" -ne "${#blobs[@]}" ] \
    || [ "${#files[@]}" -ne "${#hashes[@]}" ] \
    || [ "${#files[@]}" -ne "${#sizes[@]}" ]; then
    fail "FastEmbed cache metadata arrays are not aligned"
fi

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    else
        fail "sha256sum or shasum is required"
    fi
}

file_size() {
    local size
    if size="$(stat -L -f '%z' "$1" 2>/dev/null)"; then
        printf '%s\n' "$size"
    else
        stat -L -c '%s' "$1"
    fi
}

source_snapshot="$SOURCE_CACHE/$MODEL_CACHE_DIR/snapshots/$REVISION"
[ -d "$source_snapshot" ] || fail \
    "pinned FastEmbed snapshot is missing from $SOURCE_CACHE; run 'captain embeddings install' while the model registry is reachable or set CAPTAIN_FASTEMBED_CACHE"

stage_parent="$ROOT_DIR/dist/docker"
mkdir -p "$stage_parent"
temporary_cache="$(mktemp -d "$stage_parent/.fastembed-cache.XXXXXX")"
trap 'rm -rf "$temporary_cache"' EXIT

staged_model="$temporary_cache/$MODEL_CACHE_DIR"
staged_snapshot="$staged_model/snapshots/$REVISION"
mkdir -p "$staged_model/blobs" "$staged_model/refs" "$staged_snapshot"
# hf-hub reads this file verbatim as a directory name; a trailing newline
# would make an otherwise valid cache miss every snapshot.
printf '%s' "$REVISION" > "$staged_model/refs/main"
[ "$(file_size "$staged_model/refs/main")" = "${#REVISION}" ] || fail \
    "FastEmbed revision reference must not contain trailing bytes"
: > "$temporary_cache/CAPTAIN-SHA256SUMS"

for index in "${!files[@]}"; do
    file="${files[$index]}"
    blob="${blobs[$index]}"
    expected_hash="${hashes[$index]}"
    expected_size="${sizes[$index]}"
    source_file="$source_snapshot/$file"

    [ -f "$source_file" ] || fail "FastEmbed cache file is missing: $source_file"
    actual_hash="$(sha256_file "$source_file")"
    [ "$actual_hash" = "$expected_hash" ] || fail "FastEmbed checksum mismatch: $file"
    actual_size="$(file_size "$source_file")"
    [ "$actual_size" = "$expected_size" ] || fail "FastEmbed size mismatch: $file"

    cp -L "$source_file" "$staged_model/blobs/$blob"
    ln -s "../../blobs/$blob" "$staged_snapshot/$file"
    printf '%s  %s\n' \
        "$expected_hash" \
        "$MODEL_CACHE_DIR/snapshots/$REVISION/$file" \
        >> "$temporary_cache/CAPTAIN-SHA256SUMS"
done

(
    cd "$temporary_cache"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum --check CAPTAIN-SHA256SUMS >/dev/null
    else
        shasum -a 256 -c CAPTAIN-SHA256SUMS >/dev/null
    fi
)

rm -rf "$DESTINATION"
mv "$temporary_cache" "$DESTINATION"
trap - EXIT

printf '  FastEmbed Docker cache staged\n'
printf '  Revision: %s\n' "$REVISION"
printf '  Files:    %s verified\n' "${#files[@]}"
printf '  Path:     %s (Git-ignored)\n' "$DESTINATION"
