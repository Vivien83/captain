# Skill family

> **Status:** audited (D.7).
> See [README.md](README.md) for the index and drift policy.
> The exact tool list is pinned by
> `captain_runtime::captain_docs::SKILL_FAMILY_TOOLS`.

## Purpose

Skills are reusable procedural documents loaded on demand. Captain separates
three operations that must not be confused:

- discover and run an installed skill;
- manually scaffold a skill because the user explicitly requested it;
- learn a capability from real work through the durable Skill Learning V2
  lifecycle.

An observed workflow never becomes active merely because the model believes it
is useful. Skill Learning V2 records durable episodes, classifies the result as
memory, refinement, Skill, CapSpec, Automation, or none, stages an immutable
artifact, validates it, asks the operator, installs it, runs a canary, and can
roll it back. The configured active Captain model generates the draft; no
legacy proposer model or silent fallback exists.

## Installation Boundary

The active Skill surface is local: bundled, generated, workspace, or explicitly
installed from a reviewed directory. Remote marketplace search and installation
are absent from API and TUI, and `captain skill install` accepts only an
existing local directory. Retained compatibility clients return
`RemoteMarketplaceFrozen` before network or filesystem access.

`scan_prompt_content_advisory()` reports bounded phrase matches with
`advisory_heuristic` assurance. A match is not proof of malicious intent and no
match is not proof of safety. The loader conservatively refuses high-risk
matches, but the operator must still review the complete local source,
including scripts and linked files.

## Tools

### `skill_search`

Search installed and bundled skills by keyword or family before inventing a
manual workflow.

| Field | Required | Notes |
|---|---|---|
| `query` | no | Workflow keywords; may be empty when `family` is set. |
| `family` | no | Fixed Captain skill family id. |
| `max_results` | no | Default 8, maximum 30. |
| `include_context` | no | Include a short workflow excerpt. |
| `include_families` | no | Include family inventory, default true. |

Results do not expose absolute local paths. Use `skill_view` for one exact
result.

### `skill_view`

Load one installed skill and, optionally, one linked relative file.

| Field | Required | Notes |
|---|---|---|
| `name` | yes | Exact name returned by `skill_search`. |
| `include_context` | no | Default true. |
| `max_context_chars` | no | 500 to 20,000, default 8,000. |
| `file_path` | no | Relative file under the skill root. |

Traversal and symlink escapes are blocked. Validation reports missing tools,
environment requirements, runtime entries, and linked files without revealing
host paths.

### `skill_check`

Run a no-side-effect preflight before a scripted, brittle, or recently changed
skill.

| Field | Required | Notes |
|---|---|---|
| `name` | yes | Installed skill name. |
| `run_static_tests` | no | Default true. |
| `max_shell_blocks` | no | 1 to 50, default 20. |

Shell blocks are parsed with `bash -n`; they are not executed. A failed
preflight blocks execution. The parser subprocess crosses `guarded_exec`, so it
starts from a cleared environment with a bounded timeout and cannot bypass
critical-content policy.

### `skill_execute`

Execute a declared capability from an installed skill.

| Field | Required | Notes |
|---|---|---|
| `skill` | yes | Registered skill name. |
| `capability` | yes | Declared capability slug. |
| `args` | no | Structured arguments exposed as scoped environment values. |

The runtime uses the skill directory as its working directory. `guarded_exec`
reviews the exact capability source, clears the inherited environment, restores
only the minimal safe variables, then adds declared arguments and secret
injections explicitly. The opaque permit is bound to the skill source digest
and cannot be reused after the source changes.

## Manual Creation

### `scaffold_skill`

Create `skills/<name>/SKILL.md` under the current agent workspace only when
the user explicitly asks for a manual scaffold.

| Field | Required | Notes |
|---|---|---|
| `name` | yes | Lowercase kebab-case name. |
| `description` | yes | One concise sentence. |
| `capabilities` | no | Capability slugs for generated stubs. |

`scaffold_skill` is not an alternative activation path for automatically
observed work. It must never bypass SKILL2 staging, validation, operator
decision, canary, or rollback.

## Durable Workflow Learning

### `workflow_learning_list`

Return the same channel-neutral projection consumed by Telegram, TUI, Web, and
Desktop.

| Field | Required | Notes |
|---|---|---|
| `limit` | no | 1 to 50, default 50. |

The output includes an `engine` object followed by the learned items. `engine`
is the exact versioned Learning status: configured and actually bound model,
worker heartbeat and last scan/progress, durable job and notification queues,
retry/dead/uncertain counts, and recovery state. It never invents a progress
percentage. Schema v2 also includes bounded, code-only incident summaries and
marks whether a dead pre-approval model job is safe to retry. The item
projection includes generation, validation, proposal,
isolated test, installation, canary, activation, failure, and rollback state.
All output is operator-safe: secrets and local paths are redacted.

This tool is read-only. There is deliberately no
`workflow_learning_decide` agent tool. Exact decisions are bound to the
current revision and authenticated operator card:

- Telegram Rich callbacks;
- TUI Learning;
- Web Learning;
- Desktop, through the embedded Captain API;
- authenticated API calls carrying the exact operator token and decision
  version.

Operators can inspect the same engine status directly through
`GET /api/learning/status` or Telegram Rich `/learning`. Telegram
`/learnings` is intentionally different: it lists generic memory candidates
awaiting review.

An authenticated human can requeue the same replay-safe job through Control,
TUI, `POST /api/learning/workflows/{proposal_id}/retry`, or Telegram
`/learning_retry <proposal>`. This is not an agent tool and never replays an
uncertain effect or activation lifecycle job.

Supported card actions are `Activate`, `Test`, `Details`, `Edit`,
`Later`, and `Ignore` when the validated card exposes them. An old card,
stale version, corrupt staging area, or mismatched installation proof is
rejected.

## Existing-Skill Refinement

Refinement remains separate from newly learned capabilities:

| Tool | Use |
|---|---|
| `skill_refinement_propose` | Record a concrete improvement after real skill use and snapshot the current file-backed skill. |
| `skill_refinement_list` | Inspect pending and historical refinements. |
| `skill_refinement_decide` | Reject from a tool call or record an external authenticated approval; positive agent self-approval is blocked. |
| `skill_refinement_update` | Journal patch, test, version, and applied/restored state. |
| `skill_refinement_restore` | Restore the pre-change snapshot with a pre-restore backup. |

Secrets are rejected and host paths are redacted in stored operator text.

## Legacy Migration

Schema v32 retires SkillSynthesizer v3.13:

- every `skill_patterns` and `skill_proposals` row is copied exactly into
  `legacy_skill_patterns_archive` or
  `legacy_skill_proposals_archive`;
- pending proposals are retired and never converted into V2 evidence;
- source tables are protected by SQLite read-only triggers;
- the old REST endpoints return HTTP `410 Gone` with
  `/api/learning/workflows` as the replacement;
- old Telegram callbacks receive an explicit archive notice;
- old sessions can still deserialize historical `SkillProposalQueued`
  events, but no producer or keyboard remains active.

Previously installed files are not deleted. They remain ordinary installed
skills and can be refined or replaced through current controls.

## Sandbox

- Skills receive only declared secrets through `env_inject`.
- Skill checks and executions use `guarded_exec`; child processes start from a
  cleared, minimal environment before declared values are added.
- Critical shell patterns, literal credentials, unbounded background forms,
  timeouts, output bounds, and workspace are enforced before spawn.
- Structured lifecycle audit never records capability source or injected
  values.
- Relative linked files cannot escape the skill root.
- A learned draft can require only tools present in its canonical observed
  workflow; the active model cannot introduce new authority.
- Automatic capabilities remain inactive until durable validation and operator
  approval complete.
- Automation is installed disabled and becomes executable only after its exact
  canary passes.
- Filesystem promotion and rollback use durable journals and exact revision
  hashes.

## Operational Rule

Use memory for a durable fact. Use refinement for an installed skill that
needs improvement. Let Skill Learning V2 learn procedures from real episodes.
Use `scaffold_skill` only for an explicit manual request.

## Limites

- Memory-only, failed, secret-bearing, background-noise, and ordinary
  single-step episodes cannot become capabilities by repetition.
- The active model drafts only after deterministic eligibility and cannot add
  authority beyond tools in the canonical observed workflow.
- `workflow_learning_list` is read-only. An agent cannot approve its own
  proposal or manufacture operator evidence through a tool call.
- A staged or quarantined artifact is absent from active discovery and
  execution until exact validation, operator approval, install, and canary
  complete.
- `scaffold_skill` is not an automatic-learning fallback and must not wrap the
  SKILL2 lifecycle in a manually generated file.

## Exemples

Inspect current durable workflow proposals without deciding them:

```json
{"limit": 20}
```

Call this input through `workflow_learning_list`. Use the returned operator
token and decision version only on the authenticated Telegram, TUI, Web, or
Desktop card; do not pass them to another agent-facing tool.

Find an installed procedural capability before inventing a manual one:

```json
{"query": "verified release workflow", "max_results": 8}
```

Call this input through `skill_search`, then use `skill_view` with the exact
returned name when the full procedure is needed.
