---
name: requesting-code-review
description: Run a pre-publication review gate covering diff scope, tests, security, docs, changelog, and project handoff.
---
# Requesting Code Review

Use this skill before publishing, committing, pushing, releasing, or telling the user that a non-trivial coding task is complete.

## Trigger

- The user asks if the project is ready to publish.
- Captain has modified code, docs, release scripts, runtime behavior, or project orchestration.
- A project phase reaches VERIFY or LEARN.

## Steps

1. Inspect the diff and separate intended changes from unrelated existing work.
2. Check affected flows, tests, and docs using the project knowledge graph when available.
3. Run targeted tests or checks for the changed surface.
4. Review for secrets, destructive operations, auth boundaries, path traversal, and sandbox/permission drift.
5. Confirm runtime changelog and Captain docs are updated when behavior changed.
6. Ensure the project checkpoint captures what changed, verification evidence, blockers, and next steps.
7. Report findings first if reviewing; otherwise report only what passed, what was not run, and residual risk.

## Pitfalls

- Do not claim release readiness from a green build alone.
- Do not include unrelated user edits in the change summary as if Captain made them.
- Do not skip docs/changelog when the user has asked for product-grade publication readiness.
