#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT_DIR"

tree=$(cargo tree -p captain-node --edges normal --format '{p}')
for forbidden in \
  captain-api \
  captain-kernel \
  captain-runtime \
  captain-memory \
  captain-skills \
  captain-channels \
  captain-hands \
  captain-extensions \
  captain-graph \
  captain-console \
  captain-desktop \
  wasmtime \
  ort \
  pyo3
do
  if printf '%s\n' "$tree" | grep -Eq "^${forbidden} v"; then
    printf 'forbidden normal dependency in captain-node: %s\n' "$forbidden" >&2
    exit 1
  fi
done

printf 'captain-node lightweight dependency audit passed\n'

# The Console consumes the Client-only composition. Keep this exact feature
# boundary compilable so an accidental runtime-only import fails the gate.
cargo check -p captain-node --no-default-features --lib
printf 'captain-node client-only feature audit passed\n'

console_tree=$(cargo tree -p captain-console --edges normal --format '{p}')
for forbidden in \
  captain-api \
  captain-kernel \
  captain-runtime \
  captain-memory \
  captain-skills \
  captain-channels \
  captain-hands \
  captain-graph \
  captain-node-tools \
  captain-desktop \
  wasmtime \
  ort \
  pyo3
do
  if printf '%s\n' "$console_tree" | grep -Eq "^${forbidden} v"; then
    printf 'forbidden normal dependency in captain-console: %s\n' "$forbidden" >&2
    exit 1
  fi
done

printf 'captain-console lightweight dependency audit passed\n'
