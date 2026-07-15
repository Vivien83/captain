# Runtime changelog family

> **Status:** audited (D.18).
> See [`README.md`](README.md) for the index and drift policy.
> This family has no exclusive builtin tool. It is Captain's public-safe,
> agent-facing source of truth for real runtime updates.

## Tools

No tool belongs exclusively to this family.

Use this document through `captain_docs({family:"runtime-changelog", query:"update changelog runtime"})` when:

- the system prompt contains `Mise a jour runtime reelle`;
- the user asks whether Captain was updated;
- Captain needs to explain what changed after an install/restart;
- a capability appears to contradict an older memory or assumption.

Decision rule:

1. Do not infer update details from `git log`, local branch names, private paths, or old session summaries.
2. First read this changelog.
3. Then verify affected capabilities with `capability_search` or the relevant `captain_docs` family.
4. If this changelog has no entry for the current install, say that no agent-facing changelog entry is available and answer only from live tool schemas/docs.
5. Keep explanations public-safe: no hostnames, secrets, private aliases, personal infrastructure, local-only branch names, or one-off user paths.

## Versioned Entries

### 0.1.0-alpha.3 — Native MemPalace and durable memory continuity

Agent-facing changes:

- MemPalace is a managed core dependency when the configured memory backend is
  `mempalace`. Official installers and containers provision pinned uv 0.11.28,
  isolated CPython 3.13.14, MemPalace 3.5.0, and the frozen dependency graph
  without requiring system Python or another provider key. Daemon/Web, direct
  CLI, TUI, and Captain MCP boot paths all run the same verified repair
  preflight before starting an in-process kernel.
- `captain memory doctor` now verifies exact versions, platform, owner-only
  paths, palace access, and an actual semantic search. Daemon boot repairs a
  missing or broken runtime before kernel startup and refuses production
  readiness if that repair fails. `CAPTAIN_MEMPALACE_INSTALL=0` is an explicit
  degraded-mode opt-out.
- Repairs are serialized and transactional. They activate an immutable
  generation only after validation, preserve the user palace on failure,
  retain one rollback generation, and terminate timed-out subprocess trees.
- The managed MCP bridge always uses the same Captain executable as the
  running kernel. A manually configured MCP server named `mempalace` is still
  treated as an intentional operator override.
- Captain's local `memory_writes` database is now the durable continuity
  journal for accepted add and invalidate operations; MemPalace is its
  semantic index. A MemPalace outage cannot remove a fact from local recall.
- Failed operations use restart-safe exponential backoff. Degraded `error`
  rows remain retryable and are never age-deleted. Each resync tick stops after
  the first backend failure so one outage does not exhaust the entire queue.
- `memory_forget` preserves the original audit row and queues a durable
  `kg_invalidate`. Corrections must retract the exact old triple first and save
  the replacement only after that result. Exact legacy triples absent from the
  local journal are also invalidated durably.
- `captain doctor` and learning metrics expose pending/error counts, oldest
  backlog age, next retry, maximum attempts, and the bounded last sync error.

How to answer the user:

- Do not ask the user to install MemPalace or Python manually. Run `captain
  memory doctor`; if it fails, run `captain memory install --force` and report
  the exact failing probe. A daemon running with the explicit opt-out is
  degraded, not fully production-ready.
- If `memory_save` reports `index=pending/retry-auto` or `index=degraded`, or
  `memory_forget` reports `remote_pending`, explain that the local operation is
  safe and automatic recovery is active. Do not claim MemPalace is synchronized
  until the backlog returns to zero.

### 0.1.0-alpha.2 — Native visual inspection on the active conversation model

Agent-facing changes:

- `browser_screenshot` and the browser batch `screenshot` step accept an
  optional visual prompt. With a prompt, Captain validates the PNG and attaches
  its pixels directly to the same active conversation model through the native
  multimodal request. No Vision agent is called, no provider is changed, and no
  Mistral or other secondary API key is required.
- A screenshot without a prompt remains capture-only and returns an upload URL
  without inviting visual claims. The pixel payload is transient: it is present
  for the current model continuation but omitted from durable tool-result
  serialization, avoiding base64 growth in restored sessions.
- Codex Responses requests now carry image data URLs beside text and tool
  results. The OpenAI-compatible streaming path also preserves user text and
  image blocks around tool results instead of dropping them.
- Capability preflight resolves both `codex` and `openai-codex` aliases against
  the catalog. If an active model is genuinely text-only, Captain refuses the
  image with an actionable model-switch message instead of silently delegating
  it to another agent or provider.
- Control, Terminal and Config declare the same embedded Captain PNG as their
  browser icon and Apple touch icon. Legacy `/favicon.ico` requests return that
  PNG with its real media type instead of an empty response, so restored Web
  terminal sessions keep the Captain identity in their tab and bookmarks.

How to answer the user:

- For a visual conclusion, call `browser_screenshot` with a precise prompt and
  inspect the attached image. Do not treat an upload URL or DOM metadata as
  proof of layout or visual quality.
- If image input is rejected, report the active model and ask for an explicit
  switch to a multimodal model. Never work around the refusal by spawning or
  selecting a hidden specialist.

### 0.1.0-alpha.1 — First public early-access release

Agent-facing changes:

- Captain is now distributed as a public prerelease with five
  checksum-verified host bundles and a multi-architecture GHCR image. The
  immutable release tag is `v0.1.0-alpha.1`; prerelease installers must pin
  that tag because GitHub's `latest` endpoint intentionally excludes
  prereleases. The moving container channel is `:alpha`, not `:latest`.
  Local cross-builds propagate the release tag into the compiled Linux
  runtime, execute every macOS and Linux binary before packaging, and fail if
  its reported version differs. macOS packaging also fails closed unless the
  ad-hoc signature can be created and verified.
- Shipped prompts, fixtures, and examples are distribution-neutral. No private
  operator identity, language preference, infrastructure alias, or local path
  is part of the public runtime defaults.
- Public source is produced from committed `git archive` content with a fresh
  history. Maintainer-only plans and presentation-site sources are absent;
  exact-case Markdown links, private-path patterns, secret file types, and
  gitleaks all gate the export. GitHub Actions remain manual-only, while the
  host bundles and container manifest are assembled locally. The manual
  fallback now waits for the validated Linux bundle artifacts and restores the
  checksum-pinned embedding cache before assembling the same release image.
- This alpha is not a critical-workload release. macOS binaries are ad-hoc
  signed but not Apple-notarized, and the Windows CLI is not
  Authenticode-signed. Users should verify the published SHA-256 sidecars and
  expect an explicit operating-system approval on first launch.

- When a registered agent uses Codex, Captain now refreshes the official live
  model catalog shortly after daemon startup and then hourly. Newly listed
  model IDs are persisted as pending decisions, deduplicated across scans and
  restarts, and loaded into the runtime catalog immediately. A first refresh
  without any prior cache establishes a silent baseline instead of announcing
  every existing model as new.
- Control shows each pending Codex model in a durable notice. Keeping the
  current model closes the notice without changing configuration; switching
  requires an explicit agent plus either a fresh session or a
  provider-neutral compact summary. An active Telegram adapter with a default
  chat receives the same decision once. Captain never enables a newly visible
  model automatically.
- The authenticated API exposes `GET /api/models/updates` and
  `POST /api/models/updates/decision`. The switch path accepts only a model
  that is still pending and reuses the existing safe model-switch preflight.
  Catalog refresh declares Captain's supported Codex catalog protocol
  (`client_version=1.0.0`, distinct from Captain's product version), writes the
  cache atomically, and preserves live runtime changes made to every non-Codex
  provider. HTTP failures retain the server's bounded error detail so a missing
  or changed protocol requirement remains diagnosable.
- Runtime health now distinguishes recoverable agent failures from actual
  supervised task panics. Network/provider errors returned through normal
  `Result` paths increment `failure_count`; they no longer poison
  `panic_count` or keep operational awareness in warning state. Voluntary task
  cancellation is reported as cancellation, and cumulative supervisor counters
  are explicitly scoped to the current daemon process.
- Agent tasks are supervised at their spawn boundary, including detached
  callers, so a real panic is caught and counted once instead of disappearing
  with an unobserved `JoinHandle`. Prometheus exports the separate
  `captain_agent_failures_total`, `captain_panics_total`, and
  `captain_restarts_total` counters.
- The browser PTY preserves UTF-8 sequences that cross raw output chunk
  boundaries, and the TUI markdown renderer measures emoji and wide Unicode by
  terminal cells. Live web replies therefore keep their text and redraw
  alignment instead of depending on byte or scalar-value boundaries.
- The embedded xterm 6 terminal now loads its local Unicode 11 width provider.
  This keeps browser cell widths aligned with Ratatui: replacing text with an
  emoji also clears its continuation cell, so copied text and assistive output
  cannot retain an invisible character from an earlier frame.
- Persisted conversations are now explicitly session-scoped. A Web or API
  client sends the stored `session_id` with each turn; Captain validates its
  owner and continues that transcript without switching the agent's default
  session for Telegram, another browser tab, or another client.
- **New session** creates a detached durable conversation before opening its
  terminal. `/new` preserves the previous transcript, every persisted history
  row remains reopenable until explicit deletion, and UUID-shaped terminal IDs
  are no longer treated as stored sessions unless history validates them.
- Captain automatically names an unlabelled session from its first meaningful
  user request, skipping greetings and slash commands. Explicit names remain
  authoritative and the Web drawer no longer applies its 18-item local PTY
  cache limit to persisted history.
- Session history is one source-independent catalog across Web Control, full
  TUI, standalone TUI, `captain chat --plain`, and the Desktop wrapper. The Web
  drawer now queries global `/api/sessions`, including sessions owned by
  specialized agents. TUI/CLI `/history` and `/resume <UUID|prefix|title>` load
  the owning agent and transcript without a global session switch, including
  when no legacy local JSON file exists. JavaScript and Python SDKs expose the
  same transcript load through `sessions.get`.
- Selecting Web history starts a fresh terminal PTY around the canonical
  session UUID. This avoids reattaching a stale TUI process after another
  surface continued the conversation, and startup resolves specialized-agent
  ownership directly from the stored transcript.
- SSE `ask_user` channels are keyed by agent/session and tied to stream
  lifetime. Normal completion, answer, cancellation, and client disconnect all
  remove the channel; a stale stream cannot remove a newer replacement.
- At boot, historical TUI JSON mirrors are imported into SQLite with stable
  deterministic IDs and original timestamps. The migration is bounded,
  idempotent, ignores tool-only UI artifacts, and never overwrites a row that
  has already continued from Web, API or another client. Successful files get
  a `.json.imported` sidecar, keeping explicit later deletion durable while
  failed files remain retryable.
- Daemon shutdown gives long-lived WebSocket/SSE connections 15 seconds to
  drain, then takes the channel bridge out of shared state and gives adapters a
  separate 15-second drain period. A stuck browser, Telegram, Discord, Signal
  or Email poller can no longer retain a listener-less process and block the
  next managed restart; normal in-flight work still receives its grace period
  before residual tasks are abandoned.
- Maintainer-built release images stage a checksum-pinned FastEmbed snapshot
  from the local Captain cache before Docker assembly. The generated build
  input stays Git-ignored and outside the 20 GitHub Release assets; all five
  model files are verified on the host and again inside the image before the
  architecture-specific ONNX runtime is installed. GHCR publication therefore
  does not depend on the model CDN being reachable during the Docker build.

How to answer the user:

- If asked about a newly available Codex model, distinguish availability from
  activation. Report the current model and pending model ID, then ask the user
  to keep it or switch with an explicit session strategy. Never claim that
  detection changed the active model by itself.
- Do not call an ordinary recovered provider/network error a panic. Check the
  separate supervisor fields and mention that counts cover the current daemon
  process. A non-zero historical `failure_count` is diagnostic context, not by
  itself proof that Captain is unhealthy now.

- If asked how to install this prerelease, use the immutable release tag or the
  `:alpha` container channel. Do not recommend GitHub's `/releases/latest`
  route until a stable release exists.

### 0.1.0-dev.2026-07-12c — Repository README contracts recertified and published

Agent-facing changes:

- The four public Captain README languages keep the same release, install,
  Docker, six-hub, durable-work, and agent-as-service contract. The embedded
  `hora-graph-core` component and its C, Node.js, Python, and WebAssembly
  binding READMEs now use the constructors, entity/fact methods, persistence
  behavior, and naming conventions actually exported by their checked-in code.
  Stale `HoraGraph`, `HoraWasm`, `hora_core_*`, `addEdge`, and `add_edge`
  examples plus fixed test, line, bundle-size, and zero-unsafe claims are no
  longer presented as current facts. DOC2 now audits those component READMEs
  against the C header, language type surfaces, and WASM exports before release.
  The four binding crates are explicit nested Cargo workspaces, so their
  documented source-build commands work from the Captain checkout and are
  compiled by release readiness. Python package metadata now matches the
  supported PyO3 range, CPython 3.9 through 3.13, instead of accepting an
  unsupported 3.14 interpreter.

How to answer the user:

- This release changes documentation, package build contracts, and drift
  protection, not Captain's core agent behavior. Verify `captain --version`
  before claiming that an installation contains the revised embedded docs and
  binding checks. `07-12c` is the published runtime that contains them;
  `07-12b` remains the prior immutable release.

### 0.1.0-dev.2026-07-12b — Distinct Captain identity across every surface

Agent-facing changes:

- Captain's crown emblem keeps its established geometry, while the graphical
  `CAPTAIN` wordmark uses a distinct chamfered command face. The previous
  stepped arcade construction and duplicated wireframe contours are gone.
  Terminal surfaces no longer imitate that typography with oversized character
  art: the TUI now uses a compact, portable `CAPTAIN` signature plus the live
  version, leaving the operational state visible and rendering identically in
  native terminals and xterm.js. The web terminal displays the real graphical
  emblem in its authentication panel and top bar. Every desktop/tray icon is
  derived from the same emblem instead of the retired mascot. One embedded
  asset also supplies all four README languages, the Control login, sidebar,
  favicon, and PWA manifest; web references carry a new asset revision so stale
  branding is not retained by the browser cache.

How to answer the user:

- If asked whether an installed binary contains this identity, verify
  `captain --version` and the web/TUI surface live. The published
  `0.1.0-dev.2026-07-12a` tag predates it and remains immutable;
  `0.1.0-dev.2026-07-12b` is the published release that includes the aligned
  identity.

### 0.1.0-dev.2026-07-12a — Durable work, safe parallelism, and six-hub Control

Agent-facing changes:

- Detached tool runs now survive daemon restarts in SQLite. Runs that were
  active when the process stopped are restored as `interrupted`, while all
  still-running rows and the newest 200 terminal rows remain inspectable with
  `tool_run_list`, `tool_run_status`, and `tool_run_result`. Recent history is
  deterministic and newest-first, so Captain can inspect prior work before
  launching a duplicate diagnostic.
- Task recovery now persists the assistant `ToolUse` boundary before PRE hooks
  or execution begins. Checkpoints carry their session identity and are only
  consumed by that session. After a crash, Captain can explain that an unknown
  tool outcome must be verified instead of silently replaying a side effect or
  attaching recovery context to another conversation.
- Native multi-tool execution is available in streaming and non-streaming
  turns, but it fails closed. Only an explicit allowlist of independent,
  read-only tools may execute concurrently. Unknown tools, MCP tools, skills,
  custom tools, side effects, overlapping file paths, and calls with data
  dependencies stay sequential. Use detached runs with `depends_on` for long
  dependency graphs.
- Release automation now packages the shared installers, build `VERSION`,
  platform manifest, config example, and README through one packager contract.
  The primary local path builds x86_64/aarch64 Linux, x86_64/aarch64 macOS,
  and x86_64 Windows, validates the complete 20-asset set, pushes the
  multi-architecture image, then creates the tag and GitHub Release. The
  GitHub release workflow mirrors the contract as a manual fallback only, so
  tag pushes do not consume Actions minutes automatically.
- The three-OS CI matrix is also an explicit manual fallback. Local
  `release-readiness` remains the certification source of truth, and ordinary
  pushes or pull requests no longer start billed Actions jobs automatically.
- Windows release builds now keep the native Schannel TLS path instead of
  inheriting the Unix-only vendored OpenSSL dependency used to make Linux and
  macOS bundles self-contained.
- The Docker release is now published as one GHCR multi-architecture image for
  `linux/amd64` and `linux/arm64`, with immutable version and `latest` tags.
  The local publisher assembles both variants from the already-verified Linux
  release bundles instead of recompiling Captain under emulation.
  The base Compose file names that published image while retaining the local
  source build, and all four GitHub README languages document the same install,
  six-hub, durable-work, and agent-as-service contracts without fixed counts.
- Runtime self-update compares release tags and embedded versions canonically,
  including tags prefixed with `v`, so a matching release is not reported as a
  false update.
- Telegram callback fallback routing now keeps the original message id. Replies
  to unhandled callbacks remain attached to the right conversation message.
- Workflow history at `GET /api/workflows/{id}/runs` is now strictly scoped to
  the requested workflow, validated by UUID, ordered newest-first, and includes
  bounded `output` and `error` fields. The Control workflow view supports list,
  create, inspect, run, history, and delete against this contract.
- The authenticated Control web app now follows the same six primary hubs as
  the TUI: Chat, Projects, Automation, Learning, Capabilities, and Status.
  Automation contains Workflows, Triggers, Crons, Approbations, and Webhooks.
  Capabilities promotes Skills and Tools; Hands remains frozen and hidden from
  the active navigation.
- Public CLI help now uses the stable six-hub/durable-work contract instead of
  volatile channel/skill/model totals. The frozen `hand` command remains
  callable by its exact name for compatibility but is no longer promoted in
  top-level discovery.
- `captain doctor` no longer tells an operator to start the daemon immediately
  after confirming that it is already running. Offline healthy installs keep
  the start hint; live healthy installs end without a contradictory action.
- Status is now a full operational cockpit backed by `/api/status`: runtime
  health and actions, agents and active work, supervised processes, disk and
  shutdown, LLM readiness, access mode, detached tool runs including
  interruptions, streaming telemetry, agent API egress, budget, projects,
  goals, automation deliveries, channels, consciousness, native voice, and
  embeddings. Control assets are served with no-store semantics so a rebuilt
  daemon cannot keep stale JavaScript under a Cargo-version ETag.
- The release and Control contracts have executable local audits:
  `scripts/release-workflow-audit.sh` and `scripts/control-web-audit.sh`.
  DOC2 pins both audits and the current API/architecture/tool guidance before a
  release build.

How to answer the user:

- If asked whether work survives a restart, explain that detached run history
  is durable and in-flight work becomes `interrupted`; inspect its recorded
  state/result before deciding whether a retry is safe.
- If asked whether Captain can call tools in parallel, say yes only for
  explicitly classified independent read-only calls. Do not promise parallel
  execution for arbitrary MCP/skill/custom calls or for dependent/side-effecting
  operations.
- If asked where workflows or runtime health live in the web app, route to the
  Automation/Workflows or Status hub. The older 2026-06-29 statement that this
  cockpit was a future `WEB1` lot is historical and superseded by this entry.
- If asked for the current binary/runtime version, still verify live with
  `captain --version` or `/api/status`; never infer it only from this changelog.

### 0.1.0-dev.2026-06-29a — Core Excellence closure and agent-as-service

Agent-facing changes:

- Captain's core release-grade path is now closed against the Core Excellence
  plan: real user smoke, Hermes-vs-Captain comparison, release readiness,
  plan/approval safety, capabilities and budgets, restart recovery,
  explainable replay, streaming latency telemetry, and agent-as-service are all
  backed by reproducible gates or live smoke artifacts.
- Runtime/binary version reporting now uses the release build label consistently
  across `captain --version`, `/api/status`, `/api/version`, health, metrics,
  daemon `/version`, MCP handshakes, prompt fingerprint, bootstrap facts, and
  TUI surfaces. A current build should no longer expose only the Cargo crate
  version `0.1.0` on daemon/API surfaces.
- Tool discovery now explicitly routes binary/runtime version questions toward
  `shell_exec` so Captain can verify `captain --version`, `captain status`, or
  `/api/status` instead of answering from historical changelog search results.
- `shell_exec` now treats explicit `timeout_seconds` as a bounded review window,
  not an infinite renewal. It also blocks known live monitoring commands such as
  `pmset -g thermlog`, `log stream`, `tail -f`, `fs_usage`, `tcpdump`, and
  unbounded `top`, and kills the full process group when a hard cap is reached.
  Health checks should use finite snapshots instead of watchers.
- `shell_exec` now also refuses commands that detach lifecycle from the tool
  result (`nohup`, `disown`, unquoted `&`, or nested `bash/sh/zsh -c "... &"`).
  Start servers, watchers, REPLs and local apps with `process_start` instead.
  Hard-cap cleanup is scheduled out-of-band so the agent can regain control,
  inspect state, and decide the next step instead of being trapped in cleanup.
- `process_start` accepts `cwd` and returns it in the start response. Use it for
  project-local servers/apps so Captain can supervise the process through
  `process_poll`, `process_list`, `process_write`, and `process_kill`.
- `project_list` now returns a compact agent-safe projection instead of raw
  project metadata. It accepts `query` for partial names, slugs, and user
  references such as `projet1`, and its output includes project identity, goal,
  status, runtime state, progress, worker counts, and next actions without raw
  metadata. Captain should compare user references against slugs/names before
  interpreting a number as a menu option.
- Runtime prompts now include a bounded `Recent Projects` section for recent
  non-terminal projects. This keeps project continuity visible across Telegram,
  API, web, and CLI turns even when no project is marked active, while keeping
  MemPalace focused on learned memories rather than as the sole project index.
  For project status questions, Captain should resolve the Projects store first
  and only inspect files after durable project state points to a concrete
  workspace need.
- Memory continuity is now aligned across streaming and non-streaming LLM
  paths: both enable automatic graph recall, and `memory_context_batch` is part
  of the CORE tool surface so Captain can retrieve durable memory plus prior
  session context in one read-only call when a user references earlier
  exchanges. Use it before guessing from files or asking the user to repeat
  known context.
- `project_list` and `project_get` are also CORE read-only tools. Captain can
  now inspect durable project state directly when the user asks where a project
  stands, instead of relying on memory search or filesystem probing first.
- Common project-status questions that match a recent durable project can be
  answered by the kernel directly from Projects state before the LLM loop. This
  prevents the recurring bad path where Captain searches the workspace and says
  it cannot find a project that is already registered.
- Streaming `ask_user` now emits a tool completion event after the answer,
  timeout, or channel-closed fallback is recorded. TUI/web/chat surfaces should
  therefore close the `ask_user` tool block instead of leaving it in a running
  state while later conversation messages continue.
- `ssh_exec` now follows the same bounded-review contract for remote commands:
  explicit `timeout_secs` emits progress for a few windows, then closes the SSH
  channel at a hard cap with partial output instead of keeping Telegram/API runs
  alive indefinitely. It also refuses common unbounded remote monitors such as
  `journalctl -f`, `docker logs -f`, `tail -f`, `watch`, `pm2 logs`, and
  `docker stats` without `--no-stream`.
- Captain now exposes native detached tool runs through `tool_run_start`,
  `tool_run_status`, `tool_run_result`, `tool_run_cancel`, and `tool_run_list`.
  Long shell/SSH/code/package checks can run in the background, stream bounded
  partial output into the run preview, and be revisited by the agent instead of
  blocking the conversation. `/api/status.tool_runs` stays payload-free and only
  exposes counts/recent metadata; detailed previews are available through the
  supervision tools. Parallelism is explicit: launch independent runs in
  parallel, and use `depends_on` when a run needs another run's result before it
  can safely start.
- `agent_spawn` now shows a canonical sub-agent manifest in both tool discovery
  and `captain_docs agent-coordination`. Manifest parse errors are rendered with
  public-safe recovery hints across runtime, kernel, and API surfaces: `model`
  must be declared as `[model] provider/model`, not `model = "..."`, and child
  tool access should use `tool_allowlist = [...]` or `[capabilities] tools = [...]`
  instead of `[tools] allow = [...]`.
- Agent capability grants and operator views now treat `tool_allowlist` as the
  strict effective tool surface. `captain agent caps <agent>` and
  `/api/agents/:id.capabilities_effective` no longer report `Tools: none` for
  agents spawned from the current manifest shape, and derived network/memory
  scopes are shown consistently.
- Agent API discovery is aligned with the V6 backend: `captain_docs
  agent-coordination`, `tool_search` descriptions, the API reference, and the
  CLI now point to `/api/agents/{id}/api/manifest`,
  `/api/agents/{id}/api/token/rotate`, and `POST
  /hooks/agents/{id}/ingress`. Captain should no longer answer that a custom
  bridge is required when the user asks how an external service can call a
  running agent.
- `captain agent api <agent>` now gives operators the per-agent external API
  status, ingress URL, manifest URL, token env, egress callback operations,
  queue summary, and concrete next actions. `--manifest` prints the full
  integration manifest; `--rotate-token` explicitly generates the bearer token
  and prints it once.
- Agent creation now follows the `agent-as-service.v1` protocol: `agent_spawn`
  and `POST /api/agents` provision the ingress bearer token by default, return
  it once with the created agent's API sheet, and can configure signed egress at
  creation when `agent_api.egress_callback_url` is supplied. Status `ready` is
  reserved for full ingress + egress readiness; token-only agents report
  `ingress_ready` with a concrete egress configure action that says Captain
  cannot infer the external callback URL for outbound events.
- `captain agent caps <agent>` now gives operators a readable view of effective
  capabilities, resources, token/tool budgets, and quota actions. `/api/status`
  exposes the same budget summary in an operator-safe shape.
- Long-running goals can now report explicit progress through
  `CAPTAIN_PROGRESS=<token>` or `{"captain_progress":"<token>"}`. Repeated
  tokens are treated as non-progress and escalate with an actionable reason
  instead of looping silently.
- Tool decisions are now recorded with a short reason. `captain replay
  <session>` and project replay surfaces show action, reason, status, duration,
  and cost without exposing raw tool inputs or outputs.
- Streaming status now records active/completed streams, first visible signal,
  first token, and total time. Chat/web/CLI/SSE, OpenAI-compatible streaming,
  Telegram streaming, and long tool progress all feed the same status telemetry.
- Agents can be exposed as dedicated services: per-agent ingress with bearer
  auth, signed HMAC callbacks, callback tests, manifests, queue status, retry
  operations, idempotency, and audit events are available through the agent API.
  The live service smoke creates an agent, calls its ingress like an external
  service, verifies signed callbacks, audit trail, and an empty queue.
- The web interface remains operational for terminal, projects, system status,
  auth session, SSE chat, and events. Dedicated web cockpit pages for
  agent-as-service, agent capability cards, replay/cost views, first-token
  cards, and approvals are intentionally tracked as a future `WEB1` operator UX
  lot rather than a core release blocker.

How to answer the user:

- If asked whether this is the post-Core-Excellence build, say yes when the
  runtime changelog exposes `0.1.0-dev.2026-06-29a` and the live status comes
  from the current Captain V2 binary.
- If asked for the current binary/runtime version, verify with live status or
  `captain --version`. Do not answer from an older changelog entry such as
  `0.1.0-dev.2026-06-09a` when the live runtime says otherwise.
- If asked how to communicate with a running agent by external API, list the
  target agent id from `agent_list`, then explain: inspect
  `/api/agents/{id}/api/manifest`, rotate/generate the bearer with
  `/api/agents/{id}/api/token/rotate` if readiness says the token is missing,
  and call `POST /hooks/agents/{id}/ingress` with `Authorization: Bearer
  <token>`. Do not say there is no HTTP endpoint.
- If a user asks whether the Mac/server/service is healthy, use finite snapshot
  commands. Avoid live watchers in `shell_exec` and `ssh_exec`; if a watcher is
  intentional, use a process/supervision path and stop it explicitly.
- If several diagnostics are independent, launch them with `tool_run_start` and
  poll with `tool_run_status`; if one diagnostic depends on another run's
  result, declare `depends_on` or wait for `tool_run_result` before starting it.
- If asked about Hermes-level, say Captain now proves the core path at
  Hermes-level or better for the validated surfaces, while Hermes still remains
  the comparison reference for future UX and long-workflow hardening.
- If asked about the web UI, explain that there is no blocking backend gap, but
  the richer operator cockpit belongs to `WEB1` before a web-first launch.

### 0.1.0-dev.2026-06-09a — First-run simplification and release gate repair

Agent-facing changes:

- The first-run wizard no longer scans for or offers OpenClaw installation
  migration. New users go from the welcome step directly to provider setup.
  OpenClaw `SKILL.md` compatibility remains a separate skill-format feature;
  it is not the installation migration path.
- OpenClaw migration is no longer part of the active CLI/API surface. Captain
  keeps the inactive migration crate out of the default workspace/build path,
  while skill compatibility remains available through the skills registry.
- Full TUI and standalone `captain chat` now share the recent local slash
  command behavior for `/mouse`, `/copy`, `/tokens`, and `/cost`. The handlers
  still own terminal, clipboard, and usage-counter side effects, while the
  parsing and visible messages are tested in small shared helpers.
- Release readiness now checks this current runtime changelog entry by default.
  Its secret scan still flags plausible Slack and GitHub tokens, but no longer
  treats short fixture-like prefixes or documentation-only token prefixes as
  release-blocking secrets.
- Non-interactive install now preserves native integration secrets that use
  logical names such as `integration:tts_openai:api_key` while exporting only
  shell-safe environment variables like `OPENAI_API_KEY`. Clean install smoke
  validates packaged bundles, generated web credentials, TTS config, native
  embeddings setup, and GitHub export without carrying `dist/releases`.

How to answer the user:

- If asked about OpenClaw migration, say that installation migration is frozen
  and hidden from the active onboarding path. Do not confuse that with
  OpenClaw-format skill compatibility.
- If asked whether Captain is release-ready, say that this changelog gate and
  the secret scan pre-blocker pass, and package/install/export are now proved.
  Full release readiness still requires the remaining live user smoke,
  Hermes-vs-Captain run, and final no-skip validation.

### 0.1.0-dev.2026-05-20a — Cron delivery reliability

Agent-facing changes:

- `captain status` now surfaces active work instead of only showing a count.
  When agent runs are active, `/api/status` includes each run's agent, model,
  profile, start time, and age, and the CLI prints a compact `Running Work`
  section for quick operational diagnosis.
- `captain status` also surfaces persistent `process_start` work. The API now
  reports `active_processes` with process id, owning agent, uptime, alive state,
  and command; the CLI prints a compact background-process section when any are
  present.
- Skill discovery is now path-safe. `skill_search`, `skill_view`, and
  `skill_check` still tell agents when a skill is file-backed and list
  relative `linked_files`, but they no longer publish absolute local skill
  paths in normal output or validation blocks.
- Skill refinement is now path-safe too. `skill_refinement_propose`,
  `skill_refinement_list`, `skill_refinement_decide`,
  `skill_refinement_update`, and `skill_refinement_restore` keep a path-free
  internal snapshot id needed for rollback, but normal output only reports
  logical snapshot/backup state and no local skill, snapshot, backup, restored
  path, or snapshot id.
- Controlled-improvement stored text is now safer before it reaches durable
  review memory. `system_bug_report`/`system_bug_update` and
  `skill_refinement_*` still reject raw secret-looking values, and now redact
  local host paths in titles, descriptions, findings, evidence, suggested
  fixes, sources/channels, versions, and notes before storage, list output, or
  `self_improvement_review`.
- Learning and skill-proposal review output is now public-safe too.
  `learning_review_list`, `learning_review_decide`, `skill_proposal_list`,
  `skill_proposal_decide`, and `self_improvement_review` mask secret-looking
  strings and redact local host paths before returning tool output.
  The generated skill approval path keeps the generated file path internal and
  returns only a logical `written` state.
- `skill_proposal_decide` now projects approval/denial results through a small
  allowlist (`status`, `id`, `written`) instead of relaying kernel payloads.
  This keeps generated paths, debug fields, and legacy path aliases internal
  even if an older or extended kernel returns them.
- Generated skill and skill-refinement approvals now reject positive
  self-approval from agent tool calls. Agents can still inspect and reject
  noisy proposals with `approve:false`; `approve:true` is reserved for explicit
  human/API/channel review after external validation, so skill promotion no
  longer relies on silence or model self-evaluation.
- Learning/proposal review lists are now item-projected too. Direct lists and
  `self_improvement_review` return only the fields needed for a decision, while
  audit fields such as agent ids, source labels, origin channels, pattern
  hashes, write ids, decision timestamps, and generated paths stay internal.
- `learning_review_decide` now mirrors that contract: approve/deny results
  return only a logical `status`, the review `id`, and a `memory` state on
  approval instead of relaying the kernel write payload.
- Direct controlled-improvement review lists are now bounded like
  `self_improvement_review`: `learning_review_list.limit` and
  `skill_proposal_list.limit` are clamped to 1-50 before reaching the kernel.
- Review list output is capped after the kernel returns too, so an older or
  overeager kernel cannot publish more learning/proposal items than the
  requested review window through direct lists or `self_improvement_review`.
- `skill_proposal_decide` also keeps prefix resolution inside that bounded
  review window. Agent tool calls should reject from the visible
  `skill_proposal_list` results; positive approval belongs to explicit
  human/API/channel review, while full ids can still be passed through for
  older proposals.
- TUI file drops and `/image`/`/file` uploads now share one strict upload
  allowlist. Dropped terminal paths are accepted only when they point to an
  existing supported file, while normal pasted text and URLs continue to land
  in chat input.
- TUI `/image` upload preparation is now tested in a small helper: path
  expansion, file reads, filename detection, content-type mapping, and
  unsupported-format messages are handled before the daemon-only upload step.
- TUI hub navigation for Automation, Learning, Capabilities, and Connections
  now uses a small tested helper for wrap-around movement and the `Alt+1..n /
  Alt+←→` navigation line. Visible labels and active-tab styling stay the same.
- TUI global chrome helpers are now tested too: modal centering, the
  too-small-terminal message, and boot toasts keep the same visible behavior
  while no longer living in the main TUI coordinator.
- TUI command argument helpers now cover `/model` session-strategy parsing and
  Project URL path-segment encoding. The visible `/model` flags stay compatible
  with the existing Hermes-style `--new` and `--compact` behavior.
- TUI hub shortcuts now share the same tested decision helper as the hub
  navigation line. `Alt+←/→` and `Alt+1..n` keep their visible behavior while
  out-of-range numeric shortcuts are ignored consistently.
- TUI default chat target selection now uses a tested helper for the
  Hermes-style priority order: existing target, `.captain.toml` agent id,
  `.captain.toml` agent name, an agent named `captain`, then the first agent.
  Kernel and daemon side effects stay in the main TUI coordinator.
- TUI event feedback lines for stored memory, queued learning, and proposed
  skills now use a small tested formatter. The visible Hermes-style truncation,
  short proposal id, Captain family label, and localized trigger hint stay
  stable while the main event coordinator gets smaller.
- TUI boot auto-routing now uses a tested helper for the Hermes-style welcome
  decision. The `CAPTAIN_NO_AUTO_DAEMON` escape hatch still keeps the menu
  visible, daemon detection still connects to the daemon, and no-daemon boot
  still starts the in-process kernel.
- TUI list loading now shares tested `ListState` helpers for the two existing
  selection policies: force-select the first item after a loaded non-empty list,
  or select it only when no selection already exists. Empty-list behavior stays
  unchanged.
- TUI fetch-error routing now uses a tested helper for the active tab/sub-view
  target. Errors still land on the local status surface Hermes-style, while
  Captain's Automation, Learning, Connections, and Capabilities sub-views keep
  their more precise routing.
- TUI tick routing now uses a tested helper for periodic decisions. The
  double-Ctrl+C timeout, approval polling while streaming, and active
  tab/sub-view auto-refresh keep the existing 40/24 tick cadence and the
  Projects/Connections refresh behavior while the mutable screen effects stay
  in the main TUI coordinator.
- TUI stream event handling now uses a tested helper for applying runtime
  `StreamEvent`s to chat state. Text still flushes before tool/intermediate
  messages, token usage still updates from `ContentComplete`, `AskUser` stays
  visible in chat, and Captain's model-fallback note plus separate reasoning
  buffer remain intact.
- TUI stream lifecycle handling now uses a tested helper for stream start and
  completion. Each new turn clears stale per-turn telemetry including
  `last_cost_usd`, completion still finalizes streamed text and avoids duplicate
  responses, and the main TUI now records `AgentLoopResult.cost_usd` like the
  standalone chat path.
- Local TUI slash command helpers now cover `/copy`, `/mouse`, `/tokens`,
  `/cost`, `/undo`, `/voice`, `/queue`, and `/clear`. The main slash handler
  keeps backend effects in place, while pure parsing, telemetry messages,
  queued-message formatting, undo behavior, and identity-preserving clear reset
  are tested separately.
- TUI slash navigation now uses a tested mapper for `/home`, `/projects`,
  `/project`, Automation/Learning/Capabilities/Connections hub shortcuts, and
  Budget/Logs/Settings overlays. Hermes-style direct navigation remains intact,
  while Captain-specific hub subviews stay explicit and backend commands remain
  in the main handler.
- TUI slash info output now uses tested helpers for `/status`, `/sessions`,
  and `/agents`. The main handler still performs daemon and registry access,
  while status lines, bounded daemon session parsing, daemon/in-process agent
  rows, and empty-list fallbacks are formatted in one small module.
- TUI slash attachment commands now use a tested helper for `/image` and
  `/file`. The main handler still opens the file picker or runs the existing
  attach pipeline, while command parsing, picker kind selection, path trimming,
  and ignored commands are covered separately.
- TUI slash feedback commands now use a tested helper for `/like` and
  `/dislike`. The main handler still posts feedback to the daemon, while
  command mapping, note trimming, bounded response preview, and the JSON
  feedback payload contract are covered separately.
- TUI slash retry now uses a tested helper shared by the full TUI and
  standalone `captain chat`. Both surfaces still perform their own send/fallback
  effects, while the latest-user-message selection is covered separately.
- TUI slash reload routing now uses a tested helper shared by the full TUI and
  standalone `captain chat`. `/reload config`, `/reload daemon`, and
  `/reload daemon-config` still forward to the daemon, while other args keep
  the existing local session reload behavior.
- TUI slash fortune selection now uses a tested helper shared by the full TUI
  and standalone `captain chat`. Both surfaces still render the localized
  quote, while the Unix-seconds modulo selection and known quote-key bounds are
  covered separately.
- Standalone `captain chat` now reuses the local slash helpers for `/copy`,
  `/mouse`, `/tokens`, `/cost`, `/undo`, and `/queue`. The runner still owns the
  terminal effects, clipboard writes, and local chat mutations, while the
  parsing, queue/undo behavior, and token/cost text are covered in shared tests.
- Standalone `captain chat` scope messages now live in a tested helper. Unknown
  commands, full-TUI navigation hints, attachment/voice availability, and
  feedback persistence notices keep the same user-facing behavior while leaving
  the runner focused on dispatch and side effects.
- `/new` and `/history` session messages now use a tested helper shared by the
  full TUI and standalone `captain chat`. Backend reset, local chat reset, and
  session-picker effects remain in their handlers, while legacy French messages
  and standalone English variants are covered separately.
- `/export` result messages now use a tested helper shared by the full TUI and
  standalone `captain chat`. Markdown writing stays in `ChatState`, while
  Hermes full-TUI French success/error text and standalone localized variants
  are covered separately.
- `/kill` guard and result messages now use a tested helper shared by the full
  TUI and standalone `captain chat`. Backend deletion remains in each handler,
  while Captain protection, success/failure/error text, and no-backend output
  keep the existing Hermes/i18n behavior.
- `/help` text now uses a tested TUI helper. The full TUI still delegates to
  the localized `help.body` text, while standalone `captain chat` keeps its
  narrower SSH/chat-focused command list with advanced screens routed to
  `captain tui`.
- `/top` and `/bottom` slash scrolling now use a tested helper shared by the
  full TUI and standalone `captain chat`. The handlers still own the actual
  scroll mutations, and the existing command normalization remains in the
  handlers.
- `/model` slash dispatch now uses a tested helper shared by the full TUI and
  standalone `captain chat`. No-argument commands still open the model picker,
  argument commands still switch directly through the existing backend
  preflight path, and `--new` / `--compact` parsing stays centralized in
  `tui/command_args.rs`.
- `/think` slash dispatch now uses a tested helper shared by the full TUI and
  standalone `captain chat`. The helper only recognizes the command; the
  handlers still perform the `toggle_thinking` state mutation.
- Direct daemon slash forwarding for `/health`, `/version`, `/config`,
  `/restart`, and `/shutdown` now uses a tested helper shared by the full TUI
  and standalone `captain chat`. Canonical command construction and backend
  forwarding stay in the handlers.
- `/exit` and `/quit` slash dispatch now use a tested helper shared by the full
  TUI and standalone `captain chat`. The full TUI still routes back from chat,
  while standalone chat still sets its local quit flag.
- Standalone `captain chat` now reuses the shared slash info formatters for
  `/status`, `/sessions`, and `/agents`. Backend reads remain in the runner,
  but daemon session lists, agent rows, status text, and empty fallbacks now use
  the same bounded/operator-safe projections as the full TUI.
- Standalone `captain chat` also reuses the shared local `/clear` helper. The
  command still clears the current chat history and keeps the visible agent,
  model, and mode labels intact, matching the full TUI behavior.
- `/copy` labels, empty-state messages, and usage text now come from the shared
  local slash helper for both the full TUI and standalone chat. Clipboard writes
  remain in the handlers, while the visible French/English text stays stable.
- `/mouse` visible messages are now centralized in the shared local slash
  helper too. Terminal mouse-capture changes remain in the handlers, while the
  full TUI and standalone chat keep their existing wording and localization.
- Full-TUI `/voice` now uses the shared local slash helper for its recording
  start message. Duration parsing still falls back to five seconds, and the
  actual recording spawn remains in the TUI handler.
- `/reload` local-session result messages now use the shared reload slash
  helper in both the full TUI and standalone chat. Daemon forwarding and
  session replay effects remain in the handlers, while French Hermes text and
  standalone English variants are tested together.
- `/clear`, `/undo`, and `/queue` now resolve their shared i18n text through
  the local slash helper. Chat history mutation and staged-message state remain
  in the handlers; only the visible local command messages moved behind tests.
- Direct daemon slash commands now share their non-daemon-mode message through
  the daemon slash helper. The handlers still own backend forwarding, while the
  full TUI French text and standalone English text stay tested together.
- Unknown slash-command messages now use the same helper in the full TUI and
  standalone chat. The localized Hermes text still echoes the unknown command
  token when one is present.
- `/retry` fallback text now lives with the shared retry slash helper. The full
  TUI and standalone chat still perform their own send/fallback effects, while
  the no-previous-user-message text is tested in French and English.
- `/fortune` localized text now lives with the shared fortune slash helper. The
  full TUI and standalone chat still own the clock read, while the Hermes quote
  mapping is tested in French and English.
- Slash command parsing now lives in a shared helper used by the full TUI and
  standalone chat. Hermes-style space splitting remains covered, and Captain's
  extra trimming for tabs and invisible command characters is kept under tests.
- `/sessions`, `/tasks`, and `/agents` now resolve their empty/not-connected
  fallback text through the shared slash info helper. Backend fetches and
  daemon/in-process line construction stay in the handlers.
- `/copy` clipboard success and failure text now lives with the shared local
  slash helper. Clipboard access stays in the full TUI and standalone handlers,
  while the Hermes French and English status messages are tested together.
- Full-TUI `/like` and `/dislike` status text now lives with the shared
  feedback slash helper. The TUI handler still owns the daemon POST, while the
  Hermes French daemon-required, success, HTTP failure, and network failure
  messages are tested in one place.
- Full-TUI `/image` and `/file` upload status text now lives with the shared
  attachment slash helper. The TUI handler still owns picker opening, local
  upload preparation, daemon upload, and pending-attachment drain, while Hermes
  French status/error text is tested in one place.
- Full-TUI `/voice` completion status text now lives with the shared local
  slash helper. Recording and upload still stay in the TUI event handler, while
  Hermes French upload/error messages are tested with the existing voice start
  text.
- Full-TUI `/model` safe-switch status text now lives with the shared model
  slash helper. The TUI handler still owns model catalog reads, preflight/apply
  HTTP calls, in-process kernel calls, and model-label updates, while the
  Hermes preflight, blocked, safe-apply, and no-backend messages are tested in
  one place.
- Chat session startup help text now lives with the shared session slash
  helper. The full TUI and standalone `captain chat` keep the exact Hermes
  `"/help for commands • /exit to quit"` message while session binding and
  restore effects remain in their handlers.
- Standalone `captain chat` now reuses the shared model slash status helpers
  for empty catalogs, daemon preflight failures, blocking issues, safe-apply
  errors, in-process preflight/apply errors, and no-backend mode. Backend HTTP
  and kernel calls stay in the runner, while the visible Hermes wording is
  tested in `tui/slash_model.rs`.
- Standalone `captain chat` runtime status messages now live with the
  standalone slash helper. Stream errors, missing active connections, daemon
  spawn failures, template loading failures, and in-process spawn failures keep
  the Hermes wording while boot and spawn effects remain in `chat_runner.rs`.
- `/new` backend reset errors now live with the shared session slash helper.
  The full TUI and standalone `captain chat` keep the Hermes daemon-agent,
  in-process-agent, no-backend, and daemon HTTP error wording while reset
  effects stay in their HTTP/kernel handlers.
- File picker runtime errors now use the shared attachment slash helper. The
  full TUI keeps the Hermes `Explorateur: ...` wording while picker state and
  upload handling remain in the TUI event handler.
- Full-TUI agent spawn status text now lives in a tested TUI helper. Invalid
  manifests, in-process spawn failures, and missing backend mode keep the exact
  Hermes wording while parsing and spawn effects remain in the TUI handler.
- Full-TUI agent event status text now uses the same tested TUI helper. Agent
  kill, kill failure, skill update, and MCP server update messages keep the
  Hermes wording while list mutations and refreshes remain in the TUI handler.
- Full-TUI workflow, trigger, and cron status text now lives in a tested
  automation TUI helper. Creation, deletion, toggle, and mutation messages keep
  the Hermes wording while screen mutations and refreshes remain in the TUI
  handler.
- Full-TUI Learning, skill-proposal, and approval decision status text now uses
  a tested TUI helper. Approved/refused messages keep the Hermes French wording
  while list mutations and refreshes remain in the TUI handler.
- Full-TUI resource mutation status text now lives in a tested TUI helper.
  Session deletion, memory key save/delete, skill install/uninstall, and
  provider key save/delete messages keep the Hermes wording while storage
  mutations and refreshes remain in the TUI handler.
- The boot-time resume prompt now has its summary and relative-age formatting
  isolated behind a small tested TUI helper. The visible prompt behavior stays
  the same, including the defensive no-session fallback.
- The TUI tab bar now has tested overflow and scroll-window helpers. The active
  tab remains visible on narrow terminals, left/right indicators still show
  hidden tabs, and the Ctrl+C pending hint keeps its warning behavior.
- `skill_proposal_decide` now honors the documented operator contract for
  non-ambiguous id prefixes from `skill_proposal_list`. Controlled-improvement
  decision ids are validated before lookup/kernel access, so invalid ids do not
  echo local paths or secret-looking input in tool errors.
- Legacy controlled-improvement registry output is also projected through the
  same public-safe boundary. Older `system_bug_*` and `skill_refinement_*`
  records no longer publish raw local paths, secret-looking strings, or
  internal snapshot locators through list/decision output or
  `self_improvement_review`; system bug records are normalized on the next
  store while skill snapshot locators remain internal for rollback.
- The live schema for `skill_refinement_list` and `skill_refinement_update`
  now includes `status:"restored"`, matching the Captain-only
  `skill_refinement_restore` operation. Agents and UI clients can filter or
  journal a rollback through the normal tool contract instead of relying on a
  hidden runtime-only status.
- `skill_refinement_restore` now keeps missing-skill errors public-safe even
  for legacy registry entries. If the stored `skill` field contains an old
  local path, secret-looking value, or unreliable name, the error reports only
  that the skill is unavailable in the registry.
- Project runtime now writes incremental phase checkpoints after worker phases
  complete, block, or fail. A restart can resume with a readable phase-level
  handoff instead of waiting for the final project checkpoint.
- Project runtime checkpoints now include a structured runtime snapshot. If a
  project loses its metadata runtime but still has a checkpoint, Captain can
  hydrate from the latest checkpoint; if `Start` sees a stale active run after
  restart, it resumes without resetting completed worker phases.
- Project `ask_user` events are now recorded in the runtime as
  `user_questions`. Channel answers update that durable state, and resumed
  workers receive the recorded user decisions instead of guessing after a
  restart.
- If a project answer arrives after the in-memory worker wait is gone, Captain
  marks the runtime as `resume_pending` for that exact phase. `Start` or
  `Resume` then continues from the answered phase without resetting completed
  workers.
- Web/API clients can now answer a pending project question through
  `POST /api/projects/{id}/runtime/answer`. The endpoint delivers to the active
  worker when present, or records the answer for scoped runtime resume when the
  in-memory wait disappeared.
- The Projects web page now renders pending `runtime.user_questions` inside the
  live project run. Operators can answer from option buttons or free text; the
  page posts to the runtime answer endpoint and refreshes the project state.
- `/api/projects/{id}/runtime` now returns `operator_status`, a compact runtime
  health block for operators and integrations. It identifies pending user
  answers, resume-ready state, stale active runs after restart, running local
  orchestrators, worker status counts, the last event, and concrete API actions.
  The Projects page renders the same status beside the live run summary.
- `/api/status` now aggregates project runtimes that need operator action under
  `workload.projects.attention`. `captain status` renders them as `Project
  Attention`, covering pending user answers, resume-ready runs, stale active
  runtimes after restart, blocked phases, and failed phases without opening the
  Projects page.
- Project workers that stop with `TOOL_REQUEST` now appear as
  `tool_request_pending` in `/api/projects/{id}/runtime`,
  `workload.projects.attention`, `captain status`, and the Projects page. The
  operator status includes requested tools, reason, phase, and a resume action
  after the operator has approved or configured the tool.
- Operators can respond to a project `TOOL_REQUEST` with
  `POST /api/projects/{id}/runtime/tool-request`. Approving the request records
  a phase-scoped decision, marks the phase ready to resume, and injects the
  approved tools into the relaunching worker's allowlist and prompt. Denying the
  request records the denial and leaves the phase blocked for manual review.
- Project runtime start/resume now treats `resume_pending` as a generic
  phase-scoped resume marker, not only as a user-answer marker. If a tool
  request was approved, `Start` and `Resume` continue that phase without
  resetting completed worker phases; the Projects page shows `Resume run` when
  operator status is `resume_ready`.
- Resume-ready project status now keeps the resume reason visible. An approved
  tool request is reported as an approved tool request in
  `/api/projects/{id}/runtime`, `/api/status`, and `captain status`, instead of
  being summarized as a stored user answer.
- Denied project tool requests now remain explicitly visible. Operator status,
  the Projects page, `/api/status`, and `captain status` report
  `tool_request_denied` with the denied tools and decision reason instead of
  reducing the run to a generic blocked phase.
- Resumed project workers now receive a compact `Tool approval decisions`
  prompt section. Prior approved tools and denied tool requests are visible to
  the worker, so a denied tool is not blindly requested again and the worker can
  choose another path or return the smallest manual next action.
- `Resume` after a denied project tool request now reopens that phase as ready
  before dispatch. The denied decision stays in the runtime for prompt/context,
  but the old blocked worker status no longer prevents the relaunch.
- If the relaunched worker repeats a `TOOL_REQUEST` for tools already denied in
  that phase, Captain records `worker.tool_request.denied_repeat` and keeps the
  request denied instead of asking the operator to decide the same denial again.
  The repeated-denial flag is visible in `/api/projects/{id}/runtime`,
  `/api/status`, and `captain status`.
- `captain status --verbose` now prints the first concrete project action
  endpoint from `Project Attention`, such as answer, tool-request decision, or
  runtime start/resume. Compact status shows a hint to rerun with `--verbose`
  when an action endpoint is available.
- `captain status --verbose` now also prints the first action `body_hint` for
  `Project Attention` rows, so answer and tool-request payloads are visible next
  to the endpoint without opening another view.
- For pending project questions, `captain status --verbose` now prints a bounded
  question preview and the first options before the answer action, so the
  operator can prepare the answer directly from status.
- For pending or denied project tool requests, `captain status --verbose` now
  prints a bounded tool-request line with phase, requested tools, reason or
  denial reason, and repeated-denial state before the action endpoint.
- The first Project Attention action reason is now printed in
  `captain status --verbose`, and compact status points to verbose action
  details instead of only saying that an endpoint exists.
- The in-process Project Attention fallback metadata builder is now split out
  from the CLI renderer, keeping the display module short while preserving the
  same actionable rows during daemon upgrades or downtime.
- `captain status --verbose` now prints project runtime progress and worker
  status counts for Project Attention rows. The in-process fallback reconstructs
  the same fields from runtime metadata, matching the API `operator_status`
  shape during daemon upgrades or downtime.
- `captain project` now wraps the project runtime operator actions exposed by
  status: `status`, `answer`, `tool-request`, `start`, `resume`, `pause`, and
  `takeover`. Non-JSON output summarizes the updated operator state and prints
  the next CLI actions without echoing private answer text. `status` and
  `replay` now share the same formatter, print every runtime action exposed by
  `operator_status.actions`, and prefer the project slug over the internal id
  when both are available. Printed next commands now shell-quote dynamic project
  ids, ask ids, and runtime phases, so copy/paste hints keep stable argument
  boundaries even if a runtime identifier contains whitespace or quotes.
  Runtime `--json` output is now an operator-safe
  projection instead of the raw daemon payload: it keeps project identity,
  compact status, attention details, result flags, last event, and next
  commands, while omitting raw runtime metadata, worker prompts/tasks,
  transcripts, stored answers, chat agent ids, workspace paths, tokens, and
  secrets.
- Project runtime `operator_status` now applies the same operator-safe contract
  at the API/status source. Pending questions and pending/denied tool requests
  are allowlisted and bounded before being returned by
  `/api/projects/{id}/runtime` or aggregated into `/api/status`, omitting stored
  answers, agent/run/worker ids, nested previous requests, metadata, raw
  payloads, workspace paths, tokens, and secrets.
- The rest of `operator_status` is now finite as well: runtime status, phase,
  resume reason, worker-count buckets, last-event fields, and action body hints
  are constrained to known labels or bounded strings before they reach
  `/api/projects/{id}/runtime` or `/api/status`.
- The in-process `captain status` fallback now uses the same operator-safe
  projection when reconstructing Project Attention directly from persisted
  runtime metadata, so daemon upgrades or downtime do not re-expose raw
  questions, tool requests, worker statuses, last-event payloads, or action
  hints.
- `/api/projects/{id}/runtime` now returns projected `project`, `runtime`, and
  transcript views instead of raw metadata/runtime blobs. The response keeps the
  operator fields needed by the web and CLI runtime views while omitting raw
  project metadata, worker prompts/tasks, stored answers, event `data`, worker
  result payloads, workspace paths, tokens, and secrets.
- The same runtime response no longer includes the legacy `chat.agent_id` block.
  Project chat still uses the normal web/agent chat path, but the runtime read
  endpoint no longer publishes an extra agent id that the Projects page and CLI
  do not need.
- Project list/detail responses now use an allowlisted enriched project view
  instead of serializing raw project metadata. They keep identity, lifecycle,
  source/workspace status, counters, and a sanitized runtime preview while
  omitting the raw `metadata` object, raw runtime snapshot, unexpected source or
  workspace payloads, and clone URLs that could embed credentials.
- `/api/projects/{id}/resume` now returns an allowlisted handoff context rather
  than raw checkpoint/task/goal rows. The response keeps project identity,
  latest checkpoint summary, bounded task and goal status rows, and milestone
  counters while omitting checkpoint `state`, task descriptions/assignees,
  metadata, goal check/recovery commands, recent check logs, suggestions, and
  milestone deliverable payloads.
- Direct project resource API responses now use that same allowlisted boundary
  for tasks, goals, milestones, milestone progress, checkpoints, and project
  launch. They keep ids, status, counters, bounded names/summaries, and
  timestamps while omitting task descriptions, assignees, goal commands, recent
  checks/logs, suggestions, deliverables, checkpoint `state`, raw launch
  payloads, and `rules_file` paths.
- Project list/detail and runtime project views no longer expose local
  workspace/source paths (`workspace_path`, workspace `path`/`default_root`,
  source `path`/`local_path`). The web Projects page now displays workspace
  readiness and repository identity rather than filesystem paths, while runtime
  internals still keep the paths needed to work.
- The web Projects page no longer reads legacy `project.metadata` fallbacks for
  lifecycle, runtime, or worker task text. If a daemon/front-end version skew
  omits the sanitized fields, the UI falls back to default/empty operator views
  instead of re-opening raw metadata blobs.
- Project goal API/web views now expose only whether check/recovery commands
  are configured, not the command bodies. Web goal editing preserves existing
  commands when command prompts are left blank, replaces them only when a new
  command is entered, and asks before clearing a configured recovery command.
- `/api/projects/environment` no longer returns local `workspaces_dir` or
  `project_root` paths. The Projects web page no longer calls it; leaving the
  local folder blank uses the daemon's configured project default without
  exposing the absolute path in the browser.
- The GitHub repository list used by the Projects page now returns an
  allowlisted repository view only: id, name, full name, privacy, default
  branch, and update time. Clone URLs, browser URLs, SSH URLs, git transport
  URLs, permissions, and other raw GitHub payload fields are omitted. Project
  launch from the web sends `github_full_name` and branch only; the daemon
  derives the HTTPS clone URL internally when needed.
- GitHub project launch now treats `github_full_name` as the only repository
  identity. It validates the value as a strict `owner/repo`, ignores legacy
  `github_clone_url` values, derives the clone URL internally, and stores only
  bounded source metadata. This prevents credentialized clone URLs or raw
  repository payloads from becoming durable project metadata.
- GitHub account status and repository listing errors are now projected before
  reaching the Projects browser surface. Account status keeps only login and a
  bounded id. Raw profile URLs, names, emails, avatar URLs, plan payloads, and
  raw GitHub error bodies are omitted.
- Project launch workspace/GitHub preparation errors are now converted to
  operator-safe categories before reaching HTTP clients. Launch failures still
  tell the operator whether the source type, workspace folder, non-git target,
  or GitHub clone needs attention, but they omit local paths, external URLs,
  raw `git`/`gh` stderr, tokens, and network details from the browser surface.
- Project GitHub status, token validation, and repository-list transport
  failures now use the same operator-safe boundary. HTTP status failures remain
  visible as bounded `GitHub returned <status>` messages, while network,
  response, and parse failures omit request URLs, tokens, DNS details, raw
  bodies, and `reqwest` internals from the Projects browser surface.
- Project GitHub token save/remove failures now return storage-level operator
  guidance instead of raw `secrets.env` IO details, home paths, temp paths, or
  token-bearing strings.
- Project question answers (`/api/projects/{id}/runtime/answer`) now return the
  existing operator-safe project/runtime projections instead of raw project
  metadata or runtime payloads. The endpoint no longer echoes the submitted
  answer or raw active-worker/runtime/storage errors.
- Project tool-request decisions (`/api/projects/{id}/runtime/tool-request`)
  now return only an allowlisted project/operator result. The immediate
  response omits raw project metadata, raw runtime payloads, free-form decision
  reasons in `operator_status`, and raw lookup/update/runtime errors.
- Project storage errors across direct Projects routes and the `/project` slash
  surface now pass through a common operator-safe projection. CRUD, lifecycle,
  resume/context, tasks/goals/milestones/checkpoints, and persisted runtime
  mutations keep actionable not-found, busy, permissions, or unavailable
  guidance without exposing sqlite, home/database paths, tokens, or internal IO
  details.
- `PATCH /api/projects/{id}` no longer accepts a free-form `metadata` blob from
  HTTP clients. Operator edits stay limited to typed project fields (`name`,
  `goal`, `status`, `deadline`); runtime/source/workspace metadata changes must
  go through the dedicated project runtime and launch paths.
- The same project edit endpoint now normalizes and bounds typed fields before
  storage. `name` and `goal` are trimmed and reject empty or oversized values;
  invalid `status` returns a static allowlisted error instead of echoing the raw
  request text.
- Project task updates now use the same status boundary. Invalid
  `PATCH /api/project-tasks/{id}` status values return a static allowlisted
  error for the finite task statuses and do not echo raw request text.
- Project task create/update now normalize and bound text fields before
  storage. Titles are trimmed, required and capped; descriptions are trimmed and
  capped; invalid text returns a static validation error without mutating the
  existing task.
- Project task ids now use the same boundary before mutation. `task_id` path
  values on update/delete and `parent_id` body values on create/update are
  trimmed, bounded, and allowlisted. Invalid ids return a static validation
  error, and missing tasks return `project task not found` without echoing the
  submitted id.
- Project task collection routes now validate the path project id before list
  or create store access. Invalid project ids return a static validation error
  without echoing raw paths, tokens, or submitted values, and invalid creates do
  not leave partial task rows.
- Project milestone creation now normalizes and bounds text before storage.
  Names are trimmed, required and capped; deliverables are trimmed, empty
  entries are ignored, and size/count limits return static validation errors
  without creating a partial milestone.
- Project milestone path ids now use the same boundary before list, create,
  progress, or complete. Invalid project or milestone ids return a static
  validation error, and missing milestones return `project milestone not found`
  without echoing the submitted id.
- Project checkpoint list/create now validate the path project id before store
  access and bound history `limit` to 1..100. Invalid ids or limits return
  static validation errors without echoing raw paths, tokens, or submitted
  values.
- Runtime Project tools now apply the same small input boundary before calling
  the kernel. `project_*`, `project_task_*`, `milestone_*`, and
  `checkpoint_save` trim and bound ids, slugs, statuses, and short text fields;
  invalid values return static errors without echoing raw paths or tokens and
  without touching the project store.
- Direct project creation now normalizes and bounds text before storage.
  `POST /api/projects` trims required `name`, trims and allowlists `slug`, and
  trims optional `goal`; invalid values return static validation errors without
  creating a partial project.
- Direct project goal creation and updates now normalize and bound text before
  storage. Goal ids, names, descriptions, check commands, and recovery commands
  are trimmed and capped; invalid values return static validation errors
  without creating or mutating a partial goal.
- Project goal path ids now use the same validation rule before update,
  pause, resume, or delete. Invalid `goal_id` values return a static validation
  error, and missing or cross-project goals return `project goal not found`
  without echoing the submitted id.
- Project launch now validates text before preparing a workspace or creating a
  project. `/api/projects/launch` trims and bounds the goal, optional name/slug,
  branch labels, autonomy level, acceptance criteria, and optional goal guard;
  dangling guard settings, unsafe commands, and oversized criteria fail with
  static validation errors and leave no partial project/workspace.
- Active project selection now validates the agent id and project slug before
  touching the registry. `/api/active-project/{agent_id}` and the `/project`
  slash path trim and bound the inputs, set only after a valid project lookup,
  and return static validation/not-found messages without echoing raw slug text.
- Project API path identifiers now have the same boundary. Direct `id_or_slug`
  route values are trimmed, bounded, and allowlisted before project lookup on
  project mutation, runtime, answer, and tool-request surfaces; invalid values
  and missing projects return static messages without echoing raw path text.
- Direct project checkpoint creation no longer accepts a free-form `state`
  payload from HTTP. Operator checkpoints store an empty state, while runtime
  checkpoints keep the internal structured resume payload; `summary` and
  `session_id` are trimmed and bounded.
- Project lifecycle phase changes now use the same operator-safe boundary.
  Invalid phases return a static allowlisted error and leave the existing
  lifecycle metadata unchanged instead of echoing raw request text.
- `captain project list` now gives operators the project id/slug entry point
  for those actions, with `--attention` to focus on projects that likely need a
  response or resume. Both text and JSON modes use a sanitized projection rather
  than raw project metadata or workspace paths.
- `captain project context <project>` now wraps the durable resume-context API
  without starting the runtime. It accepts project id or slug and prints a
  bounded operator-safe view of project status, latest checkpoint, tasks, goals,
  milestone progress, and next CLI actions while omitting raw checkpoint state,
  task descriptions, goal commands, metadata, workspace paths, tokens, and
  secrets. The next-action list now points to the full reprise loop: replay,
  runtime status, workers, questions, live timeline follow, and checkpoints.
- `captain project workers <project>` now wraps the live project runtime worker
  view. It accepts project id or slug, supports `--phase`, and exposes only
  worker/sub-agent state useful for operator review: id, role, phase, status,
  agent id, tool-name counts, timing, cleanup state, bounded summary, and
  requested tool names. Text and JSON output omit worker prompts, phase task
  bodies, dependencies, raw tool inputs or outputs, tool request reasons, event
  `data`, runtime metadata, workspace paths, tokens, and secrets.
- `captain project questions <project>` now wraps the project runtime
  `ask_user` queue. It accepts project id or slug, supports `--phase`, `--all`,
  and `--json`, defaults to pending questions, and prints the bounded question,
  bounded options, phase, worker role, status, delivery/timing fields, plus the
  exact `captain project answer` next command for pending questions. Text and
  JSON output omit stored answers, agent ids, run ids, worker ids, raw timeline
  payloads, runtime metadata, workspace paths, tokens, and secrets.
- `captain project replay <project>` now prints a bounded Hermes-style reprise
  capsule from `/api/projects/{id}/runtime`. It combines runtime state,
  transcript/session counters, recent transcript events, worker summaries,
  pending questions with answer commands, and next operator actions. It accepts
  `--events`, `--workers`, and `--json`; output omits raw event `data`, worker
  prompts, phase task bodies, dependencies, raw tool payloads, stored answers,
  agent/run ids, runtime metadata, workspace paths, tokens, and secrets.
- `captain project task list|create|update|delete` now wraps the durable
  project task API. List resolves a project id or slug and supports status
  filtering; create/update/delete make task execution state operable from the
  CLI. Text and JSON output are sanitized to task identity, status, title,
  priority and timing fields, omitting descriptions, assignee ids, metadata and
  other free-form task payloads. Delete requires `--yes`.
- `captain project milestone list|create|complete|progress` now wraps the
  durable project milestone API. List/create/progress resolve project id or
  slug, complete operates on a milestone id, and output is sanitized to
  milestone identity, bounded name, status, due/completion times, deliverable
  count, and aggregate progress without raw deliverable text, metadata,
  workspace paths, tokens, or secrets.
- `captain project goal list|create|update|pause|resume|delete` now wraps the
  project-scoped continuous goal API. The CLI can send check/recovery commands
  to the daemon for create/update and can pause, resume, or delete a goal, but
  text and JSON output only expose safe operator fields: id, bounded name,
  status, interval, failure counters, LLM budget, and timestamps. It omits raw
  goal descriptions, check commands, recovery commands, escalation targets,
  recent checks, suggestions, logs, metadata, paths, tokens, and secrets.
  Delete requires `--yes`.
- `captain project timeline <project>` now prints a bounded operator-safe view
  of recent project runtime events from the persisted transcript or runtime
  timeline. It omits raw event `data` payloads and supports `--json` for the
  same sanitized view.
- `captain project timeline <project> --follow` now keeps that timeline open
  and prints only new runtime events as they arrive. It polls the runtime
  transcript, deduplicates by event id or stable event fields, preserves the
  same raw-payload sanitization, and remains text-only so JSON scripting keeps a
  bounded one-shot contract.
- Project runtime transcript reads now use the newest persisted event-log
  window and return it in chronological order. Long project runs and restarted
  daemons keep `captain project timeline`, `captain project replay`, and
  `/api/projects/{id}/runtime` focused on recent useful events instead of the
  oldest rows in the session log.
- `/api/projects/{id}/runtime` now accepts `?events=<n>` with a server-side
  clamp. `captain project timeline` and `captain project replay` pass their
  requested display window to that query, so operator commands avoid fetching a
  much larger transcript than they can show.
- The Projects web surface now does the same bounded runtime read for detail
  refreshes and live polling. Its timeline prefers the recent persisted
  transcript returned by `/api/projects/{id}/runtime?events=80`, falling back to
  the runtime timeline only when no transcript events are available.
- Project collection APIs now resolve a project id or slug to the canonical
  project id before accessing `tasks`, `milestones`, `milestones/progress`, or
  `checkpoints`. Direct API calls by slug now behave like the CLI and avoid
  empty reads or durable rows stored under a slug instead of the real project id.
- Project runtime transcript reads now filter the persisted session log by
  `project_runtime_event` before applying the recent event window and count.
  Other event types in the same session can no longer displace project timeline
  events or make `stored_count`/`truncated` look larger than the replayable
  project event set.
- Project runtime transcript responses now reapply the requested event limit
  after merging persisted events with the `runtime.timeline` fallback. Runtime
  replay, CLI timeline, and the Projects web view keep the newest chronological
  tail and never receive more events than the requested window.
- Project runtime views now expose `runtime.timeline` as an operator-safe tail
  capped at 100 events. `/api/projects/{id}/runtime` no longer carries a full
  sanitized timeline alongside the bounded transcript window.
- Project runtime views now also bound `user_questions` and `workers`. Pending
  questions and actionable workers stay prioritized, then the response fills
  from the recent tail, so operator actions remain visible without returning
  unbounded runtime arrays.
- Project runtime view windowing now lives in `project_runtime_view_window.rs`
  with focused tests for priority-plus-recent selection, keeping
  `project_runtime_view.rs` comfortably below the 500-line refactor limit.
- Project runtime `worker_results` views now include only known runtime phases
  from `observe` through `learn`. Unknown keys in persisted runtime metadata no
  longer collapse into an `unknown` bucket or carry arbitrary stale result text
  into `/api/projects/{id}/runtime`.
- `captain project workers` now mirrors the runtime API windowing after phase
  filtering: actionable workers remain visible first, then the command fills
  from the recent tail instead of showing older non-actionable workers simply
  because they appeared first in the runtime array.
- `captain project replay` now uses the same shared priority-plus-recent worker
  window as `captain project workers`. Limited replay output keeps actionable
  workers visible and fills the rest from the recent tail, rather than taking
  the first stale workers in the runtime array.
- `captain project replay` text rendering now lives in a dedicated small module.
  The JSON replay projection is unchanged, while the compact worker, question,
  transcript, and event lines stay covered by focused tests.
- Project lifecycle helpers now live outside the monolithic project routes file.
  Phase validation, lifecycle JSON fallback/update, and runtime progress values
  are unchanged, but they are covered by focused tests in a small module.
- Project metadata helpers now live in a dedicated small module. Launch metadata
  still wins over legacy source/workspace fields, initial project metadata still
  embeds lifecycle state, and the behavior is covered by focused tests.
- Project runtime worker response summarization now lives in a small tested
  module. Worker status blocks still win over raw provider transcripts, and
  tool-only transcripts still become readable operator summaries.
- Project runtime defaults now live in a small tested module. Session ids still
  derive from the trimmed project slug with an id fallback, and default worker
  parallelism remains bounded to 1-3 with a fallback of 2.
- Project runtime event helpers now live in a small tested module. Timeline
  appends still create structured events, keep the recent tail bounded, detect
  newly appended events for persistence, and deduplicate replay events by id.
- Project metadata runtime writes now share the project metadata helper module.
  Storing `metadata.runtime` still preserves existing metadata objects and
  defensively replaces malformed non-object metadata before inserting runtime.
- Project runtime worker tool-request parsing now lives with the runtime tool
  decision helpers. `STATUS: blocked`, `TOOL_REQUEST`, and `REASON` parsing is
  unchanged, but it is tested alongside approved/denied/retry handling.
- Project runtime worker prompt context now lives in a small tested module.
  Acceptance criteria, project goal check commands, phase gates, and prior phase
  summaries stay compact for Hermes-style resume without keeping that prompt
  shaping logic in the route orchestrator.
- Project launch task blueprints now live in a small tested module. The
  OBSERVE-to-LEARN backbone, descending priorities, acceptance criteria
  projection, and root-task/no-hidden-assignee contract stay unchanged while the
  launch route remains focused on orchestration.
- Project naming helpers now live in a small tested module. Goal-derived titles
  remain bounded, generated slugs stay lowercase ASCII/digit/dash with a
  defensive fallback, and route handlers no longer carry that text shaping.
- Project runtime worker tool allowlists now live in a small tested module.
  Worker tools still start from the phase profile plus discovery defaults, and
  only explicitly approved tool requests for the current phase extend that
  scope; denied or other-phase requests remain excluded.
- Project runtime worker system prompts now share the prompt-context helper
  module. The prompt still names authorized tools, requires `TOOL_REQUEST` for
  missing tools, keeps the worker out of manager scope, and asks for the compact
  handoff headings.
- Project runtime transcript merging now lives with the runtime event helpers.
  Persisted events and runtime timeline fallback are still deduplicated by id,
  sorted by timestamp/id, and capped to the recent requested window before the
  operator-safe projection is returned.
- Project runtime worker blueprints now live in a small tested module. The
  OBSERVE-to-LEARN phase order, dependency DAG, initial ready/planned statuses,
  worker ids, same-provider/fit-to-task policy, and tool allowlist projection
  are unchanged while the route orchestrator stays thinner.
- Project runtime worker state mutations now live with the worker blueprint
  helpers. Runtime resume still defensively initializes missing worker arrays,
  updates a worker without duplicating it, and recomputes `parallelism.running`
  from live worker statuses while preserving the same-provider model-fit policy.
- Project runtime worker model selection now lives in a small tested module.
  `observe`, `think`, and `learn` may still use a lighter same-provider model
  when it exists in the catalog, while heavier phases keep the default model;
  token budgets, think temperature, API key env handling, and worker system
  prompts remain unchanged.
- Project runtime worker manifest assembly now lives in a small tested module.
  Worker manifests still use the scoped project runtime identity, source/fallback
  workspace, high priority, no identity files, no fallback models, project/phase
  tags, model-selection metadata, and the same allowlist projected into both
  `tool_allowlist` and `capabilities.tools`.
- Project runtime worker user prompts now share the prompt-context module. The
  route still resolves workspace, tools, goals, questions, and tool decisions,
  while the prompt text contract, `TOOL_REQUEST` blocker path, verification gate,
  and exact handoff block are covered by focused tests.
- Project runtime orchestrator deactivation now lives with the orchestrator
  helpers. Stopping a run still preserves existing run metadata, records the
  stopped reason, keeps the generation contract, and defensively initializes a
  malformed orchestrator object.
- Project runtime orchestrator resume handling now lives with the orchestrator
  helpers. Tool-approval, user-answer, and fallback resume reasons still map to
  the same triggers/events, while resuming a run reactivates the orchestrator,
  clears pause/takeover control, updates phase/progress, and records the timeline
  event with the existing run id.
- Project runtime worker status lookup now lives with the worker helpers. Resume
  still skips completed phases, blocks on previously blocked/failed workers, and
  recovers stale running workers, while malformed worker arrays simply return no
  existing status.
- Project runtime dispatch-start state now lives with the orchestrator helpers.
  Starting a run still marks the runtime as `running` in `observe`, records the
  same progress value and `orchestrator.dispatch` event, and initializes a
  missing timeline defensively.
- Project runtime completion state now lives with the orchestrator helpers too.
  Finishing a run still marks `done` in `learn`, closes pending project
  questions for that run, deactivates the orchestrator with reason `completed`,
  and records the same `project.completed` event.
- Project runtime worker-start state now lives with the worker helpers. Starting
  a phase still marks the runtime `running`, stores the worker agent id, run id,
  start time, authorized tools, clears phase resume-pending state, and records
  the same `worker.started` event.
- Project runtime worker-skip state now lives with the worker helpers. Resuming
  a completed phase still updates the current phase/progress and records the
  same `worker.skipped` event with run and worker ids before continuing.
- Project runtime worker recovery now lives in a small dedicated helper. A stale
  `running` worker is still reset to `ready`, tagged
  `recovered_from_stale_run`, and logged with the same `worker.recovered` event
  before Captain relaunches the phase.
- Project runtime worker result writing now lives in a small tested helper.
  Completed and blocked worker turns still persist status, summary, usage,
  iterations, tool-call count, cost, `worker_results`, and timeline events,
  while repeated denied tool requests keep the same denied-repeat event and a
  blocked worker still pauses the orchestrator for operator recovery.
- Project runtime worker cleanup now lives in a small tested helper too. After
  a completed worker result is stored, Captain still records the agent stop
  outcome in the worker, `worker_results`, and the `worker.cleaned` event,
  including `cleanup_status`, `stopped_at`, and any cleanup error.
- Project runtime worker failure marking now lives in a small tested helper.
  Failed worker turns still mark the worker `failed`, keep the runtime blocked,
  pause progress for the phase, stop the orchestrator with reason `failed`, and
  record the same `worker.failed` event before the route writes the failed
  phase checkpoint.
- Project runtime waiting state now lives with the orchestrator helpers.
  When a run is paused or in manual takeover before the next worker launches,
  Captain still marks the runtime `paused`, records the current phase and
  paused progress, deactivates the orchestrator with reason `paused`, and emits
  `orchestrator.waiting`.
- Project runtime continuation gating now lives with the orchestrator helpers.
  Worker dispatch still continues only while the runtime status is `running`
  and both `control.paused` and `control.takeover` are false; route code now
  only fetches the persisted runtime before applying that gate.
- Project runtime existing-worker launch decisions now live in a small tested
  helper. Completed phases still skip, blocked or failed phases still stop for
  operator action, stale running workers still recover before relaunch, and an
  incoherent non-stale `running` worker now fails as already running instead of
  being hidden inside route branching.
- Project runtime worker manifest preparation now lives with the manifest
  helpers. Worker spawn still uses the same Hermes-style project worker name,
  high priority, phase tags, model policy, workspace fallback, project-source
  workspace metadata, and approved runtime tool allowlist; route code now only
  consumes the prepared manifest and allowlist before spawning the sub-agent.
- Project runtime worker prompt assembly now lives with the prompt-context
  helpers. Worker spawn still uses the same workspace resolution, project goals,
  approved runtime tools, acceptance criteria, prior phase context, user
  questions, tool decisions, `TOOL_REQUEST` contract, and handoff block; route
  code now only collects the runtime/project inputs before rendering the prompt.
- Project runtime state JSON assembly now lives in a small tested module. Runtime
  views still prefer persisted metadata, fall back to the latest durable runtime
  checkpoint, normalize protocol/version/status/phase/progress, inject the live
  manager agent, keep workers, timeline, control, orchestrator and
  `user_questions`; route code now only supplies the checkpoint fallback and
  manager snapshot.
- Project launch state assembly now lives in a small tested module too.
  `launch_project` still stores the same launch contract, lifecycle, board
  columns, workspace authorization, rules-file status, autonomy level and
  acceptance criteria, while the public `project.created` event remains
  operator-safe and omits raw launch/workspace paths.
- Project launch input normalization now lives with the launch input contract.
  The route still trims and bounds goal/name/slug/branches, defaults title,
  slug, criteria and autonomy level, validates goal guard commands, and rewrites
  branch/goal-guard fields before workspace preparation; focused tests cover the
  normalized request object instead of leaving that validation inline in
  `launch_project`.
- Project runtime worker stream event projection now lives in a small tested
  module. Timeline events for worker loop phases, tool start/input/result,
  non-stdout tool output, and intermediate notes keep the same kind/title/detail,
  status and runtime ids; `AskUser` remains in the route because it also records
  a runtime question and broadcasts the prompt to chat clients.
- Project runtime start state mutation now lives in a small tested module.
  Starting a project still records already-running attempts, resumes stale
  active runs after restart, resumes approved tool/user pending phases, or
  creates a fresh runtime with workers, empty results/questions, and
  `project.started` / `task_graph.created` timeline events before the route
  schedules the background orchestrator.
- Project runtime worker phase finalization now lives in a dedicated helper.
  Successful worker turns still write task status, worker result, blocked
  checkpoints, cleanup status, and done checkpoints; failed worker turns still
  mark the task blocked, write the runtime failure, append the failed phase
  checkpoint, and return the same operator-facing error text.
- Project runtime worker sub-agent launch now lives in a dedicated helper.
  The route still uses the same prepared manifest, parent manager id when
  valid, task `doing` status, `worker.started` runtime mutation, authorized
  tool list, and runtime prompt before streaming the worker turn; invalid
  manager ids remain ignored instead of blocking worker spawn.
- Project launch persistence now lives in small tested helpers. Launching a
  project still creates the active project metadata/runtime, optional guard
  goal, phase task backbone, first delivery milestone, launch checkpoint, and
  operator-safe `project.created` event, but `launch_project` is now only the
  Hermes-style HTTP orchestration wrapper.
- Project launch orchestration now uses a small tested flow helper. The route
  still normalizes input, maps workspace/storage/slug errors to the same HTTP
  responses, publishes `project.created`, and returns the same public response,
  while the helper owns workspace preparation, rules-file seeding, project
  creation, launch records, milestone, and checkpoint persistence.
- Project workspace preparation now lives in a dedicated tested helper. Local
  and GitHub project launches still choose the same source type, expand local
  paths, create or verify folders, clone through `gh`/`git` when needed, record
  branch/source metadata, and apply the same workspace authorization policy.
- Project GitHub setup routes now live in a dedicated tested route module.
  Status, token save/delete, and repository listing keep the same Projects API
  paths and safe error projections, while token validation and `secrets.env`
  updates stay single-line and operator-safe.
- Project goal HTTP routes now live in a dedicated tested route module. Goal
  list/create/update/pause/resume/delete keep the same Projects API paths,
  typed validation, operator-safe not-found errors, status events, and goal-loop
  restart behavior when an edited or resumed goal becomes active.
- Project task HTTP routes now live in a dedicated tested route module. Task
  list/create/update/delete keep the same Projects API paths, slug-to-project-id
  resolution, public resume views, bounded task ids, nullable parent updates,
  and operator-safe storage/not-found errors.
- Project milestone HTTP routes now live in a dedicated tested route module.
  Milestone list/create/complete/progress keep the same Projects API paths,
  slug-to-project-id resolution, public resume views, bounded ids, normalized
  names/deliverables, and operator-safe storage/not-found errors.
- Project checkpoint HTTP routes now live in a dedicated tested route module.
  Checkpoint list/create keep the same Projects API paths, slug-to-project-id
  resolution, public resume views, bounded history limits, normalized summary
  and session ids, and the runtime-only state guard for free-form checkpoint
  payloads.
- Active project HTTP routes now live in a dedicated tested route module.
  Per-agent get/set/clear keep the same `/api/active-project/{agent_id}` path,
  while preserving bounded agent ids, normalized slugs, project existence checks,
  registry-unavailable guidance, and operator-safe errors without echoing input.
- Project lifecycle HTTP route now lives in a dedicated tested route module.
  Lifecycle updates keep the same Projects API path, slug/id lookup, lifecycle
  metadata mutation, `project.lifecycle.updated` event, enriched public project
  response, and operator-safe invalid phase errors.
- Projects environment HTTP route now lives in a dedicated tested route module.
  `/api/projects/environment` keeps GitHub auth readiness and the default
  source type, while preserving the operator-safe response that omits local
  workspace/root paths.
- Project creation HTTP route now lives in a dedicated tested route module.
  `POST /api/projects` keeps the same Projects API path, lifecycle `observe`
  initialization, public enriched response, normalized typed input, duplicate
  slug conflict response, and operator-safe storage errors.
- Project listing HTTP route now lives in a dedicated tested route module.
  `GET /api/projects` keeps the same Projects API path, `include_archived`
  query behavior, enriched public project list, and operator-safe storage
  errors.
- Project detail HTTP route now lives in a dedicated tested route module.
  `GET /api/projects/{slug}` keeps the same Projects API path and enriched
  public response, while preserving Captain's invalid-slug/not-found guards and
  omitting raw metadata, local paths, and secret-like fields from the view.
- Project archive HTTP route now lives in a dedicated tested route module.
  `POST /api/projects/{id}/archive` keeps the same Projects API path, durable
  archived-state transition, enriched public response, and operator-safe
  invalid-id/not-found/storage errors.
- Project update HTTP route now lives in a dedicated tested route module.
  `PATCH /api/projects/{slug}` keeps the same Projects API path, slug/id lookup,
  normalized typed-field updates, and enriched public response, while preserving
  Captain's rejection of raw metadata patches and operator-safe invalid-input,
  not-found, and storage errors.
- Project delete HTTP route now lives in a dedicated tested route module.
  `DELETE /api/projects/{slug}` keeps the same Projects API path, slug/id
  lookup, linked project-goal cleanup, `project.deleted` event, and
  `{status, project_id, removed_goals}` response, while preserving operator-safe
  invalid-input, not-found, and storage errors.
- Project resume HTTP route now lives in a dedicated tested route module.
  `GET /api/projects/{id}/resume` keeps the same Projects API path and slug/id
  lookup, while preserving Captain's public resume projections for checkpoint,
  tasks, goals, and milestone progress without raw commands, descriptions,
  paths, or internal checkpoint state.
- Project runtime read HTTP route now lives in a dedicated tested route module.
  `GET /api/projects/{id}/runtime` keeps the same Projects API path, slug/id
  lookup, and `?events=` windowing, while preserving Captain's operator-safe
  runtime response without `chat`, raw event payloads, local paths, raw runtime
  metadata, or identifier echoes in lookup errors.
- Project runtime start HTTP route now lives in a dedicated tested route module.
  `POST /api/projects/{id}/runtime/start` keeps the same Projects API path,
  durable runtime mutation, and orchestrator spawn behavior. The extracted route
  keeps Captain's start/restart/resume-pending logic, public runtime response,
  and operator-safe identifier errors while making the pre-spawn response path
  directly testable.
- Project runtime pause HTTP route now lives in a dedicated tested route module.
  `POST /api/projects/{id}/runtime/pause` keeps the same Projects API path,
  durable paused-state mutation, control flags, `project.paused` event, and
  orchestrator deactivation while preserving operator-safe identifier errors.
- Project runtime resume HTTP route now lives in a dedicated tested route module.
  `POST /api/projects/{id}/runtime/resume` keeps the same Projects API path and
  orchestrator spawn behavior, while preserving Captain's pending/stale/tool
  request resume logic, typed resume events, released control flags, public
  runtime response, and operator-safe identifier errors.
- Project runtime takeover HTTP route now lives in a dedicated tested route
  module. `POST /api/projects/{id}/runtime/takeover` keeps the same Projects API
  path, durable paused-state mutation, manual takeover control flags,
  `project.takeover` event, orchestrator stop reason, public runtime response,
  and operator-safe identifier errors.
- Project launch HTTP route now lives in a dedicated tested route module.
  `POST /api/projects/launch` keeps the same Projects API path, input
  normalization before storage, project backbone creation, `project.created`
  publication, public allowlisted launch response, and operator-safe
  workspace/storage errors.
- Project runtime worker support now lives in a dedicated tested helper module.
  Worker launch/finish still receive the same prompts, workspace resolution,
  project goal context, and phase task updates, while these helpers are no
  longer carried by the Projects route surface.
- Project runtime response projection now lives in a dedicated tested helper
  module. Project and runtime responses keep the same public shape and
  transcript window behavior while preserving Captain's protections against raw
  chat ids, metadata, local paths, worker prompts, and event payload leakage.
- Project runtime worker turns now live in a dedicated tested helper module.
  Worker execution still streams through the same project-scoped chat context,
  routes `AskUser` events to project asks, projects worker stream events into
  the runtime timeline, and logs recording failures without aborting the worker.
- Project runtime mutation now lives in a dedicated tested helper. Project
  runtime updates still normalize phase/status, synchronize lifecycle metadata,
  recompute worker parallelism, persist new timeline events to the project
  transcript, refresh active project routing for the manager, and publish the
  same `project.runtime.updated` event.
- Project runtime worker `AskUser` stream handling now lives in a dedicated
  tested helper. Worker questions still register a pending project ask, update
  runtime `user_questions` and timeline with `worker.ask_user`, and broadcast
  the same `ProjectAskUser` chat event for operator response.
- Project runtime worker phase pre-launch now lives in a dedicated tested
  helper. A phase still stops on pause/manual takeover, skips completed
  workers, rejects blocked/failed/running workers with the same operator text,
  recovers stale running workers, and returns the same runtime snapshot before
  spawning the sub-agent turn.
- `captain project checkpoints <project>` now prints a bounded operator-safe
  view of recent durable project checkpoints. The command accepts a project id
  or slug, resolves the canonical id, and omits raw checkpoint `state`, metadata,
  workspace paths, and other free-form payloads while keeping the resume summary
  visible.
- `captain project archive <project>` and
  `captain project unarchive <project>` now close the durable project lifecycle
  loop from the CLI. `unarchive` marks the project active without starting the
  live runtime, and JSON output is sanitized to project identity, status,
  update time, and next action rather than raw metadata or workspace paths.
- The in-process `captain status` fallback now builds those same project action
  endpoints from persisted runtime metadata, so status remains actionable while
  the daemon is old, unavailable, or between versions.
- `captain status --verbose` now also prints the latest project runtime event
  for each `Project Attention` row. The in-process fallback reconstructs that
  `last_event` from the persisted project timeline, so blocked or stale project
  runs can be diagnosed before opening the full project runtime view.
- `POST /api/projects/{id}/runtime/answer` now returns the updated project,
  runtime, and `operator_status` after a project question is answered. If the
  active worker is gone and the answer is persisted for resume, the response
  includes the next `resume_runtime` action immediately instead of requiring a
  manual refresh.
- Project Attention rows are now priority-sorted before the status response is
  bounded. Pending user answers, tool requests, resume-ready projects, and stale
  active runtimes stay visible ahead of lower-priority blocked/failed projects,
  both in `/api/status` and the in-process `captain status` fallback.
- `workload.projects.attention_count` now remains the full number of projects
  needing operator attention even when the `attention` list is bounded to the
  top eight rows. `captain status` also shows how many prioritized rows remain
  hidden.
- `captain status` also computes the hidden Project Attention row count when an
  older daemon returns an unbounded `attention` list without `attention_count`,
  so legacy status responses do not silently hide operator actions after the
  first eight rows.
- Each agent now has a small dedicated external ingress surface. Agent creation
  responses include `agent_api`, `GET /api/agents/{id}/api` describes the
  endpoint and token environment variable, and external services can call
  `POST /hooks/agents/{id}/ingress` with that agent's bearer token to trigger a
  turn on channel `agent_api`.
- Agent API ingress can now emit a signed outbound callback after the agent
  turn. Configure `CAPTAIN_AGENT_API_CALLBACK_URL_{AGENT_UUID}` and
  `CAPTAIN_AGENT_API_CALLBACK_SECRET_{AGENT_UUID}`; Captain posts a bounded JSON
  payload with `x-captain-event`, `x-captain-agent-id`, and HMAC
  `x-captain-signature`. Callback delivery is reported separately as `egress`
  so a delivery failure does not hide the agent result.
- Failed agent API callbacks caused by transient transport errors are now
  queued durably in `agent_api_egress_queue.json`. The daemon drains due items
  automatically, marks exhausted retries as dead letters, and exposes queue
  state through `GET /api/agents/{id}/api/egress`.
- Per-agent API ingress and egress now write structured operational audit
  events without storing bearer tokens, raw messages, or callback bodies.
  `GET /api/agents/{id}/api/events` returns recent API events for that agent,
  and callback queue redeliveries are recorded when the daemon drain runs.
- Per-agent API ingress now treats `request_id` as a durable idempotency key.
  Exact duplicate retries return `status: duplicate` without starting a second
  agent run, while reusing the same `request_id` with a different body returns
  `409`. The bounded store lives in `agent_api_idempotency.json` and keeps
  in-progress/completed/failed status for 24 hours.
- Operators can now retry one queued or dead-lettered agent API callback with
  `POST /api/agents/{id}/api/egress/{queue_id}/retry`. The retry is attempted
  immediately, resets the queue attempt window for that item, writes an audit
  event, and returns `delivered`, `queued`, or `dead_letter`.
- `GET /api/agents/{id}/api` now includes an operator-safe
  `config_status`. It reports ingress readiness, callback URL/secret presence,
  egress queue health, dead letters, and concrete setup actions without
  exposing token, secret, or callback URL values.
- Agent creation responses now also include `agent_api_config_status` next to
  the existing `agent_api` descriptor, so operators can see immediately whether
  the new agent API is ready or which environment variables still need setup.
- Operators can now generate or rotate an agent API ingress bearer token with
  `POST /api/agents/{id}/api/token/rotate`. The token is stored in
  `secrets.env`, injected into the running daemon process, returned once in the
  rotation response, and omitted from normal status responses.
- Operators can configure an agent API outbound callback with
  `POST /api/agents/{id}/api/egress/configure`. Captain validates the callback
  URL, stores the URL and HMAC secret in `secrets.env`, injects them into the
  daemon process, generates a secret when requested, and only returns generated
  secrets in that one configure response.
- Operators can test an agent API outbound callback with
  `POST /api/agents/{id}/api/egress/test`. The diagnostic sends a signed
  `agent_api.test` payload through the configured callback and returns the
  delivery outcome directly without starting an agent run or adding a retry
  queue item.
- External services can now fetch a complete per-agent API contract from
  `GET /api/agents/{id}/api/manifest`. The manifest describes ingress headers,
  request schema, idempotency, callback events, HMAC signing, queue operations,
  limits, and readiness without exposing token, secret, or callback URL values.
- `/api/status` now includes `agent_api.egress_queue`, a global operator view
  of per-agent callback delivery health. `captain status` and `--verbose`
  surface pending, due, dead-lettered, and sanitized recent callback failures
  without exposing payloads, tokens, secrets, or raw callback URLs.
- `/api/status` now also includes `runtime_health`, a compact
  `ok`/`watch`/`warn`/`critical` rollup for the default LLM driver, locked
  channels, project attention, automation delivery, agent API egress, and
  operational consciousness. `captain status` shows the single health line and
  `--verbose` lists the rolled-up issues plus concrete operator actions.
- `/api/status` now exposes `disk` with free space, total size, usage percent,
  and the Captain cleanup threshold. `captain status` always shows the Disk
  line, while `runtime_health` only recommends cleanup when free space is at or
  below the 15 GiB build/debug cleanup threshold.
- The `captain status --json` in-process fallback and CLI compatibility layer
  for older daemons now include the same `disk` and `runtime_health` core
  observability fields, so status remains actionable during daemon upgrades or
  when the daemon is not running.
- Direct `channel_send` deliveries now use the same retryability policy and
  jittered backoff as cron delivery. Text, rich/button messages, media URLs,
  and local file/image sends retry transient channel failures, while ambiguous
  read/write timeouts are returned without retry to avoid duplicate platform
  messages.
- Persistent `process_start` jobs now track `idle_secs` from last stdout/stderr
  activity or stdin write. `process_list`, `/api/status.active_processes`, and
  `captain status` expose the idle signal, and process cleanup reaps old exited
  handles without killing live long-runners based on wall age alone.
- Inbound channel follow-ups are now serialized per channel/chat/user/thread.
  While one agent turn is active, normal follow-up messages are stored as a
  pending turn, rapid text bursts are appended together, and the bridge drains
  that pending turn before releasing the session. Recognized slash commands and
  plain stop/cancel messages bypass the queue so approvals, status checks, and
  cancellations remain immediate.
- The first queued inbound follow-up now sends a short visible acknowledgment
  and subsequent acknowledgments are debounced for 30 seconds per session. This
  keeps mobile channels responsive during long turns without interrupting a
  healthy active run or spamming rapid message bursts.
- Plain text follow-ups can now be interjected into an already active streaming
  agent turn, so the user can add context while Captain is working. Explicit
  slash commands, `@agent` reroutes, and media remain in the normal route/queue
  path to avoid double processing or changing agent target mid-run.
- Late follow-ups arriving after the outbound stream has closed are no longer
  accepted as active interjections. They stay on the inbound session queue and
  run as the next turn, avoiding the post-stream window where a message could be
  accepted into `user_input_rx` after the agent loop had stopped polling it.
- If the active interjection buffer is full, Captain now falls back to the
  inbound next-turn queue instead of accepting a delayed enqueue that could be
  dropped after the channel dispatcher already acknowledged it.
- Active stream interjections are now scoped by the channel session key, not
  only by agent id. Two Telegram chats, users, or forum topics targeting the
  same agent can no longer inject follow-ups into each other's stream.
- Channel follow-ups support explicit control commands inspired by Hermes:
  `/queue <message>` stores the stripped message for the next turn, while
  `/steer <message>` explicitly targets the active stream and falls back to the
  pending queue when the stream cannot accept it.
- Pending inbound channel follow-ups are now persisted in
  `channel_inbound_queue.json` with `.tmp` then atomic rename writes. On bridge
  start, each active channel recovers and drains its own pending messages, so a
  daemon restart no longer silently drops a queued follow-up from a long run.
- Recovered inbound follow-ups now remain in the durable queue as in-flight
  until dispatch completes. If Captain crashes after recovery but before the
  follow-up finishes, the next bridge start retries it instead of losing it.
- Recovered inbound follow-ups that repeatedly fail to finish are moved to a
  durable dead letter after the retry budget is exhausted. Status reports the
  dead-letter counters without exposing chat ids, user ids, thread ids, or
  message text, so operators see that the queue needs attention instead of a
  silent infinite retry loop.
- Inbound queue status now includes the recovery attempt budget and concise
  operator actions. Pending work says to let the active turn drain, in-flight
  recovery explains the retry path, and dead letters tell the operator to review
  logs and ask for a resend without exposing the private message.
- Inbound dead letters are timestamped. `/api/status.channels.inbound_queue`
  exposes the oldest dead-letter age globally and per channel, and `captain
  status` includes that age in its hint, so operators can prioritize stale
  failures without seeing message content.
- Stale inbound dead letters are pruned after 24 hours when the durable queue is
  loaded. The same atomic `.tmp` then rename path rewrites the store, keeping
  `captain status` useful after the operator has fixed the issue or abandoned
  the old message.
- Operators can now clear handled inbound dead letters explicitly with
  `DELETE /api/channels/inbound-queue/dead-letters`, optionally filtered by
  `?channel=telegram`. The response returns only aggregate cleared/remaining
  counts, never chat ids, user ids, thread ids, or message text.
- The same action is available from the CLI with
  `captain channel inbound dead-letters clear --channel telegram`. The plural
  alias `captain channels ...` is accepted, and the API records an audit entry
  with only scope and aggregate cleared/remaining counts.
- `captain stop` no longer force-kills a daemon that is still healthy and
  running agent work. `/api/shutdown` returns `status: "draining"` with an
  active run count when work is in progress, records an audit entry, and asks
  the operator to inspect `captain status` and retry after the run finishes.
- `/restart`, `/shutdown confirm`, `captain service stop`, and
  `captain service restart` now use the same active-work guard. Slash commands
  defer before scheduling shutdown/restart, and service-manager controls ask
  the operator to inspect `captain status` instead of bypassing the guard while
  active work is running.
- Destructive CLI operations that stop the daemon first now respect that
  deferred stop. `snapshot restore`, factory reset, and uninstall abort before
  mutating data if the daemon is still draining or still answers health checks;
  tmux/background service restart also stops instead of starting a second daemon
  after a deferred stop.
- OS shutdown signals now share the active-work drain guard. When the daemon
  receives `SIGINT` or `SIGTERM` while agent work is running, it audits/logs the
  deferred shutdown and waits for the active runs to finish before the HTTP
  server closes bridges and shuts down the kernel.
- Shutdown drain state is now visible in operator status. `/api/status` includes
  a `shutdown` block with idle/draining state, trigger, active run counts, age,
  and operator actions; `captain status` prints a `Shutdown` warning while a
  deferred stop is waiting for healthy agent work to finish. `runtime_health`
  also reports this as `shutdown_draining` with watch severity.
- Shutdown drain now treats live `process_start` background processes as active
  work too. API shutdown, OS signals, channel `/restart` and `/shutdown confirm`,
  `captain stop`, and `captain service stop/restart` defer while a managed
  process is alive, so Captain does not lose the process handle during a
  restart.
- Operators can now unblock such a drain intentionally with
  `captain process list`, `captain process kill <process_id>`, or
  `DELETE /api/processes/{process_id}`. The stop action records only a minimal
  audit entry with the process id.
- `process_start` writes live process metadata to `data/process_registry.json`.
  On boot, Captain recovers still-running PIDs as detached entries, shows them
  as `recovered` in `/api/status`, `captain status`, and `captain process list`,
  and can still kill them even though stdin/stdout are no longer attached.
- Recovered process rows in `/api/status.active_processes` now include
  `operator_actions`, and `captain status` prints a direct hint to use
  `captain process kill <process_id>` when a detached process is still alive.
- Channel `/restart` handling now records the processed `channel +
  platform_message_id` after a restart is successfully scheduled. If the same
  message is re-delivered within 24 hours, Captain ignores it instead of
  scheduling another daemon restart loop after a lost channel acknowledgement.
- The inbound channel queue is now observable without exposing private chat
  data. `/api/status.channels.inbound_queue` reports active sessions, pending
  sessions, pending message count, recovered in-flight retry count, dead-letter
  count, accepted interjections, and per-channel aggregate totals. `captain
  status --verbose` prints the same counters when a channel turn is active,
  queued, recovered for retry, dead-lettered, or has accepted mid-run context.
- `GET /api/channels` now reports operator-facing readiness for the active
  core channels. Telegram, Discord, Signal, and Email include `ready`,
  `missing_required_fields`, `operator_actions`, `security_state`, and setup
  notes. `allowed_users` is treated as required for Telegram/Discord/Signal,
  and `allowed_senders` is treated as required for Email, so credentials alone
  no longer appear ready when the adapter would ignore every sender. The TUI
  channel list honors the new `ready` flag.
- The TUI channel screen fallback no longer advertises the old experimental
  channel matrix when the daemon API is unavailable. It lists only the active
  core channels: Telegram, Discord, Signal, and Email.
- The TUI channel screen implementation is split so `channels.rs` owns state
  and keyboard actions while `channels_draw.rs` owns rendering. Both files stay
  short enough to inspect quickly during channel setup/debug work.
- Channel bridge startup now honors the same active-core policy. Old
  non-core channel sections in `config.toml` are detected and logged as frozen,
  but the normal runtime starts only Telegram, Discord, Signal, and Email.
- Runtime channel tools now enforce the same active-core policy. `channel_send`,
  `channel_delivery_batch`, and `channel_reconfigure` reject non-core channels
  with an actionable error naming Telegram, Discord, Signal, and Email as the active
  set.
- Channel setup now has a tested secret boundary: active channel configure
  routes write tokens/passwords to `secrets.env`, keep only environment
  variable names such as `EMAIL_PASSWORD` in `config.toml`, and reject
  multiline secret values before they can inject extra env entries.
- Discord outbound messages now set `allowed_mentions` explicitly. User
  mentions still work, but `@everyone`, `@here`, and role pings are disabled
  by default for agent-generated content.
- `/api/status` now includes a compact active-channel readiness summary with
  configured, ready, and locked channel lists plus missing fields/actions. The
  `captain status` runtime block reports configured vs ready channels and warns
  when an active channel is configured but still locked.
- `/api/status` now includes an operational `consciousness` summary. It reports
  steady/watch/warn/critical state, queued thoughts, active and escalated goals,
  prediction accuracy, supervisor panics/restarts, concrete signals, and
  operator actions; `captain status` and `captain status --verbose` surface it
  in the runtime view.
- `consciousness` now includes `projects` counters derived from
  `workload.projects.attention`: waiting answers, pending tool requests,
  denied and repeated tool denials, resume-ready runtimes, stale active
  runtimes, blocked phases, and failed phases. These project anomalies affect
  the steady/watch/warn state and operator actions, and `captain status`
  includes the project-attention count in its compact runtime line.
- Agent runs now receive a short `[OPERATIONAL AWARENESS]` prompt block only
  when runtime telemetry is useful: active or escalated goals, supervisor
  shutdown/panics/restarts, high recent error rate, elevated user frustration,
  queued graph thoughts, or low prediction accuracy. Treat it as telemetry for
  the next action, not as personality or a reason to narrate internals.
- The `[OPERATIONAL AWARENESS]` prompt now also receives compact project
  anomaly counters from persisted project runtimes: pending user answers,
  pending tool requests, denied and repeated denied tools, resume-ready runs,
  active/stale markers, blocked phases, and failed phases. This keeps model
  guidance aligned with `/api/status.consciousness` without injecting project
  transcripts or decorative self-description.
- Cron `agent_turn` and cron `workflow_run` timeouts now watch inactivity
  instead of wall-clock runtime. A long active job can keep working as long as
  it emits model/tool/phase stream events; a silent stuck job is cancelled with
  the last activity in the error.
- The default cron agent/workflow inactivity limit is 600 seconds when
  `timeout_secs` is omitted. Explicit `timeout_secs` values still set the
  inactivity/review window and may be raised up to 7200 seconds for planned
  long work.
- `shell_exec` with explicit `timeout_seconds` treats that value as a review
  window. A still-running command emits progress and keeps running instead of
  being killed at the deadline; failed/exited commands still return their exit
  status and output.
- `execute_code` keeps the default 60s hard guard, but an explicit
  `timeout_secs` is now a review window too. A live snippet emits progress and
  continues instead of being killed at the first deadline.
- `docker_exec` keeps the configured Docker timeout as the default hard guard,
  but explicit `timeout_secs` values now behave as renewable review windows. A
  live container command emits progress and keeps running; the result reports
  `timeout_mode` as `review_window` or `hard_timeout`.
- `ssh_exec` now treats explicit `timeout_secs` values as renewable review
  windows for the remote command. Connection/auth/channel setup remain bounded;
  once the command is running, a live SSH channel emits progress and is not cut
  at the first deadline. `ssh_health_check` still uses a short hard guard.
- Structured package wrappers `cargo`, `npm`, and `pip` now accept
  `timeout_seconds` and forward it to `shell_exec` as the same renewable review
  window. Long builds/tests/installs/downloads can stay inside the safer
  wrapper path instead of falling back to raw shell just to avoid the short
  default guard.
- Session compaction handoffs are now framed as reference material, not active
  instructions. The structured summary separates the latest unfinished user
  request, global goal, current state, decisions, user questions, files,
  risks, and remaining work, so a resumed model is less likely to answer old
  compacted questions instead of the latest user message. The injected
  canonical context message now uses a dedicated compaction-reference preface
  that tells the model to treat the handoff as background and prioritize the
  user message that follows it.
- The compaction split now always keeps the latest real user request in the
  uncompressed tail, even when many assistant/tool-result messages follow it.
  Tool-result blocks that use role `user` are ignored as human boundaries, so
  the active task cannot disappear into the summary.
- Cron channel and webhook delivery now retries transient transport failures
  with capped exponential backoff plus jitter, so multiple delayed deliveries
  do not all retry at the same instant after a platform outage or rate-limit.
- Delivery failures are tracked separately from agent/job execution failures.
  A platform outage or webhook 5xx should not consume the cron job's
  consecutive-error budget.
- Cron metadata now exposes `last_delivery_error` and bounded `dead_letters`
  so undelivered outputs can be inspected instead of disappearing into logs.
- Cron delivery failures are queued for later redelivery. Payload bodies are
  stored outside `cron_jobs.json`, retried on cron ticks, and removed after a
  successful delayed delivery.
- Cron metadata now exposes `redelivery_queue` so pending transport retries are
  visible.
- `/api/status.workload.automation.delivery` now aggregates cron delivery
  health: failed jobs, queued redeliveries, due retries, dead letters, and up
  to five sanitized recent error previews. `captain status` warns on Delivery
  issues, and `captain status --verbose` prints the recent cron delivery errors
  without exposing raw webhook URLs.
- Ambiguous read/write timeout sends are not retried automatically to avoid
  duplicate messages on non-idempotent channel sends.

How to answer the user:

- If a cron ran but the user did not receive the output, inspect cron status for
  `last_delivery_error`, `redelivery_queue`, and `dead_letters` before
  recreating the job. `captain status` now shows when those delivery queues
  need attention.
- If a cron reports `timeout`, read the detail: it names the inactivity window
  and the last observed agent/workflow-step activity, which is usually more
  useful than the total runtime.
- For long compiles/tests/installs, set `timeout_seconds` explicitly on
  `shell_exec` or the structured `cargo` / `npm` / `pip` wrapper. Use
  `process_start` for servers/watchers that are meant to run indefinitely.
- Treat `delivery_failed` as a transport problem first. Fix the channel/webhook
  target or credentials before modifying the scheduled prompt.

### 0.1.0-dev.2026-05-18c — Active-core surface gates and lean tool discovery

Captain now keeps frozen/experimental surfaces compiled but out of the active
tool discovery path. This reduces model hesitation and keeps the first-class
agent loop focused on Chat, Projects, Automation, Learning, Capabilities, and
Status.

- **Active surface gates**: `tool_search`, `capability_search`, grouped
  meta-tools, and `captain_docs` Live Tool Schemas omit Hands, A2A, peers, and
  fleets by default.
- **Codex-first default**: the default config and first-run fallback now prefer
  Codex with `gpt-5.5`; other providers remain compatible but no longer guide
  the critical path.
- **Operational awareness prompt**: `consciousness` guidance now describes
  runtime awareness, active goals, anomalies, failures, and loop health instead
  of decorative self-description.
- **Tool runtime split**: definitions, discovery/scoring/errors, execution
  context, streaming, security, cache/finalization, and the main runtime
  handlers now live under `tools/`. Extracted dispatch routers cover
  browser/web/shell/agent/coordination/automation/project/improvement,
  memory/config/channel/media, goal/discovery/knowledge, process,
  file/ssh/package/document, and peer/docker/location/hand/skill-runtime/A2A/canvas.
  `tool_runner.rs` now fits under 500 lines and keeps behavior as orchestration
  glue; agent scope, capability/skill search, channel reconfigure, dispatch
  contracts, error recovery, file/edit/search, SSH/package, document/web,
  image, depth/schedule, canvas, memory save/forget/context, registry/schema
  guidance, security/execute, session/workspace, improvement runtime, and
  `tool_search` regression tests are split into short files under
  `tool_runner/tests/`.
- **CLI command split started**: the `snapshot` and `reset` command family now
  lives in `captain-cli/src/snapshot.rs`; shared daemon discovery, daemon HTTP
  client/auth headers, daemon JSON error handling, and `require_daemon` moved
  into `captain-cli/src/daemon_api.rs`; shared path/permission, browser open,
  prompt, recursive copy, display truncation, and provider API-key test helpers
  moved into `captain-cli/src/cli_support.rs`; CLI version, Captain home
  resolution, tracing setup, Ctrl+C handling, and kernel boot error handling
  moved into `captain-cli/src/cli_runtime.rs`; CLI unit tests moved into
  `captain-cli/src/tests.rs`; shell completion generation moved into
  `commands/completion.rs`; daemon/API command families
  `health`, `security`, `memory`, `devices`, `webhooks`, `message`, and
  `system`, agent spawn/list/chat/kill/set/new, workflow CRUD/run, and trigger
  list/create/delete, daemon start/stop/background launch, terminal launch,
  scaffold handling, and migration handling now live under
  `captain-cli/src/commands/`. Codex auth status, doctor, login OAuth, and model command routing moved into
  `commands/auth.rs`, `commands/models.rs`, and shared
  `commands/model_state.rs`. Operational command families `approvals`, `cron`,
  and `autonomy` also moved under `commands/`. Persisted session CLI handling
  moved into `commands/sessions.rs` with daemon access in `session_api.rs` and
  text/export helpers in `session_text.rs`. Log inspection moved into
  `commands/logs.rs`, with shared structured event parsing/timestamp helpers in
  `commands/log_events.rs` for `autonomy` and session pruning. Native service
  lifecycle handling moved into `commands/service.rs`, `service_runtime.rs`, and
  `service_render.rs`; status rendering moved into `commands/status.rs`,
  `status_verbose.rs`, and `status_workload.rs`; doctor diagnostics moved into
  `commands/doctor.rs` plus focused local, environment, memory, daemon, and
  brand-audit helpers; channel list/setup/test/toggle handling moved into
  `commands/channel.rs`; plain chat routing and CLI stream rendering moved into
  `commands/chat.rs`; skill install/list/doc/remove/search/create handling
  moved into `commands/skill.rs`; Hand CLI management moved into
  `commands/hand.rs`; legacy integration add/remove/list/doc
  handling moved into `commands/integrations.rs`; credential vault commands
  moved into `commands/vault.rs`; config TOML commands, provider secret
  commands, and workspace config reports moved into `commands/config.rs`,
  `config_secrets.rs`, and `config_workspace.rs`; uninstall handling moved into
  `commands/uninstall.rs`; native integration setup/list handling moved into
  `commands/integration.rs`; SSH vault and known-hosts handling moved into
  `commands/ssh.rs`; native voice status/install/test handling moved into
  `commands/voice.rs` and `commands/voice_install.rs`; native embeddings
  status/install handling moved into `commands/embeddings.rs`; `captain init`,
  quick onboarding, desktop/web/chat launch selection, and provider autodetect
  moved into `commands/init.rs`; the `captain setup` wizard and Codex-first
  provider/model setup moved into `commands/setup.rs` and
  `commands/setup_model.rs`, with shared answer/config parsing helpers in
  `commands/setup_support.rs`, access bootstrap in `commands/setup_access.rs`,
  web/deployment surface setup in `commands/setup_surface.rs`, first-run
  profile personalization in `commands/setup_profile.rs`, non-interactive
  integration setup in `commands/setup_integrations.rs`, optional setup
  state/prompting in `commands/setup_options.rs`, and Docker install launch
  helpers in `commands/setup_docker.rs`. The top-level Clap grammar now lives
  in `cli_root.rs`, with command argument families grouped in `cli_args.rs`,
  `cli_args_config.rs`, and `cli_args_ops.rs`. The top-level command dispatcher
  now lives in `commands/dispatch.rs`; `main.rs` is limited to environment
  loading, Clap parsing, tracing/TUI-mode selection, and the dispatcher call.
  TUI chat footer rendering now lives in `tui/screens/chat_footer.rs` with
  focused tests, and the compact footer keeps session cost visible before the
  duplicate model label when horizontal space is tight. TUI chat model-label
  normalization now lives in `tui/screens/chat_model_label.rs` with focused
  tests, so session resume and model switch keep the previous good label when
  daemon metadata only reports `?/?`. TUI input cursor localization and visual
  row calculation now live in `tui/screens/chat_input_layout.rs` with focused
  tests, preserving UTF-8 cursor boundaries and wrapped multiline input height
  while continuing to shrink the legacy chat screen. TUI title status-line
  rendering now lives in `tui/screens/chat_status_line.rs` with focused tests
  for token compaction, cached-token effective input, costs, session totals,
  and streaming spinner visibility. TUI staged-image preview rendering now
  lives in `tui/screens/chat_image_preview.rs` with focused tests for when the
  preview strip is reserved or collapsed. TUI live slash-command picker
  rendering, command hints, prefix filtering, and Tab longest-common-prefix
  completion now live in `tui/screens/chat_slash_picker.rs` with focused
  tests. TUI factual welcome-summary rendering now lives in
  `tui/screens/chat_welcome_summary.rs` with focused tests for deterministic
  row building, orphan-channel diagnostics, compact byte formatting, and narrow
  viewport suppression. TUI quick-action prompt data for approvals and safe
  model switches now lives in `tui/screens/chat_quick_action_prompt.rs` with
  focused tests for approval details, recommendation labels, context summaries,
  and click-zone wrapping. The quick-action modal rendering now lives there too.
  TUI saved-session
  picker rendering now lives in `tui/screens/chat_session_picker.rs` with
  focused tests for compact age formatting, agent-name fallback, message count,
  and token totals. TUI model-picker rendering now lives in
  `tui/screens/chat_model_picker.rs` with focused tests for display-name/id
  selection, name truncation, tier labels, and selected-row scroll windows.
  TUI expanded tool-call rendering now lives in
  `tui/screens/chat_tool_expanded.rs` with focused tests for browser activity
  labels, shell copy badges, and bounded result bodies without stdout/stderr
  streams. TUI tool-call message routing and collapsed rendering now live in
  `tui/screens/chat_tool_message.rs` with focused tests for summary/duration
  output, copy badges, and running-tool delegation into the expanded renderer.
  TUI transcript layout calculations now live in
  `tui/screens/chat_transcript_layout.rs` with focused tests for logo
  bottom-anchoring, scroll-offset clamping, visible tool click-zone
  coordinates, and scroll indicator placement. TUI message-history line
  construction now lives in `tui/screens/chat_transcript_messages.rs` with
  focused tests for user wrapping, system lines, tool click-zone metadata, and
  legacy tool-message fallback. TUI live transcript tail rendering now lives
  in `tui/screens/chat_transcript_live.rs` with focused tests for streaming
  text, thinking/tool spinners, token estimates, final token/cost lines, and
  operator status messages. TUI chat-input rendering now lives in
  `tui/screens/chat_input_render.rs` with focused tests for empty input,
  multiline continuation, slash command splitting, staged-message badges, and
  UTF-8 cursor boundaries. TUI empty transcript rendering and logo rows now
  live in `tui/screens/chat_transcript_empty.rs` with focused tests for narrow
  widths and bottom-anchor padding after the logo. TUI chat screen layout now
  lives in `tui/screens/chat_screen_layout.rs` with focused tests for message,
  separator, preview, input, footer, reasoning split, and separator rows. TUI
  reasoning block rendering now lives in `tui/screens/chat_thinking_block.rs`
  with focused tests for collapsed headers, wrapping, explicit newlines, narrow
  widths, and legacy vertical trimming. TUI quick-action keyboard decisions now
  live in `tui/screens/chat_quick_action_prompt.rs` with focused tests for
  approval shortcuts, model-switch choices, natural-language answers, invalid
  replies, and input-editing keys. TUI picker keyboard decisions now live with
  their overlays in `tui/screens/chat_session_picker.rs` and
  `tui/screens/chat_model_picker.rs`, with focused tests for close,
  navigation, selection, and model-filter editing keys. TUI streaming keyboard
  decisions now live in `tui/screens/chat_keymap.rs`, with focused tests for
  exit, staging, scroll shortcuts, and input editing/navigation keys. TUI
  slash-picker keyboard decisions now live in
  `tui/screens/chat_slash_picker.rs`, with focused tests for navigation,
  cancellation, selection, and normal-input fallthrough. TUI normal-input
  keyboard decisions now also live in `tui/screens/chat_keymap.rs`, with
  focused tests for submit, Shift/Alt+Enter newline, scroll shortcuts, and
  editing/navigation keys. TUI global chat keyboard decisions (`Ctrl+C/D/L/U/W`,
  Tab, and `Ctrl+T/E/M/O`) now live in `tui/screens/chat_keymap.rs`, with
  focused tests for exit/close behavior, readline actions, slash completion,
  reasoning/tool toggles, and model-picker state guards. Applying those global
  actions is now isolated inside `ChatState`, and `Ctrl+M` during streaming is
  an explicit no-op like Hermes, so it cannot fall through into typed input.
  Session/model picker action application is also isolated behind `ChatState`
  helpers, with state tests for navigation, close, model filtering, and model
  selection. Streaming key application is isolated the same way, with state
  tests for queued follow-ups, slash-command suppression while streaming, and
  Esc exit. Slash-picker and normal-input application now also sit behind
  `ChatState` helpers, with state tests for slash selection/cancel, trimmed
  submit, and Shift+Enter newline. The normal-input helper now delegates
  submit, scroll, edit, and navigation effects to shorter helpers, with a state
  test that editing resets slash-picker selection. The transcript render
  coordinator now lives in `tui/screens/chat_transcript_render.rs`, preserving
  the Hermes-style logo, empty state, history, live tail, scroll, and tool-zone
  flow with focused tests for empty and live transcript construction. The
  public chat `draw` entrypoint now delegates to
  `tui/screens/chat_screen_render.rs`, which separates frame, body,
  transcript, and overlays while keeping focused tests around slash/model/
  session/quick-action overlay state. Quick-action application now keeps
  `handle_quick_action_key` short by delegating safe model-switch prompt
  effects to a dedicated helper while preserving the existing approval and
  model-switch tests. Streaming key application now also delegates staging,
  scroll, edit, and navigation effects to short helpers while preserving the
  Hermes-style non-slash staging behavior during an active stream. Markdown
  session export now lives in `tui/screens/chat_markdown_export.rs`, with a
  testable builder for agent filenames, session metadata, messages, and tool
  blocks while keeping the existing `~/.captain/exports` destination. Session
  replay conversion now lives in `tui/screens/chat_session_replay.rs`, with
  tests for persisted roles, tool status restoration, and runtime identity
  replacement. Quick-action prompt construction and rendering are now split
  into short approval/model-switch and modal-line helpers while preserving the
  Hermes-style approval keys, model-switch recommendation labels, and clickable
  choices. Welcome-summary diagnostics now load config/home, orphan channel
  tokens, graph size, active project, rows, and rendering through separate short
  helpers while preserving the Hermes-style factual empty-state signals.
  Expanded tool-call rendering now separates header/duration, streams, result,
  footer, and apply/edit/multi-edit diff helpers, with focused coverage for
  `edit_file` diffs while keeping the Hermes-style bounded output panel. The
  model-picker overlay now also separates popup geometry, frame, search line,
  list rows, and scroll-window construction, with tests for tiny views,
  centering, and selected-row visibility. Footer action hints now use static
  state-specific action specs for approval, model switch, pickers, slash mode,
  streaming, and idle input, with tests for state priority and preserved Hermes
  labels. The chat title/status line now also splits spinner, model/mode,
  elapsed time, last-turn tokens/cost, and session totals into short helpers,
  with focused tests for elapsed labels and cached-token effective input. The
  live slash picker now separates popup geometry, visible-row windowing,
  selected row styling, hints, and padding helpers, with tests for tiny views,
  scroll visibility, and selected-row rendering while preserving Hermes labels.
  Chat input rendering now splits prompt/cursor styles, raw input lines, line
  body rendering, slash-command highlighting, UTF-8 cursor placement, and
  staged-message badges into short helpers, with tests for explicit newlines and
  slash highlighting away from the cursor line. Live transcript rendering now
  also separates streamed markdown text, thinking, active tool, token estimate,
  last-turn usage/cost, and status-message rows, with focused tests for token
  estimate labels and empty usage suppression. Collapsed tool-call rendering
  now separates copy eligibility, summary width, duration fallback, header
  spans, and output rows while preserving Hermes expanded/running delegation,
  with tests for copy width, fallback duration, and empty-output omission.
  The setup Docker image now defaults to the public early-access channel
  `ghcr.io/vivien83/captain-agent-os:alpha` and can be overridden with
  `CAPTAIN_DOCKER_IMAGE`, so Captain neither references a third-party package
  namespace nor silently selects a stable channel that does not exist yet.
  Each new command module stays below 500 lines while `main.rs` continues to
  shrink toward command routing.
- **API route split started**: shared API state now lives in
  `captain-api/src/state.rs`, while liveness/detail health probes and
  Prometheus metrics live in `captain-api/src/health_routes.rs`. Runtime
  status now lives in `captain-api/src/status_routes.rs`; its channel summary
  reports the active core channels only: Telegram, Discord, Signal, and Email.
  Workflow CRUD/run handlers now live in `captain-api/src/workflow_routes.rs`,
  and trigger/file-trigger handlers now live in
  `captain-api/src/trigger_routes.rs`.
  Profile listing and agent mode changes now live in
  `captain-api/src/profile_routes.rs`. Agent templates now live in
  `captain-api/src/template_routes.rs`, and shared memory KV endpoints live in
  `captain-api/src/kv_routes.rs`. Persisted session browsing, session event
  replay, labels, and per-agent multi-session operations now live in
  `captain-api/src/session_routes.rs`. Agent run-control endpoints now live in
  `captain-api/src/agent_control_routes.rs`, and per-agent runtime
  configuration for model switching, tool filters, skill allowlists, and MCP
  server allowlists lives in `captain-api/src/agent_runtime_config_routes.rs`.
  Provider key/test/base-URL endpoints now live in
  `captain-api/src/provider_routes.rs`, with shared `secrets.env`
  compatibility helpers in `captain-api/src/secret_env.rs` for provider,
  channel, and OAuth callers. GitHub Copilot OAuth device-flow routes now live
  in `captain-api/src/provider_oauth_routes.rs`. Prompt-only skill creation
  moved into the existing `captain-api/src/skill_routes.rs` skill API module. Active channel API
  endpoints now live in `captain-api/src/channel_routes.rs`, with the visible
  registry narrowed to Telegram, Discord, Signal, and Email. Channel TOML writes and
  live test delivery are split into short helper files, and non-core channels
  are hidden from the active API surface while they remain frozen in the
  compiled runtime. Attempts to configure/test a known frozen channel now return
  an explicit `410 Gone` response with the active channel list and the product
  reason, instead of looking like an unknown route. Discord and Signal field
  names were checked against Hermes' local adapter contracts before keeping the
  public setup surface small.
  Usage and budget endpoints now live in
  `captain-api/src/usage_budget_routes.rs`, keeping usage summaries,
  per-model/daily breakdowns, global budget updates, and per-agent budget
  status together in one short module. Security status, migration endpoints,
  peer/network status, and the tool catalog list also moved into focused route
  modules: `security_routes.rs`, `migrate_routes.rs`, `peer_routes.rs`, and
  `tool_routes.rs`. Audit recent/verify/repair and the SSE log stream now live
  in `audit_routes.rs`. Config read/reload/raw/template/schema/set endpoints
  now live in `config_routes.rs`, and the active channel schema only advertises
  Telegram, Discord, Signal, and Email. Skill listing, install/uninstall, local skill
  creation, proposals, metrics, and marketplace search are consolidated in
  `skill_routes.rs`. MCP server listing and the HTTP MCP endpoint moved into
  `mcp_routes.rs`. Model catalog/provider/alias/custom/pricing endpoints moved
  into `model_routes.rs`, and local/external A2A endpoints moved into
  `a2a_routes.rs`. Legacy memory-backed schedules and persistent cron jobs are
  split into `schedule_routes.rs` and `cron_routes.rs`. Webhook wake/agent
  entrypoints, agent bindings, and device pairing endpoints now live in
  `webhook_routes.rs`, `binding_routes.rs`, and `pairing_routes.rs`. Agent
  identity file endpoints and upload/attachment serving are split into
  `agent_file_routes.rs` and `upload_routes.rs`. Agent JSON/SSE messaging and
  session view rendering now live in `agent_message_routes.rs` and
  `agent_session_view_routes.rs`. Agent partial update, config hot-update,
  identity, and clone endpoints moved into `agent_update_routes.rs` and
  `agent_config_routes.rs`. Agent spawn moved into `agent_spawn_routes.rs`
  with focused manifest resolution and signature validation helpers. Graph to
  MemPalace one-shot memory migration moved into `memory_migration_routes.rs`.
  Memory and skill-proposal SSE event streaming moved into
  `memory_event_routes.rs`. Agent detail/listing, kill/restart, and fleet
  metrics moved into `agent_lifecycle_routes.rs`. Telegram topic to agent
  mapping endpoints moved into `telegram_topic_routes.rs`. Browser web auth
  login/logout/check endpoints moved into `web_auth_routes.rs`; stateless token
  helpers stay in `session_auth.rs`. STT model get/update endpoints moved into
  `voice_routes.rs`. Graph memory and operational consciousness endpoints moved
  into `consciousness_routes.rs`. Human approval endpoints moved into
  `approval_routes.rs`. Version, workspace add, and shutdown endpoints moved
  into `system_routes.rs`. Agent delivery receipt endpoints moved into
  `agent_delivery_routes.rs`, and the chat command catalog moved into
  `command_routes.rs`. Agent feedback endpoints moved into
  `feedback_routes.rs`. WhatsApp QR gateway handlers moved into
  `whatsapp_routes.rs` as a frozen non-core channel surface. ClawHub handlers
  moved into `clawhub_routes.rs` as a frozen non-core marketplace surface.
  Experimental integration management handlers moved into
  `integration_routes.rs` as a frozen secondary surface. Advanced comms
  handlers moved into `comms_routes.rs` as a frozen secondary surface. Hand
  handlers split into `hand_routes.rs`, `hand_install_routes.rs`, and
  `hand_instance_routes.rs` to keep each file short; `routes.rs` is now a
  short re-export index.
  `routes.rs` keeps temporary re-exports so existing router and module imports
  remain stable during the domain split.
- Runtime compaction now uses a model-aware structured handoff contract from
  `compaction_handoff.rs`: summary size follows the effective context window
  and summary token budget, every compacted summary is normalized into stable
  reprise sections, and fallback summaries stay actionable instead of a single
  opaque sentence.
- Skill routing now follows a short-index/exact-view pattern: `skill_search`
  can return the minimal installed-skill index, and the new core `skill_view`
  loads one exact workflow plus linked `references/`, `templates/`, `scripts/`,
  and `assets/` files on demand with traversal/symlink escape checks.
- `skill_view` now returns a `validation` object for file-backed skills. It
  reports `ok`, `warn`, or `limited`, checks declared runtime entries, detects
  missing support files referenced by the skill text, flags blocked path
  escapes, and gives concrete preflight checks for scripts, env injection, and
  required tools.
- `skill_check` is now available as a deferred skill preflight tool. It checks
  one installed skill by exact name, reuses the file-backed validation signal,
  promotes blocking validation issues to failures, and runs `bash -n` on shell
  runtime entries or bash/sh fenced blocks without executing the skill.
- `skill_view.validation` now points directly to `skill_check` when preflight is
  useful. Scripted skills, bash/sh blocks, executable runtimes, and validation
  warnings set `preflight_recommended=true` and include a ready
  `preflight_tool_call`.
- `skill_execute` now performs the same no-side-effect bash syntax preflight
  before spawning a legacy `.md` capability. Invalid bash returns a structured
  `status:"blocked"` / `is_error:true` result with a next action, and no command
  from that capability is executed.
- Skill duplicate detection now treats linked Markdown as supporting context,
  not standalone skills: files under `references/`, `templates/`, `scripts/`,
  and `assets/` are ignored by `skill_diff`, reducing false duplicate matches
  and keeping the create-vs-refine gate focused on real skill entries.

How to answer the user:

- If asked why tools disappeared, explain that they are frozen from active
  discovery until the Hermes-level core is stable, not deleted from the binary.
- ClawHub/marketplace is frozen noise for now: do not present it as part of
  the core path unless a future product decision reactivates it.
- If a task needs a frozen surface, prefer an active-core route first: direct
  builtin tools, skills, MCP tools, projects, or local sub-agents.

### 0.1.0-dev.2026-05-18b — API workflow precision and skill-first routing

Captain is now more explicit about third-party API, SaaS, DevOps, custom CLI,
OpenAPI, Postman, SDK, and MCP workflows. These are treated as procedural work,
not one-off shell guesses.

- **Skill-first API routing**: the runtime prompt now tells Captain to call
  `skill_search({include_context:true})` before ad-hoc shell/code for external
  API or CLI workflows, unless an exact loaded skill or typed tool already
  covers the task.
- **Spec-before-call discipline**: when official OpenAPI/Postman/docs or CLI
  `--help` are available, Captain should extract required path/query/body
  parameters and option placement before the first endpoint/subcommand call.
  Missing-parameter 4xx errors are diagnostics, not the normal discovery path.
- **Procedural learning**: after a successful non-trivial API/CLI workflow,
  Captain should propose or refine a skill with endpoints, exact commands,
  required parameters, credential handling, safety level, and verification
  steps.
- **Telegram output clarity**: Telegram guidance now favors compact aligned code
  blocks or short bullet cards for metrics/tabular data, instead of markdown
  tables that render poorly on mobile.

How to answer the user:

- If Captain improved after an API/CLI test, explain that the change reduces
  retries by using skills/specs before shell experimentation.
- If a workflow still lacks a provider-specific skill, use `skill_search` first,
  then official docs/specs, then create/refine the skill after a verified run.

### 0.1.0-dev.2026-05-18a — Shell-safe secrets and native voice repair

Captain now treats `~/.captain/secrets.env` as a credential store, not as a
shell profile, and native voice install can repair a previously broken Python
venv. Local semantic embeddings stay enabled in release builds through a native
ONNX Runtime asset installed at setup/update time.

- **No shell warnings from logical secret keys**: the CLI dotenv loader only
  injects shell-safe keys (`NAME=value` with identifier-style names) into
  `std::env`. Entries with logical Captain identifiers are preserved on disk but
  skipped for process env injection.
- **Runtime guard**: `shell_exec` blocks commands that try to `source
  ~/.captain/secrets.env`, `. ~/.captain/secrets.env`, or use `set -a` around
  the file. This prevents noisy shell warnings and avoids leaking the entire
  secret store into arbitrary subprocesses.
- **Agent guidance**: the shell and credential docs now direct Captain to use
  `secret_read`, typed integrations, or skill `[requirements.env_inject]`
  instead of raw shell imports.
- **Native voice upgrade repair**: `captain voice install` now detects an
  existing `~/.captain/native/voice-venv` or `kokoro-venv` without `pip`, tries
  `ensurepip`, and recreates the venv if needed. This fixes upgrades from
  installs that created the venv before `python3-venv` / `python3-pip` were
  available.
- **Telegram long-run control**: exact mobile messages such as `Stop`,
  `annule`, `arrête`, or `/stop` cancel the active Telegram run instead of
  entering the normal interjection queue. Cancelled streams end silently after
  the explicit stop acknowledgement, without leaking `agent loop join error`.
- **Non-blocking interjections**: the streaming user-input queue is larger, and
  Telegram no longer returns `interjection forward failed (channel full)` to the
  user. If the queue is full, Captain now falls back to the normal inbound
  next-turn queue instead of accepting work that could be dropped after the
  dispatcher returns.
- **Less Telegram heartbeat spam**: long-running Telegram streams now show the
  first visible heartbeat after about 2 minutes, then back off to about every 10
  minutes. The heartbeat text reminds the user they can send a follow-up or
  Stop.
- **Useful `captain status` restored**: `/api/status` now exposes active runs,
  project counts, goal counts, cron jobs, triggers, file triggers, native voice
  state, deployment listen/public URL, and recent projects. The CLI status view
  keeps the old daemon/runtime/path/agent sections and adds compact Workload and
  Automation sections.
- **Native local embeddings preserved**: release builds keep the default
  `local-embeddings` feature. ONNX Runtime is loaded dynamically from
  `~/.captain/native/onnxruntime` and the installer runs
  `captain embeddings install --best-effort` automatically. If the runtime is
  missing, Captain falls back to text search instead of crashing, and
  `captain status` shows the embeddings runtime as pending.

How to answer the user:

- If a command warns about keys containing `:` while loading
  `/root/.captain/secrets.env`, explain that the old command sourced the
  credential store as if it were a shell file.
- Retry with `secret_read` / native integration / skill env injection. Do not
  ask the user to manually edit `secrets.env` unless the file is corrupt.
- If `captain voice install` says `No module named pip` inside
  `voice-venv/bin/python`, install or reinstall this runtime and rerun
  `captain voice install`; the corrected installer should repair the venv
  without manual deletion.
- If semantic memory appears to fall back to text search, check
  `captain status` or `captain embeddings status`. If native embeddings are
  pending, run `captain embeddings install` or reinstall with the default
  `CAPTAIN_EMBEDDINGS_INSTALL=1`.
- If a user sends `Stop` during a long Telegram run, answer that the run was
  cancelled. Do not treat it as a normal prompt or mention internal channel
  capacity.
- If the user asks what Captain is doing now, prefer `captain status` before
  lower-level logs: it is now the concise operational snapshot again.

### 0.1.0-dev.2026-05-17h — VPS native voice dependency bootstrap

The VPS installer now prepares the system dependencies required by native
STT/TTS before running `captain voice install`.

- **Debian/Ubuntu fix**: `install.sh` detects missing Python venv/ensurepip
  support and installs `python3-venv`/`python3-pip` plus native audio runtime
  packages before creating Captain's voice virtualenvs.
- **Native voice default**: the one-command VPS path still installs local
  Whisper + Kokoro/Piper without asking for API keys, but it no longer fails on
  fresh Debian systems that ship Python without `ensurepip`.
- **Current failure mode**: if a user saw
  `The virtual environment was not created successfully because ensurepip is not available`,
  the corrected installer is the fix; on the already-installed host, installing
  `python3-venv` and rerunning `captain voice install` repairs the voice pack.

How to answer the user:

- If native voice reports `STT: non` / `TTS: non` after a Debian VPS install,
  explain that the LLM/core install can still be healthy, but the local voice
  pack needs Python venv support.
- Prefer rerunning the corrected local installer. For an immediate manual
  repair on Debian, use `apt-get update && apt-get install -y python3-venv
  python3-pip libsndfile1 ffmpeg`, then `captain voice install` and restart
  Captain.

### 0.1.0-dev.2026-05-17g — Web terminal mobile input hardening

The web terminal input bridge was hardened for mobile keyboards, IME input,
accented characters, number rows, and paste.

- **Mobile keyboard fix**: xterm's hidden helper textarea is now configured
  with autocomplete, autocorrect, autocapitalize, and spellcheck disabled so
  mobile browsers do not rewrite the current prompt as a cumulative text value.
- **Paste/IME fix**: Captain detects cumulative text deltas and only forwards
  the real new characters to the PTY. If an IME changes an already-typed
  character, Captain emits the minimal backspace/rewrite sequence instead of
  duplicating the whole line.
- **Bracketed paste guard**: pasted text wrapped by xterm bracketed-paste mode
  is normalized without losing the bracketed-paste markers.
- **Desktop TUI preservation**: this is only in
  `crates/captain-api/static/js/pages/terminal.js`; the classic ratatui desktop
  TUI input path is untouched.

How to answer the user:

- If a web terminal repeats text while typing digits, accents, or pasted text,
  confirm they need at least `0.1.0-dev.2026-05-17g`.
- If the installed version is `17g` but this changelog entry is missing, the
  binary was packaged before the docs were embedded. Rebuild/reinstall a newer
  bundle; do not conclude the runtime behavior is absent from the source.

### 0.1.0-dev.2026-05-17f — Codex-first config, checkpoints, and learning

Captain's default config surface now reflects the product path: Codex
subscription/OAuth first, with API providers documented as alternatives.

- **Default model**: `KernelConfig::default()` now starts from
  `default_model.provider="codex"`, `model="gpt-5.5"`, and empty
  `api_key_env`. Users authenticate through `captain login codex`; OpenAI API
  mode remains provider `openai`, not `codex`.
- **Codex model truth**: runtime background models are not treated as a static
  promise. When provider is `codex`, learning, skills, and checkpoints normalize
  configured model names against the live Codex catalog/cache, then fall back to
  Captain's static Codex defaults if the catalog is temporarily unavailable.
- **Learning defaults**: `[learning].reflection_model` and `[skills].proposer_model`
  default to the Codex background model path. Override `reflection_provider` /
  `reflection_api_key_env` only when the user explicitly wants another provider.
- **Checkpoint config**: session checkpointing is now first-class under
  `[checkpoints]` with `enabled`, `model`, `provider`, `inactivity_secs`,
  `scan_interval_secs`, `per_summary_delay_secs`, `transcript_cap_chars`, and
  `emit_learning_review`.
- **Learning after inactivity**: checkpoints still emit the staged
  OBSERVE → THINK → PLAN → BUILD → EXECUTE → VERIFY → LEARN review by default,
  but this is now configurable through `checkpoints.emit_learning_review`.
- **Docs parity**: `docs/configuration.md`, `captain.toml.example`, and
  `config-secret` docs document the new config surface so Captain can safely
  read/write it without guessing.

How to answer the user:

- If asked what model Captain uses by default, answer Codex OAuth
  (`codex/gpt-5.5` or `gpt-5.5` under provider `codex`), not Anthropic.
- If asked about learning or checkpoints, mention that their configured Codex
  model is normalized against the real Codex model list/cache at runtime.
- Do not recommend API keys for the default Captain path unless the user picks
  an API provider.

### 0.1.0-dev.2026-05-17e — Configurable agent loop budget

Long analysis turns can now raise Captain's LLM/tool iteration cap from
`config.toml` without editing code.

- **Global setting**: `[agent_loop].max_iterations` controls the normal turn
  limit. The default remains `90` to protect against runaway loops.
- **Safety clamp**: values are clamped to `1..1000`; use `180` or `240` for
  heavy VPS analysis before going higher.
- **Per-agent precedence**: `AutonomousConfig.max_iterations` still overrides
  the global setting for explicit autonomous agents.

### 0.1.0-dev.2026-05-17d — Web Crons surface

Captain's web workbench now exposes native cron jobs as their own operational
surface instead of hiding them inside Triggers.

- **New `/crons` page**: the web navigation includes a dedicated Crons tab,
  with FR/EN labels, page-scoped metrics and a page-specific sparkline.
- **Real CRUD wiring**: the page lists `/api/cron/jobs`, creates native cron
  jobs, runs a job immediately, toggles enabled state, deletes jobs, and edits a
  job in place through `PUT /api/cron/jobs/{id}`.
- **Update parity with runtime tools**: the web API now routes cron edits to the
  same kernel `cron_update` path used by agent tools, so edits preserve job id,
  owner, created_at, last_run and run history instead of delete/recreate.
- **Operational visibility**: cron rows show schedule, action, delivery,
  next/last run and consecutive failures; status can be opened from the page.

How to answer the user:

- For recurring jobs, point users to `/crons` rather than `/triggers`.
- If a cron must be modified, use `cron_update` or `PUT /api/cron/jobs/{id}`;
  do not cancel/create unless the user explicitly wants a new job.

### 0.1.0-dev.2026-05-17c — Native voice pack by default

Captain now treats speech as a native runtime capability instead of an
API-key-only add-on.

- **No-key STT/TTS install path**: `captain voice install` provisions a local
  voice pack under `~/.captain/native` and `~/.captain/models`: whisper.cpp
  small for STT, Kokoro for premium local TTS when installable, and Piper as the
  guaranteed fallback.
- **Installer/update behavior**: `install.sh` runs `captain voice install
  --best-effort` automatically after setup. Existing installs are treated as
  updates: the voice pack is added, `media.audio_provider="local-whisper"` is
  set, and `[tts].provider="local-native"` is enabled without asking an extra
  question.
- **Runtime awareness**: the system prompt now includes native voice truth when
  local voice is configured. Captain should call `speech_to_text`,
  `text_to_speech`, and `channel_send` directly for voice work instead of
  spending a turn rediscovering those tools.
- **CORE voice tools**: `speech_to_text`, `text_to_speech`, and `channel_send`
  are in the always-visible CORE set so Telegram voice turns can be handled
  immediately.
- **Operational visibility**: `/api/status`, `captain status`, and `captain
  voice status --json` expose native voice readiness, effective STT provider,
  model paths, TTS engine, and install hints.
- **Self-test**: `captain voice test` synthesizes a local WAV, transcribes it
  through local whisper.cpp, and reports the transcript/provider pair.

How to answer the user:

- If Telegram audio says no STT provider is configured, check `captain voice
  status` first. A healthy install should show `local-whisper` with
  `whisper-small`.
- If Kokoro is unavailable on a host, this is not a fatal voice failure when
  Piper is ready. Captain should report `local-native` via Piper fallback and
  continue.
- For explicit voice requests, do not recommend OpenAI/Groq/ElevenLabs keys
  unless the user asks for cloud quality or native voice is not installed.

### 0.1.0-dev.2026-05-17b — SSH chat slash-command robustness

Captain keeps the scrollable ratatui `captain chat` surface and now routes
slash commands consistently between standalone SSH chat and the full desktop
TUI.

- **Robust `/...` parsing**: `captain chat` and `captain tui` normalize
  slash-command input before dispatching it, including extra spaces,
  tab-separated arguments, and invisible/control characters that can appear
  after mobile paste or SSH clients.
- **Standalone chat command coverage**: common operational commands no longer
  fall through to `unknown command` in SSH chat. `/sessions`, `/tasks`,
  `/agents`, `/tokens`, `/cost`, `/retry`, `/undo`, `/queue`, `/history`,
  `/export`, `/top`, `/bottom`, and daemon commands now have explicit handling.
- **Full-TUI navigation clarity**: commands such as `/projects`, `/memory`,
  `/learning`, `/skills`, `/logs`, and `/settings` now explain that they belong
  to `captain tui` instead of looking broken inside focused SSH chat.
- **Language-aware system replies**: standalone chat help, status, empty-state,
  retry/undo/queue, kill protection, and unknown-command messages now follow
  the configured UI language.
- **Captain protection parity**: standalone `/kill` now protects the primary
  `Captain` agent the same way as the full TUI.
- **Distribution version visibility**: maintainer builds can inject the bundle
  version into `captain --version` and `captain system version`, so VPS tests can
  distinguish `0.1.0-dev.*` bundles instead of seeing only the Cargo crate
  version.

How to answer the user:

- If `/...` returns unknown in `captain chat`, first confirm the installed
  version is at least `0.1.0-dev.2026-05-17b`.
- For page/screen commands in SSH chat, tell the user to open `captain tui`;
  focused `captain chat` intentionally stays on the chat surface.

### 0.1.0-dev.2026-05-17a — TUI chat scrollback

Captain chat keeps the ratatui TUI as the normal CLI chat surface and now treats
history navigation as an in-app scrollback feature.

- **Standalone `captain chat` TUI scrollback**: mouse/touchpad wheel scrolling
  is enabled by default for standalone chat so alternate-screen terminals,
  SSH clients, and web terminals can scroll older messages without falling back
  to a plain line chat. `CAPTAIN_TUI_MOUSE=0` or `/mouse off` restores native
  terminal selection/copy.
- **Keyboard scroll controls**: chat history supports `PgUp/PgDn`,
  `Ctrl+B/Ctrl+F`, and `Up/Down` when the draft cursor cannot move inside a
  multi-line input. `/top` jumps to the oldest visible history and `/bottom`
  returns to the live end of the conversation.
- **Operator handoff behavior**: when the user sends a new message after
  reading older history, the TUI returns to the live bottom. Background
  agent/system/tool messages preserve the manual viewport while the user is
  reading scrollback.
- **Discoverability**: `/help` and the chat footer now advertise the scrollback
  controls instead of implying that terminal-native scrollback is the primary
  answer.

How to answer the user:

- If a user cannot scroll inside `captain chat`, first mention `PgUp/PgDn`,
  mouse/touchpad scrolling, `/top`, and `/bottom`.
- If they need native copy/selection, tell them to run `/mouse off`; if they
  need TUI wheel scrolling again, run `/mouse on`.

### 0.1.0-dev.2026-05-16d — LLM readiness visible during install and status

Captain now distinguishes "daemon reachable" from "agent actually able to
answer with a configured LLM provider".

- **LLM readiness in daemon status**: `/api/status` exposes
  `llm_driver_ready` and `llm_driver_error` for the currently effective default
  provider, including hot model/provider changes after boot. If all providers
  fail at boot, the daemon still starts for recovery, but status reports that
  agents are not ready instead of hiding the stub-driver fallback behind a green
  health check.
- **Richer CLI status by default**: `captain status` again prints the provider,
  model, LLM readiness, auth mode, configured channels, TTS/media summary, and
  operational paths without requiring `--verbose`. `--verbose` remains for
  deeper service/runtime detail.
- **Systemd Codex OAuth robustness**: generated Linux services now set
  `HOME`, `CODEX_HOME`, and `CAPTAIN_HOME` explicitly so Codex OAuth tokens
  written during setup are visible to the daemon after a systemd restart.
- **Installer readiness gate**: after starting/restarting the Linux service,
  the installer checks LLM readiness in `captain status --json`. A daemon that
  is merely reachable but unable to initialize its provider now fails the
  install with the provider error and a concrete recovery command.
- **Linux bundle dependency guard**: the channel crate now explicitly activates
  `openssl-sys/vendored` for SMTP/IMAP native-tls builds, preventing Linux
  packaging from silently depending on a host `libssl-dev`/`openssl.pc` setup.
- **Atomic binary reinstall**: Linux/macOS installs now copy the new CLI to a
  temporary file in the install directory and replace `captain` with `mv -f`.
  This avoids `Text file busy` when reinstalling while an older Captain daemon
  is still running from `/usr/local/bin/captain`.
- **VPS installer systemd hardening**: `CAPTAIN_INSTALL_SERVICE=0` now really
  disables service installation, and the installer only writes/starts a systemd
  service when systemd is actually active. This keeps local/container smoke
  installs from failing just because a `systemctl` binary exists without a
  running systemd manager.
- **Private daemon API key storage**: fresh setup no longer writes the generated
  `captain_api_*` bearer token in `config.toml`. It stores
  `CAPTAIN_DAEMON_API_KEY` in `~/.captain/secrets.env` with private file
  permissions, while the daemon, CLI, web/session auth and installer smoke tests
  resolve it through the secret chain.
- **Direct VPS web terminal access**: when a VPS install has no public domain,
  setup binds the web/API surface to `0.0.0.0:50051` and the installer prints a
  direct `http://<IP_DU_VPS>:50051/terminal` access URL. When a domain is
  configured, Captain keeps the safer loopback + reverse-proxy path.
- **First-turn language contract**: the configured `language` is injected into
  every Captain prompt, including the lean direct path, so the first response
  already follows the user's configured language unless the user explicitly asks
  for another language.
- **Native VPS execution context**: prompts now receive the install profile.
  With `deployment.profile = "vps"`, Captain treats the VPS as its local
  execution environment and checks the host with `shell_exec` before reaching
  for SSH/vault. SSH remains the right rail for a different remote host or an
  explicitly named SSH alias.
- **SSH-friendly plain chat**: `captain chat` now defaults to a scrollback-safe
  plain terminal mode when launched over SSH. Set `CAPTAIN_CHAT_TUI=1` to force
  the ratatui chat surface.
- **Telegram formatting polish**: the channel formatter now targets Telegram's
  official HTML parse mode more deliberately: fenced code can keep a language,
  inline code is protected from Markdown conversion, task lists become clear
  checkboxes, spoilers/underline/strike are rendered with Telegram-supported
  tags, and Markdown tables are converted to compact monospaced blocks.
- **Principal agent model reconciliation**: on boot, the persisted `captain`
  agent is reconciled with `[default_model]` and stale provider fields
  (`api_key_env`, `base_url`, routing, fallbacks) are refreshed. Bundled
  template-style `agents/*/agent.toml` files are no longer treated as runtime
  manifests, preventing a VPS reinstall from accidentally restoring the Rust
  default `anthropic/claude-sonnet-4-20250514` while status says
  `codex/gpt-5.5`.

How to answer the user:

- Treat `/api/health` as process health only. For real usability, check
  `captain status` or `/api/status.llm_driver_ready`.
- If Telegram or chat reports "No LLM provider configured", inspect
  `llm_driver_error` first; do not ask the user to run many unrelated checks.
- If `captain status` shows a provider/model mismatch between the global
  provider and the active `captain` agent after an upgrade, restart the daemon
  once with this build; the boot repair should converge the persisted agent.

### 0.1.0-dev.2026-05-16c — Approved skills reload immediately

Skill approval now updates the active runtime registry instead of waiting for a
daemon restart.

- **Immediate generated-skill availability**: approving a skill proposal writes
  the generated `SKILL.md`, marks the proposal as written, and reloads the
  in-memory skill registry in the running daemon.
- **Family discovery stays live**: generated skills tagged with
  `family:<id>` are discoverable by `skill_search` in the same runtime session
  that approved them.
- **Local CLI search**: `captain skill search <query>` now searches installed,
  generated, and bundled skills locally by query/family. It no longer calls the
  remote marketplace search endpoint for procedural discovery.
- **Less misleading exact-name search**: slug-like searches remain exact enough
  to return "no skills found" after a generated skill is removed, instead of
  falling back to broad partial matches.
- **Reproducible Linux packaging**: the channel crate now activates the
  workspace OpenSSL dependency with the `vendored` feature so native-tls/IMAP
  builds work in Linux cross packaging without relying on host `libssl-dev`.
- **Self-contained VPS installer**: release folders now include `install.sh`,
  `install-local.sh`, and `install-git.sh`. When a Linux bundle sits next to
  `install.sh`, the installer auto-selects the local archive, verifies its
  checksum, uses the VPS profile for root/systemd installs, and installs the
  CLI into `/usr/local/bin` so `captain` is available immediately. Existing
  systemd services are restarted during reinstall so the running daemon uses the
  newly installed binary.
- **Codex setup default**: `captain login codex` now persists OAuth tokens and
  writes `codex/gpt-5.5` as the default model automatically. If the Codex model
  catalogue is temporarily unavailable, Captain keeps `gpt-5.5` without showing
  the raw backend error as an installation blocker.
- **Partial install resume**: rerunning `captain setup` now reuses existing
  provider, identity, language, timezone, Telegram, STT, and TTS values as
  defaults. Complete optional integrations are detected and skipped by default;
  partially configured integrations are explicitly reproposed so the user can
  finish them. Personalization is persisted before optional channel/voice setup
  so a later interruption still resumes with the user's previous answers.
- **Optional integration resilience**: Telegram/STT/TTS setup failures during
  the first-run wizard no longer abort the whole installation. The strict
  `captain integration setup <name>` command still exits on invalid
  credentials, but `captain setup` warns, keeps the valid steps already written,
  and continues.
- **Channel status fallback**: runtimes that do not implement the full daemon
  command surface still return basic `/status` and `/health` uptime/agent info
  instead of the generic "daemon command handling unavailable" message.

How to answer the user:

- If a user approves a generated skill, Captain may use `skill_search`
  immediately without asking for a daemon restart.
- Use `captain skill search review-release` or
  `captain skill search development-planning` as a quick local smoke for
  bundled/generated skill discovery.
- Marketplace installation is still separate from procedural discovery.
- macOS arm64 and Linux x86_64 bundles for this version are produced under
  `dist/releases/0.1.0-dev.2026-05-16c/`; copy those files to the VPS release
  mirror before publishing download links.
- For a manual VPS upload, copy the Linux bundle plus `install.sh`,
  `install-local.sh`, `install-git.sh`, `.sha256`, and manifests, then run
  `bash install-local.sh` from the release directory.

### 0.1.0-dev.2026-05-16b — Skill proposals require concrete reusable evidence

Captain now blocks low-signal generated skill proposals before they reach the
user.

- **No more zero-evidence proposals**: proposals need either an observed
  tool/step sequence or concrete procedural evidence in the summary, such as
  numbered steps from a documented API endpoint, CLI workflow, debugging path,
  project convention, or recovery procedure.
- **Underspecified copy is refused**: descriptions or trigger hints that are too
  thin to explain a reusable workflow are rejected before consuming the daily
  proposal slot.
- **Clearer Telegram card**: the skill proposal card now says what Captain
  observed, lists numbered observed steps/tools when they exist, and tells the
  user to approve only if the workflow is clear and reusable. Empty automatic
  traces no longer render as "0 steps" in French or English.
- **Hermes-style procedural split**: facts, preferences, API/CLI discoveries,
  and tool quirks belong in declarative memory; reusable procedures belong in
  skills or skill refinements, with an existing-skill search before creating a
  duplicate.

How to answer the user:

- Captain should not ask the user to approve a skill when it only captured a
  vague idea or a single validation note.
- A future skill proposal should explain the reusable purpose, when it will be
  used, why Captain thinks it is reusable, the family, and concrete observed
  steps or documented procedural evidence in the user's language.

### 0.1.0-dev.2026-05-16a — Project goals are editable

Project-scoped goals are now a real CRUD surface instead of create/pause/delete
only.

- **Goal edit API**: `PATCH /api/projects/{id}/goals/{goal_id}` updates the
  project goal name, description, check command, recovery command, interval,
  LLM cap, and escalation threshold through the same validation rules as goal
  creation.
- **Web edit action**: the Projects page now exposes `Modifier` beside each
  project goal in both detail views, then refreshes the project context after
  saving.
- **Safer check edits**: changing a check command resets the consecutive-fail
  counter and last check timestamp so stale failures do not describe the new
  guard.
- **Workspace-scoped checks**: project goal checks now execute from the
  recorded project workspace, so relative commands behave like they do in
  Codex/Claude Code style project sessions instead of depending on the daemon
  working directory.
- **Reactivate without restart**: when a paused or escalated project goal is
  corrected back to `active`, Captain starts a fresh goal loop immediately so
  the guard resumes after the edit.

How to answer the user:

- Project goals are visible and editable from the Projects web surface.
- Invalid or dangerous check/recovery commands are refused by the normal goal
  validation path instead of being persisted.
- Relative project goal checks run from the project workspace.

### 0.1.0-dev.2026-05-15h — Project runtime uses project goals as gates

Autonomous project workers now receive the project goals and check commands in
their runtime prompt instead of treating goals as dashboard-only metadata.

- **Goals are part of the worker context**: every runtime worker sees the
  project goal list, status, descriptions, check commands, recovery commands,
  and recent check state when available.
- **Verification is goal-gated**: the `VERIFY` phase is instructed that it is
  not complete until active project goal check commands pass, or the exact
  failed command/result is recorded as a blocker.
- **Build/execute align with checks**: `BUILD` is nudged to produce entrypoints
  compatible with registered checks, and `EXECUTE` is told to run or rehearse
  safe local goal checks.

How to answer the user:

- Goals are no longer only visual project metadata. They are injected into the
  autonomous project runtime and should constrain what Captain builds and
  verifies.
- If a goal check fails, Captain should surface that as a blocker instead of
  declaring the project complete.

### 0.1.0-dev.2026-05-15g — Web terminal durable session resume

The web terminal session drawer now distinguishes live PTY sessions from
durable Captain chat history.

- **No false empty resume**: old browser-only `web-*` terminal ids that no
  longer have a server replay are marked unavailable instead of opening a
  misleading blank chat.
- **Persisted history opens in chat**: durable agent sessions are listed in the
  web drawer. Opening one passes the persisted session id into `captain chat`,
  switches the backend agent to that session, and renders the saved public
  transcript before new input.
- **Mobile scrollback hardened**: the terminal canvas uses a stable full-height
  xterm viewport plus touch/pointer scroll paging so mobile users can reach
  older terminal output.
- **Tool/activity details expand in web**: the right-hand activity rail keeps
  full safe detail text and toggles expanded/collapsed state on click.
- **Live PTYs can be closed from the drawer**: live browser terminal sessions
  now expose a direct terminate action so stale reconnectable PTYs cannot fill
  the web terminal session quota and trap the user.

How to answer the user:

- If an old browser-only session cannot be replayed, be explicit: it was a
  transient PTY id, not durable chat history. Durable histories are the UUID
  entries in the session drawer.
- This change is web-terminal scoped. The native desktop TUI still uses its
  existing session picker and mouse behavior.
- If the browser reports a session limit, use the terminate action on old live
  terminal rows instead of asking the user to restart Captain.

### 0.1.0-dev.2026-05-15f — Security audit dependency tightening

The release dependency graph has been tightened after the GitHub security audit
reported a new RustSec advisory and yanked crates.

- **Runtime rand patched**: Captain's active runtime dependency graph now uses
  patched `rand` versions `0.8.6` and `0.9.4` for the affected 0.8/0.9 lines.
- **AVIF transitive chain removed**: local text embeddings no longer enable
  `fastembed` image-model features, removing the unused AVIF encoder chain and
  the yanked `core2` dependency from `Cargo.lock`.
- **Windows UDS updated**: `uds_windows` is updated from `1.2.0` to `1.2.1`.
- **Audit ignore narrowed**: the remaining `RUSTSEC-2026-0097` ignore is
  documented for a Tauri build-time-only `selectors -> phf_codegen 0.8 ->
  phf_generator 0.8 -> rand 0.7.3` path. That generator uses
  `SmallRng::seed_from_u64`, not `thread_rng`/`rand::rng`.

How to answer the user:

- Do not present this as hiding a vulnerability. The runtime rand paths were
  patched and an unused transitive image chain was removed.
- The remaining ignore is a documented Tauri build-time exception that should
  be removed once upstream Tauri/kuchikiki/selectors stop pulling
  `phf_generator 0.8`.

### 0.1.0-dev.2026-05-15e — Explicit model fallback visibility

Model fallback can no longer be silent when a streaming turn moves away from
the primary model.

- **Fallback notices are explicit**: when a fallback hop occurs, Captain emits a
  `model_fallback` phase with the visible fallback target label, localized
  reason class, and UTC timestamp.
- **Channels surface fallback**: Telegram/Discord-style stream consumers map
  fallback phases to commentary instead of dropping them; the desktop TUI also
  shows the notice in the chat transcript.
- **Unsafe fallback still refused**: authentication, missing-key, model-not-
  found, and request-contract errors do not fall through to another model.

How to answer the user:

- If a fallback appears, state the exact visible notice in the user's language:
  target provider/model, reason class, and timestamp. Do not imply Captain
  changed identity; describe it as a temporary continuity hop for that turn.
- If no `model_fallback` event exists, do not invent a fallback explanation.

### 0.1.0-dev.2026-05-15d — Conservative web terminal mobile input

The web terminal input normalizer no longer invents backspaces from plain text
events emitted by mobile keyboards or paste buffers.

- **Mobile cumulative text remains de-duplicated**: when a mobile browser sends
  the whole current helper-textarea value repeatedly, Captain forwards only the
  new suffix.
- **No inferred destructive edits**: if a later plain-text event is shorter
  than the remembered line or looks like a replacement, Captain does not send
  synthetic delete characters. Only real terminal Backspace/Delete control
  events can remove text.
- **Paste safety preserved**: repeated paste echoes are ignored instead of being
  replayed into the PTY.

How to answer the user:

- The web terminal favors not corrupting user input over trying to mirror every
  mobile autocorrect replacement. If a phone keyboard sends ambiguous plain
  text, Captain avoids destructive edits and waits for explicit terminal
  control input.

### 0.1.0-dev.2026-05-15c — Durable project transcript replay

Completed projects can now be reopened with their project chat transcript
instead of relying only on the short runtime timeline preview.

- **Project runtime transcript added**: project runtime updates append their
  new timeline events to the existing `sessions_events` store under the
  project's stable `project-<slug>` session id.
- **Runtime response includes transcript**: `GET /api/projects/{id}/runtime`
  now returns a `transcript` object with up to 10,000 ordered project events,
  while keeping `runtime.timeline` as the lightweight preview stored in project
  metadata.
- **Web project chat replays all returned events**: the projects page now
  renders the full `transcript.events` list when available and no longer cuts
  the feed to the last 100 messages.
- **Legacy fallback preserved**: older projects that do not yet have persisted
  transcript rows still show the remaining `runtime.timeline` entries.

How to answer the user:

- A finished project can be reopened from the Projects page; Captain reloads
  `/runtime` and rebuilds the project chat from the durable transcript.
- For projects created before this change, only the timeline entries still
  present in project metadata can be recovered; new project runs persist the
  full operational transcript going forward.

### 0.1.0-dev.2026-05-15b — Skill families, project context capsule, and model-aware compaction

Captain now treats procedural skills as a family-indexed capability surface,
and long-running project sessions get stronger context continuity.

- **`skill_search` added**: Captain can discover relevant SKILL.md workflows by
  query and family (`software-development`, `project-management`,
  `review-release`, `platform-devops`, `data-ai`, `product-design`,
  `business-tools`, `security-compliance`, `general-automation`) without
  guessing which skill exists.
- **Generated skills carry family metadata**: skill proposals now persist a
  normalized `family`, write it into generated SKILL.md frontmatter, and add a
  `family:<id>` tag. Generated `.md` skills under `skills/generated` are loaded
  by the registry so `skill_search` can find them after reload/restart.
- **Telegram skill proposals are readable**: routed proposals now explain in
  natural French what the skill will do, when Captain will use it, why Captain
  proposed it, which family it belongs to, the observed tool sequence, the
  confidence, and the approval id. Inline approve/reject buttons keep working.
- **Project context capsule**: active project prompts include the latest
  checkpoint, active/blocked tasks, next actions, goals, milestone state, and
  project rules from `CAPTAIN.md`.
- **Default `CAPTAIN.md` seeded**: new/opened project workspaces receive a
  native Captain rules file with the `OBSERVE -> THINK -> PLAN -> BUILD ->
  EXECUTE -> VERIFY -> LEARN` development loop and sub-agent rules.
- **Compaction follows the active model window**: session compaction decisions
  use the routed model's real context window when available instead of relying
  on a fixed default.

How to answer the user:

- For procedural discovery, call `skill_search` before recreating an existing
  workflow or saying that no skill exists.
- Explain that generated skills keep their existing file location, but family
  metadata makes them discoverable and prevents a flat unstructured skill pile.
- For Telegram proposals, summarize the purpose and rationale in the user's
  language; do not expose raw English trigger text when Captain is configured in
  French.

### 0.1.0-dev.2026-05-15a — Telegram inbound video robustness

Telegram inbound media handling now preserves actionable download failures and
routes more video shapes through the native video-analysis path.

- **Telegram getFile errors are surfaced**: when Telegram refuses a file
  download, Captain keeps the Bot API error description in the channel prompt
  instead of collapsing it to a generic "download failed" marker. Bot tokens are
  not included in the fallback text.
- **Video documents are treated as video**: documents with `video/*` MIME types
  or common video extensions (`.mp4`, `.mov`, `.webm`, etc.) now become
  `ChannelContent::Video`, so downstream Telegram handling can download them and
  offer `video_analyze` instead of treating them as opaque files.
- **Regression coverage added**: parser tests cover oversized Telegram videos,
  videos sent as documents, and video-document download failures.

How to answer the user:

- If a Telegram video cannot be downloaded, explain the concrete Telegram
  reason when available, for example that the platform refused the file, instead
  of saying only that Captain cannot download it.
- If the video was sent "as file", Captain should still try the video analysis
  flow when the MIME type or extension identifies it as video.

### 0.1.0-dev.2026-05-14h — Memory learning hardening and project deadline alerts

Captain's memory/learning runtime has been hardened so the documented product
contract matches the real daemon behavior.

- **Generated skill path fixed**: approved skill proposals now write directly
  into the configured `[skills] generated_dir`; with the default config the
  file lands at `~/.captain/skills/generated/<name>.md`, not in a nested
  `generated/generated` folder.
- **Auto-learning PII parity**: the async reflection pipeline now applies the
  same PII filter as `memory_save` before a candidate can become durable
  memory.
- **Stronger memory de-duplication**: reflection-generated facts are compared
  against recent `memory_writes` rows with normalized subject/predicate checks,
  object similarity, and salient-token overlap to reduce repeated preference or
  workflow facts.
- **Project milestone alerts wired**: the kernel now starts a deadline-alert
  loop. Active project milestones due within 24 hours are sent once to the
  configured Telegram `default_chat_id`; delivery is marked in structured
  memory only after a successful send.

How to answer the user:

- Treat these as robustness fixes, not new marketing-only features.
- If a user asks where approved generated skills are written, answer with the
  configured `generated_dir` root and the default
  `~/.captain/skills/generated/<name>.md`.
- If Telegram is not configured or inactive, milestone alerts are retried later
  and are not marked as sent.

### 0.1.0-dev.2026-05-14g — Terminal-web Captain launch site

Captain now has a standalone launch/download site source that follows the same
visual direction as the embedded web terminal while keeping release binaries on
the VPS.

- **Terminal-web direction**: `site/index.html` is built as a public launch
  terminal with topbar, command palette, session drawer, terminal transcript,
  live activity rail, and statusbar instead of a classic marketing landing page.
- **No crown on public site**: the launch surface intentionally avoids the
  terminal crown/logo and uses the product terminal itself as the visual identity.
- **Agent OS narrative**: the terminal transcript explains Captain as a
  self-hosted Agent OS for projects, subagents, webhooks, skills, memory, voice,
  Telegram, GitHub workspaces, and VPS deployment.
- **VPS release mirror contract**: download links use `/releases/...` next to
  the site. Release files remain ignored by git and must be published on the
  VPS.
- **Deployment rail**: `scripts/build-launch-site.sh`,
  `scripts/deploy-launch-site.sh`, `deploy/captain-site.caddy`, and
  `docs/deployment/launch-site.md` define the static build and Caddy deployment
  path.

How to answer the user:

- Make clear that the site source is in the repo, but production downloads must
  be served from the VPS release mirror.
- If asked about the crown, state that it was intentionally removed from the
  public site and replaced by the terminal-web product surface.

### 0.1.0-dev.2026-05-14f — Cross-platform release bundle packaging override

Release packaging can now package a binary built outside the host platform.

- **Platform override**: `scripts/package-release.sh` accepts
  `CAPTAIN_DIST_PLATFORM`, so a maintainer can package
  `x86_64-unknown-linux-gnu` or `aarch64-unknown-linux-gnu` from a macOS host
  after building the Linux binary in Docker/cross.
- **Binary override**: `CAPTAIN_BIN_PATH` points the packager at the exact
  binary to archive instead of assuming `target/release/captain`.
- **Installer contract unchanged**: the output remains
  `dist/releases/<version>/captain-<platform>.tar.gz`, matching
  `scripts/install.sh` and local VPS smoke installs.
- **Multi-platform manifest**: `manifest.json` now aggregates every packaged
  platform in the version folder, while `manifest-<platform>.json` preserves the
  exact metadata for each bundle.

How to answer the user:

- If GitHub Actions is unavailable, build the Linux binary in a Linux container,
  then run `CAPTAIN_DIST_PLATFORM=x86_64-unknown-linux-gnu
  CAPTAIN_BIN_PATH=<linux-binary> scripts/package-release.sh`.

### 0.1.0-dev.2026-05-14e — Trigger/webhook real smoke tests and page-scoped roadmap sparklines

Captain's trigger and webhook surfaces now have real HTTP integration coverage,
and the roadmap UI exposes page-specific activity sparklines.

- **Inbound webhook trigger smoke**: the integration suite boots a real API
  server, enables `/hooks/wake` with a bearer token, registers an `all` event
  trigger, calls the webhook, verifies `fire_count`, and confirms the emitted
  `webhook.wake` event appears in `/api/events`.
- **Outbound webhook CRUD smoke**: endpoint create/update/delete routes are
  exercised over HTTP, config-file persistence is checked, dry-run delivery is
  validated, and localhost/private delivery stays rejected by the SSRF guard.
- **File trigger smoke**: `/api/file-triggers` is covered through HTTP CRUD and
  a real watched temporary directory write, which must surface as a
  `file.changed` event in the native event stream.
- **Roadmap sparklines**: each roadmap page now renders a dedicated smooth
  sparkline scoped to that page's events: projects, learning, triggers,
  webhooks, system, or embed.

How to answer the user:

- Treat `/api/events` as the first live evidence source when debugging trigger
  and webhook behavior.
- Outbound webhook dry-run is a validation path, not a network delivery. Real
  delivery still requires a public HTTP(S) endpoint because private/local
  targets remain blocked deliberately.
- On roadmap pages, the sparkline is contextual to the current page rather than
  a global activity counter.

### 0.1.0-dev.2026-05-14d — Mandatory sub-agent tool scopes

Sub-agents now have an explicit, inspectable tool surface instead of relying on
implicit profiles.

- **Mandatory allowlists**: project runtime workers and lineaged sub-agents get
  a concrete `tool_allowlist` plus matching `capabilities.tools`. The list is
  stored in agent metadata for project workers as `authorized_tools`.
- **Default discovery tools**: every sub-agent keeps the minimal discovery set
  `capability_search`, `tool_search`, `captain_docs`, and `system_time` even
  when Captain assigns a narrow execution scope.
- **No unrestricted child spawn**: `agent_spawn` now rejects profile-only,
  wildcard, or missing child tool scopes. Captain must declare the child tools
  explicitly with `tool_allowlist` or `capabilities.tools`.
- **Tool-request path**: project workers are instructed to return
  `STATUS: blocked` with `TOOL_REQUEST` and `REASON` when they need a tool
  outside their assigned scope, so Captain can approve or deny the extension
  deliberately.

How to answer the user:

- When delegating, always name the child agent's permitted tools. Do not create
  unrestricted workers.
- If a worker says a tool is missing, treat it as a scoped authorization request,
  not as proof Captain lacks the capability globally.

### 0.1.0-dev.2026-05-14c — Project worker lifecycle cleanup

Completed project runtime workers are now stopped automatically after Captain
persists their result.

- **No worker leaks after success**: when a phase reaches `done`, Captain stores
  the worker summary, usage, iterations, and tool count, then stops the child
  agent so it no longer appears as an active daemon agent.
- **Traceability preserved**: runtime state keeps the worker `agent_id`,
  `summary`, `completed_at`, `cleanup_status`, and `stopped_at` fields, plus a
  `worker.cleaned` timeline event.
- **Diagnostics retained**: blocked or failed workers are not auto-stopped.
  They remain visible so the user can inspect the live agent before deciding
  whether to retry, pause, or take over.
- **Release smoke guard**: `scripts/excellence-smoke.sh` now warns about
  leftover `project-*` agents and fails that condition when
  `CAPTAIN_SMOKE_STRICT_RELEASE=1` is set.

How to answer the user:

- A finished project run should not leave completed worker agents listed as
  active agents. Use the project runtime timeline and worker result fields for
  historical evidence instead of expecting those child agents to stay alive.
- If a `project-*` agent remains active, treat it as either an in-progress run
  or a cleanup issue worth investigating before publication.

### 0.1.0-dev.2026-05-14b — Project Runtime V2 real orchestration

Project runs now execute through a backend orchestrator instead of relying on
the web client to kick off a chat prompt.

- **User-language contract**: Captain's main prompt now treats the latest user
  message language as the response language. The configured preference remains
  the fallback only when the language is ambiguous.
- **Localized skill proposals**: Skill proposal prompts, trigger hints, and
  generated approval text now use the configured user language. Legacy English
  trigger hints already waiting in the queue are localized when listed or sent
  to Telegram/TUI.
- **Real sub-agents**: `/runtime/start` spawns project worker agents for
  OBSERVE, THINK, PLAN, BUILD, EXECUTE, VERIFY, and LEARN. OBSERVE and THINK
  run in parallel; the remaining phases are dependency-gated.
- **Real workspace execution**: runtime workers use the selected project
  workspace as their shell/file cwd without writing Captain identity files into
  the repo.
- **Resume robustness**: completed workers are skipped on resume, stale
  `running` workers are recovered after a process interruption, and
  blocked/failed workers stop the run visibly.
- **Readable worker handoffs**: workers are instructed to end with `STATUS`,
  `SUMMARY`, `CHANGED_FILES`, `VERIFY`, and `NEXT`. If a provider returns raw
  tool transcripts, Captain stores a readable metadata-based fallback summary
  instead of exposing transcript noise in the project UI.
- **Live surfaces**: the web Projects page renders runtime timeline events in
  the project chat and auto-refreshes active runs; the desktop TUI Projects tab
  shows live workers and recent runtime events.

How to answer the user:

- Treat `runtime.workers[*].status`, `agent_id`, and `summary` as the live
  evidence for project progress.
- Do not send a separate kickoff chat prompt after `/runtime/start`; the
  backend orchestrator owns dispatch.
- If a phase is blocked or failed, surface the blocker and ask for a manual
  decision or start a fresh run after the user approves.

### 0.1.0-dev.2026-05-14 — Project Runtime live cockpit

Captain projects now expose a persistent live runtime surface for autonomous
development work.

- **Runtime state API**: projects store `captain.project_runtime.v1` metadata
  with status, lifecycle phase, progress, manager agent, same-provider
  parallelism policy, planned workers, and a bounded operational timeline.
- **Run controls**: `/api/projects/{id}/runtime` reads state; `/runtime/start`,
  `/runtime/pause`, `/runtime/resume`, and `/runtime/takeover` control a run.
- **Web cockpit**: the Projects page is a focused development surface with a
  project rail, live Captain chat, runtime timeline, worker graph, tasks, and
  project goals.
- **TUI parity**: the desktop Projects tab shows runtime status, progress, and
  worker count, with start/pause/resume/takeover shortcuts.

How to answer the user:

- Timeline entries are operational summaries, not hidden chain-of-thought.
  Show decisions, actions, blockers, worker state, and verification results.
- Do not claim a worker completed implementation unless the live chat/tool
  stream or runtime state recorded it. Planned workers are orchestration state.
- Development projects must keep following OBSERVE -> THINK -> PLAN -> BUILD
  -> EXECUTE -> VERIFY -> LEARN, with useful tasks/goals/checkpoints.

### 0.1.0-dev.2026-05-13e — Captain-first main chat routing

Captain is now treated as the principal default agent wherever a route or UI
needs an implicit agent.

- **Stable agent ordering**: `/api/agents` and `/api/status` return `captain`
  before secondary agents such as `vision`, even if the registry restore order
  changes after a restart.
- **Chat fallback**: CLI/web chat daemon resolution now prefers the `captain`
  agent when no explicit agent id/name is supplied, instead of taking the first
  registry entry.
- **External triggers**: routes that previously fell back to the first agent now
  prefer `captain` for implicit dispatch.

### 0.1.0-dev.2026-05-13d — GitHub setup from Projects

Captain can now configure the GitHub token needed by development projects from
the web Projects interface.

- **GitHub status/token API**: `/api/projects/github/status` reports whether
  `GITHUB_TOKEN` is configured and valid; `/api/projects/github/token` saves or
  removes it through the secret store.
- **Inline Projects setup**: selecting GitHub as a project source reveals a
  connection panel where the user can paste a token, validate it, refresh the
  repository list, or disconnect GitHub.
- **Secret boundary**: the token is never stored in project metadata; project
  records only keep repository/workspace metadata.

### 0.1.0-dev.2026-05-13c — Development workspace projects

Captain projects are now closer to Codex/Claude Code development sessions:
they are backed by a real workspace rather than a dashboard-only record.

- **Local or GitHub source**: `/api/projects/launch` accepts local folders and
  GitHub repositories. Missing local paths are created under the configured
  Captain workspaces directory instead of assuming macOS/Desktop.
- **Workspace context injection**: active project prompts now include source
  type, workspace path and repository name so Captain knows where to work.
- **Web Projects surface**: `/projects` now behaves like a development
  workspace picker/inspector with project rail, chat handoff, lifecycle, tasks,
  goals and checkpoint detail.
- **Desktop TUI Projects tab**: the ratatui UI has a Projects surface for
  local/GitHub project creation, resume, chat activation, lifecycle movement
  and project-scoped goals.

### 0.1.0-dev.2026-05-13b — Web Project OS and project goals

Captain's Project Mode is now exposed in the web workbench, not only in the
runtime/TUI surfaces.

- **Project lifecycle contract**: launched projects now carry the required
  `OBSERVE -> THINK -> PLAN -> BUILD -> EXECUTE -> VERIFY -> LEARN` lifecycle
  in metadata and seed one task per phase.
- **Project goals**: long-running autopilot goals can be attached to a project
  by `project_id`/`project_slug`; project deletion removes its scoped goals.
- **Web project detail**: `/projects` can resume a project, inspect lifecycle,
  goals, tasks and latest checkpoint, create/pause/resume/delete project goals,
  and move the lifecycle phase.
- **Dedicated project chat**: the web Projects page opens `/terminal` with a
  stable `project-<slug>` session and activates that project for Captain before
  handing control to chat.

### 0.1.0-dev.2026-05-13a — Desktop TUI focused hub navigation

Captain's ratatui desktop interface now uses focused product hubs instead of a
long flat list of top-level tabs.

- **Focused primary tabs**: `Chat`, `Projects`, `Home`, `Agents`, `Sessions`,
  `Automation`, `Learning`, `Capabilities`, `Connections`, and `Settings` are
  the top-level TUI tabs.
- **Automation hub**: workflows, triggers, cron jobs, and approvals remain fully
  interactive under `Automation`.
- **Learning hub**: review queue, skill proposals, memory, and graph views are
  grouped under `Learning`. Skill proposal notifications still appear in the
  chat and `/skills-proposed` opens the relevant review surface.
- **Capabilities hub**: installed/marketplace/MCP skills and Hands are grouped
  under `Capabilities`.
- **Connections hub**: channels, extensions, peers, and agent communications are
  grouped under `Connections`.
- **Navigation contract**: F1-F10 and Tab/Shift+Tab move between primary hubs;
  inside a hub, Alt+number and Alt+Left/Alt+Right move between subviews. Chat
  keeps slash-command Tab completion when the draft starts with `/`.

### 0.1.0-dev.2026-05-12a — Skill proposal diff gate

Captain now compares generated skill proposals against installed skills before
asking the user to approve them.

- **Deterministic skill diff**: the proposal policy scans bundled skills plus
  user/generated skill roots and computes a local name/procedure/tool overlap
  score. No remote model or embedding call is needed.
- **Duplicate suppression before review**: duplicates are rejected before they
  consume the daily proposal limit or appear in Telegram/UI validation queues.
- **Learning pipeline coverage**: both procedural learnings from the cognitive
  router and repeated-workflow proposals from the SkillSynthesizer use the same
  diff gate.
- **Product contract**: when a reusable workflow already exists, Captain should
  refine the existing skill via `skill_refinement_propose` instead of creating a
  parallel skill.

### 0.1.0-dev.2026-05-11d — Live WebRTC voice calls

Captain now has a real live-call rail in the web terminal, separate from
asynchronous voice-note transcription.

- **WebRTC call button**: the web terminal exposes `Call`, opens microphone
  audio, plays the realtime model audio response, and tears down media tracks
  on hangup/unload.
- **Real mic spectrum**: while a call is open, the footer renders a compact
  frequency spectrum driven by the browser microphone stream through Web Audio
  `AnalyserNode`; it is not a simulated animation.
- **Server-side Realtime relay**: `/api/realtime/calls` exchanges browser SDP
  with OpenAI Realtime using the daemon's server-side API key, so the browser
  never receives a standard OpenAI key.
- **Config source of truth**: `[voice_call]` controls enablement, provider,
  model, voice, optional `api_key_env`, auto-end silence/inactivity limits,
  instructions, and whether the call can hand actionable tasks back to the
  Captain agent loop.
- **Cost guardrails**: Realtime sessions explicitly use server VAD and the web
  terminal watchdog hangs up after configured microphone silence or discussion
  inactivity.
- **Captain task bridge**: live-call tool calls named `captain_message` are
  routed to `/api/agents/{id}/message` with channel `web_call`, then returned
  to the realtime session as tool output. The browser resolver accepts both the
  current `/api/agents` array response and the older `{ agents: [...] }` shape.
- **Single-agent voice rail**: Realtime is treated as Captain's audio interface,
  not a second assistant. The session requires tool use for substantive turns,
  so questions and actions route through the same Captain agent used by typed
  chat, then Realtime reads Captain's result.
- **On-demand activity feedback**: Realtime can call `captain_activity_summary`
  only when the user asks what happened or asks for status/details, avoiding a
  continuous token-heavy action feed.
- **Voice session mirror**: voice-driven tasks are mirrored into a dedicated
  terminal `Voice` panel as `voice`, `call`, and `captain` transcript lines
  without injecting text into the PTY, so the keyboard session remains
  immediately resumable.
- **Mobile input guard**: the web terminal now tracks the line actually sent to
  the PTY, so mobile keyboards that resend the whole line after numeric input or
  paste only emit the delta instead of reinjecting previously typed text.

### 0.1.0-dev.2026-05-11c — Proactive skill refinement approvals and snapshots

Captain now treats existing-skill improvements as first-class controlled
self-improvement items.

- **Visible refinement event**: `skill_refinement_propose` emits
  `SkillRefinementQueued`, and Telegram routes the prompt to the preferred
  validation chat with approve/reject buttons.
- **Dedicated callbacks**: Telegram buttons map to `/skill_refine_approve` and
  `/skill_refine_reject`, separate from generated-skill approvals.
- **Rollback snapshot**: file-backed skills are snapshotted automatically when a
  refinement is proposed. `skill_refinement_restore` can roll back from that
  pre-improvement snapshot and creates a pre-restore backup first.
- **Proactive guidance**: Captain's prompt now requires a quick post-skill-use
  review and proposal whenever a reusable skill improvement is detected.

### 0.1.0-dev.2026-05-11b — Configurable learning autonomy aggressiveness

Captain now exposes a neutral-by-default autonomy coefficient for the learning
and self-improvement loops.

- **Config knob**: `[learning] autonomy_aggressiveness = 1.0` controls how
  conservative or aggressive the learning gates are. `1.0` preserves current
  behaviour; lower values tighten gates, higher values relax candidate/proposal
  volume.
- **Bounded range**: runtime clamps the effective coefficient to a safe range
  before applying it.
- **Approval safety unchanged**: critical decisions still follow
  `learning.mode`, learning review queues, and skill proposal approvals.

### 0.1.0-dev.2026-05-11a — Preferred Telegram validation channel

Learning and self-improvement validation prompts now use Telegram as the
preferred interactive approval channel when Telegram has a configured
`default_chat_id`.

- **Preferred validation surface**: learning review items and generated skill
  proposals are sent to Telegram even when they originate from web, CLI, or a
  background learning job.
- **Interactive decisions**: Telegram prompts keep inline buttons for approve /
  reject, mapped to `learning_review_decide` and `skill_proposal_decide`.
- **Callback routing**: Telegram button clicks now preserve the clicking
  `sender_user_id` for authorization while replying in the chat/thread where the
  button was clicked.

### 0.1.0-dev.2026-05-10p — Learning loop channel and payload guard

Captain's learning pipeline now keeps origin-channel context through the policy
stage and bounds universal turn payloads before reflection.

- **Channel continuity**: `ReflectionBatch.channel` now survives
  `MemoryPolicy::spawn_filter`, so committed or queued learnings can still be
  routed back to the originating surface.
- **Bounded turn reflection**: universal `ConversationTurn` signals truncate
  both user and assistant text before entering the reflection queue, matching the
  learning bus memory-safety contract.
- **Regression coverage**: runtime tests cover channel preservation through the
  policy stage and truncation of long conversation-turn payloads.

### 0.1.0-dev.2026-05-10o — Web TUI color inheritance fix

Captain's web terminal now preserves TUI colors even when the daemon process was
started from a shell with `NO_COLOR=1`.

- **Inherited env cleanup**: PTY sessions can now remove selected inherited
  environment variables before spawning the child process.
- **Web terminal color fix**: web terminal sessions remove `NO_COLOR` while still
  setting `TERM=xterm-256color` and `COLORTERM=truecolor`, so ratatui emits
  normal ANSI/truecolor sequences for xterm.js.
- **Stream remains raw**: the browser still writes PTY output unchanged; color is
  restored at the source instead of post-processing terminal bytes client-side.

### 0.1.0-dev.2026-05-10n — Web TUI explicit new session action

Captain's web terminal now has an explicit `+` action for fresh browser chat
sessions.

- **New session button**: the top action bar exposes a `+` control that creates
  a new per-tab `web-*` session without requiring the user to edit the session
  id manually.
- **Non-destructive detach**: creating a new session detaches the current browser
  tab but does not terminate the previous PTY session, so it remains available in
  the session list with replay.
- **Immediate reconnect**: the new session updates the URL, recent-session list,
  terminal state, and opens a fresh Captain chat automatically.
- **PTY-safe stream**: the web client no longer injects ANSI color wrappers into
  PTY output. xterm receives the daemon stream unchanged, preventing statusline
  corruption when the TUI sends cursor movement or footer updates.

### 0.1.0-dev.2026-05-10m — Web TUI chroma, viewport fit, and safe session resume

Captain's web terminal now treats visual legibility and session switching as
first-class web UX concerns.

- **Terminal theme layer**: the web terminal uses a richer xterm/CSS palette but
  does not mutate the raw PTY stream. Full-screen TUI color must come from the
  terminal application itself, not from client-side output rewriting.
- **Viewport fit fix**: the workbench is pinned to explicit grid rows, so hiding
  the command palette no longer lets the footer consume the flexible row. The
  terminal now fills the available height on desktop, tablet, and mobile.
- **Responsive terminal density**: compact viewports lower xterm font size so
  the web PTY has enough columns for the Captain banner and useful chat content.
- **Safe session resume**: clicking the current session no longer resets the
  terminal. Clicking a session already attached elsewhere now shows an explicit
  warning instead of opening a blank replacement session.
- **Resume replay guard**: switching sessions clears the terminal only after the
  WebSocket attachment succeeds, preserving the previous visible state during
  reconnect attempts.

### 0.1.0-dev.2026-05-10l — Web TUI mobile app and true global usage rail

Captain's web terminal now treats mobile and usage telemetry as product-critical
surfaces instead of decorative responsive extras.

- **True usage rail**: the right rail no longer counts PTY output/replay bytes.
  It reads `/api/usage/summary` and displays global Captain LLM tokens, cost,
  and call count across agents/sessions.
- **Mobile app layout**: on phone-sized screens, sessions and activity become
  bottom sheets over the chat instead of shrinking the terminal into stacked
  page sections.
- **Mobile defaults**: session/activity/command panels start closed on compact
  viewports so the primary chat stays usable immediately.
- **Native web attachments**: the web terminal can upload images, text/PDF, and
  audio through Captain's upload API, then inject `/image <path>`, `/file <path>`,
  or an audio transcription into the live chat PTY.
- **Upload path bridge**: upload responses now include the daemon-side local path
  and authenticated download URL so browser uploads are usable by Captain tools.

### 0.1.0-dev.2026-05-10k — Web TUI v2 interactive cockpit

Captain's web chat is now an interactive TUI cockpit around the real PTY.

- **Session drawer**: a left panel lists recent browser sessions and live daemon
  terminal sessions; selecting one reopens that shared session.
- **Command palette**: the web surface can send common slash commands or a free
  text command directly into the PTY without hiding the terminal.
- **Activity rail**: terminal output is classified client-side into semantic
  cards for tools, browser activity, model signals, warnings, errors, and
  success states.
- **Color semantics**: connection state, tools, browser actions, model signals,
  warnings, and errors use distinct colors and subtle animated feedback.
- **Browser preview**: browser/web activity opens a compact live preview card so
  research/navigation work is visible outside raw terminal text.
- **Responsive cockpit**: session drawer, command bar, terminal, and activity
  rail collapse into mobile-friendly stacked panels with reduced-motion support.

### 0.1.0-dev.2026-05-10j — Web chat no-logo surface and empty terminal guard

Captain's browser chat surface no longer shows a visible logo and is harder to
leave visually blank.

- **No visible logo**: `/terminal`, `/config`, their auth cards, and browser
  metadata now use text identity only; no visible logo/icon is advertised by
  these web surfaces.
- **Modern IdeaVault motion layer**: the web terminal and config editor use a
  dotted grid background, subtle scan/progress motion, animated panel entry,
  hover lift, and reduced-motion fallbacks.
- **Per-tab chat sessions**: the web terminal no longer defaults every browser
  tab to `main`; it generates a short per-tab `web-*` session id unless the user
  explicitly provides a session id, and keeps that id in `?session=...` so the
  session is visible, shareable, and stable across reloads.
- **Blank-state guard**: the terminal frame now shows clear connection/auth
  state while the PTY is not yet producing output, and automatically retries
  with a fresh auto session if the previous browser attachment is still held.
- **Reconnect replay**: live web terminal sessions keep a bounded recent output
  buffer and replay it when the browser reconnects, so a shared/reloaded tab
  does not attach to a visually empty PTY.
- **Session picker**: the session field is still editable, but now exposes a
  dropdown of recent/local web chat sessions plus live daemon terminal sessions,
  making it easy to reopen a previous browser chat.

### 0.1.0-dev.2026-05-10i — Web crown matches the CLI identity

Captain's web identity now reuses the exact CLI crown source instead of a
generic outline icon.

- **Canonical crown asset**: `/logo.svg` is generated from the 75x27
  Braille/pixel-art crown defined in the CLI branding source, preserving the
  same glyph layout and gold segment colors.
- **Surface alignment**: `/terminal`, `/config`, their auth cards, the favicon,
  and the web app manifest all point to the same Captain crown asset.

### 0.1.0-dev.2026-05-10h — Web terminal product skin and chat-only launch

Captain's focused web surfaces now use the Captain crown identity and a
dark-first operator design inspired by the IdeaVault visual system.

- **Chat-only web terminal**: `/terminal` now launches `captain chat` for the
  browser PTY instead of the full multi-tab TUI. Raw shell remains a gated
  backend mode for explicit technical use, but the browser UI exposes only chat.
- **Captain crown branding**: old PNG/favicon references are removed from the
  embedded web pages; `/logo.svg` and `/favicon.ico` are served from the Captain
  crown asset.
- **Unified web skin**: terminal, config editor, and session auth cards share
  the same dense dark grid, lime accent, square controls, responsive mobile
  layout, and source-of-truth config visual language.

### 0.1.0-dev.2026-05-10g — Live browser activity timeline

Captain's browser automation is now observable across all chat surfaces.

- **Shared progress stream**: `browser_batch` emits semantic progress for each
  action, final observation, and completion state.
- **Surface parity**: TUI, web terminal, API streaming, and Telegram receive the
  same live browser activity. Telegram edits the active tool bubble instead of
  posting a burst of standalone progress messages.
- **Safer display**: typed secrets, tokens, API keys, and password-like fields
  are masked in the visible activity timeline; URL credentials/query secrets are
  redacted.
- **Search hygiene**: Captain is instructed to use native search rails for
  generic discovery, reserve the browser for direct pages/JS/forms/downloads, and
  switch rails when Google `/sorry`, CAPTCHA, unusual-traffic, or anti-bot pages
  appear.

### 0.1.0-dev.2026-05-10f — Session auth for CLI and web terminal

Captain's local terminal surfaces now work when web/session auth is enabled
without a persistent API key.

- **CLI/TUI local auth**: local CLI/TUI daemon clients can derive a short-lived
  web session token from the local config when no `api_key` is configured,
  avoiding `Missing Authorization: Bearer <api_key> header` in session-only
  installs.
- **Web terminal auth bridge**: the authenticated browser session is passed only
  to the Captain TUI PTY process as `CAPTAIN_SESSION_TOKEN`; raw shell mode does
  not receive that token.
- **API acceptance**: protected API endpoints accept valid web session tokens in
  `Authorization: Bearer ...` as well as the existing API key and browser
  cookie paths.
- **Security posture**: this does not make protected endpoints public. A caller
  still needs either a valid API key, a valid web login session, or local config
  file access under the same user account.

### 0.1.0-dev.2026-05-10e — Deep research source intake

Captain's native research rail now handles heavier source files without asking
the model to improvise shell downloads.

- **Source download**: new `web_download` saves public PDF/report/CSV/JSON/text
  sources into the workspace with SSRF redirect checks, size caps, overwrite
  protection, MIME detection, and SHA-256 output.
- **Document extraction**: new `document_extract` reads downloaded PDFs with
  embedded text plus text-like files and returns bounded evidence for synthesis.
  Image-only PDFs fail explicitly so Captain must use OCR/vision or another
  source instead of guessing.
- **Research behavior**: prompts and researcher hand instructions now require
  breadth search, primary-source follow-up, PDF/report extraction, contradiction
  checks, self-critique, and a final Sources section containing only sources
  actually read.
- **Tool economy**: `web_research_batch`, `web_download`, `document_extract`,
  `document_pipeline`, and native browser batches are documented as one coherent
  flow for deep research and polished deliverables.

### 0.1.0-dev.2026-05-10d — Native browser interaction parity

Captain's browser rail remains native Rust/CDP and does not depend on the
Python `browser-use` CLI. The native tool surface now covers the common browser
automation loop directly.

- **Keyboard input**: new `browser_keys` sends Enter, Tab, Escape, arrows,
  shortcuts like `Control+a` / `Meta+k`, or focused text input.
- **Native dropdowns**: new `browser_select` selects HTML `<select>` options by
  value, label, or visible text.
- **Hover UI**: new `browser_hover` moves the pointer over an element so menus,
  tooltips, and hover-only controls can be observed and clicked.
- **Grouped flows**: `browser_batch` accepts `keys`, `select`, and `hover`, so
  navigation, form filling, keyboard submit, observation, diagnostics, and
  screenshots can stay in one low-token tool call.
- **Agent guidance**: browser docs, `captain_docs`, the browser hand, grouped
  tools, compatibility mapping, and runtime hints now all expose these actions.

### 0.1.0-dev.2026-05-10c — Authenticated config page

Captain now ships a focused browser config editor at `/config`.

- **Full config editing**: `/config` loads the current `config.toml` and the
  complete default template, so every configurable field remains reachable
  without rebuilding a legacy dashboard.
- **Web-session only**: the page requires Captain web login and never asks the
  user to paste an API key into the browser.
- **Safe write rail**: raw config saves validate TOML + `KernelConfig`, create a
  timestamped backup under `config-backups/`, write atomically, roundtrip
  validate, and roll back from backup on write/validation failure.
- **Hot reload**: after save, the page calls the config reload endpoint and
  reports whether a daemon restart is still required.

### 0.1.0-dev.2026-05-10b — Web terminal auth polish

Captain's interactive browser terminal now uses only web-session login.

- **72h default**: new installs write `auth.session_ttl_hours = 72`; exposed
  VPS hosts should keep this between 24 and 72 hours.
- **No browser API-key prompt**: `/terminal` no longer asks users to paste an
  API key into localStorage. API-key auth remains accepted by the terminal
  WebSocket for technical clients and automation.
- **Docs cleanup**: current product docs now refer to `/terminal` and
  web-session auth instead of the removed browser SPA.

### 0.1.0-dev.2026-05-10a — Web terminal login source of truth

Captain now treats `config.toml` as the live source of truth for web terminal
credentials.

- **Terminal login first**: `/terminal` opens with an access prompt while
  Captain checks the current web auth state.
- **Live auth reload**: web login, API middleware and terminal WebSocket auth
  reread `[auth]` / `api_key` from `config.toml`, so credential changes do not
  require a daemon restart.
- **Native credential tool**: `web_credentials_update` lets Captain rotate the
  web username/password from natural language, writes a backup, hashes the
  password, validates the TOML roundtrip, and emits the config hot-reload event.
- **Security tightening**: `/api/config` and `/api/config/schema` now require
  session or API-key auth. The terminal loads config only after successful
  authentication.
- **Session invalidation**: browser session signatures are bound to both
  `api_key` and `password_hash`, so password rotation invalidates stale web
  sessions.

### 0.1.0-dev.2026-05-09c — Web terminal is the only browser UI

Captain no longer ships the legacy browser SPA. The
browser surface is now intentionally focused on the native web terminal.

- **Single browser surface**: `/` and `/terminal` both serve the xterm.js
  Captain terminal. The old `/legacy` browser UI route is gone.
- **Asset removal**: Alpine.js, Chart.js, Marked, Highlight.js, legacy page
  scripts, legacy layout CSS, and legacy HTML templates are no longer
  embedded in the binary.
- **CLI alignment**: `captain terminal` opens `/terminal`; the old browser UI command
  is no longer a public command.
- **Setup language**: generated first-use credentials and setup/status output
  now point users to the web terminal.

### 0.1.0-dev.2026-05-09b — Install auth bootstrap and VPS terminal readiness

Captain setup now makes the web terminal usable immediately after installation
without weakening VPS security defaults.

- **Access bootstrap**: `captain setup` generates a root `api_key` for CLI/API
  Bearer auth and enables `[auth]` web session login. Generated first-use
  credentials are written to `~/.captain/initial-credentials.txt` with private
  file permissions.
- **VPS shorthand**: `captain setup vps` is accepted as a product-friendly alias
  for `captain setup --profile vps`.
- **Terminal readiness**: setup always writes `[web_terminal]` defaults; Captain
  mode is enabled, raw Shell mode remains explicit opt-in.
- **Deployment metadata**: setup writes `[deployment]` (`profile`,
  `public_url`, `https`, `reverse_proxy`) so `config.toml` remains the source of
  truth for domain/HTTPS choices.
- **Caddy handoff**: VPS setup with `CAPTAIN_DOMAIN` or `CAPTAIN_PUBLIC_URL`
  generates `~/.captain/deploy/Caddyfile` for the HTTPS reverse proxy.

### 0.1.0-dev.2026-05-09a — Native VPS web terminal

Captain now exposes a native browser terminal at `/terminal`.

- **Bundled xterm.js**: terminal assets are vendored locally and embedded in the
  Captain binary; the terminal page does not depend on a CDN.
- **PTY backend hardening**: the terminal WebSocket requires API key or
  web session auth, validates session IDs, checks browser Origin against
  Host, caps frame sizes, limits concurrent live PTY sessions, and rejects raw
  shell mode unless explicitly enabled.
- **Captain-first default**: `/terminal` starts `captain tui` by default, so VPS
  users get the normal Captain TUI in the browser.
- **Shell opt-in**: enable `[web_terminal].allow_raw_shell = true` to expose the
  Shell mode button for administrators who need direct shell access.
- **Responsive auth UX**: the terminal page includes its own mobile-safe access
  prompt for web session login or API-key mode.
- **Deployment docs**: `docs/deployment/vps-web-terminal.md` documents the
  recommended reverse-proxy and HTTPS shape.

This turns the old PTY backend foundation into a product surface suitable for
VPS deployments while preserving secure defaults.

### 0.1.0-dev.2026-05-08d — Native service lifecycle CLI

Captain now exposes a product-grade service lifecycle CLI:

- **New `captain service` command group**: `install`, `start`, `stop`,
  `restart`, `status`, and `logs`.
- **Native service definitions**: macOS `launchd` dry-run/install support writes
  a LaunchAgent that runs `captain start`; Linux systemd support writes
  `captain.service` with `RestartForceExitStatus=75`.
- **Fallback control**: when no native service is installed, `service start` and
  `service restart` use the detached tmux daemon session, then background start
  as last resort.
- **Operational visibility**: `captain status --verbose`, `captain doctor
  --full`, and `captain service status --json` now expose selected manager,
  daemon URL, binary path, home/log paths, launchd/systemd install state and
  tmux fallback state.
- **Service logs**: `captain service logs` reads systemd journal when available,
  tmux pane output for tmux fallback, or `~/.captain/captain.log`.

This makes tmux a dev/fallback path instead of the only understandable lifecycle
mechanism.

### 0.1.0-dev.2026-05-08c — Restart lifecycle service-manager hardening

Captain's global `/restart` command now has a product-grade lifecycle strategy:

- **Service-manager first**: restart helper tries `launchd` labels, then systemd
  user/system services, before falling back to tmux and finally nohup.
- **Systemd-safe restart code**: Linux systemd installs now declare
  `RestartForceExitStatus=75`, and Captain exits with that code when it detects
  a systemd runtime restart. This lets VPS installs restart even when the daemon
  user cannot call `systemctl restart` directly.
- **Remote diagnostics**: `/restart status` reports the selected strategy,
  pending Telegram ready notifications, helper log path and recent helper log
  lines, so restart issues can be diagnosed from Telegram/API without shell
  access.

This keeps `/shutdown confirm` as a true stop while making `/restart` reliable
across desktop/dev and VPS service deployments.

### 0.1.0-dev.2026-05-08b — Telegram restart reliability fix

Captain's Telegram `/restart` command now uses a detached restart helper that
prefers the installed tmux daemon session and only falls back to nohup when
tmux is unavailable. The helper writes `restart-helper.log` for diagnostics and
keeps the existing post-boot Telegram ready notification flow.

This fixes the observed failure mode where `/restart` acknowledged the request,
wrote the pending Telegram notification, then stopped the daemon without a
reliable relaunch.

### 0.1.0-dev.2026-05-08a — Global daemon slash commands

Captain now handles daemon operations before the LLM across channel and API
chat paths:

- **Global daemon commands**: `/status`, `/health`, `/version`, `/config`,
  `/reload`, `/restart` and `/shutdown confirm` are intercepted before the
  agent loop, so they do not consume model tokens.
- **Exact config read**: `/config` returns the current `config.toml` content
  without LLM rewriting and is gated as an owner-level sensitive command.
- **Restart lifecycle feedback**: `/restart` persists a pending ready
  notification for Telegram and sends a confirmation after the daemon is back.
- **Safe shutdown**: `/shutdown` requires `/shutdown confirm`.
- **API/TUI compatibility**: JSON and SSE message endpoints return normal
  zero-token command responses, allowing TUI streaming clients to display the
  same daemon-command output.

Security note: sensitive commands are local-only by default unless the channel
sender is explicitly authorized; wildcard channel access does not grant owner
daemon control.

### 0.1.0-dev.2026-05-07c — Codex context economy hardening

Captain now treats Codex token usage as a product-quality issue, without
degrading Claude/Anthropic behavior:

- **Codex-only tool schema economy**: LLM requests to Codex keep exact tool names,
  required fields, JSON types and enums, but strip verbose schema titles and
  descriptions from the request copy. Execution still uses the full runtime tool
  catalog.
- **Codex history budget tightened**: Codex request history now keeps a smaller
  canonical + recent tail window before every LLM call, including streaming
  calls, so simple follow-up turns do not resend large stale transcripts.
- **Direct-answer detection expanded**: requests such as "réponds en deux mots",
  "just say", "no tool" or "aucun outil" enter the lean direct path when no
  action cue is present.
- **Memory noise filter hardened**: strict MemPalace context rejects negative or
  invalid similarity scores even when lexical overlap looks tempting, preventing
  old diary-like memories from polluting current answers.
- **Runtime docs alias recovery**: `captain_docs` now normalizes safe
  changelog aliases such as `changelog`, `runtime`, `runtime_changelog`, and
  query-scoped `docs` to `runtime-changelog`, avoiding repeated empty tool turns
  when Codex chooses a human-friendly family name.
- **Cold tool replay compression**: Codex request context now compacts stale
  tool results far more aggressively while preserving the newest result inside
  an active tool loop, so a polluted daemon session does not make every simple
  follow-up expensive.
- **Focused changelog reads**: `captain_docs(family:"runtime-changelog",
  query:"latest entry")` returns only the latest versioned entry instead of the
  entire runtime changelog body.
- **Codex CORE routing nudge**: the Codex economy prompt and compact tool
  descriptions now explicitly tell the model to call `captain_docs` directly
  for runtime changelog/docs/tool-behavior requests, instead of spending a
  preliminary `capability_search` turn.

Critical note: this is a Codex rail only. Anthropic/Claude receives the same
tool schema surface as before. Quality is preserved through `capability_search`,
`tool_search`, `captain_docs`, memory context batch, and dynamic tool
rehydration when a task needs more than the lean initial surface.

### 0.1.0-dev.2026-05-07b — Memory markdown retirement finalized

Captain's memory rail is now explicitly MemPalace-first end to end:

- **No legacy `MEMORY.md` prompt path**: the runtime prompt builder ignores the
  retired `memory_md` compatibility field in both full and Codex economy prompt
  profiles. Active memory remains recalled memories, canonical context, graph
  snapshots and direct MemPalace tools.
- **Workspace editor cleanup**: the API file editor no longer exposes workspace
  `USER.md`, `MEMORY.md` or `BOOTSTRAP.md`. It keeps active workspace prompt
  files only: `SOUL.md`, `IDENTITY.md`, `TOOLS.md`, `AGENTS.md`, `STYLE.md` and
  `HEARTBEAT.md`.
- **Docs aligned**: the memory family now states that workspace memory markdown
  is legacy migration input, not an active product surface.

Critical note: if an old workspace still contains `MEMORY.md`, Captain may
migrate it as historical input, but it must not use that file as current truth.
Use `memory_save`, `memory_recall` and `memory_forget` for durable memory.

### 0.1.0-dev.2026-05-07a — Phase 2 installation finalization

Captain's installation/setup rail now completes the product phase 2 contract
instead of leaving important readiness work to manual follow-up:

- **TTS source of truth fixed**: `tts_openai` and `tts_elevenlabs` setup now
  explicitly patch `[tts].enabled = true` and `[tts].provider = ...`. OpenAI TTS
  defaults to Nova when selected, and ElevenLabs writes the schema-correct
  `tts.elevenlabs.model_id` while still accepting legacy `model` input.
- **Full unattended setup**: `captain setup --from-env --answers <file> --yes`
  now configures provider/model/secrets, global user profile, Telegram, STT and
  TTS from environment variables and/or a TOML answers file. `--quick` stays
  minimal for CI/bootstrap scripts.
- **Setup interview parity**: guided CLI setup now captures the same durable
  personal fields as the kernel first-use interview: preferred name, language,
  timezone, answer style, voice preference, notification preference and privacy
  boundaries. It writes global `USER.md`, so the kernel gate does not repeat the
  interview after a complete setup.
- **Safer first-use voice hints**: the kernel still records OpenAI/ElevenLabs
  voice preference, but only enables TTS automatically when the matching runtime
  credential is actually present or already enabled.
- **VPS service finalization**: `scripts/install.sh` supports `CAPTAIN_START`
  and starts/health-checks the generated systemd service by default for
  `CAPTAIN_PROFILE=vps`.
- **Snapshot restore rollback**: failed snapshot extraction now restores the
  previous Captain home automatically instead of leaving a half-restored state.
- **Clean install smoke**: `scripts/install-clean-smoke.sh` verifies a
  precompiled local-bundle install in an isolated temporary `CAPTAIN_HOME`.

Critical note: memory cannot override these setup choices. TTS provider/voice is
runtime config, personal profile is global `USER.md`, and learned facts remain a
separate MemPalace concern.

### 0.1.0-dev.2026-05-06m — Daemon workspace cwd isolation

Captain no longer lets the shell launch directory become the daemon's local
workspace context:

- **Daemon cwd source-of-truth**: `captain start` now switches its process
  working directory to `~/.captain` before booting the kernel, so the principal
  Captain agent reports the installation workspace instead of whichever repo or
  folder happened to launch the daemon.
- **Detached starts fixed**: helper flows that spawn `captain start` in the
  background now set `current_dir` to `~/.captain`.
- **Linux service fixed**: installer-generated systemd services now use
  `WorkingDirectory=$HOME/.captain`, not `$HOME`.

Critical note: the principal `captain` agent keeps global local access by policy.
This change only prevents accidental project-context leakage from the parent
shell cwd. Sub-agents still use their own managed workspaces.

### 0.1.0-dev.2026-05-06l — Ready-first guided install

Captain's product installer now treats provider readiness as part of
installation, not as an optional follow-up:

- **Guided setup by default**: after installing the precompiled CLI, Unix/macOS/
  WSL and Windows installers ask to run `captain setup` immediately. The Unix
  installer reads from `/dev/tty`, so the prompt still works when installed via
  `curl | sh`.
- **Base config before preferences**: `captain setup` now configures provider,
  model and credentials before asking assistant/user preferences. A fresh
  install should not finish personalization while the LLM rail is still missing.
- **Ready gate**: API-key providers require a key or an explicit provider
  change; Codex asks for OAuth when missing; Ollama and Claude Code are checked
  before being accepted as ready.
- **No local build for users**: this keeps the controlled precompiled bundle
  contract from `2026-05-06k`; setup is configuration only.
- **Codex default**: Codex setup defaults to `gpt-5.5` for the primary Captain
  agent when using the ChatGPT subscription rail.

Critical note: unattended installs can still opt out with `CAPTAIN_SETUP=0` or
use `CAPTAIN_SETUP_QUICK=1` / `captain setup --from-env --yes`, but those modes
must provide credentials through environment/config if the final system is
expected to be ready.

### 0.1.0-dev.2026-05-06k — Controlled precompiled installer

Captain's installer rail no longer assumes a public GitHub repository or an
end-user source build:

- **Controlled distribution endpoint**: Unix/macOS/WSL and Windows installers
  now default to a Captain-owned release base (`https://captain.sh/releases`)
  with a simple `latest.txt` + per-version bundle layout.
- **Precompiled bundle contract**: `scripts/package-release.sh` builds the
  release binary once in CI/maintainer context and emits
  `captain-<platform>.tar.gz`, `.sha256`, `manifest.json`, and `latest.txt`.
- **Local bundle install**: `CAPTAIN_BUNDLE_PATH=/path/to/archive.tar.gz`
  installs an already compiled bundle for smoke tests and private
  distributions without network access.
- **No cargo fallback for users**: failed downloads now fail clearly instead of
  telling users to compile Captain or install from GitHub.
- **Checksum verification**: installers verify `.sha256` files when available
  and support explicit `CAPTAIN_BUNDLE_SHA256` for local/offline bundles.

Critical note: source compilation remains a maintainer/CI step only. A product
install should install a signed or checksummed bundle, then run setup/doctor.

### 0.1.0-dev.2026-05-06j — Deterministic onboarding and lean identity files

Captain's first-use interview is now enforced by the kernel before the LLM
loop, not only by prompt instructions:

- **Channel-neutral onboarding gate**: the principal `captain` agent intercepts
  the first normal user turn in both non-streaming and streaming paths before
  model execution. This covers TUI, Telegram, Web/API and WebSocket callers.
- **Seven-step personal interview**: Captain asks one short question at a time
  for preferred name, language, timezone, answer style, voice preference,
  notification preference and privacy boundaries. `passer` / `skip` provides a
  deliberate escape hatch.
- **Global USER.md source of truth**: the collected profile is written to
  `~/.captain/USER.md` and the persistent config is aligned
  (`language`, `timezone`, `[assistant].style`, onboarding completion, and
  simple TTS provider hints when the user names OpenAI/Nova or ElevenLabs).
- **No LLM/token cost during onboarding**: interview prompts are deterministic
  `AgentLoopResult`s with zero token usage.
- **Workspace markdown cleanup**: new workspaces no longer generate
  `USER.md`, `MEMORY.md`, `BOOTSTRAP.md`, `PLAYBOOK.md` or placeholder
  `TOOLS.md`/`AGENTS.md`. Captain keeps lean identity files (`SOUL.md`,
  `IDENTITY.md`) and reads custom `AGENTS.md` only when it is not a generated
  legacy placeholder.
- **MemPalace-first memory rail**: workspace `MEMORY.md` is treated as legacy
  migration input, not prompt context. Live prompt memory now comes from the
  global profile, canonical context, graph snapshot and recent journal.

Critical note: `AGENTS.md`, `STYLE.md`, `SOUL.md`, `IDENTITY.md`, `GRAPH.md`
and `HEARTBEAT.md` remain useful when they carry real custom context.
Generated or stale memory/bootstrap/playbook markdown should not be used as a
runtime source of truth.

### 0.1.0-dev.2026-05-06i — Installation excellence foundation

Captain now has the first product-grade installation and recovery rails:

- **CLI-mandatory installers**: Unix/macOS and Windows installers now fail if
  the `captain` CLI is not installed, executable, version-checkable and
  resolvable on `PATH`.
- **Linux/VPS install profiles**: `scripts/install.sh` accepts
  `CAPTAIN_PROFILE=core|vps|desktop|full-media`, can install missing OS
  packages through the detected package manager, and can install a systemd
  service for VPS deployments.
- **Non-interactive setup flags**: `captain setup --quick --profile <profile>
  --yes` and `captain setup --non-interactive --profile <profile> --yes`
  provide a scriptable setup path for installers and CI smoke tests.
- **Snapshot commands**: `captain snapshot create/list/restore/prune` can back
  up and restore local Captain state.
- **Factory reset**: `captain reset --factory` creates a recovery snapshot by
  default, stops the daemon when needed, resets local state and leaves the CLI
  installed.
- **Telegram guided setup**: CLI Telegram setup now walks through BotFather,
  validates the token with `getMe`, can discover `chat_id` / `user_id` after
  `/start`, and sends a real validation message.
- **Voice setup source of truth**: TTS setup now asks between OpenAI Nova and
  ElevenLabs, and OpenAI TTS defaults to `nova` when selected.
- **Brand audit**: `captain doctor --brand-audit` scans text assets for legacy
  public branding and reports release-blocking findings.
- **First-use interview prompt gate**: the channel-agnostic bootstrap protocol
  now asks for a personal interview when `USER.md` is empty, covering name,
  language, timezone, answer style, voice preference, notification preference
  and privacy boundaries.

Critical note: superseded by `0.1.0-dev.2026-05-06j`, which makes the
first-use interview deterministic in the kernel.

### 0.1.0-dev.2026-05-06h — API smoke and context quality fixes

Captain tightens the real API validation path and fixes two quality issues
found during release smoke testing:

- **CLI smoke flags**: `scripts/excellence-smoke.sh` now accepts explicit
  `--llm`, `--tts`, `--ssh-alias`, `--api`, `--timeout`,
  `--ready-timeout`, and `--expected-changelog` flags. Full checks no longer
  require env-prefixed commands, which makes release validation easier to run
  in restricted shells and CI harnesses.
- **Stale-binary guard**: the smoke gate now checks for the expected runtime
  changelog entry (`0.1.0-dev.2026-05-06h` by default), so a daemon that was
  not rebuilt/restarted after local changes fails clearly instead of appearing
  release-ready.
- **Local memory SSOT in grouped recall**: `memory_context_batch` now reads the
  local `memory_writes` journal before MemPalace/graph recall. Durable facts
  saved through `memory_save` can be surfaced immediately when they match the
  query, without relaxing the strict filter or waiting on external sync/ranking.
- **Document title dedupe**: `document_create` removes a leading `# Title`
  from body content when it is identical to the explicit document `title`,
  preventing generated reports from starting with duplicate H1 headings.

Critical note: the memory fix preserves strict filtering. Filtered candidates
remain non-facts; the new source only promotes exact local durable facts that
match the query terms.

### 0.1.0-dev.2026-05-06g — Agent delegation hardening

Captain's local agent orchestration now closes two excellence audit gaps:

- **WeCom bridge activation fix**: `[channels.wecom]` is now included in the
  channel bridge bootstrap gate. A WeCom-only configuration no longer exits
  early before the WeCom adapter is instantiated.
- **Channel bootstrap drift guard**: the bootstrap gate now detects configured
  channel sections structurally instead of maintaining a second manual list of
  channel names. `silent_mode` alone does not count as an active channel.
- **Spawn scope inheritance**: `agent_spawn` now rejects a child manifest that
  asks for tools outside a scoped parent's `tool_allowlist` /
  `capabilities.tools`. A restricted worker can no longer create an
  unrestricted child to bypass its own tool policy.
- **Unrestricted-child guard**: when the parent is scoped, child manifests must
  declare an explicit `tool_allowlist`, `capabilities.tools`, or a narrow
  profile. `Full`, `Custom`, wildcard, or empty tool policy is denied.
  Superseded by `0.1.0-dev.2026-05-14d`: sub-agents now require explicit
  non-wildcard tools even when the parent is unrestricted; a narrow profile
  alone is no longer accepted by `agent_spawn`.
- **Delegation depth guard**: `agent_delegate` now participates in the same
  inter-agent recursion depth guard as `agent_send`.
- **Delegation result persistence**: delegated tasks are completed with the
  worker response instead of leaving a queue item pending when the worker did
  not call `task_complete` itself.
- **Scoped delegation budget**: `agent_delegate(max_tokens)` no longer mutates
  the worker's hourly quota. The budget is scoped to the delegated run, returns
  `used_tokens` / `budget_exceeded`, and can stop additional tool execution
  once the measured budget is reached.
- **Sub-agent lineage metadata**: spawned children now receive
  `is_subagent`, `parent_agent_id`, `root_agent_id`, and `subagent_depth`
  metadata automatically, so the prompt builder can use the worker-specific
  prompt path instead of treating every spawned worker as a principal agent.
- **Depth-aware tool policy wired**: sub-agent lineage depth now drives both
  tool visibility and execution-time denial for admin/scheduling tools. Deferred
  discovery can no longer re-surface tools hidden from a worker by the depth
  policy.
- **Truthful docs**: the agent-facing docs now state that `agent_delegate` is
  synchronous today and that one LLM request can still overshoot the scoped
  budget before Captain can interrupt the next tool step.

Critical note: true async delegation remains a product gap. Use
`task_post` / `task_claim` for fire-and-forget work until the dedicated
delegation runner lands.

### 0.1.0-dev.2026-05-06f — Excellence release gate

Captain now has a product-excellence roadmap and a repeatable API smoke gate.

- **Roadmap**: `docs/excellence-roadmap.md` structures the next work into
  release validation, context economy, Telegram reliability, grouped rails,
  browser parity, document/report quality, ops polish, and product benchmarks.
- **Core smoke**: `scripts/excellence-smoke.sh` validates a live daemon without
  spending LLM tokens or sending channel messages by default.
- **Grouped rail checks**: the smoke gate verifies discovery of
  `web_research_batch`, `file_inspect_batch`, `ssh_health_check`,
  `document_pipeline`, `memory_context_batch`, `media_pipeline`, and
  `channel_delivery_batch`.
- **Quality guard**: the smoke gate includes a strict-memory regression check
  that asserts compact metadata is present and raw MemPalace dumps are not
  injected into tool output.
- **Opt-in full mode**: `scripts/excellence-smoke.sh --full` can enable live
  LLM, SSH, or TTS checks through environment flags when a release candidate
  needs end-to-end validation.

Critical note: this is a release gate, not a replacement for scenario
benchmarks. It catches wiring regressions quickly; Phase 7 remains responsible
for provider/channel quality scoring.

### 0.1.0-dev.2026-05-06e — High-confidence memory context batch

`memory_context_batch` now filters MemPalace recall before injecting it into
model context.

- **No raw MemPalace dump by default**: semantic candidates are parsed into
  compact source records instead of passing through the full `memory_recall`
  text.
- **Precision gate**: a memory candidate is kept only when it overlaps enough
  with the query terms or has a strong 0..1 similarity score
  (`memory_min_similarity`, default `0.75`).
- **Visible filtering**: each memory source reports `match_count`, `filtered`,
  `total_candidates`, and a clear message. When `match_count` is zero, the
  agent must not infer facts from filtered candidates.
- **Forensic escape hatch**: `strict_memory_filter=false` remains available for
  explicit memory audits, but normal answering should keep the strict default.

Critical note: this fixes the product issue where a grouped recall could save
tool calls while silently degrading answer quality with unrelated MemPalace
diary hits.

### 0.1.0-dev.2026-05-06d — P0/P1 grouped native rails

Captain now exposes specialized grouped tools for high-frequency product
workflows where Codex tended to burn one tool call per small step.

- **P0 rails**: `web_research_batch`, `file_inspect_batch`,
  `ssh_health_check`, and `document_pipeline` cover research, repo/file
  inspection, remote health checks, and report rendering plus optional delivery.
- **P1 rails**: `memory_context_batch`, `media_pipeline`, and
  `channel_delivery_batch` cover multi-source context recall, attachment/audio/
  image workflows, and multi-recipient delivery.
- **Token economy**: each rail keeps the model-facing output compact and
  preview-based, while preserving the original underlying tools for deeper
  second passes when quality requires it.
- **Model neutrality**: these are native Captain tools, not Codex-only hacks;
  Claude can still use the same rails, but the primary benefit is reducing the
  multi-call pressure seen in Codex sessions.

Critical note: grouped tools are not a replacement for judgement. The agent
should still use the precise single-purpose tool for a narrow one-step task, or
run a second deeper fetch/read/transcription when the preview is insufficient.

### 0.1.0-dev.2026-05-06c — Browser Excellence phase 1-3

Captain's native Browser rail now has the first advanced automation primitives:

- **Grouped browser commands**: new `browser_batch` runs up to 20 sequential
  browser actions in one tool call, with compact per-step summaries and a
  configurable final observation (`observe`, `read_page`, `status`,
  `diagnostics`, or `none`). This reduces the expensive
  navigate/wait/read/status/network multi-call pattern, especially for Codex.
- **Structured page observation**: new `browser_observe` returns a compact
  interaction map with stable refs (`@e1`, `@e2`, ...). Those refs can be used
  directly in `browser_click`, `browser_type`, `browser_wait`, or inside
  `browser_batch` steps.
- **One-call diagnostics**: new `browser_diagnostics` combines browser status,
  page observation, recent CDP network events, and browser console/page errors.
  This avoids separate status/network/debug calls during failed dynamic-page
  flows.
- **Screenshot economy**: batched screenshots store upload artifacts and do not
  replay base64 image payloads into the model context by default.

Remaining browser gaps are explicit: provider abstraction (Browserbase/Firecrawl
/Camofox), raw gated CDP passthrough, downloads/uploads, live view, and stronger
browser hardening tests are still future phases.

### 0.1.0-dev.2026-05-06b — Context Compiler v1 for Codex

Captain now adds a first native context-compiler rail for Codex/OpenAI-Codex:

- **Compact prompt profile**: Codex uses a `CodexEconomy` prompt profile that
  preserves the core contracts (tool calls, discovery, memory, safety, config
  source-of-truth) while removing long explanatory sections that are expensive
  to replay on every turn.
- **Runtime context capsule**: live state injected after the prompt builder
  (channels, TTS truth, reflections, shared knowledge, mood, temporal hints)
  is compiled into a bounded capsule for Codex instead of raw multi-section
  prompt appendices.
- **Memory capsule**: recalled memories are injected as short background facts,
  not by replaying the full memory protocol again. Raw truth remains available
  through `memory_recall`, `session_recall`, `knowledge_query`, config tools,
  and domain tools.
- **Dynamic MCP/skill surfacing for Codex**: Codex starts with the small CORE
  tool set even when MCP/skill tools are connected. `capability_search` can
  still surface matching MCP/skill schemas for the next turn, so capability is
  preserved without replaying every connected schema by default.
- **Claude untouched**: the full prompt path remains the default for non-Codex
  providers. This is intentionally provider-scoped so Sonnet behavior is not
  changed by Codex token-economy work.

Critical note: this is not lossy gzip-style compression. Captain keeps the raw
sources outside the context window and gives the model explicit rehydration
rails when exact detail matters.

### 0.1.0-dev.2026-05-06a — Codex context economy

Captain now treats Codex context replay as a product-cost problem, not merely a
context-window overflow problem:

- **Codex-only compaction profile**: Codex sessions compact earlier than Claude
  sessions (`14` messages, keep `6` recent raw messages, lower token-pressure
  trigger). Claude/Anthropic thresholds are unchanged.
- **Request-time replay cap**: Codex LLM calls keep the canonical memory context
  plus a short recent tail instead of replaying large historical transcripts by
  default. The full session remains persisted; only the per-request context is
  economized.
- **Tool-aware estimates**: pre-call compaction estimates now include the visible
  tool schema surface, so discovery/core tools are counted in token-pressure
  decisions.
- **Quality guard**: the always-visible discovery rail remains intact:
  `capability_search`, `tool_search`, and `captain_docs` stay available so the
  model can retrieve exact capabilities when needed instead of carrying every
  tool schema in every turn.

Critical note: this is intentionally scoped to Codex/OpenAI-Codex providers.
Do not apply these more aggressive replay caps to Claude unless a separate
Claude regression justifies it.

### 0.1.0-dev.2026-05-05r — Browser diagnostics rail

Captain's native CDP browser rail now exposes more product-grade diagnostics:

- **Browser status**: `browser_status` inspects the current agent's browser
  rail without creating a session. It reports Chrome availability, the isolated
  profile directory, viewport, active session count, and current page metadata
  when a browser is already open.
- **Network journal**: `browser_network_log` returns a bounded ring buffer of
  recent CDP network request/response/failure events for the active browser
  session. Use it after navigation/click/wait failures to diagnose HTTP status,
  MIME type and loading errors.
- **Grouped tool routing**: the compact `exec` tool group now exposes browser
  `run_js`, `back`, `status`, and `network_log` actions so the diagnostics are
  available even when Captain uses grouped tool prompts.
- **Browser hand docs**: the bundled browser hand now describes Captain's
  native CDP stack rather than a Playwright abstraction, and lists the full
  browser tool set.

Critical note: this is diagnostic parity work, not a full HAR recorder or
anti-bot browser suite. Response bodies, downloads, uploads, multi-tab control
and browser profile management remain future browser hardening work.

### 0.1.0-dev.2026-05-05q — Autonomy CLI status

Captain now exposes a first autonomy overview from the CLI:

- **Autonomy status**: `captain autonomy status` aggregates daemon health,
  agents, configured channels, cron jobs, triggers, workflows, pending
  approvals, recent structured actions, and recent structured errors.
- **JSON mode**: `captain autonomy status --json` returns the same inventory in
  a scriptable shape for smoke tests and external status views.
- **Recent event filters**: `--lines` controls the number of recent
  actions/errors, and `--since` accepts the same duration/timestamp syntax as
  `captain logs`.
- **Cron visibility**: the overview highlights enabled/total jobs, next enabled
  jobs, and jobs with recent/consecutive errors.

Critical note: this is an operational overview, not yet a full autonomy
control-plane. It reads existing scheduler/trigger/workflow/approval surfaces
and local structured logs without introducing new autonomous behavior.

### 0.1.0-dev.2026-05-05p — Sessions CLI UX

Captain's persisted sessions are now easier to inspect and operate from the CLI:

- **Session subcommands**: `captain sessions list`, `current`, `resume`,
  `continue`, `search`, `export`, and `prune` are available.
- **Non-mutating session read**: `GET /api/sessions/{id}` can load a persisted
  session for export/search without switching the active agent session.
- **Better metadata**: session listings include `updated_at`/`last_active` and
  `context_window_tokens`, ordered by most recently updated first.
- **Search**: `captain sessions search <query>` checks metadata first, then scans
  recent session message text through the daemon API.
- **Export**: `captain sessions export <id> --format json|markdown` produces a
  user-facing session artifact.
- **Prune safeguards**: `captain sessions prune` refuses to delete anything
  unless `--keep` or `--older-than` is provided, and actual deletion requires
  `--yes`. Use `--dry-run` first.

Critical note: session export is intentionally user-facing and compact. It is
not a raw provider transcript dump and should not expose hidden reasoning
blocks as ordinary conversation content.

### 0.1.0-dev.2026-05-05o — Model/auth CLI visibility

Captain's model and provider authentication UX is more explicit:

- **Singular alias**: `captain model ...` is now an alias for the existing
  `captain models ...` command family.
- **Current model**: `captain model current` shows the active provider/model,
  config source, API-key env var, and configured fallbacks.
- **Provider test**: `captain model test [provider]` calls the daemon's native
  provider test endpoint. Without a provider, it tests the current provider.
- **Codex test correctness**: provider tests use the live current model when
  testing the current provider, and normalize Codex model ids so ChatGPT
  subscription-backed Codex is tested with `gpt-*` rather than `codex/gpt-*`.
- **Auth commands**: `captain auth status`, `captain auth doctor`, and
  `captain auth login <provider>` expose credential readiness without making
  users hunt through config, provider lists, and login commands.
- **Provider table fix**: `captain models providers` now renders the daemon's
  `/api/providers` response shape directly instead of dumping JSON when the
  endpoint returns `{providers,total}`.

Critical note: live provider tests can spend real provider quota. `auth doctor`
does not call a model unless `--test` is passed explicitly.

### 0.1.0-dev.2026-05-05n — Unified CLI logs

Captain's CLI log surface is now more useful for operations:

- **Daemon log by default**: `captain logs` now reads the daemon log
  (`~/.captain/captain.log`) instead of the TUI-only log.
- **Targeted logs**: `captain logs <target>` supports `daemon`, `tui`,
  `events`, `tools`, `agent`, `channel`, `errors`, and `all`.
- **Runtime event logs**: structured agent/tool/channel/error views read the
  `sessions_events` timeline table, so tool calls and failures can be inspected
  without opening raw session JSON.
- **Filters**: logs support `--since`, `--agent`, `--channel`, `--lines`,
  `--json`, and `--follow` where the source can be followed.

Critical note: this is still a CLI-first observability rail. It does not yet
replace a full product log service with indexed severities, retention policies,
and per-channel observability views.

### 0.1.0-dev.2026-05-05m — Operational status polish and PLAYBOOK source demotion

Captain's ops surface now exposes more of the runtime state and avoids a
costly legacy self-inspection pattern:

- **PLAYBOOK is not canonical**: `PLAYBOOK.md` remains a workspace artifact,
  but the agent should not use it as the source of truth to self-audit,
  compare Captain with another agent, or choose an operational tool. Use live
  runtime state, `captain_docs`, and `capability_search` instead.
- **Status paths are explicit**: `/api/status` now includes the home, data,
  config, log, workspace, workflow, and session paths. `captain status` should
  no longer show `Data dir: ?` when the daemon is running.
- **Verbose status**: `captain status --verbose` prints operational paths,
  runtime flags, configured channels, media rail state, and TTS source.
- **Full doctor inventory**: `captain doctor --full` adds a daemon inventory
  pass on top of health checks, closer to a product-grade ops view.

Critical note: if a user explicitly asks about the contents of `PLAYBOOK.md`,
reading that file is fine. For product comparisons and diagnostics, it is the
wrong authority.

### 0.1.0-dev.2026-05-05l — Telegram native image normalization

Captain now treats Telegram photos as durable inbound media before the agent
turn:

- **Image file retained**: incoming channel photos are downloaded into
  Captain's inbound media directory with MIME sniffing and a 10 MB cap.
- **Model-independent vision**: Captain asks the media engine for an automatic
  description before the LLM call, then passes the model the local path, MIME,
  caption, and description. Codex and other text-first models no longer receive
  only an ephemeral image notification.
- **Recoverable fallback**: if automatic vision fails, the prompt still includes
  the local path and an explicit instruction to use `media_describe`, so the
  image is not lost after the turn.

Critical note: this intentionally prefers a native channel pre-processing rail
over forcing every model driver to support inline image blocks. Direct
multimodal blocks remain useful, but channel input must be reliable first.

### 0.1.0-dev.2026-05-05k — TTS config source-of-truth and Codex context economy guard

Captain now treats `config.toml` as the runtime source of truth for TTS and
adds a stricter Codex context economy guard:

- **TTS config wins**: when `[tts].provider` is set, `text_to_speech`
  uses the configured provider and configured voice. Memories and tool
  arguments such as `voice:"nova"` or `voice_id` cannot override it.
- **TTS hot reload**: changes under `[tts]` now update the in-memory TTS
  engine during config reload; they no longer require a daemon restart to
  take effect.
- **Auditable voice output**: `text_to_speech` returns the provider, the
  actual voice/voice_id used, and whether that voice came from config or a
  request override.
- **Config/memory alignment**: the agent prompt now includes a small runtime
  config truth section for TTS. If memory conflicts with config, the agent
  must trust config and update memory only after config changes.
- **Codex context economy**: Codex/OpenAI-Codex turns now use a stricter
  tool-result replay budget so stale medium-sized tool outputs do not inflate
  simple requests by tens of thousands of tokens.

Critical note: this is intentionally stricter for Codex because large context
does not make stale tool replay free. Quality should come from fresh,
targeted tool calls and explicit re-queries, not hidden replay of old blobs.

### 0.1.0-dev.2026-05-05j — Telegram long-stream and voice reply hardening

Captain now closes two Telegram product issues seen on long, tool-heavy Codex
turns and voice-response flows:

- **Long stream cadence**: Telegram live edits now respect the time cadence
  even after the text buffer crosses the usual edit threshold. This avoids
  hammering `editMessageText` on every delta and triggering platform
  flood-control during long research/tool chains.
- **Final delivery after flood-control**: normal `sendMessage` delivery now
  honors Telegram `retry_after` responses before failing the chunk. A final
  response is no longer immediately retried as plain text while the channel is
  still rate-limited.
- **ElevenLabs TTS voice routing**: OpenAI voice aliases such as `nova` or
  `alloy` are ignored when the active TTS provider is ElevenLabs; Captain uses
  the configured ElevenLabs `voice_id` unless an explicit `voice_id` is passed.
- **Native audio upload**: local audio files sent through `channel_send` are
  delivered as Telegram audio/voice media instead of generic documents. MP3
  TTS output uses `sendAudio`; OGG/Opus uses `sendVoice`.

Critical note: this targets Telegram delivery and TTS routing only. The STT
pre-processing from `0.1.0-dev.2026-05-05i` remains the inbound voice path.

### 0.1.0-dev.2026-05-05i — Telegram voice STT and first-turn routing guard

Captain now treats Telegram voice messages as a channel-native input instead
of asking the LLM to discover transcription tooling:

- **Voice pre-processing**: inbound Telegram voice files are downloaded into
  Captain's inbound directory before the agent call, capped at 25 MB, then
  transcribed through the configured media STT rail when available.
- **ElevenLabs STT**: audio transcription now supports ElevenLabs Scribe via
  `ELEVENLABS_API_KEY` (`scribe_v2` by default, override with
  `ELEVENLABS_STT_MODEL`), in addition to Parakeet MLX, Groq Whisper, and
  OpenAI Whisper.
- **LLM prompt quality**: when transcription succeeds, the agent receives the
  transcript directly plus the saved audio path for audit/retry. If
  transcription fails, Captain still provides the local path and a clear
  fallback instruction.
- **Routing guard**: the first LLM interaction in a fresh session now uses the
  agent's configured primary model before complexity routing can optimize
  later turns.

Critical note: this fixes the product path for channel voice inputs. The
`media_transcribe` and `speech_to_text` tools remain available for explicit
file transcription and fallback workflows.

### 0.1.0-dev.2026-05-05h — Native image generation parity pass

Captain's `image_generate` tool now moves closer to a complete product rail:

- **Multi-provider image generation**: `provider=auto` can use FAL.ai via
  `FAL_KEY` for fast multi-model generation, or OpenAI Images via
  `OPENAI_API_KEY`.
- **FAL model rail**: supports the main fast image models including
  `fal-ai/flux-2/klein/9b`, `fal-ai/flux-2-pro`,
  `fal-ai/gpt-image-1.5`, `fal-ai/nano-banana-pro`,
  `fal-ai/ideogram/v3`, `fal-ai/recraft/v4/pro/text-to-image`, and
  `fal-ai/qwen-image`.
- **Current OpenAI model aliases**: adds `gpt-image-2`,
  `gpt-image-1.5`, and `gpt-image-1-mini` alongside the existing
  `gpt-image-1`, `dall-e-3`, and `dall-e-2` path.
- **Artifact parity**: FAL URL outputs are downloaded, size-bounded,
  base64-normalized, saved into `output/`, and exposed through the same web
  upload preview path as OpenAI-generated images.

Critical note: this is direct `FAL_KEY` / `OPENAI_API_KEY` support. The
managed FAL gateway is a separate product capability; Captain
does not yet proxy image generation through a managed subscription gateway.

### 0.1.0-dev.2026-05-05g — Telegram stream parse recovery and heartbeat timing

Captain's Telegram stream path now closes two product-grade edge cases seen
with long Codex turns:

- **Edit fallback parity**: `editMessageText` now retries strict HTML parse
  failures as plain text, matching the existing `sendMessage` delivery
  fallback. This protects live streams from agent/research markers such as
  `<<<EXTCONTENT_...>>>` being interpreted as Telegram HTML.
- **Stronger HTML sanitizer**: unknown tags and nested angle brackets are
  escaped recursively enough that marker-shaped text cannot leak as a raw
  Telegram start tag.
- **Heartbeat lifecycle**: Telegram visible progress messages are now owned
  by the streaming bridge and aborted before any final fallback response is
  posted. The first visible heartbeat is delayed to 75 seconds, avoiding
  "still working" messages on normal 30-60 second answers while preserving a
  signal for genuinely long turns.

Critical note: this only changes the Telegram streaming rail. Non-Telegram
channels keep the existing generic long-turn heartbeat, and healthy Telegram
streams still use rich live edits when Telegram accepts the HTML.

### 0.1.0-dev.2026-05-05f — Telegram streaming hardening for Codex

Captain's Telegram live stream is now safer when the active model emits
Codex-style large deltas or when Telegram rejects a progressive edit.

- **Complete-answer fallback**: if the Telegram stream pump fails after a
  partial live render, Captain sends the canonical final response as a
  normal Telegram message instead of suppressing the final send.
- **Large first-delta split**: the shared stream consumer now splits an
  oversized first text delta before opening the live message, so Telegram's
  4096-character cap cannot leave the consumer editing the wrong chunk.
- **Final flush split**: an oversized final body is split before the last
  edit/send, keeping each Telegram body under the platform limit.
- **HTML delivery fallback**: when `sendMessage` rejects a rich HTML chunk,
  Captain retries that chunk as plain text before surfacing an error.

This is intentionally mostly invisible for healthy Claude/Sonnet Telegram
streams. It targets failure modes seen more often with Codex due to slower
tool loops and less predictable streaming chunk shape, while preserving the
same live UX when Telegram accepts edits normally.

### 0.1.0-dev.2026-05-05e — Native document generation rail

Captain can now create document artifacts through the deferred builtin
`document_create`.

- **Native formats**: `pdf`, `docx`, `html`, and `markdown`.
- **Structured input**: Markdown-like content plus optional structured
  `sections`, tables, bullets, and `citations`.
- **No external binary dependency**: the base PDF/DOCX renderers are built into
  the runtime so a fresh install can produce a usable report without `pandoc`,
  `typst`, Chrome, or wkhtmltopdf.
- **Product guardrails**: output paths stay inside the workspace sandbox,
  existing files are protected unless `overwrite=true`, and obvious raw secrets
  are rejected before writing.
- **Agent workflow**: use `document_create` for polished deliverables, then
  `channel_send` with `file_path` when the artifact should be delivered
  to a chat channel.

Critical note: this is the reliable base rail for reports, summaries, memos,
invoices and handoff documents. Highly branded publishing, complex typography,
charts, PDF/A/PDF/UA conformance and visual QA still belong to a dedicated
document skill or a future Typst-backed premium renderer.

### 0.1.0-dev.2026-05-05d — Codex token economy: small CORE, dynamic tools, compact tool history

Captain now treats tool schemas and tool outputs as a first-class context
budget problem, with a Codex-friendly default:

- **Small permanent CORE**: only `capability_search`, `tool_search`,
  `captain_docs`, `ask_user`, `memory_save`, `memory_recall`,
  `session_recall`, and `system_time` are always visible by default.
  Domain tools remain available, but are deferred until discovery.
- **Dynamic deferred-tool surfacing**: when `capability_search` or
  `tool_search` returns a deferred builtin, the runtime adds the real
  tool schema to the next LLM turn. The agent should call
  `capability_search` when the capability is absent from CORE, ambiguous,
  or before claiming it lacks access. It should not call discovery for
  simple chat or when a visible CORE tool is already clearly enough.
- **RTK-inspired tool-result compaction**: large/noisy outputs from shell,
  SSH, process, package, and similar tools are deduplicated and summarized
  before re-entering model context. Signal lines such as errors, failed
  services, load, ports, and running status are preserved before head/tail
  truncation.
- **Cache telemetry is visible**: usage records and the TUI session footer
  now track cached input and cache-creation tokens separately. The footer
  uses current-turn context pressure instead of cumulative session tokens,
  so a long session no longer looks like every small prompt is sending the
  whole transcript again.
- **Delegation guardrail**: `agent_spawn` / `agent_delegate` descriptions
  now frame delegation as a budgeted, independent, verifiable workflow, not
  a reflex. Delegation can save context only when scoped; otherwise it can
  increase total token use.

Product note: this is the native Captain version of the useful parts of
schema minimization, RTK-style context compression, and critical
delegation. It preserves quality by keeping discovery explicit and by
retaining high-signal tool output instead of blindly clipping raw logs.

### 0.1.0-dev.2026-05-05c — RBAC pass 3: 24 remaining adapters gated

The B.8 deny-by-default contract now reaches every channel adapter
that carries a sender identity. The previously-unguarded surfaces
listed at the bottom of `2026-05-05b` are all closed:

- **Live gate enforced** (22 adapters, all on the inbound hot path):
  `twitch`, `reddit`, `mastodon`, `bluesky`, `revolt`, `threema`,
  `pumble`, `flock`, `zulip`, `google_chat`, `nextcloud`, `guilded`,
  `keybase`, `nostr`, `webex`, `twist`, `discourse`, `dingtalk`,
  `dingtalk_stream`, `webhook`, `linkedin`, `rocketchat`. Each one
  follows the same pattern: `<Channel>Config::allowed_users:
  Vec<String>` with `#[serde(default)]`, the constructor takes the
  list as a new argument, and the parse / poll / WebSocket handler
  calls `crate::rbac::is_authorized` immediately after the bot/self
  filter, before any other content extraction or topical allowlist.

- **Wiring completed for an existing config field** (1 adapter):
  `feishu`. The `allowed_users` field had been declared on
  `FeishuConfig` in pass 1 but never reached the adapter — V2 schema
  events called `parse_event` without it and the V1 legacy path had
  no gate at all. `FeishuAdapter::with_config` now takes the list as
  its 9th argument, the V2 path threads it into `parse_event`, and
  the V1 path runs the same `rbac::is_authorized` check on
  `event.open_id` before producing a `ChannelMessage`. Both paths
  obey the same B.8 contract.

- **Gate field exposed for a stub adapter** (1 adapter): `xmpp`.
  `XmppAdapter::start()` returns an error explaining that the
  `tokio-xmpp` dependency is required, so there is no inbound stanza
  pipeline today. The config field is added now to keep
  `CHANNEL_REGISTRY` uniform across the channel system, and the
  struct-level docstring records that the future stream handler MUST
  consult `self.allowed_users` via `rbac::is_authorized(&allowed,
  from_jid)` before forwarding messages.

- **Two adapters intentionally skipped** (push-only, senderless):
  `gotify` and `ntfy`. Both deliver server-generated push
  notifications — `parse_ws_message` returns
  `(app_id, title, message, priority, date)` for Gotify and
  `parse_sse_data` returns `(topic, message, title, sender_optional)`
  for ntfy. Neither carries a verifiable human sender, so there is
  no RBAC surface to gate. They are documented here as deliberate
  exclusions, not oversights.

After this pass, the channel matrix is **40 adapters with the B.8
gate live + 2 push-only adapters skipped + 1 stub with the gate
plumbed**. Every channel a `channel_reconfigure` call can activate
now obeys deny-by-default.

Per-adapter sender id used for the rbac match (chosen to match what
operators write in their `config.toml`):

| Adapter | sender match | Notes |
|---|---|---|
| twitch | IRC nick | exact, case-sensitive |
| reddit | `data.author` | skips `[deleted]` / `[removed]` |
| mastodon | `account.acct` | not the opaque internal id |
| bluesky | `author.handle` | not the DID |
| revolt | `data.author` | _id |
| threema | `from` | gateway id |
| pumble | `event.user` / `event.user_id` | |
| flock | `message.from` | `u:user123` form |
| zulip | `sender_email` | inline parse |
| google_chat | `message.sender.name` | `users/12345` form |
| nextcloud | `actorId` | |
| guilded | `message.createdBy` | |
| keybase | `sender.username` | |
| nostr | `event.pubkey` | per-relay loops |
| webex | `personId` of fetched message | |
| twist | `comment.creator` | stringified |
| discourse | `post.username` | |
| dingtalk | `senderId` | |
| dingtalk_stream | `senderStaffId` (with `senderId` fallback) | |
| webhook | `body.sender_id` | parse default `"webhook-user"` is denied unless listed |
| linkedin | `element.from` member URN | `urn:li:person:abc` |
| rocketchat | `msg.u._id` | username still used for self-filter |
| feishu | `sender.sender_id.open_id` | V2 + V1 paths |
| xmpp | future: stanza `from` JID | stub, gate plumbed only |

Tests: every adapter ships two new RBAC tests
(`test_<adapter>_rbac_empty_denies` and `_rbac_explicit_list`), and
every existing constructor / parse-function call site is updated to
pass `&["*".to_string()]` so legacy semantics are preserved in tests.

CHANNEL_REGISTRY in `routes.rs` exposes `allowed_users` as a
list-valued field for every newly-gated channel, with a
production-flavoured `config_template` that includes
`allowed_users = ["*"]  # tighten to specific … in production`.

**Migration note for users** (same as the `2026-05-05a` migration
note): a config that previously omitted `allowed_users` now denies
every sender. Add `allowed_users = ["*"]` to the relevant
`[channels.<name>]` section to restore the legacy permissive
behaviour. Public release gate is now **unblocked** from the RBAC
side — every channel adapter that exposes a human sender enforces
deny-by-default.

### 0.1.0-dev.2026-05-05b — RBAC pass 2: viber + messenger + gitter + mumble

Four more wired-up adapters now enforce the B.8 deny-by-default contract,
bringing the gated count to 16/38 (telegram, discord, signal, whatsapp,
slack, matrix, email, teams, mattermost, irc, line, wecom, viber,
messenger, gitter, mumble).

`<Channel>Config::allowed_users: Vec<String>` is `#[serde(default)]`,
constructors take the list, the parse function checks
`rbac::is_authorized` after the bot/self filter and before any other
allowlist (room/channel/tenant). For Mumble, the gate is on the synthetic
`session-{actor}` since the protobuf TextMessage carries a session id
rather than a stable user id.

22 adapters still wait for the same gate:
google_chat, twitch, rocketchat, zulip, xmpp, reddit, mastodon, bluesky,
revolt, nextcloud, guilded, keybase, nostr, webex, gotify, threema,
dingtalk, dingtalk_stream, discourse, pumble, flock, twist, ntfy,
webhook, linkedin, feishu (Feishu has the field but no enforcement
yet — wiring missed). All of them remain a `channel_reconfigure` away
from being live; until the next pass lands, treat the public release
gate as **blocked**.

### 0.1.0-dev.2026-05-05a — RBAC gate on every wired-up channel adapter

Eight channel adapters previously delivered every inbound message to the
agent regardless of who sent it. A workspace member outside the operator's
allowlist could execute `/clear`, `/compact`, `/stop`, `/usage`, `/think`,
`agent_send` and friends. The four already-guarded adapters (telegram,
discord, signal, whatsapp) now have eight new neighbours:

- `slack` — `parse_slack_event` checks `rbac::is_authorized` after the
  bot/self filter.
- `matrix` — `/sync` loop gates by `event["sender"]`.
- `email` — `allowed_senders` semantics inverted from "empty allows all" to
  "empty denies all"; `["*"]` is the explicit opt-in. Domain-prefix
  matching (`["@example.org"]` admits the whole domain) is preserved.
- `teams` — `parse_teams_activity` checks `from.id` between the bot/self
  filter and the tenant filter.
- `mattermost` — `parse_mattermost_event` checks `user_id` between the
  bot filter and the channel filter.
- `irc` — `parse_privmsg` checks the sender nickname.
- `line` — webhook handler checks `source.userId` after signature
  verification, before building the reply context.
- `wecom` — webhook callback handler checks `FromUserName` immediately
  after decoding the XML body, before any branch in the message flow.

Every config struct gains an `allowed_users: Vec<String>` field with
`#[serde(default)]`, and every adapter constructor takes the list as a new
argument. Empty list denies (B.8 contract); `["*"]` opts back into
permissive intake.

**Migration note for users**: a config that previously omitted
`allowed_users` now denies every sender. Add `allowed_users = ["*"]` to
the relevant `[channels.<name>]` section to restore the legacy permissive
behaviour. The change is intentional — the silent permissive default is
the exact failure mode B.8 was created to prevent.

Tests: 3520+ workspace-wide tests stay green; new RBAC-specific tests
pin the deny-by-default contract and the `["*"]` opt-in for each
adapter.

### 0.1.0-dev.2026-05-04i — Workspace + todo hardening (Codex review)

Follow-up to entries `2026-05-04g`, `2026-05-04h`. Addresses four points
flagged by an out-of-tree review of the workspace and todo surfaces:

- **Single source for the credential blocklist.** The CLI's
  `validate_extra_paths` and the kernel's
  `KernelHandle::blocked_workspace_paths` both read from
  `captain_kernel::default_blocked_workspace_paths(captain_home)`. Adding a
  new entry (`~/.aws/credentials`, …) only needs to land in one place.
- **`extra_paths` in `.captain.toml` actually reach the kernel.** A new
  thin route `POST /api/workspace/add` lets daemon-mode TUIs apply paths
  at bind time; in-process mode still calls `add_workspace_path` directly.
  Failures degrade to `warn!` — `extra_paths` is best-effort.
- **`~/.ssh` written literally in `.captain.toml` is no longer accepted.**
  The sandbox now expands a leading `~` before canonicalising, so the
  string match against the blocklist works whether or not `canonicalize`
  succeeds.
- **`todo_list` is paginated.** `limit` defaults to 200, hard-capped at
  1000. A user with thousands of todos cannot blow the LLM context window
  with a single tool call. The cap is documented in the input schema so
  Captain can opt into a larger page when really needed.

### 0.1.0-dev.2026-05-04h — Cross-session todos + per-project `.captain.toml`

Two complementary additions to make Captain feel "at home" inside a project
and across daemon restarts:

- **`.captain.toml`** at the root of a project (or any ancestor up to `$HOME`)
  auto-binds the TUI to the right agent. Recognised keys: `agent` (uuid),
  `agent_name`, `project_slug`, `tool_profile`, `extra_paths`. The walk stops
  at `$HOME` so a stray system-wide config cannot contaminate a project that
  did not opt in. `extra_paths` is sandboxed against the credential blocklist
  (`~/.ssh`, `secrets.env`, `vault.enc`, …) before any path reaches the
  kernel. A new `captain config workspace` subcommand prints the resolved
  config so an operator can debug the bind without launching the TUI.
- **Cross-session todos** as a new `scheduling` family surface:
  `todo_create`, `todo_list`, `todo_complete`, `todo_reopen`, `todo_delete`.
  Stored in the new `todos` table inside `~/.captain/data/captain.db`
  (migration v19), so items survive both daemon restarts and conversation
  compactions. Deliberately minimal — no priority, no tags, no project FK,
  no agent FK. Heavier intents stay in `cron_create` (timed),
  `goal_create` (autopilot loops), or `project_task_*` (project DAGs).

### 0.1.0-dev.2026-05-04g — File-change triggers exposed to the agent + observability + sandbox

The `file-change trigger` system shipped in 2026-05-04b but was wired only at
the REST layer — Captain itself could not arm a watcher. That gap, plus a
silent observability hole (the fire log was emitted at `debug!` while the
daemon defaults to `info!`), made the feature look broken in live tests.

Concrete changes shipping in this build:

- **Agent-facing tools** (`scheduling` family, deferred): `file_trigger_register`,
  `file_trigger_list`, `file_trigger_set_enabled`, `file_trigger_remove`. Captain
  can now react to filesystem events without delegating to the user. See
  [`scheduling.md`](scheduling.md) for the full spec.
- **Path sandbox**: `file_trigger_register` reuses the kernel's
  `blocked_workspace_paths()` and refuses any path that resolves inside
  `~/.ssh/`, `~/.gnupg/`, `~/.captain/secrets.env`, `~/.captain/secrets-backups/`,
  `~/.captain/vault.enc`, `~/.captain/.env*`. The error names the violated
  prefix.
- **Lazy canonicalisation**: a path that does not exist yet is now legal — the
  watcher arms on the closest existing ancestor. Use this to react to a file
  the moment it appears.
- **Observability**: trigger fires log at `info!` level with `trigger_id`,
  `agent_id`, `path`, `kind`. The dispatch path also emits a per-fire summary
  so the chain fire → send_message is visible in `captain logs`.
- **Boot-time hardening**: persisted triggers whose paths have vanished while
  the daemon was down are auto-disabled at load with a `warn!` line naming the
  casualty. Triggers that fail to arm at boot are auto-disabled too — the
  persisted state can no longer claim `enabled = true` while the watcher is
  dead.

### 0.1.0-dev.2026-05-04f — Skill curator daily background pass

A new built-in cron `skill_curator` runs daily at 03:00 Europe/Paris with
silent delivery (no `channel_send`). The cron prompts Captain to scan the
installed skills, flag candidates (idle > 30 d, failure_rate > 50 %,
semantic duplicates), and queue `skill_refinement_propose` for each.

Inspired by curator-style maintenance, but routed entirely
through the existing `skill_refinement_*` approval rail — no skill is
ever modified silently. A per-run report lands in
`~/.captain/data/curator-reports/<YYYY-MM-DD>.md` for after-the-fact
review.

To opt out, flip `enabled = false` on the cron (don't delete: the
builtin is idempotent and gets re-created on the next boot).

### 0.1.0-dev.2026-05-04e — Runtime changelog now triggers the boot-time update notice

The fingerprint Captain compares at every boot (kernel commit pending)
now includes a BLAKE3 hash of this very file. Concrete consequence: any
edit to `runtime-changelog.md` shipped in a new build flips the
fingerprint and triggers `runtime_update_notice`, which already tells
Captain to read `captain_docs({family:"runtime-changelog"})` before
acting on stale assumptions.

Before: a changelog-only PR (no other binary diff that affected size or
mtime sufficiently) might not flip the fingerprint, so Captain could
miss the update.

Now: every meaningful entry here is guaranteed to surface as a boot
notice on the next install.

### 0.1.0-dev.2026-05-04d — Channel hot-reload, model_switch rail Telegram, TTL plans, RBAC matrix

Channel runtime is fully hot: a channel that was never booted (no
section in `config.toml` at startup, or section present but token
absent) can be brought up without restarting the daemon.

Agent-facing changes:

- **`secret_write` mirrors into `std::env`** (kernel commit `4a8ad2ef`).
  After you write a new token via the agent tool, `read_token` (used
  by the channel bridge) sees it immediately. No daemon restart
  needed for the new value to be visible to channel adapters.
- **Hot-reload re-reads `secrets.env`** (api commit `8ec38058`).
  When you call `channel_reconfigure`, the listener re-injects every
  key/value from `~/.captain/secrets.env` into `std::env` before
  rebuilding the bridge. Covers the case where someone edits
  `secrets.env` by hand instead of going through `secret_write`.
- **`get_channels_context` distinguishes ACTIVE vs CONFIGURED**
  (kernel commit `aa089b89`). The block injected into your prompt
  now reports `ACTIVE` only when the adapter is live in
  `kernel.channel_adapters`; `CONFIGURED` means TOML section exists
  but no live adapter (token missing or boot skipped). Don't trust
  a plain "ACTIVE" anymore — read the literal label.
  - When you see `CONFIGURED`: post the missing secret with
    `secret_write` then call `channel_reconfigure({channel})`.
- **Brand-new channel workflow**:
  `secret_write` → `config_setup`/`config_write` →
  `channel_reconfigure({channel})` → `channel_send`. Detailed in
  `docs/captain-tools/channel.md` under "Adding a brand-new channel".
- **`channel_reconfigure` description updated** (runtime commit
  `e1d66474`) to cover both ROTATION (existing token change) and
  BOOTSTRAP (channel never booted) — same tool, two cases.

Telegram safe model-switch:

- **Rail `model_switch_plan` → `model_switch_apply` exposed via
  Telegram inline keyboard** (commit `e97a73f3`). When you receive
  `/model X` from Telegram, the bridge now calls `model_switch_plan`
  first; if `session_strategy_required = true`, an inline keyboard
  with `Nouvelle session` / `Resume compact` / `Annuler` is sent and
  the user picks. The callback `model:{plan_id}:{choice_id}` then
  drives `model_switch_apply`. Same safe preflight as the TUI.
- **Pending plans expire after 5 minutes** (commit `8e25fb3a`).
  If the user clicks an old button after the TTL, the callback
  returns `Ce choix de switch a expiré (5 min). Relance /model <modèle>.`
  Don't tell the user to click again — relaunch a fresh `/model X`
  to issue a new plan.

Streaming default:

- **`[channels.telegram] streaming = true` is now the default** (commit
  `fe09d35e`). New installs and configs without an explicit `streaming`
  field get live message rendering.

Slash commands routing:

- **`handle_command` resolves the agent on the actual channel**
  (commit `9cf43dfe`). Previously `/new`, `/clear`, `/compact`,
  `/model`, `/stop`, `/usage`, `/think` always looked up the agent
  via `ChannelType::CLI`, which made them ignore Telegram/Discord
  bindings posted via `/agent`. The slash router now uses the
  channel from which the command originated.

RBAC coverage:

- **Matrix documented** in `docs/captain-tools/channel.md` under
  "RBAC coverage matrix". `/model` is double-protected on Telegram
  (adapter parse + kernel-level `authorize_channel_user`). Other
  sensitive commands (`/new`, `/clear`, `/compact`, `/stop`, `/usage`,
  `/think`) rely on adapter-level RBAC only — currently enforced for
  Telegram, Discord, Signal, Email, WhatsApp; the remaining adapters do
  not yet gate. If you act on a non-gated channel, treat any
  authenticated user with caution.

### 0.1.0-dev.2026-05-04c — Telegram streaming on by default

Agent-facing change:

- `[channels.telegram] streaming` now defaults to `true`. New installs (and existing configs that never set the field explicitly) get live message rendering: each text segment is edited in place with a progress cursor, tool calls land as separate intercalated bubbles, and a 4096-byte cap split happens automatically.
- To opt out, set `streaming = false` under `[channels.telegram]`.
- Existing configs that already set `streaming = true` or `streaming = false` keep their explicit value.

If a user reports that Telegram replies feel different (live edits vs single final block), the new default is the cause.

### 0.1.0-dev.2026-05-04b — Visible skill proposals

Agent-facing changes:

- Reflection-generated skill proposals now emit `SkillProposalQueued` instead of only landing silently in the review database.
- The event carries the origin channel when known, so CLI and Telegram can show the proposal in the active conversation.
- Telegram has dedicated skill proposal actions: `/skill_proposals`, `/skill_approve <id>`, `/skill_reject <id>`, plus inline buttons on routed proposal prompts.
- CLI daemon chat sends `channel_type:"cli"` so self-improvement feedback keeps the correct origin.
- Generated skills remain critical durable changes: Captain must not write them without explicit approval.

How to answer the user:

- If a repeatable workflow is detected, tell the user what skill was proposed and that it awaits approval.
- Use `skill_proposal_list` to inspect pending proposals and `skill_proposal_decide` to approve/reject. On Telegram, the slash commands above are equivalent user-facing controls.
- If the user asks whether learning/self-improvement is visible, explain that memory commits use `MemoryStored`, memory approvals use `MemoryQueued`, and skill proposals use `SkillProposalQueued`.

### 0.1.0-dev.2026-05-04a — CLI table rendering and capability-first action routing

Agent-facing changes:

- The CLI/TUI Markdown renderer now handles Markdown tables. Two-column status tables render as copy-friendly `Label: value` lines instead of glued cell text.
- Fresh actionable requests should start with `capability_search` unless they are pure conversation, slash commands, trivial no-tool answers, or an immediate continuation of an already routed task.
- SSH alias resolution accepts natural-language phrases by matching safe alias tokens, then fails closed if several aliases match.
- `shell_exec` is explicitly documented as the wrong first move for Captain SSH/vault diagnosis. Prefer `capability_search`, native SSH tools, and the SSH docs family.

How to answer the user:

- For a new task like checking a remote server, first call `capability_search` with the user's task, then use the selected native tools.
- If an SSH alias phrase fails, read the tool error. Retry exact aliases named by the error or call `captain_docs({family:"ssh"})`; do not inspect Captain's vault through shell commands.
- Markdown tables are safe to use in CLI replies, but short bullet/key-value output remains better when the user may copy command-like content.

### 0.1.0-dev.2026-05-03m — Channel-neutral safe model switch

Agent-facing changes:

- `model_switch_plan` and `model_switch_apply` are now core visible tools. Do not route model/provider changes through shell commands, raw config writes, or secret probing.
- A successful `model_switch_plan` stores a short-lived pending choice so channels outside the TUI can complete the same flow when the user replies `Nouvelle session` or `Résumé compact`.
- The kernel now detects clear natural-language model switch requests before invoking the LLM. Example: switching from Codex back to Anthropic Sonnet prepares the safe switch directly, then waits for the user's session strategy.
- Telegram and API messages therefore get the same two-step safety behavior as the TUI: prepare switch, ask strategy, apply on the next short reply.
- `captain models set` now routes through the safe model-switch API when a principal agent is running, instead of writing only `default_model.model`.

How to answer the user:

- If the user asks to change Captain's default model/provider, use the safe switch rail only.
- If Captain already shows a prepared switch prompt and the user replies `Nouvelle`, `Nouvelle session`, `Résumé compact`, or `Annule`, treat that reply as the model-switch decision.
- Never ask for provider secrets manually before `model_switch_plan`; the plan reports driver/auth readiness.

### 0.1.0-dev.2026-05-03l — Codex request contract and supported model routing

Agent-facing changes:

- Codex OAuth requests no longer send `max_output_tokens`; the ChatGPT/Codex backend currently rejects that parameter.
- The Codex OAuth catalog is now hydrated from the official local Codex cache at `~/.codex/models_cache.json` when present. Static Codex models are only a fallback.
- `codex/o4-mini` was removed from the Codex OAuth fallback catalog and aliases because the backend returns: `The 'o4-mini' model is not supported when using Codex with a ChatGPT account.`
- Default routing for a Codex principal agent uses accessible cached Codex models when available: simple prefers `codex/gpt-5.4-mini`, medium prefers `codex/gpt-5.4`, and complex uses the selected Codex model.
- Restored agents repair stale Codex routing if an older manifest still points to `codex/o4-mini` or another unavailable Codex model.
- Fallback no longer continues silently after request-contract errors such as HTTP 400/422. These are implementation/configuration issues and must be surfaced.

How to answer the user:

- If Codex says a model is unsupported under a ChatGPT account, do not switch providers silently. Choose a supported Codex model or ask the user to switch provider explicitly.
- Prefer the live/dynamic Codex model list over remembered model names or old docs.
- Do not claim `o4-mini` is available through provider `codex`. Use provider `openai` for OpenAI API models, provider `codex` for ChatGPT/Codex OAuth models.
- Treat the older 2026-05-03i Codex routing note as superseded for the simple tier.

### 0.1.0-dev.2026-05-03k — Codex OAuth chat login and corrected readiness

Agent-facing changes:

- Codex OAuth readiness no longer rejects valid ChatGPT device-code tokens solely because `api.responses.write` is absent. Live validation showed connector-scoped ChatGPT tokens can call the Codex Responses endpoint.
- New chat tools: `codex_auth_status`, `codex_login_start`, and `codex_login_poll`.
- If Codex is not authenticated, Captain should guide the user in the current chat: start the device-code login, show `verification_url` and `user_code`, poll after the user validates, then run `model_switch_plan`/`model_switch_apply`.
- `captain login codex --with-model` now includes `gpt-5.5` and keeps Codex OAuth separate from OpenAI API keys.

How to answer the user:

- If the user asks to switch to Codex and auth is missing, do not abandon and do not silently fallback. Use `codex_login_start`, tell the user to validate the code, then use `codex_login_poll`.
- After successful login, ask/confirm `new_session` vs `compact_session` when provider context migration is needed, then use `model_switch_apply`.
- Treat the older 2026-05-03j scope-gate note as superseded by this entry.

### 0.1.0-dev.2026-05-03j — Codex OAuth scope gate and auth fallback hardening

Superseded by `0.1.0-dev.2026-05-03k` for Codex scope readiness. Keep only the fallback-auth rule from this entry.

Agent-facing changes:

- Superseded: do not reject Codex OAuth solely because `api.responses.write` is absent.
- Provider `codex` no longer borrows OpenAI API key auth. OpenAI API and Codex OAuth are separate provider paths.
- If the primary LLM provider fails due to missing/invalid authentication, Captain now surfaces the auth/config error instead of silently continuing through fallback providers.
- Fallback providers remain appropriate for transient rate-limit/overload failures, not for broken credentials.

How to answer the user:

- Use the newer `0.1.0-dev.2026-05-03k` instructions for Codex OAuth readiness and chat login.
- Do not claim Codex is active when an OpenRouter/OpenAI fallback produced the answer.
- If the user needs OpenAI API mode, use provider `openai`; if the user needs ChatGPT/Codex OAuth mode, use provider `codex`.

### 0.1.0-dev.2026-05-03i — Codex OAuth routing isolation and GPT-5.5 catalog entry

Agent-facing changes:

- `codex/gpt-5.5` is now a built-in catalog model with alias `codex-5.5`.
- Codex OAuth routing is isolated from OpenAI API routing. A `codex/*` or provider `codex` model must route to Codex models, not to OpenAI API fallback models.
- Superseded by `0.1.0-dev.2026-05-03l`: Codex simple routing must not use `codex/o4-mini`; use `codex/gpt-5.4`.
- When the principal Captain agent switches model/provider through the safe rail, `[default_model]` in `config.toml` is updated too. Specialized agents can still have their own model without changing the global default.
- `config_read("default_model.*")`, `/api/status`, and `/api/config` should report the effective runtime default, including safe model-switch updates applied after boot.

How to answer the user:

- If the user asks to switch Claude -> Codex, use the safe model-switch rail and verify the target with live model metadata.
- If the user asks what model/provider is active for Captain, verify both live agent metadata and `default_model.*`; they should match after a successful principal switch.
- Do not infer that a Codex `gpt-*` model needs `OPENAI_API_KEY`; Codex OAuth and OpenAI API are separate auth paths.
- If self-reporting contradicts live metadata/config, say the self-report is unreliable and verify through the API/runtime state.

### 0.1.0-dev.2026-05-03h — TUI quick-action prompt consolidation

Agent-facing changes:

- The TUI now uses one local quick-action prompt renderer/resolver for approval decisions and safe model-switch decisions.
- Approval prompts support mouse clicks when mouse mode is enabled, in addition to keyboard choices.
- The existing approval semantics are unchanged: once, session, always, or reject.
- The existing safe model-switch rail is unchanged: model/provider changes still require an explicit session strategy when needed.

How to answer the user:

- If the user asks why approval/model-switch prompts look more consistent, explain that the TUI decision layer was consolidated.
- If the user asks whether Telegram uses this generic action layer yet, say no: this entry covers the local TUI only. Telegram generic action cards remain future work.

### 0.1.0-dev.2026-05-03g — TUI model switch decision prompt

Agent-facing changes:

- The TUI no longer asks the user to retype `/model ... --new` or `/model ... --compact` when a provider/model switch requires a session strategy.
- It opens an in-chat decision prompt with `Nouvelle session` and `Resume compact`.
- The prompt accepts mouse clicks when mouse mode is enabled, keyboard shortcuts `1` / `2`, Enter for the recommended option, and natural answers such as `nouvelle session`, `garde le contexte`, `resume`, or `annule`.
- The safe rail is unchanged: the actual mutation still goes through `model_switch_plan` then `model_switch_apply`.

How to answer the user:

- If the user switches model/provider from the TUI and a choice is required, tell them to use the visible prompt instead of typing a manual command.
- If mouse mode is off, tell them to press `1` for a fresh session, `2` for a compact summary, or type the choice naturally and press Enter.
- Do not bypass the safe model-switch rail.

### 0.1.0-dev.2026-05-03f — Safe model/provider switch rail

Agent-facing changes:

- Changing the principal agent's model/provider is now a safe two-step flow: `model_switch_plan` then `model_switch_apply`.
- `model_switch_plan` is read-only and reports auth readiness, model/provider capability, active context, risk, blockers, and whether a session strategy is required.
- `model_switch_apply` requires `session_strategy`: `new_session` starts clean; `compact_session` stores a provider-neutral context summary, then starts a new active session.
- Raw `config_write` calls to `default_model.provider` and `default_model.model` are refused from the tool layer. Use the model-switch rail instead.
- `self_configure` no longer silently changes model/provider without an explicit session strategy.
- Saving a new provider key no longer silently changes the default provider; it returns a switch suggestion for the safe flow.

How to answer the user:

- If the user wants Claude -> Codex, Codex -> Claude, or any provider switch, call `model_switch_plan` first.
- If the plan says a session strategy is required, ask the user to choose `new_session` or `compact_session`.
- Never carry raw provider-specific tool-call history across providers.

### 0.1.0-dev.2026-05-03e — TUI native selection and mouse mode

Agent-facing changes:

- The TUI no longer captures the mouse by default. Native terminal text selection and right-click copy should work immediately.
- The main chat frame no longer draws left/right borders, so copied selections do not include decorative `│` characters.
- Command-like tool calls expose an exact-copy path: use `/copy command`, or click `[copy]` on a command tool call after `/mouse on`.
- `/mouse on` enables mouse capture for clickable tool-call headers and wheel scrolling.
- `/mouse off` disables mouse capture again and restores native selection/copy.
- `CAPTAIN_TUI_MOUSE=1` starts the TUI with mouse capture enabled for users who prefer mouse-first interaction.

How to answer the user:

- If the user cannot select text in the CLI/TUI, tell them to use `/mouse off`.
- If the user needs to copy a command without layout artifacts, tell them to use `/copy command` or `/mouse on` then the `[copy]` button.
- If the user wants clickable tool-call expansion or wheel scrolling, tell them to use `/mouse on`.
- Keyboard scrolling still works when mouse mode is off.

### 0.1.0-dev.2026-05-03d — Assistant identity/style onboarding

Agent-facing changes:

- `config.toml` now has an `[assistant]` block with `display_name`, `style`, and `onboarding_completed`. The internal principal-agent slug remains `captain`; the configured name is user-facing only.
- The runtime prompt injects the configured assistant identity and communication style on every turn, alongside any workspace `STYLE.md`.
- `captain setup` asks for the assistant name, answer style, user preferred name, language, timezone, Telegram, STT, and TTS basics.
- First-run API/provider keys and native integration credentials are persisted through `secrets.env`; legacy `.env` remains a fallback.
- Native Telegram setup now exports `TELEGRAM_BOT_TOKEN` through the same credential flow used by STT/TTS, so the channel config's env pointer resolves after setup.

How to answer the user:

- If asked "who are you?" or "what style should you use?", respect `[assistant]` first, then workspace `STYLE.md`, then the user's current instruction.
- If the user wants to rename the assistant, change `assistant.display_name`; do not rename the internal `captain` agent unless the user explicitly asks for a deeper routing change.
- If the user wants a different tone, set `assistant.style` to `balanced`, `concise`, `professional`, `developer`, `friendly`, `classic`, or a clear custom label.
- For first-run/channel credentials, use typed setup or `secret_write`; never place raw tokens in `config.toml`, scripts, docs, memory, or logs.

### 0.1.0-dev.2026-05-03c — CLI/Telegram runtime cleanup, credential SSOT, structured tool errors

Agent-facing changes:

- Agent streams now clean up their `running_tasks` entry when the run ends. Cancellation still targets the current run only.
- Credentials resolve through one preferred chain: `secrets.env` first, then legacy `vault.enc`, then `.env`, then process env. Values written with `secret_write` are visible to MCP/integration resolution without restart.
- MCP integration install no longer treats an in-memory key as configured unless the credential is actually persisted and resolvable.
- Tool failures now include a `[tool_error]` JSON block with `code`, `retryable`, `severity`, `next_action`, and `docs_query`.

How to answer the user:

- If an MCP/API credential seems missing, check the required key with `secret_read`, then use `secret_write` or the typed installer credentials object. Do not inspect raw credential files.
- If a tool fails, read the `[tool_error]` block and follow `next_action`; call `captain_docs` using `docs_query` before giving up.
- For security-blocked raw secrets, stop using files/scripts/commands as carriers. Store the value with `secret_write`, then use an integration or skill `env_inject`.

### 0.1.0-dev.2026-05-03b — Runtime progress stream, MCP install tools, controlled learning signals

Agent-facing changes:

- Long-running tool progress now flows through the same runtime stream as the TUI. Channels may render `progress` deltas as short status messages instead of raw stdout/stderr.
- MCP setup is now exposed through typed tools: `mcp_catalog_search`, `mcp_integration_install`, and `mcp_status`. Prefer those before shelling into project-specific install commands.
- Captain can access its `~/.captain/` workspace for self-configuration, but raw credential stores remain guarded. Use typed tools such as `secret_write`, config tools, MCP install tools, or vault/env injection instead of editing secrets directly.
- Cron webhooks reject localhost/private/link-local targets before storage and delivery.
- Learning signals now preserve tool outcome ordering for retry detection. Plain tool successes are buffer-only; repeated failures and retry-success patterns are the durable self-improvement signal.
- Reactive chat agents are allowed to sleep while unused. Inactivity alone must not mark them `Crashed`; heartbeat crash recovery is reserved for autonomous agents that declare a heartbeat, and for agents already in `Crashed` recovery.

How to answer the user:

- If a long task is running, send concise progress through the current chat when the runtime stream exposes it.
- If asked to install or debug an MCP integration, start with `mcp_catalog_search` / `mcp_status`, then use `mcp_integration_install` when a known template exists.
- If a tool recovers after retry or a repeated failure appears, use `self_improvement_review` before proposing durable changes.
- If an unused chat agent appears idle, describe it as sleeping/available rather than crashed unless the live state is explicitly `Crashed` after a real runtime failure.

### 0.1.0-dev.2026-05-03 — Tool identity, runtime update grounding, memory retractions

Agent-facing changes:

- Tool execution results now carry `tool_use_id` through runtime, API streams, timelines, TUI, and web UI. When multiple calls use the same tool name in one turn, associate every input/result by id, not by name or order.
- A real runtime update notice now means only: the binary/capability fingerprint changed. It is not proof that Captain has read release notes. Verify this changelog and live docs before describing changes.
- `memory_forget` records active retraction guards so archived checkpoints and summaries can remain historical while stale facts stop being treated as active truth.
- `memory_recall` must apply those same retraction guards to MemPalace and graph recall results. If a query or result matches a retracted term, return no active memory instead of exposing archival diary content.
- Successful memory writes/retractions should remain visible in the current chat so the user can see what future behavior changed.

How to answer the user:

- If asked "what changed?", cite the bullets above only after reading this family.
- For tool changes, also call `capability_search` or the relevant family docs before using a newly discovered parameter.
- For memory corrections, distinguish archive history from active memory. Never reassert a retracted fact as current user truth.

### 0.1.0-dev.2026-05-02 — Autonomous capability discovery and controlled learning

Agent-facing changes:

- `capability_search` is the first routing step when the right tool, skill, MCP, Hand, or docs family is unclear.
- `captain_docs` is the canonical recovery surface when a Captain tool fails or has unclear semantics.
- Learning and self-improvement must be visible in chat. Non-critical memory can be committed directly; critical changes such as skills, config, goals, routing, prompts, or global behavior require approval.
- Skills can propose refinements after real use when a better precondition, recovery path, or version bump is discovered.
- System bugs can be reported into Captain's internal register instead of being forgotten after the turn.

How to answer the user:

- Prefer "I will verify my live capability surface" over "I cannot".
- Use docs and capability search before asking the user how Captain works.
- Do not store private infrastructure names or secrets in reusable learning.

## Sandbox

This changelog is read-only documentation bundled into the binary through `captain_docs`.

It must stay:

- public-safe;
- generic enough for any future user;
- version-controlled with the code that implements the behavior;
- structured for an LLM reader, not as a human marketing changelog.

## Limites

- The changelog is manual. A code change is not agent-visible here until a developer adds an entry.
- The changelog describes intended behavior, not proof that a specific daemon has a feature enabled. Confirm with live tool schemas when the exact parameter contract matters.
- It is not a substitute for `captain_docs` family docs. Use it to know what changed, then use the family docs to know how to operate the capability.
- It must not contain secrets, private machine paths, private server names, personal aliases, API keys, or customer/user-specific data.

## Exemples

### User asks what changed after a restart

1. Read `captain_docs({family:"runtime-changelog", query:"update runtime"})`.
2. Summarize only the relevant entry.
3. Verify exact tools with `capability_search` if the user wants to use the new capability.

### No matching entry exists

Answer:

> Je sais qu'un vrai update runtime a eu lieu, mais je n'ai pas d'entree changelog agent-facing pour cette installation. Je vais verifier les schemas live avant d'affirmer un changement precis.

Then use `capability_search` / family docs.
