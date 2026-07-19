# Captain Forge / CapSpec

Status: implementation and process-level certification complete for the
`CAPSPEC1` goal. Compilation, versioned activation, hot reload, durable
execution, native ToolRunner dispatch, live discovery, controlled agent
authoring, and authenticated operator surfaces are implemented. Control, API,
TUI, and Telegram resolve exact-hash approvals and uncertain-node decisions
without model mediation. On 2026-07-18, the reproducible real harness passed
130 checks across 14 durable runs on source commit
`38ecebaf4e34fcf955c99ee13682b54a70e1c938`; see the
[certification evidence](evidence/CAPSPEC1_REAL_CERTIFICATION_2026-07-18.md).
This certifies the implementation promoted into the `v0.1.0-alpha.8` release
candidate. Publication provenance is recorded only after the public tag,
bundles, and multi-platform image have been verified live; the immutable
`v0.1.0-alpha.7` release does not contain Captain Forge.

## Decision

Captain Forge uses a hybrid architecture:

1. A human writes or copies one readable `*.captain` file.
2. Captain compiles it once into a typed deterministic plan.
3. The runtime executes only that plan through the existing ToolRunner.
4. Permissions, approvals, audit, checkpoints, restart recovery, and rollback
   remain kernel-owned.

The source is not reinterpreted by an LLM during every run. Natural language is
useful for authoring and descriptions, but it cannot silently invent tools,
permissions, dependencies, retries, or destructive behavior.

## Alternatives Rejected

| Option | Strength | Blocking weakness |
|---|---|---|
| Extend only `SKILL.md` | Familiar and easy to copy | Prompt instructions are not a typed durable execution contract |
| Interpret prose on every run | Almost no syntax | Model drift makes replay, audit, and crash recovery non-deterministic |
| Strict general-purpose DSL | Deterministic and expressive | Too much language surface and user learning cost |
| Python, Node, or shell as default | Arbitrary power | Weak portability and an unsafe default trust boundary |
| WASM or MCP only | Good low-level extension boundary | Poor format for routine human-authored workflows |

CapSpec therefore composes existing typed Captain tools by default. A future
WASM or MCP adapter may provide a genuinely new primitive, but it remains an
explicit sandboxed dependency referenced by the readable CapSpec.

## Source Format 1

The extension is `.captain`; the content is strict TOML so editors, humans, and
Captain all parse the same bytes. The file stem must match `name`. Capability
names use lowercase ASCII, digits and hyphens; underscores are rejected so two
different source names can never normalize to the same provider tool name.

```toml
format = 1
name = "project-summary"
description = "Read the important project manifests and return them together."
version = "1.0.0"
output = { readme = "{{steps.readme.output}}", manifest = "{{steps.manifest.output}}" }

[inputs.root]
type = "string"
description = "Project root"

[permissions]
tools = ["file_read"]
read_paths = ["{{input.root}}/**"]

[policy]
timeout_secs = 60
max_parallel = 2

[[steps]]
id = "readme"
tool = "file_read"
needs = []
with = { path = "{{input.root}}/README.md" }

[[steps]]
id = "manifest"
tool = "file_read"
needs = []
with = { path = "{{input.root}}/Cargo.toml" }
```

Omitting `needs` means "after the preceding step", which keeps the common file
short. An explicit empty list creates an independent root step. Only independent
read steps whose underlying tools are runtime-reviewed as read-only may run in
parallel.

Templates are data substitution, not code. Format 1 accepts only:

- `{{input.name}}`;
- `{{steps.step_id.output}}` and nested output fields;
- `{{run.id}}` for idempotency keys.

## Safety Contract

- A step tool must be listed in `permissions.tools`.
- File, network, SSH, shell, memory, and secret tools require their matching
  scopes.
- `web_download` requires both an allowed network host and an allowed write
  path. URL credentials and path traversal are rejected before dispatch.
- A source cannot downgrade the kernel's minimum effect classification.
- Non-read steps cannot claim unconditional safe idempotency.
- Manual-idempotency steps are never retried or replayed blindly.
- Unknown tools fail closed as external and sequential.
- Format 1 forbids nested CapSpec calls; recursion can only be added with an
  explicit cycle and budget contract.
- Files are bounded to 256 KiB, 64 inputs, 64 steps, one hour per run, and 16
  parallel nodes.

Effective authority is always the intersection of the caller agent's grants,
the CapSpec declaration, Captain policy, and any required human approval.

## Activation Contract

The versioned registry now implements and tests:

- global discovery plus registered project discovery and deterministic project
  override;
- strict regular-file loading without following source or root symlinks;
- automatic activation of a first read-only revision and of an update contained
  by authority already approved for the active revision;
- exact-hash approval or rejection for new shell, network, write, secret,
  remote, or destructive authority;
- SQLite WAL history with `synchronous=FULL`, `fullfsync`,
  `checkpoint_fullfsync`, a private database file, and an atomic revision/slot
  transition;
- retention of the last active revision across an invalid edit and restart;
- disable-with-history on deletion, exact-source reinstall, revision listing,
  and durable source rollback;
- fail-closed project shadowing while a local definition is invalid or awaiting
  approval.

An owned debounced filesystem watcher now reloads registered roots without a
daemon restart and stops cleanly when the kernel drops. Its status records
watched roots, successful and failed reloads, the last report, and the last
error. If the OS watcher cannot arm, the kernel remains usable and reloads at
turn boundaries instead of silently freezing the catalog.

The kernel now adds active definitions to the caller's normal tool catalog and
`capability_search` exposes them as `capfile_tool` candidates for the current
workspace. The reserved `cap_*` namespace dispatches only through CapSpec; it
cannot fall through to an unrelated provider. Each primitive step re-enters the
central `ToolRunner` with the same caller identity, primitive grants, hard
blocklist, workspace, sandbox, approval policy, and channel origin.

The deferred `capability_forge` tool exposes only `list`, `inspect`, `validate`,
and `propose`. Only the principal Captain agent may persist a proposal. The
source is validated before the durable write and the response distinguishes
`ready`, `human_action_required`, the selected and pending hashes, permissions,
revision history, and the exact next action. No agent-facing action can approve,
reject, roll back, or delete its own proposal.

The authenticated operator API now exposes:

- `GET /api/capabilities/native` and `GET /api/capabilities/native/{name}`;
- `POST /api/capabilities/native/validate` and `/install`;
- exact-hash `POST /api/capabilities/native/{name}/decision`;
- `POST /api/capabilities/native/{name}/rollback` and durable-history
  `DELETE /api/capabilities/native/{name}`;
- public-safe run metadata through `GET /api/capabilities/native/runs` and
  `/runs/{run_id}`;
- exact uncertain-node recovery through
  `POST /api/capabilities/native/runs/{run_id}/decision`, bound to node ID,
  attempt number, and tool-use ID.

The server fixes the audit actor to `control-web`; request bodies cannot forge
it. Project `.captain` ancestors and source roots reject symlinks before any
out-of-workspace creation. A deleted project override no longer masks an active
global capability, matching the real runtime catalog. The Control Capabilities
hub promotes `Natives` as its first tab and uses only those authenticated
routes; pending entries are ordered first and every approval sends the full
pending BLAKE3 hash. Source remains opt-in. The TUI Capabilities hub likewise
opens on `Natives`, supports effective/global/project views, keeps pending
entries first, and performs approve/reject, rollback, or confirmed disable
directly through the daemon API or the in-process kernel. It never delegates an
operator decision to the model, and its `/capabilities`, `/native`, and
`/capspec` commands open the same view. Control and TUI expose the same exact
retry, confirm-succeeded, and mark-failed decisions for uncertain nodes.

Telegram is also a native operator surface. A state scanner lists durable
pending revisions and uncertain node attempts, so unresolved cards reappear
after restart instead of depending on a transient event. Its compact callback
token is only a lookup key: the kernel requires a unique current match and then
applies the complete source hash or run/node/tool-use/attempt identity. The
callback is acknowledged at the adapter, resolved by the bridge and kernel
before any session dispatch, audited as the allowlisted Telegram user, and the
original card is edited with its keyboard removed. Duplicate, stale, or
ambiguous clicks cannot start an agent turn or overwrite a newer state.

## Durable Execution Contract

The executor is owned at kernel boot. Active CapSpec definitions are visible to
the model only through the same per-agent catalog filtering used by native
tools. A direct or composed call is denied again at dispatch if the capability
or one of its primitive tools is absent from the caller's grants or present in
the caller's hard blocklist.

For every accepted call, the executor now:

1. validates and normalizes the runtime input before assigning a run ID;
2. persists the run and every DAG node in one SQLite transaction before the
   first tool call;
3. encrypts runtime inputs, rendered step arguments, idempotency keys, outputs,
   and errors with AES-256-GCM and a private random state key;
4. records a deterministic tool-use ID and attempt counter before dispatch;
5. encrypts the caller's primitive allowlist, environment boundary, execution
   policy, and initial subagent depth with the run;
6. claims the run exclusively so two concurrent resume requests cannot
   dispatch the same persisted node twice;
7. checks rendered file, network, SSH, shell, memory, and secret scopes again
   immediately before each call;
8. executes only contiguous independent read nodes in parallel, and preserves
   dependency barriers and source order around mutations;
9. persists each confirmed output before making it available to dependent
   templates;
10. renders the final output and marks the run succeeded only after all nodes
   are durably confirmed.

Runs are pinned to the exact source hash from their first transaction. A later
hot reload therefore affects new runs only. Runtime payloads are capped at
4 MiB and public run listings expose metadata and node state, not decrypted
arguments or outputs.

At boot, a node left `running` is recovered according to proof, not optimism:

- a reviewed read or a keyed tool whose adapter explicitly proves
  idempotency becomes pending and resumable;
- a manual mutation or an unproven keyed mutation becomes `uncertain` and the
  run waits for an explicit operator decision;
- an operator may confirm the external success with its output, explicitly
  retry, or mark the node failed; one transaction compares status, attempt, and
  tool-use ID before updating the node and run, so a stale second decision
  cannot overwrite the first;
- an explicit retry grants one persisted operator retry permit without erasing
  or decrementing attempt history, including when the source policy originally
  allowed only one attempt.

Retry and confirmation also persist an operator-resume intent in that same
transaction. Dispatch claims it as `in_progress`; the kernel scans this durable
queue at boot and while running. If Captain stops after the decision commit but
before or during dispatch, boot converts the abandoned claim back to
`requested` and resumes it. Runs interrupted without this exact operator intent
are never enrolled automatically, so Stop and ordinary interrupted work do not
silently become new authorization.

Before retry or confirmation, the kernel reloads the encrypted authority
snapshot and intersects it with the caller's current mode, manifest grants,
hard blocklist, environment allowlist, execution policy, and subagent lineage.
Current policy may revoke an in-flight run but can never expand its pinned
authority. Mark-failed remains available after revocation because it performs
no primitive side effect.

The same classification happens immediately if a live execution future is
cancelled by Stop, channel closure, or task abort. Safe work becomes pending
without consuming a retry; non-replayable work becomes uncertain before the
run lease is released. CapSpec owns its durable run deadline, so the generic
short tool wall cannot erase this state transition.

Project capabilities also require the execution workspace to canonicalize to
their owning project. The executor rejects cross-project reuse even if a caller
somehow retains an old compiled object.

Unit and runtime integration coverage proves parallel reads,
dependency fences, safe retry, keyed retry proof, scope denial before
invocation, encrypted state, immediate abort and restart recovery, exclusive
run ownership, uncertain manual resolution, timeout uncertainty, source hash
pinning, project/workspace isolation, central ToolRunner re-entry, grant and
blocklist intersection, native discovery, reserved-prefix failure, exact
single-use uncertain decisions, current-authority revocation, Telegram
callback RBAC, restart-surviving prompts, decision-to-dispatch crash recovery,
ordinary-interruption exclusion, and direct callback resolution without an
agent turn. The process-level harness below independently exercises the real
daemon, persisted state, operator surfaces, and external protocol boundaries.

## Real Certification Matrix

The goal did not close on unit tests alone. Its reproducible real smoke covers:

1. text and structured-data transformation;
2. project file reads and controlled writes;
3. developer repository inspection and a real test command;
4. an allowed HTTP host and a denied host;
5. durable memory save and recall;
6. independent parallel reads and dependency-ordered steps;
7. approval continuation and denial;
8. invalid edit, valid edit, delete, reinstall, and rollback;
9. daemon kill during a run and recovery on the same home;
10. global versus project scope and copy-to-another-home portability;
11. path traversal, undeclared tool, secret leak, permission expansion, and
    malicious template attempts;
12. invocation and status from chat/TUI, Control/API, and Telegram.

Every test records the source hash, run ID, node states, tool calls, result, and
audit outcome. The 2026-07-18 run passed all 130 assertions, produced 14 durable
runs, verified both SQLite databases with `integrity_check=ok`, and validated a
36-entry audit chain. It also exercised a fresh home containing only the
principal `captain` agent, a real `SIGKILL` between dispatch and resolution,
and direct CLI, TUI, Control, API, and Telegram paths.

The checked-in certificate preserves the complete run/source identity list and
the raw evidence digest without committing transcripts or fixture credentials:
[CAPSPEC1 real certification, 2026-07-18](evidence/CAPSPEC1_REAL_CERTIFICATION_2026-07-18.md).
The harness can be rerun with `scripts/capspec-real-certification.sh`; generated
evidence stays under `target/capspec-real-certification/` and is intentionally
untracked.
