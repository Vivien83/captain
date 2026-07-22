# Skill Learning V2

Status: implementation contract for goal `SKILL2`. This document describes the
target runtime behavior. A section is not a release claim until its executable
gate is green and the release notes identify the shipped version.

## Product rule

Captain learns a reusable capability from completed useful work, not from an
arbitrary repetition of tool names. Learning must remain understandable,
recoverable after an abrupt stop, and easy to accept or dismiss from every
operator surface.

The default is controlled improvement:

- observation and draft validation may be automatic;
- a generated durable capability never gains authority silently;
- one operator action is enough when all objective checks are green;
- a rejection, delay, validation failure, or delivery failure remains durable;
- the exact configured active Captain model is the only proposer; there is no
  separate proposer override or silent fallback.

## Episode boundary

A workflow episode is one completed user turn, project task, automation run, or
delegated task. It has a stable `episode_id`, `session_id`, `turn_id`, agent,
origin surface, optional project/workspace, start/end timestamps, completion
status, and an ordered dependency-aware trace of tool attempts.

Each tool attempt records only the information needed to compare procedures:

- tool-use id, normalized tool name, position and dependency ids;
- a redacted input shape and stable value classes, never raw credentials;
- success, failure, retry, duration, and an output class or verification marker;
- effect/risk classification from the runtime tool registry.

The recorder lives at the Kernel/ToolRunner boundary so CLI, TUI, Control,
Telegram, API, projects, automations, and subagents produce the same durable
shape. A web-only timeline is not sufficient authority.

An episode becomes successful only after its owning turn or task closes
successfully. Stopped, uncertain, unverified mutation, and unresolved-error
episodes remain useful diagnostics but cannot independently justify a new
capability.

## Eligibility and grouping

Automatic eligibility requires all of the following:

1. At least three successful episodes from distinct turns and at least two
   sessions, unless the user explicitly asks to make the current procedure
   reusable.
2. A meaningful multi-step procedure or one high-value integration step with
   stable typed inputs and an explicit verification result.
3. No unresolved failure, secret-bearing input, path escape, or authority drift.
4. Novelty relative to installed skills, active CapSpecs, automations, native
   tools, and pending proposals.
5. A portable procedure: transient ids, timestamps, host paths, user content,
   and credentials can be parameterized or removed.

The following never qualify by repetition alone:

- memory recall/save loops;
- repeated search, fetch, shell, or file calls without a task boundary;
- a single ordinary tool call;
- background-agent heartbeat or scheduled maintenance noise;
- retries and failed runs without a later verified successful episode;
- overlapping sub-sequences from the same execution.

Episodes are grouped using a deterministic canonical action graph: normalized
tool roles, dependency edges, input schema, effect class, and verification
shape. Semantic model judgment may refine an already eligible group, but it
does not manufacture recurrence or override a deterministic rejection.

## Classification

Every eligible group is classified before drafting:

| Result | Use when |
|---|---|
| `memory` | The reusable value is a fact, preference, decision, or lesson. |
| `refinement` | An installed skill or CapSpec already owns the procedure. |
| `skill` | Reuse depends on model judgment, guidance, or domain knowledge. |
| `capspec` | Existing typed tools form a deterministic DAG with typed inputs. |
| `automation` | The main reusable property is a schedule or external trigger. |
| `none` | The work is specific, noisy, unsafe, redundant, or low value. |

Deterministic procedures prefer Captain Forge. Generated `.captain` files use
the certified compiler, effective-authority intersection, exact-hash approval,
durable executor, recovery, and rollback. `SKILL.md` remains the prompt and
knowledge workflow format. Learning does not generate arbitrary native Rust,
shell, Python, or Node code as a trusted primitive.

## Drafting and validation

The proposer receives a redacted evidence bundle, not a raw transcript. It uses
the exact active configured model and returns a strict versioned schema.
Invalid structured output is a durable failed job with bounded retry; Captain
does not extract the first brace-delimited substring from prose and does not
silently switch providers or models.

The model cannot widen authority. Every initial `required_capabilities` entry
must be an exact tool name in the canonical observed graph, and refinement may
only preserve or reduce the parent authority. A prompt instruction is not the
security boundary; Runtime rejects the draft before staging when this
deterministic intersection fails.

Drafts are written under a staging root that the active skill registry and
CapSpec watcher do not scan. Before an operator sees an activation action,
Captain records concrete evidence:

- schema/compiler result and exact source hash;
- duplicate/refinement diff;
- secret, injection, path, permission, and effect scans;
- deterministic replay or fixture result when applicable;
- required capabilities and expected benefit;
- validation limitations and the exact reason human judgment is still needed.

`schema reviewed`, `diff reviewed`, and `tests reviewed` are generated facts,
not booleans supplied by a client to assert that invisible work happened.

## Durable state machine

The source of truth is SQLite, not an in-memory channel:

```text
observed -> eligible -> drafting -> validating -> proposed
         -> dismissed | snoozed | superseded
proposed -> approved_pending_install -> active_canary -> active
         -> rejected | install_failed | rolled_back
```

Every transition uses compare-and-set, an immutable revision hash, an actor,
timestamp, reason, and audit event. Jobs use durable leases, bounded retries,
and backoff. Notification delivery has its own outbox and idempotency key.
Restart reconciliation resumes incomplete work and never duplicates a model
call, card, file promotion, or external effect.

File promotion is recoverable across the SQLite/filesystem boundary: write and
fsync a sibling temporary file, atomically rename, fsync the directory, verify
registry activation, then commit the active state. A restart reconciler can
finish or roll back every intermediate state.

Quarantine is enforced. Staged or quarantined skills are absent from discovery,
prompt injection, tool selection, and execution. `active` means the exact
approved revision was loaded and verified; it is not a metadata label.

## Operator card

All surfaces consume one channel-neutral `ProposalCard` projection. It includes
kind, risk, purpose, trigger, evidence counts, compact steps, validation facts,
required authority, expected benefit, revision hash, state, and recommended
action.

Telegram renders native Rich Markdown and inline actions. Low-risk fully
validated drafts offer one-tap `Activate`; mutation-capable drafts recommend
`Test` first. Secondary actions are `Details`, `Edit`, `Later`, and `Ignore`.
Callbacks resolve directly in the control plane using a compact lookup token
plus the full current revision identity. They never start an LLM turn or become
a slash command. The original card is edited in place and its keyboard is
removed after a terminal decision.

TUI and Control show the same proposal, evidence, state, and decisions under the
existing Learning hub. Capabilities lists active artifacts, not pending drafts.
No new primary navigation entry is introduced.

## Migration

Schema v32 retires the legacy sliding-window detector transactionally. Existing
`skill_patterns` and `skill_proposals` rows are copied into exact read-only
archive tables, pending proposals are retired, and SQLite triggers reject every
future mutation of the source tables. No legacy row is converted into V2
evidence because it lacks immutable staging and validation proof. Existing
generated files are preserved as installed artifacts. The old REST routes
return HTTP `410 Gone` with `/api/learning/workflows` as the replacement,
and old Telegram callbacks render an archive notice without a decision.

## Certification gate

The goal cannot close without executable proof of:

- positive episodes for release work, repository checks, VPS health, sourced
  research, document processing, API integration, and scheduled reporting;
- negative episodes for memory loops, repeated shell/search calls, one-tool
  work, background-agent noise, failed work, secrets, and project-specific data;
- classification into memory, refinement, skill, CapSpec, automation, and none;
- duplicate suppression and explicit-user fast track;
- strict structured proposer output using the configured active model;
- queue saturation, provider outage, restart, and `SIGKILL` at each durable
  transition;
- concurrent approve/reject, stale callback, duplicate callback, install error,
  reload error, canary failure, and rollback;
- equivalent Telegram Rich, TUI, Control, API, and CLI state;
- SQLite integrity, audit-chain integrity, public-safe output, docs/code drift
  checks, and a clean-install smoke.

The legacy tables, a polished card, or unit tests alone do not satisfy this
gate. The process-level run must use the real daemon, Kernel, ToolRunner,
SQLite, registries, and operator routes.
