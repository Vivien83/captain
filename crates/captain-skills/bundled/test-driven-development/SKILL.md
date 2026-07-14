---
name: test-driven-development
description: Use tests to define and protect behavior before or while implementing features and bug fixes.
---
# Test-Driven Development

Use this skill when changing behavior, fixing a bug, adding a feature, or touching shared code where regressions matter.

## Trigger

- The user asks for a feature, bug fix, refactor, or reliability improvement.
- A failing behavior can be reproduced.
- Existing tests cover nearby code or the project has a test framework.

## Steps

1. Locate the project test framework and the narrowest relevant test command.
2. Reproduce the current behavior with an existing test, smoke command, or minimal new test.
3. Add or update a focused failing test when the behavior contract is clear.
4. Implement the smallest change that satisfies the behavior.
5. Run the focused test, then broaden to adjacent tests when shared contracts were touched.
6. Record commands and results in the project checkpoint.

## When Not To Force TDD

- Pure copy, docs, styling, or trivial mechanical edits may need only a smoke check.
- If the framework is absent or setup is broken, document the blocker and use a safe manual verification instead.

## Pitfalls

- Do not add brittle tests that assert implementation details.
- Do not skip verification because the change looks simple.
- Do not create heavyweight test infrastructure for a narrow bug without evidence it is needed.
