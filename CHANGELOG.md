# Changelog

All notable public changes to Captain are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and version numbers
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0-alpha.14] - 2026-08-13

Early-access release focused on one authoritative Captain Hub, lightweight
Clients, outbound execution Nodes, and crash-safe distributed work.

### Added

- A versioned Hub/Client/Node protocol now carries capabilities, logical
  workspaces, explicit grants, leases, monotonic sequences, acknowledgements,
  heartbeats, idempotency keys, progress, cancellation, and terminal evidence.
- Lightweight terminal, TUI, and Desktop Clients pair through a browser code,
  reuse the Hub's sessions and work surfaces, and carry no competing provider
  loop or memory database.
- Optional macOS, Linux, and Windows Nodes connect outward over HTTPS 443,
  preferring authenticated WebSocket with bounded HTTPS stream and long-poll
  fallback. Environment and explicit proxies plus enterprise CA bundles are
  supported.
- The authenticated Devices registry exposes roles, presence, versions,
  capabilities, approved grants, transport, offline reasons, and revocation.
  Enrollment is closed by default and opens only for a bounded operator window.
- Sessions and projects can select Auto, Hub, or one capable Node without
  exposing the Node's physical workspace path.

### Reliability

- Hub and Node rails persist envelopes, ACKs, leases, approvals, cancellation,
  outboxes, and terminal evidence before delivery. A possible interrupted side
  effect becomes `uncertain` and is never replayed blindly.
- The local Node worker repeats workspace, path, family, mutation, approval,
  and runtime guards before execution. Idempotent replay returns reconciled
  evidence without running the same effect twice.
- `captain service start/restart` now waits up to a bounded 90 seconds for cold
  boot health and reports whether the service manager remains active on
  timeout.

### Security

- One per-device credential produces short-lived, role-scoped access tokens;
  revocation is checked by the Hub and raw credentials stay out of durable Hub
  state, status, errors, and logs.
- Client work authority excludes secrets, configuration, installation,
  updates, shutdown, device administration, and persistent approval grants.
- Nodes keep physical paths local, accept only explicitly bound logical
  workspaces and tool families, and require a fresh local approval proof for
  sensitive effects.
- The TUI stack now uses Ratatui 0.30 with fixed `lru 0.18.2`; the stale
  Windows-only `quick-xml 0.37.5` path is also gone. The unfiltered RustSec
  release baseline contains no vulnerability record, and source builds now
  require Rust 1.88 or newer.

## [0.1.0-alpha.13] - 2026-08-09

Early-access release focused on adaptive delivery verification, crash-safe
evidence, discreet multi-surface progress, and safer host updates.

### Added

- Effectful tool calls now produce ordered, redacted verification receipts.
  Captain accepts a delivery only when the current receipts prove the relevant
  post-condition; read-only conversation keeps its direct path.
- Missing evidence triggers at most two targeted correction rounds. Pending
  jobs, failed effects, stale checks, and uncertain external outcomes end as an
  explicit incomplete result rather than a success claim or blind replay.
- Verification lifecycle records survive abrupt restarts using durable leases,
  fixed gap codes, digests, and timestamps without persisting raw input,
  output, paths, model drafts, or hidden reasoning.
- Projects, delegated jobs, immutable artifacts, and Live Runs now project the
  same evidence strength and subject status into completion decisions.
- Ratatui, standalone terminal, Web Control, Web terminal, and Telegram share
  transient verification phases. Fast successful checks remain silent;
  corrections, slow verification, and incomplete delivery are visible.

### Reliability

- The macOS updater now prepares an adjacent private candidate, requires
  ad-hoc signing, executes an exact `--version` preflight, syncs the candidate,
  and only then performs the atomic replacement. Failure leaves the installed
  binary untouched.
- Wasmtime timeout watchdogs are cancelled and joined when execution ends,
  preventing completed sandbox calls from retaining engines and delayed Mach
  exception handlers.
- Power-loss certification now proves verification interruption, session and
  project recovery, Live Run reconciliation, MemPalace continuity, audit-chain
  continuity, and SQLite integrity after a real `SIGKILL`.

### Security

- Verification evidence is a sanitized metadata contract. It never grants
  new authority, bypasses approvals or budgets, changes the configured model,
  or replays an uncertain effect.
- Streaming drafts for a delivery that still needs verification are withheld
  behind a bounded fail-closed buffer. Failed provider attempts and superseded
  drafts are discarded; the final verified or honestly incomplete answer is
  emitted once.
- Every held continuation segment must match the provider text and its single
  terminal event before release. Direct streaming may retry before output
  starts, but never after a visible delta, preventing duplicated prefixes.
- Budget, continuation, and iteration stops with unresolved evidence persist
  an incomplete verification outcome and replace any held draft with one
  honest terminal response.

## [0.1.0-alpha.12] - 2026-08-09

Early-access release focused on durable tool execution, evidence-grounded Web
research, immutable artifacts, managed VPS domains, and continuous deployment
readiness.

### Added

- Foreground and detached tool calls now share one durable Live Runs ledger.
  Bounded, secret-redacted and checksum-verified output stays outside the model
  context and remains readable, searchable, or tail-able after a surface
  disconnect. Explicit retries are input-digest-bound; interrupted effects
  require uncertainty acknowledgement and never replay automatically.
- Authenticated `/api/tool-runs` routes expose selective metadata and a tail
  capped at 200 lines and 32 KiB, without raw input, raw result, preview,
  filename, or managed path. Control Web consumes this contract from a global
  drawer and cancellation succeeds only for an active run with a real abort
  handle.
- `web_research_batch` now distinguishes discovery-only search snippets from
  fetched, citation-ready evidence with canonical/final URL, HTTP status,
  retrieval time and content SHA-256. The dependent `web_citation_audit`
  refetches sources, rejects invented URLs or missing quotes, measures inline
  provenance, and preserves unsourced claims as `[unverified]`.
- User files can be published as immutable checksum-bound artifact versions,
  delivered through existing native channel authority, inspected through
  authenticated APIs, previewed under a sandboxed Web CSP, and downloaded
  only after byte-count and SHA-256 verification. Control adds a global
  artifact drawer and Ratatui adds a metadata-only `/artifacts` overlay.
- The VPS installer accepts one public `CAPTAIN_DOMAIN`, keeps Captain on
  loopback, configures Caddy transactionally, preserves existing sites, opens
  an active host firewall, validates TLS plus public/local version parity, and
  reports the owner-only browser credentials path without echoing secrets.
- A continuous Deployment Readiness worker checks local/public health, DNS,
  TLS, reverse-proxy routing, ports and exact version parity every five
  minutes. Doctor, CLI/Ratatui Status, API Status and Control read the same
  private crash-safe snapshot with deduplicated remediation actions.
- A global Ratatui `/runs` overlay now exposes the authenticated Live Runs
  inventory without adding another hub. It provides local status filters,
  bounded redacted tails, stale-response protection, and two-step confirmed
  cancellation only for runs backed by a live abort handle. Standalone
  `captain chat` and the Web terminal receive a metadata-only summary capped at
  twelve rows.
- The core can now form disabled-by-default approval suggestions from repeated
  one-time Low/Medium approvals bound to the exact agent, tool, and complete
  action digest. It persists no raw action, never changes a prompt or authority
  by itself, and requires a separate explicit acceptance to create an existing
  revocable exact rule. Boot reconciles a rule committed immediately before
  power loss with its stale suggestion. This checkpoint does not yet claim
  operator-facing list, accept, or dismiss controls.

### Security

- Live Runs operator access stays behind Captain's existing API-key or Web
  session authentication. Its projection omits raw action and output fields,
  applies a second bounded-tail secret scan, and sends nothing to Telegram or a
  model provider.
- Artifact preview is fail-closed: text and Markdown are escaped, HTML/raster/
  PDF use CSP `sandbox`, SVG and unknown active formats are download-only, and
  no destructive artifact route or silent pruning policy is included.

### Reliability

- Boot reconciles every unfinished tool call to `interrupted`, finalizes only
  matching partial evidence, sanitizes legacy retained rows, and refuses an
  unredactable value instead of exposing a raw fallback.
- The reproducible daemon power-loss smoke now uses a private ephemeral API
  key instead of relying on obsolete credentialless loopback access. Its real
  `SIGKILL` cycle additionally proves that a detached cross-surface session can
  be reopened and activated, that the pre-crash audit tip remains in the same
  valid audit epoch, and that SQLite integrity is preserved. A synthetic
  in-flight Live Run is also restored as `interrupted`: its partial owner-only
  capture is finalized and redacted, remains discoverable through the private
  operator API, and cannot be ambiguously cancelled after restart.

## [0.1.0-alpha.11] - 2026-08-08

Early-access release focused on secure extensibility, durable external
operations, native account integrations, and locally enforced contribution and
release gates.

### Added

- Native Gmail OAuth supports multiple named accounts, least-privilege
  send/read/assistant profiles, encrypted token rotation, live identity checks,
  deterministic Gmail-to-agent rules, bounded mailbox tools, and explicit
  dead/uncertain delivery recovery.
- The conversational Email channel supports multiple named IMAP/SMTP accounts,
  account-specific allowlists and agents, one explicit default, live IMAP/SMTP
  probes, and schema-driven configuration shared by TUI, Desktop, Web Terminal,
  API, and CLI.
- xAI is available during first-use provider setup with non-billable identity
  and API-key ACL validation. Externally issued OAuth bearers are recognized,
  while unsupported native OAuth login is reported honestly.
- Provider-confirmed subscription resets produce one durable Telegram Rich
  notification and content-free Status/Budget queue health. Captain requires
  both a new provider reset identity and replenished reported capacity.
- A local pull-request portal verifies the exact PR SHA in a disposable Lima
  guest and publishes `captain/local-pr-gate` without automatic paid GitHub
  Actions.
- Repository discovery metadata, crawler policy, schema.org data, sitemap, and
  IndexNow handoff are reproducible and read-back verifiable.

### Security

- Private API and web routes now fail closed when setup has not provisioned a
  daemon API key or browser credentials. Credentialless development access
  requires the explicit `auth.allow_unauthenticated_loopback = true` opt-out
  and the actual client must be loopback, including behind a declared reverse
  proxy. Existing configurations with explicit `auth.enabled = false` migrate
  once to that visible compatibility flag; setup and credential rotation
  always disable it.
- Direct-program execution permits now bind to a versioned, domain-separated,
  length-prefixed encoding of the executable and every argument. Human-readable
  review text is kept separate, so embedded NUL bytes or argument boundaries
  cannot alias the authorization digest.
- Login-limit capacity pressure no longer evicts an active IP or username
  block. If all 4,096 slots are actively blocked, Captain applies a logged
  five-second global fail-closed backoff; public deployments still require an
  upstream edge limiter because this daemon-local state resets on restart.
- Control, Web Terminal, Config, and the retained Desktop wrapper no longer
  grant inline or evaluated JavaScript authority. Every production script is
  an embedded same-origin asset, ES modules use canonical vendored URLs, and
  CSP rejects script attributes, plugin objects, base-tag changes, and framing.
  Markdown is reduced to a fixed passive tag/attribute set with safe link
  protocols; a Chromium smoke proves malicious Markdown, tool output, and
  session labels remain inert.
- Host execution now has explicit `personal_workstation`, `remote_operator`,
  and `untrusted_execution` deployment profiles. The structural policy default
  is `allowlist`; `remote_operator` cannot exceed allowlist semantics and
  `untrusted_execution` cannot start agent-controlled host processes, even
  when an agent manifest requests `full`. Per-agent and daemon policies are
  intersected before tool visibility and dispatch. `process_start` now uses
  the same content-bound permit as other guarded subprocesses. Docker and WASM
  remain explicit rails: Captain never auto-routes or falls back to the host.
  CLI, Doctor, TUI, Control, health and Security expose configured versus
  effective policy plus Docker readiness.

### Reliability

- The Email rail now pins `imap 3.0.0-alpha.15` with
  `imap-proto 0.16.7`. This removes the parser macros that future Rust versions
  will reject and also removes the unsound legacy `lexical-core 0.7.6` chain.
  The release dependency gate binds the exact prerelease, parser parent,
  `native-tls` feature, and vendored TLS path until upstream publishes 3.0
  stable.
- The credential vault master key now lives in macOS Keychain, Windows
  Credential Manager, or Linux Secret Service/keyutils. Headless deployments
  use an explicit `CAPTAIN_VAULT_KEY`; legacy migration is verified before the
  obsolete local key file is deleted.
- IMAP messages are accepted durably before Captain marks them seen. Mailbox,
  folder, UIDVALIDITY, and UID form the idempotency identity, so crash recovery
  can re-acknowledge a retry without starting a second agent turn.
- Email configuration writes use a bounded inter-process lock and a rollback
  boundary spanning TOML plus secret persistence. Concurrent account updates
  preserve both effects, and a failed secret write restores the preceding
  configuration.
- Channel clients no longer treat every HTTP 200 envelope as success, leave a
  failed list request spinning forever, expose fake local enable toggles, or
  render typed secrets in setup previews.

## [0.1.0-alpha.10] - 2026-07-30

Early-access production-hardening release focused on a deny-by-default
security perimeter, guarded host execution, durable crash recovery, coherent
operator controls, and locally attested sequential release builds.

### Added

- Context compaction now exposes one typed live operation across Ratatui,
  Control Web, Desktop, the Web terminal, Telegram Rich, agent WebSocket, and
  daemon SSE. Exact chunk counts produce a real gauge; opaque model work stays
  explicitly indeterminate instead of inventing a percentage.
- `captain sessions export --all [--agent <name|id>]` emits a versioned JSONL
  catalog without switching any active conversation. Single-session JSON and
  Markdown exports remain compatible; file destinations are replaced
  atomically and kept owner-private on Unix.
- Generic channel replies now enter a durable delivery ledger before the
  adapter sends them. Final agent replies, sanitized agent errors, auto-replies,
  broadcasts, and command responses share idempotency keys, bounded leases,
  retry backoff, dead-letter state, and exact transport receipts.
- `captain status` and `GET /api/status` expose pending, attempting, delivered,
  dead, and potentially ambiguous outbound deliveries.
- `$CAPTAIN_HOME/secret-sources.toml` maps logical credential keys to
  read-only mounted files for Docker, Kubernetes, systemd credentials, or
  secret-manager sidecars. `captain vault sources [--json]` and full doctor
  expose redacted readiness without values or individual paths.

### Reliability

- Local releases now publish deterministic in-toto/SLSA v1 provenance for all
  host assets. Every subject is bound to the public Git revision and
  `Cargo.lock`; Docker architectures build one at a time with host-capacity
  checkpoints and BuildKit provenance before the multi-architecture index is
  assembled.
- The public Alpha 10 tag dereferences to source commit
  `48f898a9e4d38e8b8c7627644b66e22076a39364`. Its 22 GitHub assets match the
  locally certified SHA-256 digests, and the immutable plus moving `alpha`
  images share OCI index
  `sha256:c54d1319b5173ca55540dc69e0f965a31b51cdfccb497ca77882882a16b4e477`.
  Anonymous ARM64/AMD64 execution passed and GitHub Actions reported zero runs.
- Dependency release checks now audit both the configured RustSec view and a
  complete unfiltered view with an exact reviewed baseline. The vulnerable
  `time 0.3.45` and both unresolved RSA branches are gone; SSH accepts
  Ed25519/ECDSA P-256 and fails clearly on RSA. The sole remaining
  vulnerability exception is the unreachable parser surface of
  `quick-xml 0.37.5` in the Windows notification backend, pinned by package,
  parent, and advisory until its upstream chain moves to 0.8.
- The persistent runtime cache now uses a versioned JSON envelope instead of
  the unmaintained `bincode` 1.x format. Upgrade atomically discards only the
  recomputable legacy cache table, and malformed cache entries become purged
  misses instead of failing an agent turn.
- Remote skill marketplace installation is now fail-closed and absent from the
  active API and TUI. `captain skill install` accepts only an existing reviewed
  local directory; retained marketplace clients return a typed frozen error
  before network or filesystem access. Skill prompt-text review now reports
  explicit `advisory_heuristic` assurance. High-risk matches remain
  conservatively refused, but Captain no longer presents phrase matching or a
  self-declared SHA256 digest as publisher-backed proof.
- New installations keep routine host execution available under
  `full`/`safe`, while recognized catastrophic commands fail closed. Detection
  normalizes flag order, whitespace, long flags, common wrappers, and nested
  shell payloads. Status, health, Security, doctor, TUI, and API surfaces now
  report the exact host posture: `host_process`, `environment_scrub`, and
  `os_isolation: false`. Docker and WASM remain explicit isolation backends.
- The misleading internal `subprocess_sandbox` boundary and unused
  `sandbox_command` helper are gone. Environment inheritance now lives in
  `subprocess_env_scrub`, command/path checks live in `subprocess_guard`, and
  every agent-controlled process still enters through `guarded_exec`.
- Security audit history now uses a versioned SHA-256 hash chain with
  injective, length-prefixed fields. SQLite schema v36 preserves legacy rows
  without rehashing and adds append-only recovery epochs: corruption seals the
  affected epoch as invalid and opens a `ChainRecovery` epoch anchored to its
  stored terminal digest. Audit writes persist before in-memory validation;
  failures are returned, logged, and exposed through health, metrics, CLI, and
  TUI. The history-rewriting HTTP repair endpoint has been removed.
- Browser-origin security is now fail-closed independently of API-key
  presence. CORS uses exact loopback, declared public, or explicitly configured
  origins plus reviewed methods and headers. A request `Host` allowlist rejects
  missing, ambiguous, malformed, and undeclared hosts before routing to prevent
  loopback DNS rebinding. `[api].allowed_origins` extends the policy explicitly
  and requires a daemon restart.
- API authentication is now deny-by-default whenever credentials are
  configured. Only Control boot/static assets, minimal health/version,
  login/check/logout, and the exact self-authenticated per-agent ingress route
  bypass the global middleware. Operational reads, Config/Terminal pages, A2A,
  and provider OAuth are protected; Control returns to login on a protected
  `401`.
- All agent-controlled subprocess sinks now share one guarded execution
  boundary. Shell/package tools, goals, skills, code, workflows, static skill
  checks, Hand installers, and WASM host calls apply execution policy,
  content guards, `env_clear()` plus explicit injection, workspace, timeout,
  bounded output, and command-free audit events. Interactive approvals produce
  a content-bound permit; unattended critical commands fail closed. A
  mandatory source audit rejects raw process construction in these sinks.
- Browser sessions now use a per-install 32-byte CSPRNG signing key generated
  and durably persisted at first boot. The key is never derived from an API
  key or password hash. Session tokens carry a managed credential epoch;
  password rotation through setup or the native web-credentials tool advances
  that epoch and immediately invalidates older sessions. Config display
  surfaces redact the signing key and password hash, while raw edits preserve
  but cannot replace the managed state.
- Global budget edits now validate finite bounded values, persist one complete
  TOML snapshot atomically, and publish it to the live runtime only after that
  write succeeds. Concurrent API, WebSocket, Control, channel, spawn, restore,
  hot-reload, streaming, and non-streaming paths observe one coherent budget;
  the previous shared-reference pointer mutation has been removed.
- Explicit per-turn memory opt-out now suppresses the core agent-loop
  finalizer's episodic embedding and local semantic fragment on both streaming
  and non-streaming turns. The resumable session transcript and mandatory
  operational/audit records remain retained.
- SQLite schema v35 records each active compaction in the same transaction as
  its append-only session event. Normal cancellation emits `interrupted`, and
  startup closes operations left by an earlier runtime instance while keeping
  the original session recoverable.
- A daemon restart reclaims responses owned by the previous process without
  rerunning the model or its tools. When the remote outcome is unknowable, the
  replay is visibly marked as a possible duplicate. A channel send failure can
  no longer be recorded as successful delivery.
- Durable outbound payloads are size-bounded. Terminal records discard response
  content and display identity while retaining the minimal audit and
  idempotency metadata.
- TUI and Web terminal background rendering is frame-bounded and reuses parsed
  settled transcript history. Control Web and Desktop batch ordered streaming
  deltas without loss, contain restored rows, and retain live-tail or operator
  scrollback intent across DOM growth.
- External secret mappings are authoritative: an unavailable source never
  falls back to stale local state. Strict schemas, bounded one-descriptor
  reads, permission checks, file-tool blocklists, local-write refusal, and
  fail-closed signed webhooks protect the path. Resolver-backed consumers
  observe file rotation live; cached adapters require reload, while registry
  edits and boot credentials such as the daemon API key require restart.
- Per-agent API bearer auth, signed callback delivery and durable retries use
  the same resolver; externally managed values are neither returned nor
  overwritten during provisioning.
- CLI channel setup now uses the same resolver instead of legacy `.env`,
  preserves external secret pointers without copying values, and emits the
  actual `phone_number` Signal field with TOML-safe user input.

## [0.1.0-alpha.9] - 2026-07-22

Early-access learning and operations release focused on durable native
workflow acquisition and model-independent release updates.

### Added

- Durable Workflow Learning V2 replaces the retired SkillSynthesizer path with
  evidence-bound proposals for Skills, readable native CapSpecs, Automations,
  and refinements. Exact operator decisions, isolated tests, installation,
  canary, rollback, Telegram, TUI, Web, and Desktop now project the same
  crash-recoverable lifecycle.
- Captain checks its compatible official release channel after startup and
  every 12 hours. An explicitly authorized Telegram operator receives a Rich
  card to update, defer for 24 hours, or refuse only that version. The callback
  bypasses the model, stale cards fail closed, host bundles require SHA-256,
  and Docker/manual updates remain operator-owned.
- `captain status` and `GET /api/status` expose release-check cadence, pending
  version, detached installation state, and durable notification retries.

### Reliability

- Release discovery, decisions, installer results, and Telegram delivery are
  persisted across restart. Orphaned attempts recover after a bounded timeout,
  malformed result files are quarantined, future state schemas are not
  overwritten, and failed Telegram delivery is reopened on a later 12-hour
  check without duplicating an already delivered card.

### Publication

- The annotated `v0.1.0-alpha.9` tag dereferences to public source commit
  `1248c5928dd4968b6ff7c62ef79a607fb8d94348`. Its 20-asset GitHub prerelease
  and anonymous multi-platform GHCR image were published locally with zero
  GitHub Actions runs.
- The immutable OCI index digest is
  `sha256:b043ec5637551c2e238be15c32033ca693ecc2f765a470ba721a5986709fd692`;
  the moving `:alpha` channel resolved to that same digest at publication.

## [0.1.0-alpha.8] - 2026-07-19

Early-access extensibility and observability release focused on governed,
human-readable native capabilities and truthful live subscription limits.

### Added

- Captain Forge compiles reviewed `*.captain` TOML files from global or
  project `.captain/` directories into typed, hot-reloaded `cap_*` tools.
- CapSpec executions are durable dependency-aware DAGs. Primitive steps always
  re-enter the central ToolRunner with intersected caller authority, approvals,
  audit, deadlines, interruption state, exact uncertain-node recovery, revision
  history, disablement, and rollback.
- Authenticated API, Control, TUI, and Telegram surfaces expose source review,
  exact-hash approvals, revision control, run inspection, and recovery without
  model-mediated operator decisions.
- Codex subscription allowances are observed from the authenticated account
  usage endpoint and from official response-header/SSE signals. Captain
  persists every provider-reported window, percentage, reset, plan label, and
  credit state without hard-coding hourly, five-hour, or weekly limits.
- `captain status`, the TUI Budget view, Control Status, `/api/status`, and
  `/api/budget` now separate provider-owned subscription allowances from
  Captain's internal token and cost guards. Missing observations are reported
  as unavailable, never as unlimited.
- Full-screen Ratatui chat, the xterm web terminal, Web Control, and the
  retained desktop compatibility wrapper now keep a compact bottom status
  band synchronized from Captain's local snapshot. It names the active model
  first and gives live gauges only to provider-wide windows and limit families
  matching that model. Other model-specific families are summarized as outside
  the active model; Status and Budget retain the exhaustive provider report.

### Fixed

- Per-agent hourly token enforcement now uses the durable SQLite usage ledger
  as a rolling one-hour window, so restarting Captain cannot reset or bypass
  the guard.
- Quota rejections from agent message endpoints return structured HTTP `429`
  responses with a stable code, scope, usage, limit, and provider-reported
  reset metadata when available. Subscription exhaustion is not retried and
  cannot silently fall through to another provider.

### Publication

- The annotated `v0.1.0-alpha.8` tag dereferences to public source commit
  `d82f120153b8e83e9be82df6748f928f8d4aa6b9`. Its 20-asset GitHub prerelease
  and anonymous multi-platform GHCR image were published locally with zero
  GitHub Actions runs.
- The immutable OCI index digest is
  `sha256:af32a605de0a019482ff3aadcee07179171630ccfb45c9b88fbcf135d2680230`;
  the moving `:alpha` channel resolved to that same digest at publication.

## [0.1.0-alpha.7] - 2026-07-17

Early-access reliability release focused on durable committed state, abrupt
restart recovery, truthful model context, and memory tools that keep working
when the TUI or CLI runs Captain in process.

### Fixed

- Context budgeting now follows the configured model's live catalog window on
  every turn and after model switches. Codex uses the active
  `context_window`, not the larger optional override ceiling. Agent/session
  APIs and the TUI expose the same effective capacity, while the TUI meter
  tracks the latest active prompt instead of cumulative lifetime usage.
- The macOS launchd service now remains supervised after login and restarts
  Captain after an unexpected exit. A deliberate `captain service stop`
  unloads the LaunchAgent, so the supervisor does not immediately respawn it.
- Direct TUI and CLI streaming turns now receive the live in-process kernel
  handle when no daemon is available. Kernel-backed tools such as
  `memory_save` no longer fail solely because Captain fell back to local mode.
- Captain confirms a durable memory write only after `memory_save` succeeds;
  a failed tool call is reported as not stored instead of being acknowledged.
- File-backed Captain state now has an explicit power-loss commit boundary.
  SQLite uses WAL and `synchronous=FULL`; Captain-managed files use a synced
  sibling temporary file, atomic replacement, and directory synchronization on
  Unix, including `F_FULLFSYNC` for committed files on macOS.
- A reproducible full-daemon test commits memory, project, and configuration
  state, terminates Captain with `SIGKILL`, restarts the same home, and verifies
  the values plus SQLite integrity. External work that was still in flight
  remains subject to its own interruption and idempotency contract.
- Fresh-home MemPalace repair now runs before the asynchronous daemon runtime
  starts, avoiding a blocking-client runtime-destruction panic during native
  bootstrap.

## [0.1.0-alpha.6] - 2026-07-16

Early-access Telegram UX release focused on native Rich Messages, coherent
tool activity, ephemeral long-run presence, and reliable interactive controls.

### Added

- Telegram final answers use native Bot API 10.2 Rich Messages, preserving GFM
  tables, lists, code, and collapsible details with explicit legacy fallback.
- Independent tools share one live activity board and remain correlated by
  tool-call id even when results arrive out of order.
- Private chats use ephemeral Rich drafts for response formation and idle
  operational presence without adding persistent heartbeat messages.

### Fixed

- Dependent tool calls open a new activity board instead of being presented as
  parallel work; successful rows collapse while failures remain visible.
- `ask_user` button and freeform answers now resolve the waiting agent turn
  before confirmation. Invalid indices preserve the active question, and
  answered or expired cards explicitly remove stale inline keyboards.
- Telegram turn and stale-callback errors are sanitized and rendered as Rich
  control cards without provider payloads or secrets.
- Explicit unsupported-endpoint responses may use the legacy HTML/plain path,
  while ambiguous network or server failures never trigger a duplicate send.

### Known issues

- The per-turn memory write opt-out does not yet suppress the core agent-loop
  finalizer's local episodic interaction fragment. Post-turn graph, MemPalace,
  reflection, and learning paths are suppressed, while the normal transcript
  and mandatory audit remain intentionally retained.

## [0.1.0-alpha.5] - 2026-07-16

Early-access reliability release focused on clean runtime lifecycle, explicit
per-turn memory privacy, truthful live model identity, configured-model
authority, and a single-agent first boot.

### Fixed

- Graceful shutdown now drains persistent Web terminal PTYs and terminates
  their child `captain chat` processes before the API server exits. A stop or
  restart no longer leaves an orphan terminal process or listener behind.
- An explicit instruction not to remember the current message now takes
  precedence over remember-like wording in that same message. Captain keeps
  the conversation transcript and mandatory operational audit, but suppresses
  semantic graph facts, MemPalace mirroring, reflection, conversation/workflow
  learning, and other long-term memory writes derived from the turn.
- Fresh `setup`, `init`, and factory-reset paths no longer copy the bundled
  template catalog into the runtime agent directory. First boot creates only
  the principal `captain` agent; every specialist remains an explicit user
  action.
- All prompt profiles now receive the exact live provider and model selected
  for the current turn. Direct TUI questions can no longer infer Captain's
  model identity from a peer agent or stale session history.
- Automatic complexity routing is removed. Streaming and one-shot turns use
  the agent's configured provider/model instead of silently substituting a
  small, medium, or frontier model.
- Fresh agents no longer infer fallback models from other provider credentials
  found on the host. Failure-only fallback chains remain available only when
  explicitly configured.
- Release-candidate cleanup now captures and terminates the complete isolated
  daemon process tree before removing its temporary home, including native
  MemPalace bootstrap workers after a timeout or interrupted gate.

## [0.1.0-alpha.4] - 2026-07-16

Early-access release focused on authoritative memory corrections, complete
active-journal recall, and reliable cross-surface CLI continuation.

### Added

- Durable memory recall now searches the complete active local journal before
  semantic archives and returns exact active triples to `memory_recall`.
- Memory-save receipts repeat the bounded stored object so an agent can verify
  the effective value before confirming it to the user.

### Fixed

- A correction in the latest user message now overrides recalled background
  facts. Precise product/session markers outrank generic older corrections,
  while active replacement facts are no longer hidden by fuzzy archive guards.
- Automatic memory mirroring applies the same sensitive-field filter as
  explicit memory writes, preventing verification codes, tokens, passwords,
  and similarly named secrets from bypassing the durable-memory guard.
- `captain message` now accepts an agent name as documented, resolves it to the
  unique daemon UUID, and identifies one-shot turns as originating from CLI.

## [0.1.0-alpha.3] - 2026-07-15

Early-access release focused on a self-contained semantic-memory runtime and
durable memory continuity through backend outages and restarts.

### Added

- Official host and container installs now provision an isolated,
  Captain-managed MemPalace 3.5.0 runtime with pinned uv 0.11.28, CPython
  3.13.14, and a frozen checksum-bound dependency lock. No system Python,
  manual `pip install`, secondary model provider, or API key is required.
- Daemon/Web, direct CLI, TUI, and Captain MCP boot paths now share the same
  fail-closed MemPalace readiness and transactional repair preflight.
- Accepted memory additions and invalidations now enter a durable local
  continuity journal before MemPalace synchronization. Local recall therefore
  remains available during a semantic-index outage.

### Fixed

- Daemon boot now performs a live palace and semantic-search probe, repairs a
  missing, corrupt, cross-platform, or insecure managed runtime before kernel
  startup, and fails closed when the configured MemPalace backend cannot be
  made production-ready.
- Managed runtime upgrades use an interprocess lock, immutable generations,
  atomic activation, owner-only memory paths, process-tree timeouts, and a
  bounded active-plus-rollback retention policy. A failed repair preserves the
  active runtime and user palace.
- The core MemPalace MCP bridge launches through the exact Captain executable
  that booted the kernel instead of resolving a potentially older binary from
  `PATH`; explicit operator MCP overrides still take precedence.
- Degraded memory operations are never age-deleted or dropped after a retry
  cap. Restart-safe exponential backoff, bounded batches, and first-failure
  isolation keep them recoverable without hammering an unavailable backend.
- `memory_forget` preserves audit history and journals idempotent MemPalace
  invalidations. Correction guidance now enforces retract-old, then save-new.
- Doctor and learning metrics report memory backlog age, next retry, attempt
  count, and bounded last error instead of presenting unsynced memory as healthy.

## [0.1.0-alpha.2] - 2026-07-14

Follow-up early-access release focused on native visual inspection and a
consistent Captain identity across browser surfaces.

### Added

- Browser screenshots with a visual prompt are attached directly to the active
  conversation model through native multimodal input. This path requires no
  separate Vision agent or secondary provider key.

### Fixed

- Control, Terminal, Config, Apple touch metadata, and `/favicon.ico` now use
  the same embedded Captain logo instead of leaving terminal tabs unbranded.
- Codex and OpenAI-compatible streaming requests preserve images beside tool
  results, while durable sessions omit transient screenshot base64 payloads.
- Text-only active models now reject images with an actionable switch message
  instead of silently delegating them to another agent or provider.
- Release gates can reuse the release Cargo profile explicitly, avoiding a
  second debug artifact tree during local publication.

## [0.1.0-alpha.1] - 2026-07-14

First public early-access release. This is a prerelease: interfaces, storage
formats, and behavior may change before `0.1.0`.

### Added

- A persistent Rust daemon with CLI/TUI, authenticated Control web, Telegram,
  Discord, Signal, Email, and HTTP/WebSocket API surfaces.
- Durable conversations, cross-surface session restore, automatic session
  titles, projects, goals, checkpoints, schedules, workflows, and detached
  tool runs that remain inspectable after interruption or restart.
- Capability-scoped tools, explicit approvals, per-agent budgets, loop guards,
  hash-chained audit events, snapshots, and operational health diagnostics.
- Bounded memory injection with durable user facts, session recall, MemPalace,
  a knowledge graph, and optional local ONNX embeddings.
- Isolated agent delegation and an agent-as-service protocol with authenticated
  ingress, signed egress callbacks, readiness reporting, and explicit operator
  action when an external callback URL is not yet known.
- Live Codex catalog refresh with durable notifications and explicit keep or
  switch decisions. Captain never switches models automatically.
- Five checksum-verified CLI bundles: macOS and Linux on ARM64/x86_64, plus a
  Windows x86_64 CLI zip. GHCR images support Linux AMD64 and ARM64.

### Changed

- Built-in prompts are distribution-neutral: no private operator identity,
  language, infrastructure, or filesystem path is shipped.
- Independent read-only work may run concurrently; dependent or side-effecting
  calls remain ordered and fail closed.
- Supervisor telemetry distinguishes recoverable failures, cancellations, and
  actual task panics.
- Web Control, TUI, CLI, API, and the frozen Desktop wrapper use one canonical
  persisted session catalog.

### Fixed

- UTF-8 output split across browser PTY chunks and wide Unicode terminal cell
  widths are handled consistently across Web and TUI.
- Stale `ask_user` channels are removed after answer, completion, cancellation,
  or disconnect.
- Long-lived WebSocket/SSE clients and channel adapters have bounded shutdown
  windows, so they cannot retain a listener-less daemon indefinitely.
- Public source export is generated from committed `git archive` content,
  starts with a new history, excludes maintainer-only material, checks exact
  Markdown links, scans secrets, and keeps GitHub Actions manual-only.
- Linux cross-builds now receive the release version inside their containers;
  macOS and Linux binaries are executed before packaging, and macOS signing
  fails closed if its ad-hoc signature cannot be verified.

### Known limitations

- This alpha is not intended for critical workloads. Keep backups and review
  every capability before enabling it.
- macOS binaries are ad-hoc signed but not Apple-notarized. The Windows CLI is
  not Authenticode-signed. Verify the published SHA-256 sidecars and expect an
  operating-system approval prompt on first launch.
- Captain binds to loopback by default. Any remote deployment must use Captain
  authentication plus HTTPS/reverse-proxy controls; do not expose an
  unauthenticated daemon directly to the Internet.
- The presentation site is maintained separately and is not included in the
  public source repository or this release.

[Unreleased]: https://github.com/Vivien83/captain/compare/v0.1.0-alpha.13...HEAD
[0.1.0-alpha.13]: https://github.com/Vivien83/captain/compare/v0.1.0-alpha.12...v0.1.0-alpha.13
[0.1.0-alpha.12]: https://github.com/Vivien83/captain/compare/v0.1.0-alpha.11...v0.1.0-alpha.12
[0.1.0-alpha.11]: https://github.com/Vivien83/captain/compare/v0.1.0-alpha.10...v0.1.0-alpha.11
[0.1.0-alpha.10]: https://github.com/Vivien83/captain/compare/v0.1.0-alpha.9...v0.1.0-alpha.10
[0.1.0-alpha.9]: https://github.com/Vivien83/captain/compare/v0.1.0-alpha.8...v0.1.0-alpha.9
[0.1.0-alpha.8]: https://github.com/Vivien83/captain/compare/v0.1.0-alpha.7...v0.1.0-alpha.8
[0.1.0-alpha.7]: https://github.com/Vivien83/captain/compare/v0.1.0-alpha.6...v0.1.0-alpha.7
[0.1.0-alpha.6]: https://github.com/Vivien83/captain/compare/v0.1.0-alpha.5...v0.1.0-alpha.6
[0.1.0-alpha.5]: https://github.com/Vivien83/captain/compare/v0.1.0-alpha.4...v0.1.0-alpha.5
[0.1.0-alpha.4]: https://github.com/Vivien83/captain/compare/v0.1.0-alpha.3...v0.1.0-alpha.4
[0.1.0-alpha.3]: https://github.com/Vivien83/captain/compare/v0.1.0-alpha.2...v0.1.0-alpha.3
[0.1.0-alpha.2]: https://github.com/Vivien83/captain/compare/v0.1.0-alpha.1...v0.1.0-alpha.2
[0.1.0-alpha.1]: https://github.com/Vivien83/captain/releases/tag/v0.1.0-alpha.1
