---
name: subagent-driven-development
description: Split development work into bounded subagent tasks with explicit ownership, allowed tools, and integration checks.
---
# Subagent-Driven Development

Use this skill for large, parallelizable, or high-context development tasks where Captain should act as project manager.

## Trigger

- The task can be split into independent research, implementation, verification, or review slices.
- The main session would be polluted by verbose logs or broad exploration.
- Multiple models or tool profiles could be useful while staying inside the configured provider boundary.

## Steps

1. OBSERVE dependencies and identify which tasks are blocking vs parallel.
2. Keep immediate critical-path work in the main session unless it can run independently.
3. Create subagent tasks with: objective, workspace path, files or responsibility, allowed tools, expected output, edit permission, and verification gate.
4. Use minimal default discovery tools plus the exact tools required by the assignment.
5. Require workers to request additional tools from Captain instead of expanding their own permissions.
6. Merge results into the project task graph and checkpoint; do not blindly trust worker output.
7. Stop or clean up workers when their task is complete.

## Pitfalls

- Do not spawn subagents for tiny edits or tasks that are fully blocking the next action.
- Do not give two workers overlapping write ownership unless coordination is explicit.
- Do not let a worker widen the security boundary beyond the parent agent.
- Do not lose the user-facing project narrative; summarize worker progress in the live project timeline.
