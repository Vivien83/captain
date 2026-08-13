# Captain Documentation

Welcome to the Captain documentation. Captain is the open-source Agent Operating
System: one Rust runtime for agents, tools, memory, projects, channels,
automation, observability, and external agent APIs.

For current source-of-truth rules, read [Docs Status (DOC2)](DOCS_STATUS.md).
Prefer live commands such as `captain status`, `captain doctor --full`,
`captain models providers`, and `captain agent api <agent>` over old fixed
counts when validating an installed runtime.

---

## Getting Started

| Guide | Description |
|-------|-------------|
| [Getting Started](getting-started.md) | Installation, first agent, first chat session |
| [Hub, Clients, and Nodes](hub-clients-nodes.md) | Pair lightweight interfaces and outbound execution machines to one authoritative Hub |
| [Configuration](configuration.md) | Operational settings, secrets, and the live schema workflow |
| [CLI Reference](cli-reference.md) | Every command and subcommand with examples |
| [Troubleshooting](troubleshooting.md) | Common issues, FAQ, diagnostics |

## Core Concepts

| Guide | Description |
|-------|-------------|
| [Architecture](architecture.md) | Workspace structure, kernel boot, agent lifecycle, memory substrate |
| [Hub/Client/Node Protocol](HUB_CLIENT_NODE_PROTOCOL.md) | Alpha 14 authority, outbound Node, pairing, grants, and durable rail contract |
| [Performance Budgets](performance-budgets.md) | Reproducible TUI, Web terminal, Control, and Desktop rendering contracts |
| [Agents](agent-templates.md) | Manifests, lifecycle, capabilities, and external in/out API |
| [Captain Forge / CapSpec](CAPTAIN_FORGE_CAPSPEC.md) | Certified contract for readable, hot-reloaded, durable native capabilities |
| [Skill Learning V2](SKILL_LEARNING_V2.md) | Production contract for episode-based capability learning and Rich review |
| [Workflows](workflows.md) | Multi-agent pipelines with branching, fan-out, loops, and triggers |
| [Security](security.md) | Defense-in-depth security model and audit trail |

## Integrations

| Guide | Description |
|-------|-------------|
| [Channel Adapters](channel-adapters.md) | Telegram, Discord, Signal, and Email setup and operation |
| [LLM Providers](providers.md) | Live catalog, configured-model authority, consent, and explicit fallbacks |
| [Skills](skill-development.md) | Bundled, generated, and reviewed local skills; remote marketplaces disabled |
| [MCP](captain-tools/mcp.md) | Active external tool-server and transport contract |

## Reference

| Guide | Description |
|-------|-------------|
| [API Reference](api-reference.md) | REST/WS/SSE endpoints and external agent API contracts |

## Release & Operations

| Guide | Description |
|-------|-------------|
| [Deploying Captain](DEPLOY.md) | Pinned alpha install, Docker, Compose, HTTPS, backup, and update |
| [Release provenance](release-provenance.md) | Local SLSA statement, sequential builds, verification, and signing limits |
| [Repository governance](repository-governance.md) | Branch protection and the local-only CI policy |
| [0.1.0-alpha.14 release notes](releases/v0.1.0-alpha.14.md) | One Hub, lightweight Clients, outbound Nodes, and crash-safe distributed work |
| [0.1.0-alpha.13 release notes](releases/v0.1.0-alpha.13.md) | Adaptive delivery verification, crash-safe evidence, and safer host updates |
| [0.1.0-alpha.12 release notes](releases/v0.1.0-alpha.12.md) | Durable Live Runs, grounded research, verified artifacts, and managed VPS domains |
| [0.1.0-alpha.11 release notes](releases/v0.1.0-alpha.11.md) | Native email, durable integrations, audit closure, and local CI |
| [0.1.0-alpha.10 release notes](releases/v0.1.0-alpha.10.md) | Production hardening, durable operations, and attested local releases |
| [0.1.0-alpha.9 release notes](releases/v0.1.0-alpha.9.md) | Durable workflow learning and native release updates |
| [0.1.0-alpha.8 release notes](releases/v0.1.0-alpha.8.md) | Captain Forge native capabilities and truthful live subscription quotas |
| [0.1.0-alpha.7 release notes](releases/v0.1.0-alpha.7.md) | Durable committed state, supervised restart, truthful context, and direct TUI memory writes |
| [0.1.0-alpha.6 release notes](releases/v0.1.0-alpha.6.md) | Telegram Rich Messages, live tool boards, ephemeral progress, and reliable controls |
| [0.1.0-alpha.5 release notes](releases/v0.1.0-alpha.5.md) | Clean daemon shutdown and explicit memory write opt-out |
| [0.1.0-alpha.4 release notes](releases/v0.1.0-alpha.4.md) | Authoritative corrections, complete active recall, and CLI continuation |
| [0.1.0-alpha.3 release notes](releases/v0.1.0-alpha.3.md) | Managed MemPalace, durable memory recovery, and alpha limitations |
| [0.1.0-alpha.2 release notes](releases/v0.1.0-alpha.2.md) | Native visual inspection, browser identity, and alpha limitations |
| [0.1.0-alpha.1 release notes](releases/v0.1.0-alpha.1.md) | Public early-access scope and known limitations |
| [Release Readiness](../scripts/release-readiness.sh) | Executable current release gate for docs, tests, build, and live smoke |
| [Public Source Audit](../scripts/public-release-audit.sh) | Secrets, private paths, manual-only Actions, and local-link checks for the public export |

## Additional Resources

| Resource | Description |
|----------|-------------|
| [CONTRIBUTING.md](../CONTRIBUTING.md) | Development setup, code style, PR guidelines |
| [SECURITY.md](../SECURITY.md) | Security policy and vulnerability reporting |
| [CHANGELOG.md](../CHANGELOG.md) | Release notes and version history |

---

## Quick Reference

### Start in 30 Seconds

```bash
captain init
captain login codex
captain start
# Open http://127.0.0.1:50051
```

### Live Verification

| Need | Command |
|------|---------|
| Runtime version and daemon health | `captain --version && captain status` |
| Full local diagnosis | `captain doctor --full` |
| Provider/model catalog | `captain models providers && captain models list` |
| Model aliases | `captain models aliases` |
| Per-agent external API contract | `captain agent api <agent> --manifest` |
| Documentation coherence | `scripts/docs-global-audit.sh && scripts/docs-release-audit.sh` |
| Control web contract | `scripts/control-web-audit.sh` |
| Control CSP/XSS browser contract | `scripts/control-xss-smoke.mjs` |
| Release workflow contract | `scripts/release-workflow-audit.sh` |

### Important Paths

| Path | Description |
|------|-------------|
| `~/.captain/config.toml` | Main configuration file |
| `~/.captain/data/captain.db` | SQLite database |
| `~/.captain/skills/` | Installed skills |
| `~/.captain/daemon.json` | Daemon PID and port info |
| `agents/` | Agent template manifests |

### Key Environment Variables

| Variable | Provider |
|----------|----------|
| `ANTHROPIC_API_KEY` | Anthropic (Claude) |
| `OPENAI_API_KEY` | OpenAI |
| `GEMINI_API_KEY` | Google Gemini |
| `GROQ_API_KEY` | Groq |
| `DEEPSEEK_API_KEY` | DeepSeek |
| `XAI_API_KEY` | xAI (Grok) |

Only one provider credential is needed to get started; Codex can instead use a
ChatGPT subscription through device login.
