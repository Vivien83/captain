---
name: development-planning
description: Plan software development work before implementation, with project context, risks, task graph, and verification gates.
---
# Development Planning

Use this skill when a request is multi-step, touches a project, changes architecture, or needs more than a direct edit.

## Trigger

- The user asks to build, refactor, migrate, integrate, debug a non-trivial issue, or launch a project.
- The task has dependencies, unknowns, multiple files, or potential parallel work.
- Captain is in project mode or a `CAPTAIN.md` / `AGENTS.md` / `CLAUDE.md` file is present.

## Steps

1. OBSERVE: inspect the current project state, local rules, existing docs, relevant files, active tasks, goals, and checkpoints.
2. THINK: identify constraints, risks, assumptions, dependencies, and which work can run in parallel.
3. PLAN: create or update project tasks with clear ownership, allowed tools, and verification gates.
4. BUILD: implement only after the plan has enough evidence for the next safe step.
5. EXECUTE: run the planned workflow or implementation path.
6. VERIFY: run targeted checks first, then broader checks when the blast radius justifies it.
7. LEARN: checkpoint decisions, blockers, verification evidence, and any reusable workflow or durable fact.

## Output Contract

- Keep the plan short enough to execute.
- Include the next concrete action, not just strategy.
- Do not create a separate project plan file unless the user asks or the project is large enough to justify durable planning.
- When already in a Captain project, reflect the plan into project tasks/checkpoints instead of relying on chat text alone.

## Pitfalls

- Do not plan around tools that the agent is not authorized to use.
- Do not hide uncertainty; mark unknowns and resolve them with the smallest useful inspection.
- Do not over-plan one-file fixes.
