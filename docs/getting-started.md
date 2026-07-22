# Getting Started with Captain

This guide explains what Captain is, how to install it, and how to use it for
the first time.

## What Captain Is

Captain is a local Agent OS. It runs a persistent daemon that connects:

- one explicitly configured LLM provider/model per agent
- agent sessions
- tools such as files, shell, SSH, browser, documents, memory, and media
- channels such as Telegram and Discord
- approvals, logs, snapshots, workflows, and scheduled jobs
- a CLI/TUI, web terminal, and HTTP/WebSocket API

The goal is simple: Captain should become the single operational interface for
your personal or self-hosted AI system.

## Choose an Install Profile

| Profile | Best for | Notes |
|---|---|---|
| `core` | Minimal local install | CLI, config, daemon basics |
| `desktop` | Mac/Linux workstation | Local chat, TUI, web terminal, channels |
| `vps` | Server deployment | Service install, restart behavior, optional domain/HTTPS |
| `full-media` | Media-heavy setup | Adds extra dependencies for audio/media workflows |

Most users should start with `desktop` on a local machine or `vps` on a server.

## Install from GitHub Releases

Captain should be installed from a prebuilt bundle. You should not have to
compile Rust code on the target machine.

The public alpha and its checksums are readable without a GitHub token. GitHub
does not return prereleases from `/releases/latest`, so every alpha install
below pins `v0.1.0-alpha.9` explicitly.

### macOS / Linux Desktop

```bash
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.9/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.9 CAPTAIN_PROFILE=desktop bash
```

### Linux VPS

```bash
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.9/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.9 \
    CAPTAIN_PROFILE=vps \
    CAPTAIN_DOMAIN=captain.example.com \
    bash
```

The VPS profile installs the binary, prepares local state, runs setup, and
installs a service when supported. With `CAPTAIN_DOMAIN`, Captain can prepare
domain/HTTPS wiring where the host supports it.

### Windows

```powershell
$env:CAPTAIN_VERSION = "v0.1.0-alpha.9"
irm https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.9/install.ps1 | iex
```

Windows support targets the CLI first. WSL remains the recommended path for a
Linux-like server workflow on Windows. The alpha Windows executable is not
Authenticode-signed; verify the release checksum before approving first launch.

The macOS alpha binary is ad-hoc signed but not Apple-notarized. Verify its
SHA-256 sidecar before granting the operating system's first-launch approval.

## Configure Captain

Run the setup wizard:

```bash
captain setup
```

On a fresh home, setup prepares an empty runtime `agents/` directory and first
boot creates only the principal `captain` agent. Bundled specialist templates
remain available through explicit agent creation; installation does not enable
or materialize them automatically.

For unattended installs, provide credentials through environment variables and
run:

```bash
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.9/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.9 \
    CAPTAIN_PROFILE=vps \
    CAPTAIN_YES=1 \
    CAPTAIN_SETUP=1 \
    bash
```

Useful provider commands:

```bash
captain auth status
captain auth doctor
captain auth login codex
captain models providers
captain models current
captain models set <provider> <model>
captain models test
```

At least one model provider must be ready before Captain can answer. Depending
on your setup, this can be OAuth, API keys, Ollama, LM Studio, or another
OpenAI-compatible endpoint.

## Start Captain

Start the daemon:

```bash
captain start
```

Then open another terminal and run:

```bash
captain status
captain doctor --full
captain chat
```

Default local URLs:

- Control web: `http://127.0.0.1:50051/`
- Expert terminal: `http://127.0.0.1:50051/terminal`
- Health API: `http://127.0.0.1:50051/api/health`

If Captain is installed as a service:

```bash
captain service status
captain service restart
captain logs -f
```

## First Conversation

Start with:

```bash
captain chat
```

Good first prompts:

```text
Introduce yourself and explain what you can do on this installation.
Run a complete Captain diagnostic and tell me what is missing.
Start my user interview so you can learn my preferences.
Show me the useful commands for operating Captain.
```

Captain should use native tools when it needs configuration, logs, sessions,
memory, files, SSH, browser, or documents. You should not have to manually paste
runtime state back into the model.

## Daily Commands

```bash
captain chat                 # quick terminal chat
captain tui                  # full terminal UI
captain terminal             # open browser terminal
captain status               # daemon and agent status
captain doctor --full        # complete health check
captain auth status          # credentials and model auth
captain models current       # active provider/model
captain sessions list        # recent sessions
captain logs daemon          # daemon logs
captain snapshot create      # backup local state
captain reset --help         # reset options
```

## Telegram Setup

Telegram is the recommended mobile channel.

```bash
captain channel setup telegram
captain channel test telegram
```

The setup flow should guide you through:

1. creating a Telegram bot with BotFather
2. pasting the bot token into Captain
3. identifying your Telegram user ID
4. validating the channel with a test message

Once configured, Telegram can handle text, images, audio transcription, inline
approvals, and daemon commands such as status, restart, and shutdown.

## Control Web and Expert Terminal

The authenticated Control app is the primary browser surface:

```text
http://127.0.0.1:50051/
```

The browser-based expert terminal remains available at:

```text
http://127.0.0.1:50051/terminal
```

On a VPS, put it behind HTTPS and authentication. Raw shell mode should remain
explicitly enabled, not exposed by accident.

`captain setup` creates the initial web username/password in
`~/.captain/initial-credentials.txt`. Opening either surface shows the login
prompt first. To rotate the login from a conversation, ask Captain to change or generate web terminal
credentials; it uses the native `web_credentials_update` tool and updates
`config.toml` as the source of truth.

New installs keep web sessions for 72 hours by default. Use
`web_credentials_update` or edit `[auth].session_ttl_hours` to choose a shorter
window such as 24 hours on exposed VPS hosts.

Configuration can be edited from the browser at:

```text
http://127.0.0.1:50051/config
```

The page uses the same web login as `/terminal`, edits the complete
`config.toml`, validates before saving, creates a backup, and reloads hot
settings after a successful write.

## Documents and Research

Captain can be used for research and clean deliverables:

```text
Research this topic, cite the important sources, and create a PDF summary.
Turn this analysis into a structured Markdown report.
Create a client-ready document from these notes.
```

For product-quality output, prefer explicit requests:

```text
Use a native document workflow. Verify the file exists and tell me where it is.
```

## State, Backups, and Reset

Captain stores local state under `~/.captain/` by default:

```text
~/.captain/
  config.toml
  data/
  logs/
  sessions/
  snapshots/
```

Create a snapshot before risky changes:

```bash
captain snapshot create
captain snapshot list
```

Reset options are available through:

```bash
captain reset --help
```

Use reset when you want Captain to behave like a fresh installation. Keep a
snapshot if the current memory, config, or sessions matter.

## Troubleshooting

Run diagnostics first:

```bash
captain doctor --full
captain health
captain logs errors
```

Common issues:

- No answer from the model: check `captain status` first. If `LLM` is `NOT READY`, fix the provider shown in `LLM error`, then restart Captain.
- Telegram does not respond: check `captain channel test telegram` and daemon logs.
- Web terminal unavailable: check `captain status` and the configured listen address.
- VPS service not running: check `captain service status` and `captain logs daemon`.
- Release download fails: verify the selected version and GitHub availability.
  `CAPTAIN_GITHUB_TOKEN` is only needed for a private fork or mirror.

## Next Steps

- Read the [CLI Reference](cli-reference.md).
- Review [Deployment](DEPLOY.md) before using Captain on a VPS.
- Use [Troubleshooting](troubleshooting.md) for operational issues.
- Review [Security](security.md) and [VPS Web Terminal](deployment/vps-web-terminal.md)
  before exposing any web surface publicly.
