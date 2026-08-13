<p align="center">
  <img src="assets/logo.png" alt="Captain" width="280">
</p>

<h1 align="center">Captain</h1>

<p align="center"><b>The self-hosted Agent OS with production discipline.</b></p>

<p align="center">
  <a href="https://captainagent.fr/"><b>captainagent.fr</b></a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Built%20in-Rust-B7410E?style=for-the-badge&logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/License-MIT%20%2F%20Apache--2.0-green?style=for-the-badge" alt="License">
  <img src="https://img.shields.io/badge/Platforms-macOS%20%C2%B7%20Linux%20%C2%B7%20Windows%20%C2%B7%20Docker-blue?style=for-the-badge" alt="Platforms">
</p>

<p align="center">
  <b>English</b> ·
  <a href="README.fr.md">Français</a> ·
  <a href="README.es.md">Español</a> ·
  <a href="README.zh.md">中文</a>
</p>

**One persistent AI operator on your own hardware.** Captain is an open-source,
self-hosted autonomous AI agent runtime and Agent OS written in Rust. It keeps
conversations, projects, memory, scheduled work, and agent state across
sessions and restarts. It can execute real tools, delegate to isolated agents,
expose an agent through a secured API, and stay observable while work runs in
the background. Approval gates, budgets, loop guards, checkpoints, and an audit
trail constrain that autonomy. Run it on a Mac, a home server, a VPS, Windows,
or Docker, then use it from the terminal, authenticated Control web app,
Telegram, or Discord.

> **Public alpha:** Captain is under active development. Expect bugs, rough
> edges, and breaking changes between prereleases. Keep backups, review every
> granted capability, and do not rely on this alpha for critical workloads.

<table>
<tr><td width="220"><b>One binary, one daemon</b></td><td>A compiled Rust core orchestrates agents, tools, memory, channels, schedules, and approvals. Starts in seconds, idles light, survives reboots as a native service (launchd/systemd), and updates itself — ask it to in chat, approve, done.</td></tr>
<tr><td><b>Durable work</b></td><td>Projects, goals, checkpoints, workflows, and foreground or detached Live Runs are persisted. Long tool output stays outside the model context in bounded, redacted, checksum-verified evidence that Captain can read, search, or tail later. After a restart, incomplete work becomes inspectable as <code>interrupted</code> instead of disappearing or being replayed blindly. Retries are explicit, digest-bound and audited; uncertain interrupted effects require acknowledgement. An authenticated operator API exposes metadata and a bounded sanitized tail, and can cancel only an active run backed by a real abort handle. Control opens the same private inventory from its global top bar, with filters, live refresh, bounded text-only tails, and cancellation only when the runtime reports the run as actually cancellable. Full Ratatui exposes that inventory through <code>/runs</code> with stale-tail protection and two-step cancellation; standalone chat and the Web terminal stay metadata-only and bounded.</td></tr>
<tr><td><b>Evidence before delivery</b></td><td>Read-only conversation stays immediate. After effectful work, Captain evaluates ordered, redacted receipts only at useful milestones and before delivery. A mutation needs a newer relevant post-condition; pending jobs and failed checks remain explicit. Captain performs at most two targeted correction rounds, then returns an honest incomplete result instead of claiming success. Verification state survives abrupt restarts without persisting hidden reasoning or replaying uncertain effects.</td></tr>
<tr><td><b>Grounded research</b></td><td>Grouped research searches and retrieves independent sources concurrently, but keeps dependent verification ordered. Search snippets stay discovery-only; a source becomes citation-ready only after a successful page read with final URL, timestamp, and content SHA-256. For important claims, Captain refetches cited pages, rejects invented links or quotes absent from the source, measures declared provenance, and renders the Sources block from audited URLs. The audit certifies retrieval and verbatim evidence presence, never semantic truth by assertion.</td></tr>
<tr><td><b>Real execution, guarded</b></td><td>Shell, files, SSH, browser automation, web research, code execution, documents, and media. Sensitive calls use approvals; critical shell patterns are blocked; budgets limit tokens, cost, and tool-call rate. Captain shows its durable rolling guard separately from provider-owned subscription windows; Codex percentages and reset times come from official live account and response signals, never a copied quota table. Compact Chat status gives each provider-wide window and each limit matching the active model its own gauge; other model-specific families are summarized as outside the active model, while Status and Budget remain exhaustive. Ratatui, Web Control, and the retained desktop compatibility wrapper share that contract. Independent read-only tools may run concurrently, while dependent or side-effecting work remains ordered.</td></tr>
<tr><td><b>Readable native capabilities</b></td><td>Drop a reviewed <code>*.captain</code> file into a global or project <code>.captain/</code> directory and Captain hot-loads it as a typed <code>cap_*</code> tool. Captain Forge keeps dependencies, permissions, approvals, durable DAG execution, crash recovery, revision history, rollback, and exact operator decisions under kernel control.</td></tr>
<tr><td><b>Memory that follows the conversation</b></td><td>Session recall, durable user facts, project state, a knowledge graph, and optional local ONNX embeddings provide bounded context without dumping raw history into every turn. Accepted facts enter a local continuity journal first, remain available during a MemPalace outage, and resynchronize automatically with bounded backoff.</td></tr>
<tr><td><b>One Captain, every machine</b></td><td>Run one complete Hub and pair lightweight terminal, TUI, or Desktop Clients to the same sessions, projects, memory, model, and approvals. Optional execution Nodes act on local workspaces through outbound HTTPS 443 only, with proxy/enterprise-CA support, explicit grants, local guards, durable acknowledgements, and no blind replay after a disconnect.</td></tr>
<tr><td><b>Any model, no lock-in</b></td><td>Codex through your ChatGPT subscription, Anthropic, OpenAI, Mistral, Groq, Gemini, OpenRouter, and local models through Ollama. Captain discovers the live catalog and configured credentials instead of relying on fixed provider or model counts; context budgeting follows the selected model's live window. Per-agent reasoning controls preserve the model default in Auto; when Codex advertises Ultra, Captain applies maximum model effort plus bounded proactive root-agent delegation. For Codex, an hourly refresh surfaces newly listed models in Control and, when configured, Telegram; Captain never switches without your explicit decision and session strategy.</td></tr>
<tr><td><b>Email built for real work</b></td><td>Connect several Gmail accounts through OAuth, or several provider-neutral mailboxes through IMAP/SMTP. Search, read, draft, send, label, save attachments, and route deterministic matches to an exact agent. Credentials remain outside public config; accepted work, automation cursors, and ambiguous crash outcomes are durable and inspectable.</td></tr>
<tr><td><b>Six operational hubs</b></td><td>Chat, Projects, Automation, Learning, Capabilities, and Status are the shared primary surface in the TUI and Control web app. Automation groups Workflows, Triggers, Crons, Approvals, and Webhooks. Learning shows the exact model actually bound to its worker, heartbeat, durable queues, retries, and recovery instead of a decorative activity state.</td></tr>
<tr><td><b>Agents as services</b></td><td>Each agent can receive authenticated external ingress and emit signed HTTP callbacks. Captain provisions ingress automatically and reports the exact external callback URL still required before egress can be ready.</td></tr>
<tr><td><b>Operable like real software</b></td><td><code>captain doctor</code> explains what's broken and how to fix it. Snapshots and factory reset (backup first, always). Hash-chained audit trail. Health endpoints. A setup wizard that ends with a running, verified daemon — not a wall of next steps.</td></tr>
</table>

---

## Quick Install

Current public early-access release:
[v0.1.0-alpha.14](https://github.com/Vivien83/captain/releases/tag/v0.1.0-alpha.14).
Immutable Docker image: `ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.14`;
moving alpha channel: `ghcr.io/vivien83/captain-agent-os:alpha`.

### macOS / Linux / VPS

```bash
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.14/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.14 bash
```

The official repository, release assets, checksums, and container image are
public. No GitHub token or registry login is required.

The installer downloads a prebuilt, checksum-verified bundle for your
platform (no compilation, no toolchain), verifies the CLI end to end, and
runs a guided setup that **finishes with Captain actually running** as a
background service.

The same install provisions Captain's managed memory runtime before daemon
boot: uv 0.11.28, isolated CPython 3.13.14, MemPalace 3.5.0, and a frozen
checksum-bound dependency lock. No system Python, manual `pip install`, or
secondary API key is required. `captain memory doctor` verifies it live;
startup repairs a missing, corrupt, or insecure runtime and verifies a real
semantic read before boot. If repair fails, Captain does not report a
production-ready daemon without semantic memory.

Release assets cover `aarch64` and `x86_64` for macOS and Linux, plus an
`x86_64-pc-windows-msvc` CLI zip. Every archive has a SHA-256 file and a
platform manifest; the release also contains an aggregate manifest and the
Unix installers.

> **Alpha signing notice:** release archives and checksums are published, but
> macOS binaries are only ad-hoc signed and are not Apple-notarized. The
> Windows CLI is not Authenticode-signed. Verify the SHA-256 sidecar and expect
> the operating system to request explicit approval on first launch.

### Headless VPS (fully non-interactive)

```bash
export ANTHROPIC_API_KEY=...       # or any supported provider key
export TELEGRAM_BOT_TOKEN=...      # optional — see below
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.14/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.14 CAPTAIN_PROFILE=vps \
    CAPTAIN_DOMAIN=agent.example.com CAPTAIN_YES=1 bash
```

The `vps` profile installs a systemd service, starts it, and validates health.
If a Telegram token is present, Captain validates it against the
Telegram API, discovers your chat from the bot's pending messages, and
**sends you a confirmation message — your first contact with your agent
happens on your phone, seconds after install.**

Captain can manage one public HTTPS domain end to end. Create its DNS record
first, then set `CAPTAIN_DOMAIN` as above or answer the interactive installer
prompt. Captain keeps its API on loopback, configures Caddy transactionally,
and finishes only after public TLS, Control, health, and version parity pass.

### Headless VPS with your ChatGPT subscription (Codex, no API key)

Codex is Captain's built-in default provider — no `ANTHROPIC_API_KEY` or
similar needed, just your ChatGPT Plus/Pro/Pro+ login. `CAPTAIN_START=0`
installs everything (binary, systemd service) without starting the daemon
yet, so the readiness check below doesn't run before you've logged in:

```bash
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.14/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.14 CAPTAIN_PROFILE=vps \
    CAPTAIN_DOMAIN=agent.example.com CAPTAIN_YES=1 CAPTAIN_START=0 bash

captain login codex        # prints a URL + code — open it on your phone, no local browser needed
systemctl start captain    # non-root install: systemctl --user start captain
```

### Docker

The public alpha publishes `linux/amd64` and `linux/arm64` images to GitHub
Container Registry. Pulling the image does not require authentication:

```bash
docker run -d --name captain --restart unless-stopped \
  -p 50051:50051 \
  -v captain-data:/root/.captain \
  -e CAPTAIN_LISTEN=0.0.0.0:50051 \
  ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.14
```

First boot generates the daemon API key and persists it — along with all
state — in a named volume that survives image updates. The local embeddings
and managed MemPalace runtimes are provisioned in the image. The entrypoint
runs the live semantic doctor on every boot and repairs a missing, corrupt, or
insecure runtime before the daemon starts, including when a bind mount hides
the image's seeded state.

The public Compose file deliberately mounts only the named Captain state
volume. It does not expose the host filesystem, Docker socket, PID namespace,
or privileged mode. Pull and run the immutable image with:

```bash
git clone https://github.com/Vivien83/captain.git && cd captain
CAPTAIN_IMAGE_TAG=v0.1.0-alpha.14 docker compose pull
CAPTAIN_IMAGE_TAG=v0.1.0-alpha.14 docker compose up -d
```

Configure the chosen model provider after first boot. Add host access only as
an explicit, locally reviewed deployment change; broad host-access overlays
are not part of the public release contract.

---

## Getting Started

```bash
captain setup       # guided wizard: provider → preferences → channels → Captain running
captain             # full terminal UI
captain chat        # fast terminal chat
captain doctor      # diagnose anything, with fixes
captain update      # self-update (or just ask Captain to update itself)
captain status      # daemon, agents, channels, budgets, disk, health
```

Recommended providers to start:

- **Codex** — `captain auth login codex`. Uses your ChatGPT subscription; no
  API key to manage.
- **Claude** — export `ANTHROPIC_API_KEY` before setup.

First conversation triggers a seven-question onboarding interview (name,
language, timezone, style, voice, proactive notifications, and privacy) —
once, across every interface, stored durably. TUI, Control Web/Desktop, and
Telegram present clickable choices where a bounded answer is possible while
keeping free-text replies available.

The authenticated Control web app is available at
`http://127.0.0.1:50051/` by default. Its six hubs mirror the TUI, so projects,
automation, capabilities, and runtime health do not move between interfaces.
The expert terminal remains available at `http://127.0.0.1:50051/terminal`.

---

## CLI vs Messaging

Run the daemon once; talk to it from anywhere. Channels are **deny-by-default**:
every adapter requires an explicit user allowlist before it answers anyone.

| Action | Terminal | Telegram / Discord |
|---|---|---|
| Talk to Captain | `captain chat` or the TUI | message the bot |
| Approve a sensitive action | TUI approvals panel | inline buttons |
| Interrupt current work | `Esc` / `Ctrl+C` | `/stop` |
| Daemon status / restart | `captain status` / `captain service restart` | `status` / `restart` in chat |
| Voice | `captain voice` (local Whisper STT + Kokoro TTS) | send a voice note |
| Update Captain | `captain update` | "update yourself" → approval → done |

Captain checks its official release channel after startup and then every 12
hours. With an exact Telegram operator chat and explicit user allowlist, a Rich
card offers **Update now**, **Remind me in 24h**, or **Decline this version**.
The control plane handles the decision without a model turn; nothing installs
without a click. Host updates verify the release SHA-256, while Docker and
unsupported platforms stay operator-owned and are checked again. The durable
monitor state is visible in `captain status` and `GET /api/status`.

On a configured public domain, Captain also checks local/public ports and
health, DNS, TLS, reverse-proxy routing, and exact version parity every five
minutes. `captain doctor --full`, API Status, and Control read the same
crash-safe snapshot and show concrete remediation without exposing raw network
errors.

---

## What You Can Ask It

```text
Check my VPS: disk, memory, failed services — fix what's safe to fix.
Research X across the web and produce a sourced PDF report.
Watch this folder and summarize new documents to me on Telegram.
Every morning at 8: my calendar, the weather, anything unhealthy in the logs.
SSH into the backup server and verify last night's job actually ran.
Update yourself.
```

Under the hood, built-in tools are selected semantically so only relevant
schemas reach the model. Captain also supports governed skills, MCP tool
servers, multi-agent delegation, workflows, browser automation, and durable
tool runs that the agent can revisit, cancel, or order with dependencies.

---

## Documentation

| Guide | What's covered |
|---|---|
| [Getting Started](docs/getting-started.md) | Install → setup → first conversation |
| [Configuration](docs/configuration.md) | `config.toml`, providers, models, every option |
| [CLI Reference](docs/cli-reference.md) | All commands and flags |
| [Providers](docs/providers.md) | Model providers, auth, configured-model authority, explicit fallbacks |
| [Channel Adapters](docs/channel-adapters.md) | Telegram, Discord, Signal, Email setup |
| [Security](docs/security.md) | Authentication, capabilities, secrets, approvals, and audit trail |
| [Built-in Tools](docs/captain-tools/) | Per-family tool documentation |
| [Architecture](docs/architecture.md) | Crates, agent loop, kernel design |
| [API Reference](docs/api-reference.md) | REST endpoints, auth, streaming |
| [Hub, Clients, and Nodes](docs/hub-clients-nodes.md) | One central Captain, lightweight interfaces, and outbound local execution |
| [VPS Deployment](docs/deployment/github-vps-install.md) | Headless installs, reverse proxy, HTTPS |
| [MCP](docs/captain-tools/mcp.md) | External tool servers and transport contract |
| [Troubleshooting](docs/troubleshooting.md) | Common issues and their fixes |
| [0.1.0-alpha.14 Release Notes](docs/releases/v0.1.0-alpha.14.md) | One Hub, lightweight Clients, outbound Nodes, and crash-safe distributed work |
| [0.1.0-alpha.13 Release Notes](docs/releases/v0.1.0-alpha.13.md) | Adaptive delivery verification, crash-safe evidence, and safer host updates |
| [0.1.0-alpha.12 Release Notes](docs/releases/v0.1.0-alpha.12.md) | Durable Live Runs, grounded research, verified artifacts, and managed VPS domains |
| [0.1.0-alpha.11 Release Notes](docs/releases/v0.1.0-alpha.11.md) | Native email, durable integrations, audit closure, and local CI |
| [0.1.0-alpha.10 Release Notes](docs/releases/v0.1.0-alpha.10.md) | Production hardening, durable operations, and attested local releases |
| [0.1.0-alpha.9 Release Notes](docs/releases/v0.1.0-alpha.9.md) | Durable workflow learning and native release updates |
| [0.1.0-alpha.7 Release Notes](docs/releases/v0.1.0-alpha.7.md) | Durable committed state, supervised restart, truthful context, and direct TUI memory writes |
| [0.1.0-alpha.6 Release Notes](docs/releases/v0.1.0-alpha.6.md) | Telegram Rich Messages, live tool boards, ephemeral progress, and reliable controls |
| [0.1.0-alpha.5 Release Notes](docs/releases/v0.1.0-alpha.5.md) | Clean shutdown, memory privacy, live model identity, and single-agent first boot |
| [0.1.0-alpha.4 Release Notes](docs/releases/v0.1.0-alpha.4.md) | Authoritative corrections, complete active recall, and CLI continuation |
| [Docs Status (DOC2)](docs/DOCS_STATUS.md) | Current contracts, frozen surfaces, and historical documents |

---

## Security Posture

- API binds `127.0.0.1` by default and **refuses to start** on a public
  interface without authentication configured.
- Web/API access requires a session login or bearer API key; the web config
  editor is authenticated.
- Sensitive tools go through the approval flow; hyper-critical shell
  patterns are blocked or force a one-shot approval regardless of policy.
- Per-agent limits bound LLM usage, spending windows, and tool-call frequency.
- Loop guard: repetition, ping-pong, and consecutive-failure circuit
  breakers.
- Channel allowlists deny by default; hash-chained audit trail; secrets live
  in `secrets.env`, the encrypted vault, or authoritative read-only files
  declared in `secret-sources.toml`, never in config files. The vault master
  key is held by macOS Keychain, Windows Credential Manager, or Linux Secret
  Service; headless deployments must set `CAPTAIN_VAULT_KEY` explicitly.

State lives under `~/.captain/` — `config.toml` is the single source of
truth, hot-reloaded on change.

---

## Development

```bash
cargo test --workspace              # full suite
cargo build --release -p captain-cli
scripts/release-readiness.sh         # complete local release gate
CAPTAIN_VERSION=vX.Y.Z scripts/release-all.sh  # all 5 CLI targets locally
CAPTAIN_VERSION=vX.Y.Z scripts/publish-release-local.sh
docker build --build-arg CAPTAIN_BUILD_VERSION=vX.Y.Z -t captain:vX.Y.Z .
```

`release-all.sh` builds the two macOS, two Linux, and Windows CLI bundles one
target at a time; the Windows cross-build uses `cargo-xwin`, LLVM, and NASM.
After a clean release gate, `publish-release-local.sh` validates 20 host assets,
adds a SLSA v1 provenance statement and checksum (22 uploaded assets), pushes
the current branch, builds and pushes `linux/amd64` and `linux/arm64`
sequentially, then assembles the GHCR index and publishes the tag and GitHub
Release. The image reuses the two verified Linux release binaries instead of
recompiling Captain under emulation. Before image assembly, the publisher
stages a checksum-pinned FastEmbed snapshot from the maintainer's local Captain
cache into Git-ignored `dist/docker/`; it is neither committed nor added to the
22 release assets, and the Docker build verifies it again instead of depending
on a live model CDN. The provenance contract and its current signing limits are
documented in [Release Provenance](docs/release-provenance.md). Authenticate once with
`gh auth refresh -h github.com -s read:packages,write:packages`; do not pass a
token on the command line. The GitHub release workflow is an explicit manual
fallback and tag pushes do not start it. CI remains available for formatting,
strict Clippy, security/secret audits, and workspace checks/tests through an
explicit manual dispatch.

---

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at
your option.
