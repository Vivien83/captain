# Captain Hub, Clients, and Nodes

This document defines the Alpha 14 distributed runtime contract. It is a
product and security contract; the Rust wire types live in
`captain-wire::hub_protocol`.

## Roles

- **Hub** is a complete Captain installation and the only authority for agent
  loops, providers, memory, sessions, projects, goals, channels, automation,
  audit, skills, sub-agents, and the durable work queue. A Hub can also execute
  tools locally.
- **Client** is a light Web, TUI, or Desktop interface. It reads and controls
  the Hub through the authenticated Captain API. It has no competing memory
  database or provider loop.
- **Node** is an optional local executor for files, commands, processes, and
  workspaces on macOS, Linux, or Windows. It never becomes a second authority.
- **Standalone** remains the default experience and composes a local Hub, Node,
  and Client in one installation.

There is no multi-primary database synchronization. A project has one active
logical workspace target at a time.

## Network Contract

A Node always initiates its connection to a configured Hub URL over standard
HTTPS port 443. No inbound Node port, NAT rule, UDP discovery, or local-network
broadcast is required.

Transport preference is deterministic:

1. authenticated WebSocket over TLS;
2. HTTPS streaming when WebSocket upgrade is blocked;
3. bounded HTTPS long polling when intermediaries buffer streaming responses.

The same protocol envelopes, sequence numbers, acknowledgements, and
idempotency keys are used on every transport. Switching transport cannot create
a second logical connection or duplicate a run.

The first accepted `Hello` for a logical connection is also its durable
bootstrap envelope. The Node retains that exact envelope after acknowledgement.
After an abrupt disconnect it may replay the bootstrap unchanged to reactivate
the same connection or select a fallback transport without consuming a new
Node-origin sequence. The Hub either reuses the still-pending `Welcome` or
atomically supersedes stale delivery rows and emits one fresh `Welcome`; an
ambiguous retry cannot allocate another response. A replayed bootstrap never
trusts its historical active-run list. The Node sends a current heartbeat after
`Welcome`, then follows the heartbeat interval negotiated in that message even
when the active-run set is unchanged (15 seconds with the current Hub, inside
its 60-second presence lease). Long-poll waits are bounded by that interval and
complete before the refresh is emitted; the runtime does not abandon an active
poll merely because its heartbeat deadline arrived. WebSocket and HTTPS stream
receives may be interrupted safely at the negotiated deadline. An unacknowledged
heartbeat for the same run set is reused; after acknowledgement, the next
refresh receives a new monotonic sequence. This keeps an idle Node online
without producing duplicate outbox records, and that durable observation
remains authoritative for live leases.

Nodes honor conventional `HTTPS_PROXY` and `NO_PROXY` configuration, optional
proxy authentication from Captain's secret store, and an operator-selected
enterprise CA bundle. Diagnostics must distinguish DNS, TCP, proxy, TLS,
authentication, protocol, and Hub-version failures. A network that explicitly
blocks the configured Hub hostname cannot be bypassed by Captain.

Public examples use neutral hosts such as `hub.example.com`; deployments choose
their own private hostname.

## Pairing and Device Credentials

Pairing is operator-mediated and fail-closed:

1. The Node generates a high-entropy device credential locally and sends only
   its SHA-256 digest in a pairing claim.
2. The Hub returns a short display code and an authenticated browser approval
   path. Pending claims are rate-limited, durable, and expire automatically.
3. The operator reviews device name, role, platform, version, capabilities,
   workspaces, and requested grants before approval.
4. Approval binds the digest to a new device identifier. The device credential
   itself never enters Hub storage or logs.
5. The Node exchanges its device credential for a short-lived access token.
   Access tokens are scoped to one device, expire, and are reissued after Hub
   restart. Revocation invalidates both active tokens and future exchanges.

Only an approved poll response carries a device identifier and the exact grant
approved by the operator. Pending, denied, and expired responses carry neither.
The Node validates the initial grant against both the advertised capabilities
and the original request before persisting it atomically. Every short-lived
token exchange returns the Hub's current grant again, allowing the local
runtime to refresh authority without persisting the bearer. A missing grant in
an older local state is interpreted as an empty grant and therefore grants no
execution authority until a successful refresh.

The Node persists its generated credential before submitting the claim. If a
response is lost, it may repeat the exact same claim while an operator-opened
enrollment window is active. The Hub keeps the durable request identifier,
atomically rotates its one-time display and polling challenges, invalidates the
previous challenge, and returns the replacement without creating a duplicate.
Any metadata change for the same credential fails closed. A Hub restart always
closes enrollment; the operator must explicitly reopen Add device before such a
recovery can proceed.

The Node keeps this bootstrap state in a private, versioned file replaced with
an atomic write and a synchronized directory entry. A process lock prevents two
Node runtimes from advancing the same identity concurrently. The state is bound
to a digest of the configured Hub origin, rejects symlinks and malformed or
oversized contents, and never stores the short-lived access bearer. On Unix the
state directory is `0700` and files are `0600`; Windows relies on the private
ACL inherited from the selected per-user state directory. The long-lived device
credential remains local and is zeroized from temporary buffers where the
runtime controls their lifetime.

Pairing codes and polling credentials are one-use secrets. Failed attempts are
rate-limited without letting stale authenticated approval clicks trigger a
brute-force lockout.

## Capabilities and Grants

The Node handshake advertises:

- Captain and protocol versions;
- platform and supported outbound transports;
- tool families available locally;
- logical workspace identifiers and labels;
- output streaming support;
- additive extension metadata.

Raw local paths are never advertised to the Hub. The Node stores the mapping
from a logical workspace identifier to its local root. Tool arguments crossing
the wire use workspace-relative paths, and results virtualize any local path as
`workspace://<workspace-id>/...` before transmission.

Advertisement is not authorization. The Hub owner grants an explicit subset of
workspace identifiers and tool families, with a separate mutation permission.
The Node checks the grant again and applies its local execution policy,
allowlists, sandbox, destructive-action guards, and approvals. A compromised
Hub credential cannot weaken Node policy.

Before accepting an offer, the Node also requires an exact runtime-reviewed
tool contract: tool name, family, and effect class must all agree with the
lease. Unknown tools and effect mismatches fail closed. A mutation requires
both the device mutation grant and a writable local workspace binding. The
binding stores only the canonical local root, rechecks that root before each
authorization, has no serialization implementation, and redacts the path from
debug output. The resulting rejection evidence contains a stable code and a
sanitized explanation, never the physical path.

The first local runtime surface is intentionally exact rather than inferred:
all nine canonical `file` tools and `shell_exec` are reviewed locally. Aliases,
unknown builtins, malformed required fields, and a Hub-provided effect class
that differs from the local classifier are rejected before a claim. The
runtime repeats the durable rail's one-MiB input ceiling and fully validates
every operation in a grouped file inspection before accepting it. Ordinary
turn verification may call a successful build or test a verification. The
distributed classifier instead treats build, test, health, and status commands
as local mutations because they may run project hooks, populate caches, or
contact a daemon. Only a narrow command made entirely of known observation
utilities is `read_only`; an unknown shell command is an external effect. An
unnecessary approval or non-replayable mutation is safer than replaying an
unobserved remote side effect.

Execution reuses Captain's builtin dispatcher and file sandbox. It never uses
the process-wide tool-result cache: that cache is keyed by tool and input, not
by physical workspace, so sharing it across Node workspaces could return a
read from another machine. New nested file destinations are resolved from the
nearest existing canonical ancestor; `..` traversal and an existing symlink
that escapes the workspace remain denied. The canonical root is replaced by a
virtual workspace marker before a result can leave the runtime adapter.

Host `shell_exec` is environment-scrubbed but is not an OS namespace,
container, chroot, or filesystem proof. A Node therefore applies at least the
`remote_operator` execution profile and the operator's local command allowlist,
even if the standalone policy was broader. External-effect shell commands need
the exact durable action digest. Hyper-critical commands and `paranoid` shell
reviews remain locally denied on a remote Node: Hub approval is never converted
directly into Captain's critical-shell permit. Explicit Docker/WASM isolation
remains a separate tool surface and is not silently selected or advertised by
this initial Node adapter.

## Turn Routing and Remote Dispatch

The Kernel resolves one concrete execution target before each LLM turn starts.
An explicit project binding takes precedence over a session binding. `Auto`, a
missing binding, and an explicit `Hub` binding resolve conservatively to the
Hub. A resolved Node target contains only its paired device identifier and
logical workspace identifier. The target is task-local and immutable for the
whole turn, including a detached streaming turn, so a concurrent UI change can
affect only a later turn.

Selecting a Node does not move Captain's agent loop or Hub-only tools. Only the
canonical local Node file and shell surface is dispatched through the durable
rail; memory, projects, providers, web, automation, and every other Hub tool
continue to execute on the Hub. A paired Client can see and invoke local file
or shell tools only while such an explicit Node route is active. The same
Client request remains denied on a Hub route, and administrative tools remain
denied on both routes.

Before a Node run enters SQLite, the Hub validates every structured path with
Captain's existing parser and rejects absolute, home-relative, parent,
workspace-URI, Windows-drive, and UNC paths. `apply_patch` source and move
destinations are validated independently, and shell commands that disclose a
physical path never enter the rail. Immediately before execution, the Node
reviews shell policy first so a critical-command denial remains authoritative,
then repeats the same path checks.

A remote tool call derives stable run and idempotency identifiers from the
session scope, agent, tool-call identifier, exact tool, target, and complete
input digest. The raw input is never embedded in identifiers or debug output.
Remote dispatch bypasses Captain's process-wide result cache and retry path and
has no local fallback. Progress is relayed to the owning tool call. A Node
approval is surfaced through Captain's shared approval manager but forwarded
only as a one-shot decision; Client session or persistent approval scopes never
cross the Node boundary. Cancelling or dropping the owning turn requests a
durable cancellation and removes any orphaned approval prompt.

Transport closure immediately reconciles every run owned by that connection,
even when its lease has not expired. A read-only run becomes eligible for a
later reconnect; a started local mutation or external effect becomes
`uncertain` and is never replayed automatically. SQLite remains authoritative;
process-local notifications only shorten observation latency.

## Durable Run Rail

Remote work reuses Captain Live Runs and Tool Runs. Each run has a stable run
identifier, attempt number, idempotency key, effect class, lease, and target
device/workspace.

Every direction has a monotonically increasing sequence. Acknowledgements are
cumulative. The Node persists unacknowledged terminal evidence in a local
outbox and retains it across restart. The Hub deduplicates by device,
connection, sequence, run, attempt, and idempotency key.

Run effects are classified as:

- `read_only`: safe to retry after the previous attempt is known not to be
  running;
- `local_mutation`: retry requires deterministic post-condition evidence or an
  explicit operator decision;
- `external_effect`: never replayed automatically after an ambiguous loss.

If connectivity disappears after dispatch and completion cannot be proven, the
run becomes `uncertain`. Captain reports the uncertainty and asks for an
operator decision instead of claiming failure or replaying the effect.

The Node sends heartbeat state for active leases and also refreshes presence
while idle. The Hub can cancel only an active cancellable run. Expired leases
prevent new work but do not erase an already-running process; reconciliation
decides its terminal state.

A Node may refuse an offer before any effect begins. `RunRejected` binds the
sanitized reason to the exact run and attempt, requires path virtualization,
and makes the Hub transition terminally instead of waiting for an opaque lease
timeout. When a local guard requires human approval, `RunApprovalRequired`
contains only a virtualized summary, risk level, expiry, and the non-reversible
digest of the complete action. Raw input and local paths are not duplicated in
approval storage. The Hub persists the request before exposing it and sends an
operator decision back as a sequenced `RunApprovalDecision` bound to the same
run, attempt, approval identifier, and digest. The Node remains the final
enforcement boundary and cannot send `RunAccepted` until that exact decision
is approved. A Hub restart or expired pending approval closes the work before
the effect and records an explicit timeout; it is never inferred as a started
mutation.

The local Node rail uses schema version 5 for this execution ledger. Opening a
version 1, 2, 3, or 4 rail migrates it transactionally without changing pairing,
connection, inbox, outbox, or acknowledgement cursors. The oldest unapplied
`RunOffer`, its exact tool input digest, the local run state, the approval or
rejection evidence, the sequenced Node response, and the inbox application
cursor commit together. A crash can therefore leave either the complete
decision or the original pending offer, never an applied offer without its
durable response. Approval digests are recomputed from the complete offered
tool name and JSON input before storage. An identical re-offer may renew the
lease and advance the inbound sequence, but it never emits a second decision;
a changed payload, concurrent attempt, or reused idempotency key for different
work fails closed. On every reopen the Node cross-checks decision digests,
approval evidence, terminal evidence, active attempts, idempotency bindings,
and any unacknowledged response still required in the outbox.

An operator denial or timeout closes the Hub run before effect in the same
transaction that persists and sequences `RunApprovalDecision`; it does not
wait for a later restart sweep. The Node stores that exact inbound decision and
terminal evidence before applying its inbox cursor. An approved decision emits
`RunAccepted` only when both the local approval and execution lease are still
valid. If a valid approval arrives after either local expiry, the Node remains
`not_started` and returns a correlated `RunRejected(approval_expired)`; the Hub
accepts that proof for the still-owned attempt even when the lease clock has
just elapsed. This prevents a read-only retry or mutation uncertainty from
being inferred after an action that never began.

`CancelRun` is also an ordered durable input. If no effect claim exists, the
Node commits the cancellation, a `RunCompleted(cancelled)` response, and inbox
application together. A pending approval is retained as historical evidence
but can no longer authorize work. If a terminal result already won the race,
the cancellation is recorded and acknowledged without replacing or duplicating
that result. An exact cancellation resequenced after reconnect updates only its
inbound evidence. A changed reason for the same durable cancellation fails
closed. Every `RunCompletion.result_sha256` is recomputed over the exact stored
UTF-8 result content by the wire contract; a merely well-formed but unrelated
digest is invalid.

No runner receives work merely because the Node emitted `RunAccepted`. The
exact acceptance sequence must first be acknowledged by the Hub, the lease
must still be valid, and no matching durable cancellation may be waiting in
the inbox. The Node then commits a unique execution claim and marks the effect
started before returning the input to the runner. The claim identifier is a
local authority: it is excluded from generic serialization and redacted from
debug output. Completion requires that exact claim and atomically stores the
terminal evidence plus its sequenced outbox record. An exact completion replay
does not allocate another sequence; changed content or a different claim fails
closed.

Only one physical rail may be opened for a Node state root inside a process;
components share cloned handles. Releasing the last handle makes the next open
a recovery boundary. A claimed read-only run interrupted at that boundary
returns to `accepted` with a new claim required, unless cancellation was already
durable, in which case it closes as `cancelled`. A claimed local mutation or
external effect interrupted before exact terminal evidence closes as
`uncertain` and is never reclaimed automatically. Claim history is retained,
bounded, tied to the exact run and attempt, and checked before recovery so a
missing or fabricated claim cannot be repaired into apparent authority.

On process recovery, the Node enumerates accepted, unclaimed runs directly
from this ledger; it does not depend on an in-memory queue or on the Hub
replaying an offer. Before each claim it repeats runtime review and local grant
authorization. If the exact tool, effect, workspace, or current local policy no
longer agrees, the Node commits a sanitized `RunRejected` terminal and its
outbox record atomically before any effect. A `CancelRun` already waiting in
the ordered inbox wins that race, so policy re-evaluation can never overtake an
operator cancellation.

The local worker applies every durable inbound decision before collecting a
finished task or claiming new work. A queued cancellation therefore wins over
a result that completed locally but has not yet been committed. At most four
runs execute concurrently, and deterministic ledger order chooses the next
claim after capacity becomes available. Every external effect requires an
approval even if a runtime adapter misclassifies its approval requirement. The
worker rereads the approved digest from SQLite immediately before the claim,
repeats runtime review, and passes that exact stored digest to the driver. A
changed digest, policy, workspace, tool, family, or effect closes before work
starts.

Cancellation first signals the driver through a cooperative token. A driver
has a bounded two-second grace period to stop its local process before the
worker aborts the task wrapper. A read-only claim then records `cancelled`;
local mutations and external effects record `uncertain`, because stopping the
future cannot prove that an already-claimed effect did not occur. Terminal
content is UTF-8, capped at one MiB, checked for raw Unix, macOS, Windows, and
UNC paths, and hashed over the exact retained bytes before it enters the
durable outbox.

## Versioning

The Alpha 14 protocol starts at `1.0`.

- A minor version is additive. Unknown JSON object fields are ignored.
- Peers with the same major version negotiate the lower minor version.
- A major mismatch fails before any run is accepted and produces an actionable
  upgrade message.
- Capability changes are explicit in the next handshake and never inferred
  from a previous connection.

## User Experience

The Devices surface lists paired Clients and Nodes with role, presence,
version, capabilities, grants, last-seen time, transport, and a revoke action.
Offline or incompatible devices remain visible with an actionable reason.

Execution selection offers `Auto`, `Hub`, and online capable Nodes. `Auto` is
the default and follows the project's logical workspace binding. A session or
project can pin a target; users are not prompted for every tool call.

Web, TUI, and Desktop use the same Hub sessions and history. Switching surfaces
does not fork memory or create a hidden local agent.

### Local Node CLI

The first production composition path is explicit and restartable:

```bash
captain node pair --hub https://hub.example.com --workspace /path/to/project
captain node status --json
captain node run
```

Pairing is read-only unless `--allow-mutation` is supplied, and that flag is
only a request: the effective authority is the subset approved by the Hub and
accepted by the local policy. `captain node status` reports requested and
effective authority separately. It exposes logical workspace labels and safe
rail/runtime facts, but never the configured Hub URL, physical workspace roots,
proxy credentials, tool inputs, or retained output.

The local configuration and runtime status use private, bounded, versioned
files replaced atomically. Runtime status is observational rather than an
authority: the pairing lock and durable rail remain authoritative after a
crash. A clean Ctrl+C, worker failure, status-write failure, or transport error
all converge through one shutdown path. Live task wrappers receive cooperative
cancellation and are then aborted within a bounded grace period; a claimed
mutation without exact terminal evidence remains `uncertain` on recovery.

The foreground `captain node run` command is the certified initial service
surface. A later service-manager wrapper may supervise that same command, but
must not create another identity, rail, policy, or execution dispatcher.

## Reproducible Distributed Smoke

`scripts/hub-node-distributed-smoke.sh` is the bounded Alpha 14 integration
smoke. It runs each case by its exact Rust test name, refuses a zero-test green
result, and keeps local-embedding features disabled. The smoke proves:

- production Node origins remain exact HTTPS on port 443, with no credentials
  or path accepted in the Hub origin;
- environment and authenticated explicit proxy policy fail closed and keep
  credentials outside logs and configuration projections;
- a real outbound WebSocket can upgrade against the full Hub API, while an
  unavailable preferred transport falls back through the bounded HTTP rail
  without changing the durable Hello;
- Node and lightweight Client identities pair through the public HTTP
  endpoints and receive separate short-lived, role-scoped credentials;
- two Client surfaces observe and update the same durable Hub session target;
- the full Hub service, Node rail, local policy, worker, and guarded Runtime
  execute real workspace-relative file mutation and read operations;
- a process/transport interruption after a local mutation but before terminal
  delivery reports `uncertain`, drains the exact durable outbox after
  reconnect, and never executes an idempotent replay twice;
- result and target projections never disclose the physical workspace path.

The socket integration uses an isolated loopback Hub so it is deterministic
and does not require a public deployment or weaken the production HTTPS rule.
It does not, by itself, certify a particular external VPS, DNS, reverse proxy,
corporate firewall, or certificate chain. A release deployment must run a
separate end-to-end smoke against its actual HTTPS Hub before it is described
as externally certified.

The published Alpha 14 release also passed that separate deployment smoke on
an isolated external HTTPS Hub and a distinct macOS Node. A paired lightweight
Client restored the same pinned session from two terminal processes, executed
exactly one guarded `file_write` and one `file_read` on the Node, and received
the expected terminal result. After a clean disconnect and one full lease, the
target became offline and non-selectable with an actionable reason; after
reconnect and another full lease, it remained online and selectable. Session
history contained no duplicate tool call, the local proof stayed byte-exact,
and neither API nor Hub logs contained the Node's physical workspace path. The
deployment coordinates, credentials, device identifiers, session identifiers,
and local paths are intentionally excluded from repository evidence.

## Deferred Scope

Alpha 14 does not add application connectors, a Telegram machine tunnel, or a
complete mobile application. Future connector hosts may advertise additional
capabilities through this protocol, but no local application data is implicitly
available or authorized by this release.
