# Changelog

All notable public changes to Captain are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and version numbers
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

No public changes are queued yet.

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

[Unreleased]: https://github.com/Vivien83/captain/compare/v0.1.0-alpha.4...HEAD
[0.1.0-alpha.4]: https://github.com/Vivien83/captain/compare/v0.1.0-alpha.3...v0.1.0-alpha.4
[0.1.0-alpha.3]: https://github.com/Vivien83/captain/compare/v0.1.0-alpha.2...v0.1.0-alpha.3
[0.1.0-alpha.2]: https://github.com/Vivien83/captain/compare/v0.1.0-alpha.1...v0.1.0-alpha.2
[0.1.0-alpha.1]: https://github.com/Vivien83/captain/releases/tag/v0.1.0-alpha.1
