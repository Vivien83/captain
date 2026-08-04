#!/usr/bin/env bash
set -euo pipefail

SCRIPT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
ROOT_DIR="${1:-$SCRIPT_ROOT}"
[ "$#" -le 1 ] || {
  printf 'Usage: %s [source-root]\n' "$0" >&2
  exit 2
}
ROOT_DIR="$(cd "$ROOT_DIR" && pwd -P)"
cd "$ROOT_DIR"

CONTROLLED_SINKS=(
  crates/captain-runtime/src/goal_loop.rs
  crates/captain-runtime/src/skill_execute.rs
  crates/captain-runtime/src/tools/shell_ops.rs
  crates/captain-runtime/src/tools/code_execution.rs
  crates/captain-runtime/src/tools/skill_check.rs
  crates/captain-runtime/src/tools/package_ops.rs
  crates/captain-runtime/src/process_manager.rs
  crates/captain-runtime/src/tools/process_ops.rs
  crates/captain-runtime/src/host_functions.rs
  crates/captain-kernel/src/workflow.rs
  crates/captain-api/src/hand_install_routes.rs
)

fail=0

find_unreviewed_raw_shell_constructors() {
  local scan_root="$1"
  local file line previous_line

  while IFS=: read -r file line _; do
    [[ -n "$file" ]] || continue
    [[ "$file" == */guarded_exec.rs ]] && continue
    previous_line="$(sed -n "$((line - 1))p" "$file")"
    if [[ "$previous_line" != *"guarded-exec-audit: fixed-command"* ]]; then
      printf '%s:%s\n' "$file" "$line"
    fi
  done < <(
    rg -n 'Command::new\("(bash|sh|cmd)"\)' "$scan_root" --glob '*.rs' || true
  )
}

for file in "${CONTROLLED_SINKS[@]}"; do
  if ! rg -q 'guarded_exec' "$file"; then
    printf 'guarded-exec audit: sink does not reference guarded_exec: %s\n' "$file" >&2
    fail=1
  fi
  if rg -n 'Command::new\(' "$file"; then
    printf 'guarded-exec audit: raw process constructor in controlled sink: %s\n' "$file" >&2
    fail=1
  fi
  if rg -n '\.(env|envs|env_clear)\(' "$file"; then
    printf 'guarded-exec audit: direct environment mutation in controlled sink: %s\n' "$file" >&2
    fail=1
  fi
done

raw_violations="$(find_unreviewed_raw_shell_constructors crates)"
if [[ -n "$raw_violations" ]]; then
  while IFS= read -r violation; do
    printf 'guarded-exec audit: raw shell constructor needs the shared boundary: %s\n' \
      "$violation" >&2
  done <<<"$raw_violations"
  fail=1
fi

self_test_dir="$(mktemp -d)"
trap 'rm -rf "$self_test_dir"' EXIT
printf '%s\n' 'let _ = Command::new("bash");' >"$self_test_dir/unreviewed.rs"
if [[ -z "$(find_unreviewed_raw_shell_constructors "$self_test_dir")" ]]; then
  printf 'guarded-exec audit: scanner self-test accepted an unreviewed constructor\n' >&2
  fail=1
fi

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

printf 'guarded-exec audit passed: %s controlled sinks, no unreviewed raw shell constructor.\n' \
  "${#CONTROLLED_SINKS[@]}"
