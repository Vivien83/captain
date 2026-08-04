#!/usr/bin/env bash
# Build a clean, audited Captain source tree for the public repository.

set -euo pipefail

SCRIPT_ROOT=$(cd "$(dirname "$0")/.." && pwd -P)
SOURCE_ROOT="$SCRIPT_ROOT"
EXPORT_YES="${CAPTAIN_EXPORT_YES:-}"
ALLOW_DIRTY="${CAPTAIN_EXPORT_ALLOW_DIRTY:-0}"
INIT_GIT="${CAPTAIN_INIT_GIT:-1}"
SKIP_AUDIT=0
DEST=""

usage() {
  cat <<'USAGE'
Usage: scripts/prepare-github-export.sh [--yes] [--allow-dirty] [--no-git]
       [--source-root PATH] [--skip-audit] [destination]

The normal path requires a clean worktree and exports committed HEAD through
git archive. --allow-dirty exists only for pre-commit audit rehearsal: it reads
tracked working-tree files and must never be used for publication.

--source-root and --skip-audit are internal composition controls for the local
PR gate. Skipping the audit also requires --yes and --no-git; the trusted caller
must run public-release-audit.sh separately against the sealed result.
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
    --source-root)
      shift
      [ "$#" -gt 0 ] || fail "--source-root requires a path"
      SOURCE_ROOT="$1"
      ;;
    --skip-audit)
      SKIP_AUDIT=1
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

SOURCE_ROOT=$(cd "$SOURCE_ROOT" && pwd -P)
[ "$(git -C "$SOURCE_ROOT" rev-parse --is-inside-work-tree 2>/dev/null || true)" = "true" ] \
  || fail "source root is not a Git checkout: $SOURCE_ROOT"
if [ "$SKIP_AUDIT" = "1" ]; then
  [ "$EXPORT_YES" = "1" ] || fail "--skip-audit requires --yes"
  [ "$INIT_GIT" = "0" ] || fail "--skip-audit requires --no-git"
fi

DEST="${DEST:-$HOME/Desktop/captain-public}"
mkdir -p "$(dirname "$DEST")"
DEST_PARENT=$(cd "$(dirname "$DEST")" && pwd -P)
DEST="$DEST_PARENT/$(basename "$DEST")"

case "$DEST" in
  "$SOURCE_ROOT"|"$SOURCE_ROOT"/*)
    fail "destination must be outside the source checkout"
    ;;
esac

if [ -e "$DEST" ] && [ -n "$(find "$DEST" -mindepth 1 -print -quit 2>/dev/null)" ]; then
  fail "destination must be absent or empty: $DEST"
fi

dirty=$(git -C "$SOURCE_ROOT" status --porcelain)
if [ -n "$dirty" ] && [ "$ALLOW_DIRTY" != "1" ]; then
  git -C "$SOURCE_ROOT" status --short >&2
  fail "source worktree must be clean; --allow-dirty is for audit rehearsal only"
fi

SOURCE_COMMIT=$(git -C "$SOURCE_ROOT" rev-parse HEAD)

printf '\n  Captain Public Source Export\n'
printf '  ============================\n'
printf '  Source:      %s\n' "$SOURCE_ROOT"
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
  git -C "$SOURCE_ROOT" ls-files --cached --others --exclude-standard -z \
    | while IFS= read -r -d '' relative; do
        if [ -e "$SOURCE_ROOT/$relative" ]; then
          printf '%s\0' "$relative"
        fi
      done \
    | tar -C "$SOURCE_ROOT" --null -T - -cf - \
    | tar -xf - -C "$DEST"
else
  git -C "$SOURCE_ROOT" archive --format=tar HEAD | tar -xf - -C "$DEST"
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
  scripts/runtime-capability-benchmark.sh
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

if [ "$SKIP_AUDIT" = "0" ]; then
  "$DEST/scripts/public-release-audit.sh" "$DEST"
else
  printf '  Audit:       deferred to trusted sealed-tree caller\n'
fi

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
