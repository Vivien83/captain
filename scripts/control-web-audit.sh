#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

printf '== Captain Control web audit\n'
while IFS= read -r file; do
  node --check "$ROOT_DIR/$file"
  printf '   ok syntax %s\n' "$file"
done < <(
  cd "$ROOT_DIR"
  find crates/captain-api/static/js -type f \( -name '*.js' -o -name '*.mjs' \) | LC_ALL=C sort
)

node -e 'JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"))' \
  "$ROOT_DIR/crates/captain-desktop/tauri.conf.json"
printf '   ok JSON crates/captain-desktop/tauri.conf.json\n'

node "$ROOT_DIR/scripts/control-web-contract-test.mjs"
node "$ROOT_DIR/scripts/control-chat-performance-test.mjs"
