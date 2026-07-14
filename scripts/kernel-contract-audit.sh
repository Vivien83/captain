#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

echo "== kernel public contract audit =="

kernel_lines=$(wc -l crates/captain-kernel/src/kernel.rs | awk '{print $1}')
echo "kernel.rs lines: ${kernel_lines}"

if ! rg -q "Root contract:" crates/captain-kernel/src/kernel.rs; then
  echo "kernel.rs is missing the documented root contract" >&2
  exit 1
fi
echo "ok root contract is documented"

if rg -n "captain_kernel::kernel::" crates/captain-api/src; then
  echo "captain-api must use stable captain_kernel re-exports, not captain_kernel::kernel::*" >&2
  exit 1
fi
echo "ok captain-api avoids internal kernel module paths"

pub_use_matches=$(rg -n "pub use .*kernel" crates/captain-kernel/src crates/captain-api/src || true)
disallowed=$(
  printf '%s\n' "${pub_use_matches}" \
    | awk '
      /^$/ { next }
      /^crates\/captain-kernel\/src\/lib.rs:[0-9]+:pub use kernel::(CaptainKernel|DeliveryTracker|default_blocked_workspace_paths|shared_memory_agent_id);$/ { next }
      /^crates\/captain-kernel\/src\/kernel.rs:[0-9]+:pub use kernel_(delivery_tracker|running_tasks|tool_filter|workspace_security)::/ { next }
      { print }
    '
)

if [[ -n "${disallowed}" ]]; then
  echo "unexpected kernel public re-export path(s):" >&2
  printf '%s\n' "${disallowed}" >&2
  exit 1
fi
echo "ok kernel public re-exports are explicit and stable"
