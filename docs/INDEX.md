# Captain Documentation

Captain is a self-hosted Agent OS: a Rust daemon plus CLI/TUI, web terminal,
channels, tools, memory, sessions, snapshots, and automation.

Use this index to find the right document quickly.

## Start Here

| Document | Purpose |
|---|---|
| [Getting Started](getting-started.md) | What Captain is, how to install it, and first commands |
| [Docs Status (DOC2)](DOCS_STATUS.md) | Current, historical, frozen, and agent-facing documentation rules |
| [CLI Reference](cli-reference.md) | Command reference for the `captain` binary |
| [Troubleshooting](troubleshooting.md) | Common operational issues and diagnostics |
| [Release Readiness](../scripts/release-readiness.sh) | Executable release gate: docs, tests, build, and live smoke |

## Install and Deploy

| Document | Purpose |
|---|---|
| [Deployment](DEPLOY.md) | Deployment guidance for local machines and VPS hosts |
| [GitHub + VPS Install](deployment/github-vps-install.md) | Pinned public prerelease install and service setup |
| [VPS Web Terminal](deployment/vps-web-terminal.md) | Browser terminal, auth, shell mode, and HTTPS considerations |
| [0.1.0-alpha.3 Release Notes](releases/v0.1.0-alpha.3.md) | Managed MemPalace, durable memory recovery, and alpha limitations |
| [0.1.0-alpha.2 Release Notes](releases/v0.1.0-alpha.2.md) | Native visual inspection, browser identity, and alpha limitations |
| [0.1.0-alpha.1 Release Notes](releases/v0.1.0-alpha.1.md) | Early-access scope, install paths, and known limitations |

## Product Surfaces

| Surface | Entry point |
|---|---|
| CLI chat | `captain chat` |
| Terminal UI | `captain tui` or `captain` |
| Control web | `http://127.0.0.1:50051/` |
| Web terminal (expert) | `http://127.0.0.1:50051/terminal` |
| API | `http://127.0.0.1:50051/api/` |
| Telegram | `captain channel setup telegram` |
| Discord | `captain channel setup discord` |
| Signal | `captain channel setup signal` |
| Email | `captain channel setup email` |

## Capability Docs

| Family | Document |
|---|---|
| Tools overview | [Captain Tools](captain-tools/README.md) |
| Browser | [Browser Tools](captain-tools/browser.md) |
| Channels | [Channel Tools](captain-tools/channel.md) |
| Documents | [Document Tools](captain-tools/document.md) |
| Memory | [Memory Tools](captain-tools/memory.md) |
| Multimedia | [Multimedia Tools](captain-tools/multimedia.md) |
| Network | [Network Tools](captain-tools/network.md) |
| SSH | [SSH Tools](captain-tools/ssh.md) |
| Session/workspace | [Session Workspace](captain-tools/session-workspace.md) |
| Runtime changelog | [Runtime Changelog](captain-tools/runtime-changelog.md) |

## Architecture and APIs

| Document | Purpose |
|---|---|
| [Architecture](architecture.md) | Runtime structure, boot, sessions, tools, and operational boundaries |
| [Channel Adapters](channel-adapters.md) | Setup and operation of Telegram, Discord, Signal, and Email |
| [API Reference](api-reference.md) | HTTP/WebSocket API reference |
| [Agents](agent-templates.md) | Manifests, lifecycle, capabilities, and external in/out API |
| [Security](security.md) | Authentication, capability controls, secrets, and audit trail |
| [Workflows](workflows.md) | Durable automation and dependency-aware execution |

## Operational Commands

```bash
captain setup
captain doctor --full
captain start
captain status
captain chat
captain terminal
captain auth status
captain models current
captain sessions list
captain logs daemon
captain snapshot create
```

## Development Notes

This repository may be exported from a larger local development checkout. Before
publishing, maintainers should use:

```bash
scripts/prepare-github-export.sh --yes "${TMPDIR:-/tmp}/captain-public-source"
```

The exporter requires a clean commit, uses `git archive`, removes the
maintainer-only site and historical plans, then runs the public source and
Markdown-link audits before initializing a new one-commit Git history.
