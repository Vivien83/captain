# Changelog

All notable public changes to Captain are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and version numbers
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

No public changes have been queued after the first alpha.

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

[Unreleased]: https://github.com/Vivien83/captain/compare/v0.1.0-alpha.1...HEAD
[0.1.0-alpha.1]: https://github.com/Vivien83/captain/releases/tag/v0.1.0-alpha.1
