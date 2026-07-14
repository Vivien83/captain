#!/usr/bin/env bash
# Compile every independently packaged captain-graph language binding.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CAPTAIN_GRAPH_BINDINGS_TARGET_DIR:-$ROOT_DIR/target/captain-graph-bindings}"

run() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
  "$@"
}

find_supported_python() {
  local candidate
  local candidates=()

  if [[ -n "${PYO3_PYTHON:-}" ]]; then
    candidates+=("$PYO3_PYTHON")
  else
    candidates+=(python3.13 python3.12 python3.11 python3.10 python3.9 /usr/bin/python3 python3)
  fi

  for candidate in "${candidates[@]}"; do
    if ! command -v "$candidate" >/dev/null 2>&1; then
      continue
    fi
    if "$candidate" -c 'import sys; raise SystemExit(0 if (3, 9) <= sys.version_info[:2] <= (3, 13) else 1)' 2>/dev/null; then
      command -v "$candidate"
      return 0
    fi
  done

  return 1
}

cd "$ROOT_DIR"
export CARGO_TARGET_DIR="$TARGET_DIR"

PYTHON_BIN="$(find_supported_python || true)"
if [[ -z "$PYTHON_BIN" ]]; then
  printf 'No supported CPython interpreter found (expected 3.9 through 3.13).\n' >&2
  exit 1
fi

run cargo check --manifest-path crates/captain-graph/bindings/c/Cargo.toml
run cargo check --manifest-path crates/captain-graph/bindings/node/Cargo.toml
printf '+ PYO3_PYTHON=%q cargo check --manifest-path %q\n' \
  "$PYTHON_BIN" crates/captain-graph/bindings/python/Cargo.toml
PYO3_PYTHON="$PYTHON_BIN" cargo check \
  --manifest-path crates/captain-graph/bindings/python/Cargo.toml
run cargo check --manifest-path crates/captain-graph/bindings/wasm/Cargo.toml

printf 'captain-graph binding checks passed (Python: %s).\n' "$PYTHON_BIN"
