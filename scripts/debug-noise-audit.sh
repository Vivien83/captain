#!/usr/bin/env bash
# Reproducible release-surface audit for accidental debug/noise leftovers.

set -u

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="${TMPDIR:-/tmp}/captain-debug-noise-audit.$$"
PASS=0
FAIL=0
WARN=0

mkdir -p "$TMP_DIR" || exit 1
trap 'rm -rf "$TMP_DIR"' EXIT

note() { printf '   %s\n' "$*"; }
pass() {
  PASS=$((PASS + 1))
  printf '   ok %s\n' "$1"
}
warn() {
  WARN=$((WARN + 1))
  printf '   warn %s\n' "$1"
}
fail() {
  FAIL=$((FAIL + 1))
  printf '   FAIL %s\n' "$1" >&2
}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    fail "missing required command: $1"
    finish
  fi
}

scan_rust() {
  local pattern="$1"
  local out="$2"
  rg -n "$pattern" "$ROOT_DIR/crates" \
    --glob '*.rs' \
    --glob '!**/tests/**' \
    --glob '!**/*_tests.rs' \
    --glob '!**/test_*.rs' \
    --glob '!crates/captain-migrate/**' \
    >"$out" || true
}

filter_known_todo_terms() {
  rg -v \
    -e '^.*/crates/captain-memory/src/todo.rs:' \
    -e '^.*/crates/captain-kernel/src/kernel_project_prompt.rs:' \
    -e '^.*/crates/captain-types/src/config/channel_frozen.rs:' \
    -e '^.*/crates/captain-runtime/src/tools/file_definitions.rs:' \
    -e '^.*/crates/captain-runtime/src/prompt_builder_tool_docs.rs:' \
    "$1" >"$2" || true
}

filter_allowed_stdout_boundaries() {
  rg -v \
    -e '^.*/crates/captain-cli/src/' \
    -e '^.*/crates/captain-extensions/src/' \
    "$1" >"$2" || true
}

line_count() {
  wc -l <"$1" | tr -d ' '
}

show_findings() {
  local file="$1"
  local limit="${2:-20}"
  sed -n "1,${limit}p" "$file"
}

finish() {
  printf '\n========================================\n'
  if [ "$FAIL" -eq 0 ]; then
    printf 'Debug-noise audit passed: %s checks' "$PASS"
    if [ "$WARN" -gt 0 ]; then
      printf ', %s warnings' "$WARN"
    fi
    printf '.\n'
    exit 0
  fi
  printf 'Debug-noise audit failed: %s failed, %s passed, %s warnings.\n' \
    "$FAIL" "$PASS" "$WARN"
  exit 1
}

cd "$ROOT_DIR" || exit 1
need_cmd rg
need_cmd wc
need_cmd sed

printf '== Debug-noise audit\n'
note "root=$ROOT_DIR"

hard_macros="$TMP_DIR/hard-macros.txt"
scan_rust '\b(dbg!|todo!|unimplemented!)' "$hard_macros"
if [ -s "$hard_macros" ]; then
  fail "debug/panic placeholder macros outside tests/frozen"
  show_findings "$hard_macros"
else
  pass "no dbg!/todo!/unimplemented! outside tests/frozen"
fi

stdout_all="$TMP_DIR/stdout-all.txt"
stdout_bad="$TMP_DIR/stdout-bad.txt"
scan_rust '^\s*(println!|eprintln!)\s*\(' "$stdout_all"
filter_allowed_stdout_boundaries "$stdout_all" "$stdout_bad"
if [ -s "$stdout_bad" ]; then
  fail "stdout/stderr macros outside CLI/operator extension boundaries"
  show_findings "$stdout_bad"
else
  pass "stdout/stderr macros are scoped to CLI/operator extension boundaries"
  note "classified stdout/stderr macro lines: $(line_count "$stdout_all")"
fi

todo_all="$TMP_DIR/todo-all.txt"
todo_bad="$TMP_DIR/todo-bad.txt"
scan_rust 'TODO|FIXME|XXX|HACK' "$todo_all"
filter_known_todo_terms "$todo_all" "$todo_bad"
if [ -s "$todo_bad" ]; then
  fail "unclassified TODO/FIXME/XXX/HACK markers outside tests/frozen"
  show_findings "$todo_bad"
else
  pass "TODO/FIXME/XXX/HACK markers are classified domain strings or docs"
  if [ -s "$todo_all" ]; then
    warn "classified TODO-like domain references: $(line_count "$todo_all")"
  fi
fi

finish
