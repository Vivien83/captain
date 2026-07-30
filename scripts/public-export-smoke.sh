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

blocked_probe="$(printf '\150\145\162\155\145\163')"
printf '%s\n' "$blocked_probe" >"$EXPORT_DIR/.public-boundary-probe"
if guard_output=$("$EXPORT_DIR/scripts/public-boundary-guard.sh" "$EXPORT_DIR" 2>&1); then
  printf 'Public export smoke failed: content guard accepted a blocked probe\n' >&2
  exit 1
fi
if printf '%s\n' "$guard_output" | grep -Fqi "$blocked_probe"; then
  printf 'Public export smoke failed: content guard disclosed a blocked probe\n' >&2
  exit 1
fi
rm -f -- "$EXPORT_DIR/.public-boundary-probe"

mkdir "$EXPORT_DIR/$blocked_probe"
if guard_output=$("$EXPORT_DIR/scripts/public-boundary-guard.sh" "$EXPORT_DIR" 2>&1); then
  printf 'Public export smoke failed: path guard accepted a blocked probe\n' >&2
  exit 1
fi
if printf '%s\n' "$guard_output" | grep -Fqi "$blocked_probe"; then
  printf 'Public export smoke failed: path guard disclosed a blocked probe\n' >&2
  exit 1
fi
rmdir "$EXPORT_DIR/$blocked_probe"
"$EXPORT_DIR/scripts/public-boundary-guard.sh" "$EXPORT_DIR" >/dev/null

(
  cd "$EXPORT_DIR"
  scripts/docs-global-audit.sh
  scripts/docs-release-audit.sh
)

printf 'Public export smoke passed.\n'
