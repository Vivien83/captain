#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
PROFILE=${CAPTAIN_GATE_CARGO_PROFILE:-dev}
case "$PROFILE" in
  dev)
    cargo build --quiet -p captain-node --bin captain-node
    BINARY="$ROOT_DIR/target/debug/captain-node"
    ;;
  release)
    cargo build --quiet --release -p captain-node --bin captain-node
    BINARY="$ROOT_DIR/target/release/captain-node"
    ;;
  *)
    printf 'unsupported CAPTAIN_GATE_CARGO_PROFILE: %s\n' "$PROFILE" >&2
    exit 2
    ;;
esac

WORK_DIR=$(mktemp -d "${TMPDIR:-/tmp}/captain-node-smoke.XXXXXX")
trap 'rm -rf -- "$WORK_DIR"' EXIT
HOME_DIR="$WORK_DIR/captain-home"

VERSION=$($BINARY --version)
case "$VERSION" in
  captain-node\ *) ;;
  *)
    printf 'unexpected captain-node version output\n' >&2
    exit 1
    ;;
esac

STATUS=$($BINARY --home "$HOME_DIR" status --json)
printf '%s' "$STATUS" | grep -Fq '"configured": false'
printf '%s' "$STATUS" | grep -Fq '"state": "unconfigured"'
if [[ -e "$HOME_DIR" ]]; then
  printf 'read-only status created local state\n' >&2
  exit 1
fi

if $BINARY --home "$HOME_DIR" reset >"$WORK_DIR/reset.out" 2>"$WORK_DIR/reset.err"; then
  printf 'unconfirmed reset unexpectedly succeeded\n' >&2
  exit 1
fi
grep -Fq 'explicit confirmation' "$WORK_DIR/reset.err"
if [[ -e "$HOME_DIR" ]]; then
  printf 'rejected reset created local state\n' >&2
  exit 1
fi

printf 'captain-node standalone smoke passed\n'
