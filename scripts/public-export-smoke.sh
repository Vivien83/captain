#!/usr/bin/env bash
# Rehearse and validate the reduced public source tree before committing.

set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd -P)
TMP_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/captain-public-smoke.XXXXXX")
EXPORT_DIR="$TMP_ROOT/source"

cleanup() {
  rm -rf -- "$TMP_ROOT"
}
trap cleanup EXIT

CAPTAIN_EXPORT_YES=1 \
CAPTAIN_EXPORT_ALLOW_DIRTY=1 \
CAPTAIN_INIT_GIT=0 \
  "$ROOT_DIR/scripts/prepare-github-export.sh" \
    --yes --allow-dirty --no-git "$EXPORT_DIR"

git -C "$EXPORT_DIR" init -q -b main
git -C "$EXPORT_DIR" add -A

(
  cd "$EXPORT_DIR"
  scripts/docs-global-audit.sh
  scripts/docs-release-audit.sh
)

printf 'Public export smoke passed.\n'
