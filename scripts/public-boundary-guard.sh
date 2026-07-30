#!/usr/bin/env bash
# Reject maintainer-only references from a public source tree.

set -euo pipefail

ROOT_DIR="${1:-$(cd "$(dirname "$0")/.." && pwd -P)}"
ROOT_DIR=$(cd "$ROOT_DIR" && pwd -P)

fail() {
  printf 'Public boundary guard failed: %s\n' "$*" >&2
  exit 1
}

for command in find grep rg; do
  command -v "$command" >/dev/null 2>&1 || fail "$command is required"
done

# Encode blocked names so the guard cannot reintroduce them into the tree it
# audits. Both the ASCII and accented spellings are matched case-insensitively.
blocked_pattern="$(printf '\150\145\162\155\145\163')|$(printf '\150\145\162\155\303\250\163')"
blocked_paths=$(find "$ROOT_DIR" -mindepth 1 -print \
  | grep -Ei "$blocked_pattern" || true)
blocked_contents=$(rg -n --hidden --ignore-case \
  --glob '!.git/**' \
  "$blocked_pattern" \
  "$ROOT_DIR" || true)

if [ -n "$blocked_paths" ] || [ -n "$blocked_contents" ]; then
  fail "public export contains a forbidden internal reference"
fi

printf 'Public boundary guard passed.\n'
