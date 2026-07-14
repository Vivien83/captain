#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

files=(
  crates/captain-api/static/js/app/main.js
  crates/captain-api/static/js/app/api.js
  crates/captain-api/static/js/app/control_contract.mjs
  crates/captain-api/static/js/app/status_model.mjs
  crates/captain-api/static/js/app/components/Shell.js
  crates/captain-api/static/js/app/views/Automation.js
  crates/captain-api/static/js/app/views/Workflows.js
  crates/captain-api/static/js/app/views/Triggers.js
  crates/captain-api/static/js/app/views/Capabilities.js
  crates/captain-api/static/js/app/views/Status.js
  crates/captain-api/static/js/pages/terminal.js
)

printf '== Captain Control web audit\n'
for file in "${files[@]}"; do
  node --check "$ROOT_DIR/$file"
  printf '   ok syntax %s\n' "$file"
done

node "$ROOT_DIR/scripts/control-web-contract-test.mjs"
