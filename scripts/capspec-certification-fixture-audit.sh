#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURE_DIR="$ROOT_DIR/tests/fixtures/capspec-certification"

for script in \
  "$ROOT_DIR/scripts/capspec-certification-fixture.mjs" \
  "$ROOT_DIR/scripts/capspec-control-certification.mjs"; do
  node --check "$script"
done

for script in \
  "$ROOT_DIR/scripts/capspec-certification-lib.sh" \
  "$ROOT_DIR/scripts/capspec-certification-lifecycle.sh" \
  "$ROOT_DIR/scripts/capspec-real-certification.sh"; do
  bash -n "$script"
done

python3 - "$ROOT_DIR/scripts/capspec-tui-certification.py" <<'PY'
import ast
import pathlib
import sys

source = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
ast.parse(source, filename=sys.argv[1])
PY

for script in \
  "$ROOT_DIR/scripts/capspec-certification-fixture.mjs" \
  "$ROOT_DIR/scripts/capspec-certification-fixture-audit.sh" \
  "$ROOT_DIR/scripts/capspec-certification-lifecycle.sh" \
  "$ROOT_DIR/scripts/capspec-real-certification.sh"; do
  if [[ ! -x "$script" ]]; then
    printf 'certification script is not executable: %s\n' "$script" >&2
    exit 1
  fi
done

count="$(find "$FIXTURE_DIR" -type f -name '*.captain' | wc -l | tr -d ' ')"
if [[ "$count" != "10" ]]; then
  printf 'expected 10 CapSpec certification fixtures, found %s\n' "$count" >&2
  exit 1
fi

if find "$FIXTURE_DIR" -type l -print -quit | grep -q .; then
  printf 'certification fixtures must not contain symlinks\n' >&2
  exit 1
fi

printf 'CapSpec certification harness audit passed: %s readable sources.\n' "$count"
