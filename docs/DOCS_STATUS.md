# Captain Docs Status (DOC2)

DOC2 defines which documentation is allowed to describe the current Captain
runtime contract. It exists to keep Captain aligned with its own system prompt,
tool docs, CLI, API, and release gates.

## Current Release Candidate

`v0.1.0-alpha.11` is the current locally certified release candidate. It closes
the remaining Alpha 10 audit gaps and adds the native Gmail OAuth rail,
crash-safe Gmail automations, named IMAP/SMTP accounts, provider-confirmed
subscription reset notifications, first-use xAI access, reproducible public
discovery metadata, and an exact-SHA local pull-request gate.

The candidate host contract remains exactly 22 files: five archives, five
SHA-256 sidecars, five platform manifests, four installers, one aggregate
manifest, one deterministic in-toto/SLSA v1 provenance statement, and its
SHA-256 sidecar. Host targets and Docker architectures must be built strictly
one at a time with disk/load checkpoints.

Public source commit, annotated tag object, publication time, GitHub asset
digests, OCI index and architecture manifests are unset until the live release
is published and observed. No Alpha 10 provenance value is evidence for this
candidate. The same boundary applies to external authorization: source gates
prove OAuth/IMAP/SMTP protocol, storage, rollback, recovery and non-disclosure,
but a real account login requires an operator-owned credential and consent.

## Current Public Release

`v0.1.0-alpha.10` is the current public prerelease. It promotes the
deny-by-default API and browser perimeter, guarded host execution, append-only
audit recovery, crash-safe outbound delivery and delegation, evidence-bound
project completion, synchronized budgets, complete memory write opt-out,
exact Codex quota/reasoning controls, truthful compaction progress, and
authoritative external secret sources.

Its host release contract contains exactly 22 files: five archives, five
SHA-256 sidecars, five platform manifests, four installers, one aggregate
manifest, one deterministic in-toto/SLSA v1 provenance statement, and its
SHA-256 sidecar. The five host targets and both Docker architectures must be
built strictly one at a time with disk/load checks between them.

Its verified public surfaces are:

- release:
  <https://github.com/Vivien83/captain/releases/tag/v0.1.0-alpha.10>
- source commit: `48f898a9e4d38e8b8c7627644b66e22076a39364`
- source tree: `ba37632b5e2b6a3923a2241e1d38d7903bdb95f1`
- annotated tag object: `b58f7561d0014228cc523b1770b5c411b017ef52`
- publication time: `2026-07-30T04:00:03Z`
- immutable image:
  `ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.10`
- OCI index digest:
  `sha256:c54d1319b5173ca55540dc69e0f965a31b51cdfccb497ca77882882a16b4e477`
- `linux/amd64` manifest:
  `sha256:c641b4e43369da4f76c20c2f775bb3cea5a2567288e22cf735aa5dc6b53cc91c`
- `linux/arm64` manifest:
  `sha256:d48cf9f5a1682c539c2224e3ce1de3b25b1dbe5c3c797428f2d283d3a30db96a`
- AMD64 attestation manifest:
  `sha256:e446c5a56f2a1e34126f4e96df8694a8ab5a6370b164d7d98082ab15791612ef`
- ARM64 attestation manifest:
  `sha256:060b06cd91cbf65bedbb81439e354339895a38fd8918c6eff7629d4fc5be76bb`

The annotated tag dereferences to the source commit above. The GitHub Release
contains exactly 22 uploaded assets, and every GitHub-reported SHA-256 digest
matches the corresponding locally certified file. The immutable image and
moving `:alpha` channel resolved to the same OCI index. Anonymous HTTP checks
reached the checksum and provenance assets, and anonymous Docker pulls
executed `captain --version` successfully on ARM64 and AMD64. The GitHub
Actions API returned zero runs because the complete release was built and
published locally.

## Previous Public Release

`v0.1.0-alpha.9` is the previous public prerelease. It combines durable
Workflow Learning V2 with Captain's native release monitor. Its immutable
public surfaces are:

- release: <https://github.com/Vivien83/captain/releases/tag/v0.1.0-alpha.9>
- image: `ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.9`
- source commit: `1248c5928dd4968b6ff7c62ef79a607fb8d94348`
- annotated tag object: `da41c2ffd4ccaf5561f446d3eeb8b73d1506b501`
- OCI index digest:
  `sha256:b043ec5637551c2e238be15c32033ca693ecc2f765a470ba721a5986709fd692`
- `linux/amd64` manifest:
  `sha256:245f7d75657e35b15d085e51ba6fcf31187aaa9849eb610e11fe60184d9e12dd`
- `linux/arm64` manifest:
  `sha256:b84c03fd4ad11914f7c2e92312bf07670f933e3a74ab66089db1016f9350f79c`
- host asset contract: exactly 20 files covering five platforms, checksums,
  manifests, and four installers

The annotated tag dereferences to the source commit above. At publication time,
the immutable image and moving `:alpha` channel resolved to the same OCI index
digest. Anonymous checks downloaded `manifest.json` and `install.sh` with their
published SHA-256 digests, then executed the image successfully on
`linux/amd64` and `linux/arm64`. The GitHub Actions API returned zero runs
because the release was built and published locally.

Known `alpha.9` limitation: an explicit per-turn memory write opt-out still
allows the core agent-loop finalizer to write one local episodic interaction
fragment. Normal transcript and audit retention remain intentional.

The published `alpha.10` release closes that limitation. The shared
streaming/non-streaming finalizer checks the explicit per-turn opt-out before
embedding or storing its episodic interaction. The normal resumable transcript
and mandatory operational/audit records remain intentional.

## Alpha 11 Candidate Hardening

The post-Alpha 10 source fails closed when neither browser authentication nor a
daemon API key is configured. Credentialless development access is no longer
an implicit consequence of empty credentials: it requires the durable
`auth.allow_unauthenticated_loopback = true` opt-out and the actual client must
be loopback. Existing configurations that explicitly disabled auth migrate to
that visible compatibility flag. Setup and web credential rotation always
write `false`, while Status and Doctor distinguish `unconfigured`,
`unauthenticated_loopback`, and protected modes.

The post-Alpha 10 credential vault also replaces its historical obfuscated
local master-key file with macOS Keychain, Windows Credential Manager, or Linux
Secret Service. A generated key is never printed and must survive a verified
readback before initialization succeeds. Headless deployments use the explicit
`CAPTAIN_VAULT_KEY` override. Legacy migration writes and verifies the native
copy before deleting the old file; mismatches fail closed.

Detached delegation in the post-Alpha 10 source now has a second, durable
lineage boundary. Nested jobs persist `root_job_id`, `parent_job_id`, and
one-based `depth`; enqueue verifies the active parent and caller under an
immediate SQLite transaction. Depth is capped at 10. A separate lineage ledger
atomically reserves every job's requested tokens up to 500000 for the complete
tree and never refunds reservation on completion, retry, crash recovery, or
partial history pruning. Status events and agent job projections expose bounded
lineage metadata and remaining reservation without task or result content.

Direct-program execution permits in the post-Alpha 10 source no longer derive
authority from their human-readable review string. The authorization digest
uses the fixed `captain.exec-permit.program.v1` domain, then a big-endian `u64`
length and raw bytes for the executable, a big-endian `u64` argument count, and
one length-prefixed byte string per argument. This encoding is injective even
when a Rust string contains NUL; review and audit text remain readable and
command-free respectively.

The login limiter also fails closed under distributed capacity pressure. Its
separate IP and normalized-username maps keep at most 4096 keys each, but an
entry with an active retry delay is never evicted. A full map of active blocks
starts a logged five-second global backoff and returns the existing `429` plus
`Retry-After` path. The state is deliberately process-local and bounded; an
Internet-facing deployment must additionally rate-limit login traffic at the
reverse proxy, firewall, or edge.

The post-Alpha 10 browser surfaces also use a strict executable-content
boundary. Control no longer needs an inline import map, and Control, Terminal,
Config, and the retained Desktop wrapper load every script from an embedded
same-origin asset. Their CSP has no `unsafe-eval` and grants no inline script
authority; script attributes, plugins, base-tag mutation, and framing are
denied. Dynamic first-party layout still requires `style-src 'unsafe-inline'`.
LLM Markdown is reduced by DOMPurify to a fixed passive allowlist and safe link
protocols, while ordinary tool/session strings remain Preact text nodes.
`scripts/control-xss-smoke.mjs` exercises all three attacker-controlled
surfaces in Chromium under the production CSP.

The post-Alpha 10 execution contract now separates deployment posture from
command policy. The typed default is `personal_workstation` plus `allowlist`;
guided local setup may record an explicit trusted-workstation `full` choice.
`remote_operator` imposes allowlist semantics and `untrusted_execution` denies
agent-controlled host processes. Per-agent policies are intersected with the
daemon boundary before tool discovery and dispatch. `process_start` now
requires an exact-program guarded permit. Docker and WASM remain explicit
rails with no auto-routing or host fallback, and every status surface reports
configured versus effective mode and Docker configuration readiness.

Native Gmail accounts use a crash-safe multi-account OAuth lifecycle. Public
metadata stays in SQLite while client material, access tokens and refresh
tokens remain in the encrypted vault. Official builds may embed only Captain's
public Google Desktop client ID; development and organization builds can use a
reviewed client JSON. PKCE, random state, an exact loopback callback and
versioned secret replacement remain mandatory. Deterministic Gmail rules and
deliveries expose bounded metadata, preserve audit history and require explicit
inspection before an uncertain outcome can be retried.

The independent conversational Email channel supports named IMAP/SMTP accounts
under `channels.email.accounts`. Each active account has its own allowlist,
credential, folders, adapter and default agent; one account is the explicit
default for bare `email`. The bridge persists mailbox/folder/UIDVALIDITY/UID
acceptance before marking mail seen, so restart recovery can acknowledge a
duplicate without replaying the model. CLI, API, TUI, Desktop and Web Terminal
share the same readiness and typed form schema. Passwords never enter TOML or a
rendered preview.

xAI API keys are a native first-use path validated without a billed completion.
Captain can identify an externally issued OAuth bearer, but does not claim a
third-party login or refresh flow that xAI has not published. Provider quota
reset cards likewise depend on two official observations and replenished
reported capacity, never on a copied entitlement table or local timer.

The local PR portal verifies an exact GitHub head SHA in a disposable Lima VM
using root-owned policy and a sealed source tree before guest networking is
removed. It publishes `captain/local-pr-gate` and recovers orphaned pending
states after restart. Remote repository protection and publication still
require a valid authenticated GitHub session and must be read back before they
are described as active.

## Alpha 10 Release Provenance

Local release builds remain the source of truth and do not consume automatic
GitHub Actions minutes. The five host targets are built sequentially with a
disk/load checkpoint before and after each target. The publisher adds a
deterministic in-toto/SLSA v1 statement and checksum to the 20 base assets,
making 22 uploaded assets for this release. The statement binds every base
asset to the public Git commit/tree and exact `Cargo.lock`; verification rejects
asset, source, or lockfile drift.

Docker `linux/amd64` completes and is remotely inspectable before
`linux/arm64` begins. Only then is the multi-architecture index assembled.
Each architecture carries BuildKit provenance `mode=max`. The host statement
is SHA-256-bound but not independently signed in this alpha, so Captain does
not claim a SLSA certification level.

The versioned public `main` policy requires reviewed pull requests for non-admin
contributors, resolved conversations, linear history, and forbids force-pushes
or deletion. Administrators retain the audited local publication path. The
post-Alpha 10 source additionally requires `captain/local-pr-gate`, produced by
a trusted local portal in a disposable Lima VM and bound to the exact PR SHA;
the three-OS workflow remains a manual fallback. The Alpha 10 policy was
applied and read back before publication, but this stricter post-release policy
must not be described as remotely active until a fresh authenticated
`--apply` and `--verify` both pass.

## Alpha 10 Live Budget Contract

The boot configuration is immutable; the current global budget has one
synchronized live authority. An API update validates a complete candidate,
persists the exact TOML snapshot atomically, and publishes it only after the
write succeeds. A persistence failure keeps the preceding live state. Reads
from REST, WebSocket, Control/channel status, agent spawn/restore, and config
hot-reload therefore cannot observe a partially updated budget.

Finite non-negative cost limits are capped at `1_000_000_000` USD,
`alert_threshold` is restricted to `[0,1]`, and token limits must fit TOML's
signed integer representation. Streaming and non-streaming turns enforce the
same live global cost guard before the per-agent scheduler quota. Agent token
edits update that scheduler immediately.

## Alpha 10 Guarded Execution Contract

`captain-runtime::guarded_exec` is the single security boundary for
agent-controlled subprocesses. It covers the shell and package tools, goal
checks/recovery, Markdown skill capabilities, `execute_code`, workflow shell
actions, static skill checks, Hand dependency installation, and WASM host
execution. Each surface applies execution policy and critical-pattern review,
clears the inherited daemon environment, restores only safe or explicitly
authorized values, sets workspace and timeout bounds, limits captured output,
and emits command-free structured audit events.

Only the interactive shell rail can request one-shot operator approval, and
its permit is bound to the exact content digest and execution surface.
Unattended surfaces block critical commands. `scripts/guarded-exec-audit.sh`
runs in tranche and release gates and rejects raw process construction or
environment mutation in every covered sink.

## Alpha 10 Host Execution Posture

New installations use execution policy `full` with critical mode `safe`.
Routine host commands remain available, while recognized catastrophic commands
fail closed. `open` remains an explicit operator opt-in for content-bound
approval, and `paranoid` requests approval for every shell-affecting operation.

The host backend reports `host_process`, isolation level `environment_scrub`,
`os_isolation: false`, and danger guard
`normalized_lexical_heuristic` through CLI, TUI, authenticated health/status,
and Security API surfaces. Environment clearing, workspace/process bounds, and
normalized command recognition are not an operating-system sandbox. Docker and
WASM are separate explicit isolation backends.

## Alpha 10 Browser Session Contract

Each installation owns one 32-byte browser-session signing key generated by the
operating-system CSPRNG and durably persisted at first boot. Session signing is
independent of the daemon API key and password hash. Tokens include a managed
credential epoch; setup and `web_credentials_update` advance it whenever an
existing password changes, invalidating older sessions immediately.

The signing key and password hash are redacted from CLI and API config-display
surfaces. Raw config writes preserve the managed values without returning them
and reject attempts to replace the key or change the epoch directly. A
persisted auth table with a missing, malformed, or incomplete signing state
fails closed instead of falling back to stale in-memory key material.

New hashes are salted Argon2id PHC strings. A successful legacy SHA-256 login
atomically migrates the persisted hash before issuing a session. A bounded
limiter tracks IP and normalized username independently, starts exponential
backoff after five failures, and caps it at 15 minutes. Cookies have explicit
`auto`/`always`/`never` Secure policy. Browser WebSocket/SSE transports use
30-second path/IP/epoch-bound tickets removed on their first consume attempt;
query-string API keys and session tokens are rejected. This closes F3.

## Alpha 10 API Authentication Contract

With API-key or browser-session authentication enabled, the API is
deny-by-default. One typed source allowlist contains only `GET /`, embedded
`/assets/*` and boot icons/manifests, minimal `GET /api/health`,
`GET /api/version`, browser login/check/logout, and the exact UUID-shaped
per-agent ingress route. That ingress remains protected by its own per-agent
Bearer token, request bounds, idempotency, and rate limit.

Operational state is private: detailed health/status, agents, sessions,
approvals, budgets, channels, logs, models/providers, Config/Terminal pages,
A2A discovery/tasks, and GitHub Copilot OAuth all require global
authentication. The OAuth flow is deliberately private because completion can
persist a provider credential; A2A discovery is private because it returns
agent manifests. Control consumes protected-route `401` responses centrally
and returns to the login screen.

## Alpha 10 Browser Origin Contract

CORS is fail-closed independently of daemon API-key presence. Its default
origins are the API port on `localhost`, `127.0.0.1`, and IPv6 loopback.
`deployment.public_url` and exact entries in `[api].allowed_origins` extend
that list explicitly; malformed or non-HTTP(S) entries are ignored rather than
opening the policy. Methods and request headers are enumerated, never wildcard.

An outer request middleware rejects a missing, ambiguous, malformed, or
undeclared `Host` with `400` before the route, authentication, or application
handler runs. Loopback, a concrete configured listen IP, and hosts derived
from the declared origins are accepted. Because these layers are constructed
at daemon startup, changing `[api].allowed_origins` requires a restart.

## Alpha 10 Audit Hash Chain Contract

Captain's audit trail is a versioned linear SHA-256 hash chain, not a tree.
Version 2 prefixes every encoded field with its `u64` big-endian byte length,
so different field boundaries cannot produce the same serialized input.
SQLite schema v36 retains version-1 rows byte-for-byte and records the hash
version and epoch for every entry.

An append reaches durable storage before the in-memory tip advances. A failed
write returns an error, discards the candidate entry, emits a high-severity
alert, and degrades audit health. Operations that cannot be rolled back use
the explicit `record_or_alert` policy; they never treat a failed candidate as
part of the validated chain.

Startup verifies the active epoch. If it was altered, Captain never rewrites
the original rows: it seals that epoch as invalid and transactionally opens a
new epoch with a `ChainRecovery` entry anchored to the preceding stored
terminal digest. Every sequence at or after the active epoch start must belong
to that epoch, so altering an entry's epoch cannot hide it from verification;
recovery IDs also skip any value present in altered rows. Historical corruption
remains visible after every restart while the recovery epoch remains writable.
Unknown action names retain their exact stored value.

Authenticated `/api/health/detail`, Prometheus metrics, `captain security`,
`captain doctor --full`, and the TUI expose the same integrity state. The
public health probe reports only overall `ok` or `degraded` plus version.
There is no repair operation and no mounted HTTP repair endpoint.

## Previous Public Release

`v0.1.0-alpha.8` is the previous public prerelease. It combines Captain Forge's
readable native capabilities with durable internal hourly token guards and
provider-reported Codex subscription windows. Its immutable public surfaces
are:

- release: <https://github.com/Vivien83/captain/releases/tag/v0.1.0-alpha.8>
- image: `ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.8`
- source commit: `d82f120153b8e83e9be82df6748f928f8d4aa6b9`
- annotated tag object: `2e59fc0e3daed8d306b6efcd8fff24913ba83503`
- OCI index digest:
  `sha256:af32a605de0a019482ff3aadcee07179171630ccfb45c9b88fbcf135d2680230`
- `linux/amd64` manifest:
  `sha256:f55c91a3610560fbe06558721100bd5ab8faef12f4d7e6927d62ff28c9718184`
- `linux/arm64` manifest:
  `sha256:598c067a4ca105a463bca253d62633b22533d19ecf6003467ffbd0a94940745d`
- host asset contract: exactly 20 files covering five platforms, checksums,
  manifests, and four installers

The annotated tag dereferences to the source commit above. At publication time,
the immutable image and moving `:alpha` channel resolved to the same OCI index
digest. Anonymous checks downloaded `manifest.json` and `install.sh`
byte-for-byte and inspected both image architectures successfully. The GitHub
Actions API returned zero runs because the release was built and published
locally. A real `captain update --yes --version v0.1.0-alpha.8` then verified
the public checksum, replaced the installed binary, restarted the daemon, and
passed health, full doctor, SQLite integrity, and retained-state checks.

Known `alpha.8` limitation: an explicit per-turn memory write opt-out
suppresses the post-turn graph, MemPalace, reflection, and learning paths, but
the core agent-loop finalizer still writes its local episodic interaction
fragment. The normal transcript and audit remain intentional; this extra
semantic fragment does not. Treat the opt-out as incomplete until a later
immutable release closes the core finalizer path.

## Alpha 9 Contract

The published `alpha.9` release promotes two contracts developed after
`alpha.8`:

- Skill Learning V2 replaces the active SkillSynthesizer v3.13 path with one
  durable lifecycle for evidence-bound Skills, CapSpecs, Automations, and
  refinements. Telegram, API, TUI, Control Web, and Desktop consume the same
  exact operator projection.
- The native Captain release monitor checks after startup and every 12 hours.
  It follows the installed stable/prerelease channel, requires complete host
  bundle/checksum assets, and persists candidate, exact decisions, detached
  install result, and leased Telegram Rich delivery. **Update**, **Defer 24 h**,
  and **Refuse this version** bypass the model and require the exact configured
  Telegram chat plus an explicit numeric user. Docker/manual procedures never
  gain host authority and stay observable until a later runtime check.

`captain status --json` and `GET /api/status` expose this monitor under
`runtime_update`. The implementation deliberately preserves the exact release
tag for GitHub asset download while using its canonical semantic version only
for comparison and display. Power loss between decision, child launch, result,
restart, and notification is bounded by durable state, timeout recovery,
quarantine, and delivery leases.

## Alpha 10 First-Use Interaction Contract

The seven-question first-use interview remains deterministic, durable, and
free of LLM token cost. Bounded preferences are projected as non-blocking
suggested replies on Ratatui, Control Web/Desktop, and Telegram; free-text
answers remain valid everywhere. The shared stream contract distinguishes
these suggestions from blocking `ask_user` decisions, and an empty suggestion
set explicitly clears stale controls. Explicit bounded notification answers
also control `channels.silent_mode`; ambiguous free text remains profile data
without silently changing runtime policy.

## Alpha 10 Credential Contract

The published `alpha.10` release has one credential resolution contract:
an explicitly mapped file in `secret-sources.toml` is authoritative, followed
only when no mapping exists by `secrets.env`, `vault.enc`, legacy `.env`, and
the process environment. Active LLM drivers and provider status, channels,
event webhooks, per-agent API ingress and signed callback egress, MCP
injection, CLI credential writes, provider tests, update checks, and doctor use
that same chain. An
unavailable external file fails closed and never reveals or revives a stale
fallback value.

External mappings accept no command source. The versioned registry and source
files are bounded, permission-checked, blocked from generic file tools, and
their values are zeroized after use. `captain vault sources [--json]` exposes
logical keys and stable readiness codes only; individual paths and values are
not an operator status surface. Resolver-backed consumers observe file
rotation live; cached adapters require explicit reload, while registry edits
and boot credentials such as the daemon API key require restart.

## Alpha 10 Learning Visibility Contract

Skill Learning V2 persists a process-scoped heartbeat in schema v34 and
projects one public-safe status snapshot from a single SQLite read transaction.
It distinguishes disabled, starting, healthy, active, recovering, degraded,
and stalled states; a model binding failure is degraded and a stale heartbeat
requires operator attention. The snapshot includes only bounded error scopes
and codes, never provider output, raw jobs, credentials, or host paths.

The exact same contract is consumed by `GET /api/learning/status`, TUI
Learning, Control Web and Desktop, Telegram Rich `/learning`, and the `engine`
object returned by `workflow_learning_list`. `/learnings` remains the memory
review queue. None of these surfaces claims percentage progress for opaque
model work.

## Alpha 10 Compaction Progress Contract

Context compaction has one typed, session-scoped progress contract across
Ratatui, Control Web and Desktop, the Web terminal, Telegram Rich, agent
WebSocket, and daemon SSE. The wire object contains phase, state, operation,
runtime, agent and session identity, context pressure, and optional exact chunk
units. It deliberately contains no percentage. Consumers derive a gauge only
when both completed and total units are present and the total is non-zero;
opaque model calls remain visibly indeterminate.

Manual compaction remains conservative. Captain prunes only eligible old tool
payloads and summarizes only a complete older conversational prefix. When the
recent coherent turn consumes the whole history, no empty model request is
issued, no prior summary is replaced, and the stored session is not rewritten
or repaired as a side effect. The terminal result reports that the intact
recent context was retained instead of claiming that zero messages were
summarized.

Telegram renders every compaction gauge inside a fixed visible track. Exact
running chunk units retain their derived percentage, opaque running work stays
indeterminate, and a successful terminal state always overrides stale
intermediate units with a full `100%` gauge. Failure and interruption never
claim completion.

SQLite schema v35 stores only active compaction operations. Every progress
event and active-state transition shares one transaction with the append-only
session timeline. A normal cancellation or dropped task emits an
`interrupted` terminal state. After an abrupt process or host stop, startup
closes operations owned by the previous runtime instance and retains the full
recoverable session. Reconciliation is idempotent and never relies on a guessed
timeout.

The versioned `scripts/compaction-progress-terminal-smoke.mjs` proof renders
the exact and indeterminate contracts on desktop and mobile. The standard
`chat` surface gate also runs the cross-crate compaction tests and the Control
performance smoke, so release validation cannot silently omit either Web UI.

## Earlier Public Release

`v0.1.0-alpha.7` is an earlier public prerelease. It keeps kernel-backed tools
available in direct TUI/CLI turns, supervises the macOS service after unexpected
exits, follows the active model catalog window, and gives committed SQLite and
file state an explicit power-loss boundary. Its immutable public surfaces are:

- release: <https://github.com/Vivien83/captain/releases/tag/v0.1.0-alpha.7>
- image: `ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.7`
- source commit: `dc2f64603eff708a8eab5735121cfc1a2d39386f`
- OCI index digest:
  `sha256:e49e1ad02d6a65742343aaf7abcd1c4fcfd277dab605d3d284830f03c7d42354`
- host assets: exactly 20 files covering five platforms, checksums, manifests,
  and four installers

The annotated source tag dereferences to the source commit above. At publication
time, the immutable image tag and moving `:alpha` channel resolved to the same
digest; anonymous release download and OCI pull both succeeded for
`linux/amd64` and `linux/arm64`. The GitHub Actions API returned zero runs: the
release was built and published locally.

## Alpha 8 Contract

Captain Forge / CapSpec is implemented and process-level certified in the
published runtime. The reproducible harness passed 130 checks across 14
durable runs on implementation commit
`38ecebaf4e34fcf955c99ee13682b54a70e1c938`. The human-readable certificate is
`docs/evidence/CAPSPEC1_REAL_CERTIFICATION_2026-07-18.md`; the raw transcripts,
temporary homes, and fixture credentials are regenerated locally and remain
outside the public source tree.

The published runtime separates Captain's durable rolling per-agent token
guard from provider-owned subscription allowances. Codex allowance
observations come from its authenticated account usage endpoint, dynamic
response headers, and `codex.rate_limits` stream events. Provider windows and
resets are never hard-coded or inferred from local token totals. CLI, TUI,
Control, `/api/status`, and `/api/budget` expose the same persisted observation;
missing data is `unavailable`, stale data is explicit, and an exhausted
provider allowance produces a structured HTTP `429` without retry or silent
fallback. Compact Chat surfaces identify the configured model and render gauges
only for provider-wide or matching model-specific families. This contract also
belongs to alpha.8 and is not an alpha.7 claim.

## Earlier Verified Public Release

`v0.1.0-alpha.6` remains the preceding verified public provenance. Its
annotated source tag dereferences to commit
`797d093b44a93850b40f058691931c25f1701900`; its 20-asset GitHub Release and
anonymous AMD64/ARM64 OCI image are pinned by:

- release: <https://github.com/Vivien83/captain/releases/tag/v0.1.0-alpha.6>
- OCI index digest:
  `sha256:1054e053d7f20664c4098db04d653e44b261d6cc4bac092a5fbc10a9e76c9318`

At publication time, `ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.6` and
the moving `:alpha` channel resolved to that digest, and the GitHub Actions API
returned zero runs. Production automation must pin an immutable version tag or
digest explicitly.

## Current Contract Docs

These files are maintained as current operator or runtime-facing references:

- `README.md`, `README.fr.md`, `README.es.md`, `README.zh.md`
- `CHANGELOG.md`, `CONTRIBUTING.md`, `SECURITY.md`
- `docs/README.md`, `docs/INDEX.md`, `docs/getting-started.md`,
  `docs/troubleshooting.md`, `docs/DEPLOY.md`
- `docs/cli-reference.md`, `docs/api-reference.md`, `docs/configuration.md`
- `docs/channel-adapters.md`, `docs/providers.md`, `docs/skill-development.md`
- `docs/performance-budgets.md`
- `docs/SKILL_LEARNING_V2.md`
- `docs/CAPTAIN_FORGE_CAPSPEC.md`
- `docs/evidence/CAPSPEC1_REAL_CERTIFICATION_2026-07-18.md`
- `docs/architecture.md`, `docs/security.md`, `docs/workflows.md`,
  `docs/agent-templates.md`
- `docs/captain-tools/*.md`
- `docs/deployment/github-vps-install.md`,
  `docs/deployment/vps-web-terminal.md`
- `docs/releases/*.md`
- `crates/captain-graph/README.md`
- `crates/captain-graph/bindings/{c,node,python,wasm}/README.md`

Current contract docs must avoid volatile exact totals unless the number is
generated, tested, or directly tied to an executable gate. Prefer live commands:

Every tracked `README*` file must appear in the DOC2 audit inventory. Adding a
README without classifying and validating it is a documentation gate failure.

The public navigation exposes only current install, operation, API, security,
and contributor guidance. Historical migrations, superseded deployment
profiles, internal plans, research, and phase-oriented implementation notes are
excluded by `git archive` and rejected by the public source audit.
Unverified one-shot launchers, broad host-access Compose overlays, the frozen
migration crate, and the stale Desktop-oriented Nix flake are excluded for the
same reason.
The standalone A2A compatibility guide is also excluded; active MCP behavior
remains documented by `docs/captain-tools/mcp.md`.

```bash
captain --version
captain status
captain doctor --full
captain agent api <agent>
captain models providers
captain models aliases
captain models list
scripts/docs-global-audit.sh
scripts/docs-release-audit.sh
scripts/control-web-audit.sh
scripts/control-xss-smoke.mjs
scripts/control-chat-performance-smoke.mjs
scripts/launch-site-audit.sh
node scripts/launch-site-browser-smoke.mjs
scripts/web-terminal-unicode-smoke.mjs
scripts/release-workflow-audit.sh
scripts/release-readiness.sh
```

## Agent-Facing Source

`docs/captain-tools/*.md` is compiled into the runtime through `captain_docs`.
These files are the source of truth for tool-family guidance shown to agents.
Any runtime-visible tool behavior change must update the corresponding
`captain_docs` family and pass the `captain_docs` tests.

Markdown below `skills/`, bundled crate assets, and selected crate directories
can also be executable or build-time source. These files remain in the public
repository for reproducible builds even when they are not linked from the
human documentation index. They are not additional product promises.

## Historical Docs (Maintainer-Only)

The private maintainer checkout retains implementation plans and historical
design documents. They are not part of the public source export and are not the
current runtime contract unless a section explicitly says it was refreshed
under DOC2:

- `docs/launch-roadmap.md`
- `docs/PREPUBLICATION_24H_PLAN.md`
- `docs/excellence-roadmap.md`
- `docs/installation-excellence-roadmap.md`
- `MIGRATION.md`
- `docs/SECURITY-PROFILES.md`
- `docs/ssh-setup.md`
- `docs/v3.*.md`

Historical docs may contain old counts, old completion markers, or pre-DOC2
product assumptions. They must carry a DOC2 historical banner when they contain
release-like completion labels or exact global test/API/model/channel totals.
`.gitattributes` marks this material
`export-ignore`, and `scripts/public-release-audit.sh` rejects it from a public
tree.

## Frozen Compatibility

Remote skill marketplaces are disabled rather than merely de-emphasized: active
HTTP routes and TUI actions are absent, `captain skill install` accepts only an
existing local directory, and retained compatibility clients fail before
network or filesystem access. Reopening them requires publisher-backed
integrity.

The skill prompt-text scanner reports `advisory_heuristic` assurance. The
loader's conservative refusal of high-risk phrase matches is a policy choice,
not proof that matched content is malicious or unmatched content is safe.
Operator review of complete local source remains required.

Long-tail channels, desktop packaging, and other non-core surfaces may exist in
code or compatibility docs, but they must not be presented as active
production-grade product paths unless the current plan explicitly reopens them.
Current docs must label them as frozen, compatibility, historical, or outside
the active release path.

The private checkout retains the old Tauri packaging references in
`docs/desktop.md` and `docs/production-checklist.md`; both are excluded from the
public source export. The active desktop experience is the CLI/TUI plus the
authenticated Control web app; the active release artifact is the cross-platform Captain CLI
bundle.

## Active Product Contract

The operator experience has exactly six primary hubs on TUI and Control web:
Chat, Projects, Automation, Learning, Capabilities, and Status. Automation owns
Workflows, Triggers, Crons, Approbations, and Webhooks. Capabilities promotes
Native capabilities, Skills, and Tools; Hands and marketplace-style surfaces
remain frozen. The Control `Natives` tab validates and installs readable
`.captain` source, binds approvals to the exact pending hash, restores known
revisions, disables source without erasing history, and shows public-safe runs.
The TUI opens the same hub on `Natives`; it selects effective, global, or
project scope, keeps source opt-in, and sends approval, rejection, rollback, or
confirmed disable directly to the authenticated daemon API or in-process
kernel. Those decisions never pass through the LLM.
Status is the operational cockpit backed by `/api/status`, including runtime health,
active work, detached tool runs, agent API egress, budgets, channels,
consciousness, streaming, disk, shutdown, and native media/embedding readiness.
Its budget surface keeps `Captain internal spend` separate from
`Provider subscription (reported)` and preserves provider-reported dynamic
windows and reset times. Full-screen Ratatui Chat and the xterm Web terminal
share a compact bottom band that names the active model and gives gauges only
to provider-wide windows and matching model-specific families. Other families
are summarized as outside the active model; Status and Budget keep every
primary/secondary window. Control web and the frozen desktop wrapper render the
equivalent responsive band. All four surfaces refresh from Captain locally
every five seconds and preserve the last valid observation through transient
daemon errors; only the daemon talks to the provider.

Persisted chat sessions are durable and independently addressable. New Web/API
clients create detached sessions, each turn carries its validated `session_id`,
and reopening one conversation must not switch another channel or tab. Session
reset preserves the previous transcript; explicit history deletion is the only
destructive path. Unlabelled sessions derive a bounded title from the first
meaningful user request, while explicit labels remain authoritative. The Web
drawer exposes every persisted session even though its local PTY convenience
cache remains bounded. The full TUI, standalone TUI, line-based CLI and Web
Control all read this same SQLite catalog and can reopen a session by UUID,
unique prefix or title. Legacy
`$CAPTAIN_HOME/sessions/*/*.json` files (`~/.captain` by default) are imported
at kernel boot with deterministic UUIDs and preserved timestamps; successful
files receive a `.json.imported` sidecar so migration stays one-shot.
The frozen Tauri Desktop wrapper serves the same Control app and kernel, so it
inherits this contract rather than maintaining a separate history.

`captain sessions export --all [--agent <name|id>]` reads this global catalog
without activation and emits one `captain.session.export.v1` JSONL record per
session in newest-first catalog order. A file destination is written atomically
and owner-private on Unix. The artifact is a sensitive, user-facing export for
inspection or external archiving, not a raw hidden reasoning dump and not a
supported restore format.

Tool approvals are one shared operator contract across TUI, authenticated
Control Web/Desktop, API, and Telegram Rich. Interactive session and durable
decisions bind the exact agent, tool, and action digest; only the administrator
configuration can still grant a broad `allow_always` override. Durable rules
are human-readable in `approval-rules.json`, crash-safe, bounded, secret-
scanned, fail closed when corrupt, and revocable by ID. Denial reasons are
bounded and reach the blocked agent. Raw actions are not persisted in rules or
audit entries. The digest is computed from the complete untruncated tool input,
never from its bounded display preview.

Codex model availability is live runtime state, not a fixed documentation
list. With a Codex agent registered, the daemon refreshes the official catalog
after startup and hourly, persists newly seen IDs as deduplicated pending
decisions, and exposes them through authenticated Control/API plus configured
Telegram delivery. Availability never changes an active model by itself:
keeping is explicit, and switching requires an agent and a provider-portable
session strategy (`new_session` or `compact_session`).

Reasoning selection is durable per agent and shared by Ratatui, Control,
Desktop compatibility, REST, CLI and Telegram. Auto means no request override
and stays distinct from explicit `none`. A Codex catalogue may expose Ultra as
a product mode even though the response endpoint accepts `max` as its highest
wire effort; Captain therefore persists and displays `ultra`, maps the provider
request to `max`, and enables proactive delegation only for a depth-zero agent
that can reach native coordination tools. Sub-agents never inherit that
proactive policy.

Context capacity is model-scoped live metadata. Every turn resolves the
configured provider/model from the runtime catalog; compaction, agent/session
APIs, restored sessions, and the TUI use that same effective window. Codex
uses the active `context_window`, never the optional `max_context_window`
override ceiling. Capacity, approximate active transcript occupancy, and
cumulative usage are distinct values and must remain distinct in docs and UI.

Each agent's configured provider/model is authoritative for every normal turn.
Captain does not substitute a cheaper or larger model from message complexity,
token count, session age, or channel. Specialization uses an explicitly created
or delegated sub-agent. Failure-only fallbacks are opt-in: Captain never derives
them from unrelated provider credentials found on the host.

Images and prompted browser screenshots stay on the active conversation model.
Captain sends their pixels through the provider's native multimodal request and
never auto-spawns a Vision agent or changes provider behind the user's back. A
text-only active model receives an actionable refusal before the request and
must be changed explicitly. Browser captures without a visual prompt remain
share-only and cannot support visual claims.

The standalone presentation site is publicly reachable at
`https://captainagent.fr/` (with `https://www.captainagent.fr/` as an alias),
but its source remains maintainer-only and deliberately absent from the public
Git repository. In the private checkout, `site/index.html`,
`site/assets/site.css`, `site/assets/site.js`, and
`site/assets/terminal-demo.js` remain a separately audited product surface.
Building or deploying that site never changes the public source export,
release bundles, or authenticated Control app; the local browser smoke proves
the build, not the state of the separately deployed host.

## Reproducible Gates

DOC2 is enforced by:

- `scripts/docs-global-audit.sh` for global doc/status coherence.
  It also pins each `captain-graph` binding README to the symbols exported by
  its checked-in header, type surface, or implementation source.
- `scripts/captain-graph-bindings-check.sh` for isolated C, Node.js, Python,
  and WebAssembly binding compilation with a supported CPython interpreter.
- `scripts/docs-release-audit.sh` for high-risk release-facing claims.
- `scripts/control-web-audit.sh` for the six-hub Control contract and JavaScript
  syntax. `scripts/control-xss-smoke.mjs` executes malicious Markdown, tool
  output, and session-label probes under the production CSP.
  `scripts/control-chat-performance-smoke.mjs` certifies exact delta batching,
  long transcript hydration, tail pinning, and desktop/mobile layout.
- `scripts/docs-global-audit.sh` also parses the bundled JavaScript/Python API
  clients and pins their cross-surface session primitives.
- In the private maintainer checkout only, `scripts/launch-site-audit.sh` and
  `scripts/launch-site-browser-smoke.mjs` certify the presentation site. Both
  scripts and the site itself are excluded from the public source tree.
- `scripts/web-terminal-unicode-smoke.mjs` for the embedded xterm Unicode width
  contract, including double-width emoji redraw and copied buffer text.
- `scripts/release-workflow-audit.sh` for release targets, manifests, installers,
  and publish dependencies.
- `scripts/prepare-github-export.sh` for a committed, history-free public source
  tree and `scripts/public-release-audit.sh` for forbidden paths, gitleaks,
  manual-only Actions, exact-case Markdown links, and the encoded
  `scripts/public-boundary-guard.sh` policy. The guard scans hidden paths and
  file contents without spelling maintainer-only names in the public tree.
  `scripts/public-export-smoke.sh` repeats that export from a dirty development
  tranche, proves that forbidden path and content probes are rejected, and
  executes DOC2 inside the reduced tree before commit.
- `scripts/release-readiness.sh`, which runs both docs audits before release.
- `scripts/core-surface-gates.sh --surface settings-status`, which includes the
  docs audits in the status/settings surface gate.
