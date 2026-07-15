#!/usr/bin/env bash
# Build a clean, audited Captain source tree for the public repository.

set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd -P)
EXPORT_YES="${CAPTAIN_EXPORT_YES:-}"
ALLOW_DIRTY="${CAPTAIN_EXPORT_ALLOW_DIRTY:-0}"
INIT_GIT="${CAPTAIN_INIT_GIT:-1}"
DEST=""

usage() {
  cat <<'USAGE'
Usage: scripts/prepare-github-export.sh [--yes] [--allow-dirty] [--no-git] [destination]

The normal path requires a clean worktree and exports committed HEAD through
git archive. --allow-dirty exists only for pre-commit audit rehearsal: it reads
tracked working-tree files and must never be used for publication.
USAGE
}

fail() {
  printf '  Error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "$1 is required"
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --yes|-y)
      EXPORT_YES=1
      ;;
    --allow-dirty)
      ALLOW_DIRTY=1
      ;;
    --no-git)
      INIT_GIT=0
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -* )
      fail "unknown option: $1"
      ;;
    *)
      [ -z "$DEST" ] || fail "only one destination is allowed"
      DEST="$1"
      ;;
  esac
  shift
done

need_cmd git
need_cmd tar

DEST="${DEST:-$HOME/Desktop/captain-public}"
mkdir -p "$(dirname "$DEST")"
DEST_PARENT=$(cd "$(dirname "$DEST")" && pwd -P)
DEST="$DEST_PARENT/$(basename "$DEST")"

case "$DEST" in
  "$ROOT_DIR"|"$ROOT_DIR"/*)
    fail "destination must be outside the source checkout"
    ;;
esac

if [ -e "$DEST" ] && [ -n "$(find "$DEST" -mindepth 1 -print -quit 2>/dev/null)" ]; then
  fail "destination must be absent or empty: $DEST"
fi

dirty=$(git -C "$ROOT_DIR" status --porcelain)
if [ -n "$dirty" ] && [ "$ALLOW_DIRTY" != "1" ]; then
  git -C "$ROOT_DIR" status --short >&2
  fail "source worktree must be clean; --allow-dirty is for audit rehearsal only"
fi

SOURCE_COMMIT=$(git -C "$ROOT_DIR" rev-parse HEAD)

printf '\n  Captain Public Source Export\n'
printf '  ============================\n'
printf '  Source:      %s\n' "$ROOT_DIR"
printf '  Commit:      %s\n' "$SOURCE_COMMIT"
printf '  Destination: %s\n' "$DEST"
if [ "$ALLOW_DIRTY" = "1" ]; then
  printf '  Mode:        tracked working tree (rehearsal only)\n'
else
  printf '  Mode:        committed HEAD via git archive\n'
fi
printf '\n'

if [ "$EXPORT_YES" != "1" ]; then
  read -r -p "  Create and audit this public source tree? [y/N] " confirm
  case "$confirm" in
    y|Y|yes|YES) ;;
    *) fail "aborted (pass --yes or set CAPTAIN_EXPORT_YES=1 to skip this prompt)" ;;
  esac
fi

mkdir -p "$DEST"
if [ "$ALLOW_DIRTY" = "1" ]; then
  git -C "$ROOT_DIR" ls-files --cached --others --exclude-standard -z \
    | while IFS= read -r -d '' relative; do
        if [ -e "$ROOT_DIR/$relative" ]; then
          printf '%s\0' "$relative"
        fi
      done \
    | tar -C "$ROOT_DIR" --null -T - -cf - \
    | tar -xf - -C "$DEST"
else
  git -C "$ROOT_DIR" archive --format=tar HEAD | tar -xf - -C "$DEST"
fi

# Defense in depth for dirty rehearsals and older Git archive implementations.
# The same policy is declared in .gitattributes for the normal clean export.
private_paths=(
  .mcp.json
  AGENTS.md
  MIGRATION.md
  start.sh
  docker-compose.personal.yml
  docker-compose.trusted.yml
  docker-compose.yolo.yml
  flake.nix
  crates/captain-migrate
  site
  deploy/captain-site.caddy
  docs/CAPTAIN_CORE_EXCELLENCE_PLAN.md
  docs/CAPTAIN_LONG_FILE_EXCEPTIONS.md
  docs/CONSCIOUSNESS-FR.md
  docs/CONSCIOUSNESS.md
  docs/PREPUBLICATION_24H_PLAN.md
  docs/autonomy-audit.md
  docs/benchmarks
  docs/deployment/launch-site.md
  docs/desktop.md
  docs/excellence-roadmap.md
  docs/installation-excellence-roadmap.md
  docs/launch-roadmap.md
  docs/mcp-a2a.md
  docs/production-checklist.md
  docs/research
  docs/SECURITY-PROFILES.md
  docs/ssh-setup.md
  scripts/build-launch-site.sh
  scripts/deploy-launch-site.sh
  scripts/hermes-vs-captain-benchmark.sh
  scripts/launch-site-audit.sh
  scripts/launch-site-browser-smoke.mjs
  skills/resawod.md
  target
  dist/releases
)
for relative in "${private_paths[@]}"; do
  rm -rf -- "$DEST/$relative"
done
find "$DEST/docs" -maxdepth 1 -type f -name 'v3*.md' -delete

"$DEST/scripts/public-release-audit.sh" "$DEST"

if [ "$INIT_GIT" = "1" ]; then
  git -C "$DEST" init -q -b main
  git -C "$DEST" add -A
fi

printf '\n  Export ready: %s\n' "$DEST"
printf '  Source commit: %s\n' "$SOURCE_COMMIT"
if [ "$ALLOW_DIRTY" = "1" ]; then
  printf '  Rehearsal only: rebuild from a clean commit before publication.\n'
elif [ "$INIT_GIT" = "1" ]; then
  printf '  Next: review the staged root tree and create its single public commit.\n'
fi
printf '\n'
