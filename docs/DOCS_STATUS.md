# Captain Docs Status (DOC2)

DOC2 defines which documentation is allowed to describe the current Captain
runtime contract. It exists to keep Captain aligned with its own system prompt,
tool docs, CLI, API, and release gates.

## Current Public Release

`v0.1.0-alpha.9` is the current public prerelease. It combines durable
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

Marketplace, ClawHub, long-tail channels, desktop packaging, and other non-core
surfaces may exist in code or compatibility docs, but they must not be presented
as active Hermes-level product paths unless the current plan explicitly reopens
them. Current docs must label them as frozen, compatibility, historical, or
outside the active release path.

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

Codex model availability is live runtime state, not a fixed documentation
list. With a Codex agent registered, the daemon refreshes the official catalog
after startup and hourly, persists newly seen IDs as deduplicated pending
decisions, and exposes them through authenticated Control/API plus configured
Telegram delivery. Availability never changes an active model by itself:
keeping is explicit, and switching requires an agent and a provider-portable
session strategy (`new_session` or `compact_session`).

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
  syntax.
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
  manual-only Actions, and exact-case Markdown links.
  `scripts/public-export-smoke.sh` repeats that export from a dirty development
  tranche and executes DOC2 inside the reduced tree before commit.
- `scripts/release-readiness.sh`, which runs both docs audits before release.
- `scripts/core-surface-gates.sh --surface settings-status`, which includes the
  docs audits in the status/settings surface gate.
