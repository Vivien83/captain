---
name: systematic-debugging
description: Debug failures methodically with reproduction, hypotheses, evidence, minimal fixes, and regression checks.
---
# Systematic Debugging

Use this skill when tests fail, runtime behavior is wrong, logs show errors, or the user reports a bug.

## Trigger

- The user mentions a bug, failure, crash, flaky behavior, broken UI, wrong output, or unexpected fallback.
- A command exits non-zero or a test fails.
- Captain observes conflicting state between expected and actual behavior.

## Steps

1. Reproduce or collect the smallest available evidence: command, logs, screenshot, trace, API response, or failing test.
2. Define the expected behavior and the actual behavior in one or two concrete sentences.
3. Form 2-4 hypotheses, ordered by likelihood and blast radius.
4. Inspect the narrowest code/data path that can prove or disprove the top hypothesis.
5. Fix the root cause, not just the symptom.
6. Add or update a regression check when feasible.
7. Verify the failing path and any adjacent path likely affected.
8. Save a project checkpoint with cause, fix, verification, and residual risk.

## Pitfalls

- Do not patch blindly before reproducing or gathering evidence.
- Do not trust stale session context over current code, logs, or test output.
- Do not claim a fix if verification was skipped; say what blocked it.
