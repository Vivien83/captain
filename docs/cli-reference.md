# Captain CLI Reference

Complete command-line reference for `captain`, the CLI tool for the Captain Agent OS.

## Overview

The `captain` binary is the primary interface for managing the Captain Agent OS. It supports two modes of operation:

- **Daemon mode** -- When a daemon is running (`captain start`), CLI commands communicate with it over HTTP. This is the recommended mode for production use.
- **In-process mode** -- When no daemon is detected, commands that support it will boot an ephemeral in-process kernel. Agents spawned in this mode are not persisted and will be lost when the process exits.

Running `captain` with no subcommand launches the interactive TUI (terminal user interface) built with ratatui.

## Installation

### From source (cargo)

```bash
cargo install --path crates/captain-cli
```

### Build from workspace

```bash
cargo build --release -p captain-cli
# Binary: target/release/captain (or captain.exe on Windows)
```

### Docker

```bash
docker run -it ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.9
```

### Shell installer

```bash
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.9/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.9 bash
```

## Global Options

These options apply to all commands.

| Option | Description |
|---|---|
| `--config <PATH>` | Path to a custom config file. Overrides the default `~/.captain/config.toml`. |
| `--help` | Print help information for any command or subcommand. |
| `--version` | Print the version of the `captain` binary. |

**Environment variables:**

| Variable | Description |
|---|---|
| `RUST_LOG` | Controls log verbosity (e.g. `info`, `debug`, `captain_kernel=trace`). |
| `CAPTAIN_AGENTS_DIR` | Override the agent templates directory. |
| `EDITOR` / `VISUAL` | Editor used by `captain config edit`. Falls back to `notepad` (Windows) or `vi` (Unix). |

---

## Command Reference

### captain (no subcommand)

Launch the interactive TUI.

```
captain [--config <PATH>]
```

The TUI provides a full-screen terminal interface focused on six operational hubs: Chat, Projects, Automation, Learning, Capabilities, and Status. Automation contains Workflows, Triggers, Crons, Approbations, and Webhooks; Learning contains review, skill proposals, memory, and graph; Status carries operational visibility such as daemon health, logs, budget, usage, settings, and related overlays. Frozen or advanced surfaces such as Hands, peers, advanced comms, marketplace-style views, and experimental connections are not promoted in the primary navigation. Tracing output is redirected to `~/.captain/tui.log` to avoid corrupting the terminal display.

Top-level `captain --help` follows the same rule. Frozen compatibility commands
may remain callable by an exact name for existing operators, but they are not
listed as primary product paths.

Use `F1`-`F6` or `Tab`/`Shift+Tab` to switch primary hubs. Inside a hub, use `Alt+1`..`Alt+N` or `Alt+Left`/`Alt+Right` to switch subviews. In Chat, `Tab` still completes slash commands when the current draft starts with `/`.

The Chat footer renders `ctx used/window` from the active agent's live model
catalog entry. `used` follows the latest provider-reported prompt rather than
the cumulative session bill; the denominator refreshes after model switches
and cross-surface session restores.

Full-screen Chat also reserves a compact bottom subscription band. It names the
active model first. Every provider-wide primary or secondary window, plus any
limit family matching that model, receives its own gauge, percentage, dynamic
duration, and reset in the operator's local time. Other model-specific families
are summarized as outside the active model and remain exhaustive in Budget;
critical pressure is still surfaced. Plan, credits, stale state, and exhaustion
remain visible when supplied. The band polls Captain's local persisted
`/api/budget` snapshot every five seconds and never calls Codex directly. It is
shared by `captain`, full-screen `captain chat`, and the xterm Web terminal.
Control web renders the equivalent band, which is also what the frozen desktop
wrapper embeds. Narrow terminals pack the applicable gauges onto bounded rows
and direct overflow details to Budget rather than resizing the input or
overlapping the transcript.

Press `Ctrl+C` to exit. A second `Ctrl+C` force-exits the process.

---

### captain init

Initialize the Captain workspace. Creates `~/.captain/` with subdirectories
(`data/`, `agents/`) and a default `config.toml`. The `agents/` directory is
runtime state, not a template catalog: a fresh first boot creates only the
principal `captain` agent. Specialist templates remain embedded for explicit
creation with `captain agent new` and are never copied automatically.

```
captain init [--quick]
```

**Options:**

| Option | Description |
|---|---|
| `--quick` | Skip interactive prompts. Auto-detects the best available LLM provider and writes config immediately. Suitable for CI/scripts. |

**Behavior:**

- Without `--quick`: Launches an interactive 5-step onboarding wizard (ratatui TUI) that walks through provider selection, API key configuration, and optionally starts the daemon.
- With `--quick`: Reuses Codex subscription credentials first when
  `~/.codex/auth.json` is available, then checks configured API-key providers,
  `GOOGLE_API_KEY`, and local Ollama. If none is ready, it writes the Codex
  `gpt-5.5` default and asks for `captain login codex` before the first LLM
  call; it does not silently switch the default to Groq.
- File permissions are restricted to owner-only (`0600` for files, `0700` for directories) on Unix.

**Example:**

```bash
# Interactive setup
captain init

# Non-interactive (CI/scripts)
export GROQ_API_KEY="gsk_..."
captain init --quick
```

---

### captain start

Start the Captain daemon (kernel + API server).

```
captain start [--config <PATH>]
```

**Behavior:**

- Checks if a daemon is already running; exits with an error if so.
- Boots the Captain kernel (loads config, initializes SQLite database, loads agents, connects MCP servers, starts background tasks).
- Starts the HTTP API server on the address specified in `config.toml` (default: `127.0.0.1:50051`).
- Writes `daemon.json` to `~/.captain/` so other CLI commands can discover the running daemon.
- Blocks until interrupted with `Ctrl+C`.

**Output:**

```
  Captain Agent OS v0.1.0

  Starting daemon...

  [ok] Kernel booted (codex/gpt-5.5)
  [ok] Runtime model catalog loaded
  [ok] 3 agent(s) loaded

  API:        http://127.0.0.1:50051
  Terminal:   http://127.0.0.1:50051/terminal
  Provider:   codex
  Model:      gpt-5.5

  hint: Open the web terminal in your browser, or run `captain chat`
  hint: Press Ctrl+C to stop the daemon
```

**Example:**

```bash
# Start with default config
captain start

# Start with custom config
captain start --config /path/to/config.toml
```

---

### captain status

Show the current kernel/daemon status.

```
captain status [--json]
```

**Options:**

| Option | Description |
|---|---|
| `--json` | Output machine-readable JSON for scripting. |

**Behavior:**

- If a daemon is running: queries `GET /api/status` and displays agent count, provider, model, LLM readiness, auth mode, configured channels, media/TTS summary, uptime, API URL, operational paths, active agents, Captain's internal rolling token guard, provider-reported subscription windows, and the durable Captain release monitor.
- If no daemon is running: boots an in-process kernel and shows persisted state. Displays a warning that the daemon is not running.
- The principal `captain` agent is expected to match the global `[default_model]`. On startup, Captain repairs stale persisted principal-agent manifests so `captain status` does not report a global provider/model that differs from the active Captain agent.
- Provider subscription state is never inferred from local usage. `unavailable`
  means no official observation is available; `stale` means the last one is
  older than fifteen minutes. Window duration and reset are printed from the
  provider response rather than from a fixed hourly or weekly assumption.

**Example:**

```bash
captain status

captain status --json | jq '.agent_count'
```

---

### captain update

Download, verify, atomically install, and restart on an official Captain host
bundle.

```bash
captain update [--check] [--yes] [--version <release-tag>]
```

| Option | Description |
|---|---|
| `--check` | Resolve the compatible release channel without installing. |
| `--yes` | Skip the interactive CLI confirmation. Control-plane approvals remain exact and explicit. |
| `--version <release-tag>` | Install one exact tag, for example `v0.1.0-alpha.9`. |

Stable installations do not opt into prereleases. An existing prerelease may
advance to a newer prerelease or the corresponding stable version. The archive
and its `.sha256` asset are both mandatory. Captain stages the binary beside
the installed target, performs an atomic swap, and then restarts through the
platform service manager. Inside a container the command refuses to mutate the
image and returns an orchestrator-owned procedure.

The daemon independently checks after startup and every 12 hours. An exact
Telegram operator can update, defer for 24 hours, or refuse only the offered
version from a Rich card. These callbacks never enter a model turn. Inspect the
same durable state with:

```bash
captain status --json | jq '.runtime_update'
```

---

### captain doctor

Run diagnostic checks on the Captain installation.

```
captain doctor [--json] [--repair]
```

**Options:**

| Option | Description |
|---|---|
| `--json` | Output results as JSON for scripting. |
| `--repair` | Attempt to auto-fix issues (create missing directories, config, remove stale files). Prompts for confirmation before each repair. |

**Checks performed:**

`doctor` reports grouped diagnostics rather than a version-stable fixed count:
Captain home/config permissions and TOML validity, daemon discovery and stale
state, configured listen-port availability, SQLite integrity, disk capacity,
agent manifests, provider credentials/readiness, active channel configuration
(Telegram, Discord, Signal, Email), config/environment consistency, native
runtime dependencies, and local toolchain availability. Use `--json` when an
operator or test needs the exact checks exposed by the installed build.

**Example:**

```bash
captain doctor

captain doctor --repair

captain doctor --json
```

---

### captain terminal

Open the web terminal in the default browser.

```
captain terminal
```

**Behavior:**

- Requires a running daemon.
- Opens the terminal URL (e.g. `http://127.0.0.1:50051/terminal`) in the system browser.
- Copies the URL to the system clipboard (uses PowerShell on Windows, `pbcopy` on macOS, `xclip`/`xsel` on Linux).

**Example:**

```bash
captain terminal
```

---

### captain completion

Generate shell completion scripts.

```
captain completion <SHELL>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<SHELL>` | Target shell. One of: `bash`, `zsh`, `fish`, `elvish`, `powershell`. |

**Example:**

```bash
# Bash
captain completion bash > ~/.bash_completion.d/captain

# Zsh
captain completion zsh > ~/.zfunc/_captain

# Fish
captain completion fish > ~/.config/fish/completions/captain.fish

# PowerShell
captain completion powershell > captain.ps1
```

---

## Agent Commands

### captain agent new

Spawn an agent from a built-in template.

```
captain agent new [<TEMPLATE>]
```

**Arguments:**

| Argument | Description |
|---|---|
| `<TEMPLATE>` | Template name (e.g. `coder`, `assistant`, `researcher`). If omitted, displays an interactive picker listing all available templates. |

**Behavior:**

- Templates are discovered from: the repo `agents/` directory (dev builds), `~/.captain/agents/` (installed), and `CAPTAIN_AGENTS_DIR` (env override).
- Each template is a directory containing an `agent.toml` manifest.
- In daemon mode: sends `POST /api/agents` with the manifest. Agent is persistent,
  and the output includes the `agent-as-service.v1` API sheet with the ingress
  token returned once.
- In standalone mode: boots an in-process kernel. Agent is ephemeral.

**Example:**

```bash
# Interactive picker
captain agent new

# Spawn by name
captain agent new coder

# Spawn the assistant template
captain agent new assistant
```

---

### captain agent spawn

Spawn an agent from a custom manifest file.

```
captain agent spawn <MANIFEST>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<MANIFEST>` | Path to an agent manifest TOML file. |

**Behavior:**

- Reads and parses the TOML manifest file.
- In daemon mode: sends the raw TOML to `POST /api/agents` and prints the API
  provisioning sheet: status, manifest/events URLs, ingress bearer token
  returned once, egress configure/test URLs, and any operator action.
- In standalone mode: boots an in-process kernel and spawns the agent locally.

**Example:**

```bash
captain agent spawn ./my-agent/agent.toml
```

---

### captain agent list

List all running agents.

```
captain agent list [--json]
```

**Options:**

| Option | Description |
|---|---|
| `--json` | Output as JSON array for scripting. |

**Output columns:** ID, NAME, STATE, PROVIDER, MODEL (daemon mode) or ID, NAME, STATE, CREATED (in-process mode).

**Example:**

```bash
captain agent list

captain agent list --json | jq '.[].name'
```

---

### captain agent caps

Show one agent's effective tools, capability scopes, resource limits, and live
budget.

```
captain agent caps <AGENT_ID|NAME|PREFIX> [--json]
```

**Arguments:**

| Argument | Description |
|---|---|
| `<AGENT_ID|NAME|PREFIX>` | Agent UUID, unique UUID prefix, or exact agent name. |

**Options:**

| Option | Description |
|---|---|
| `--json` | Output raw agent and budget JSON for scripting. |

**Example:**

```bash
captain agent caps veille-technologique
```

---

### captain agent api

Inspect or prepare one agent's external agent-as-service API.

```
captain agent api <AGENT_ID|NAME|PREFIX> [--json] [--manifest] [--rotate-token]
```

**Arguments:**

| Argument | Description |
|---|---|
| `<AGENT_ID|NAME|PREFIX>` | Agent UUID, unique UUID prefix, or exact agent name. |

**Options:**

| Option | Description |
|---|---|
| `--json` | Output raw JSON for scripting. |
| `--manifest` | Print the full external integration manifest from `/api/agents/{id}/api/manifest`. |
| `--rotate-token` | Generate the ingress bearer token, store it in `secrets.env`, and print it once. |

**Behavior:**

- Default mode shows readiness, ingress URL, token env, manifest URL, callback
  endpoints, queue health, and concrete operator actions.
- External services call `POST /hooks/agents/{id}/ingress` with
  `Authorization: Bearer <token>`.
- `--rotate-token` is explicit because the returned token is a secret.

**Example:**

```bash
captain agent api veille-technologique
captain agent api veille-technologique --manifest
captain agent api veille-technologique --rotate-token
```

---

### captain agent chat

Start an interactive chat session with a specific agent.

```
captain agent chat <AGENT_ID>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<AGENT_ID>` | Agent UUID. Obtain from `captain agent list`. |

**Behavior:**

- Opens a REPL-style chat loop.
- Type messages at the `you>` prompt.
- Agent responses display at the `agent>` prompt, followed by token usage and iteration count.
- Type `exit`, `quit`, or press `Ctrl+C` to end the session.

**Example:**

```bash
captain agent chat a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

---

### captain agent kill

Terminate a running agent.

```
captain agent kill <AGENT_ID>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<AGENT_ID>` | Agent UUID to terminate. |

**Example:**

```bash
captain agent kill a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

---

## Workflow Commands

All workflow commands require a running daemon.

### captain workflow list

List all registered workflows.

```
captain workflow list
```

**Output columns:** ID, NAME, STEPS, CREATED.

---

### captain workflow create

Create a workflow from a JSON definition file.

```
captain workflow create <FILE>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<FILE>` | Path to a JSON file describing the workflow steps. |

**Example:**

```bash
captain workflow create ./my-workflow.json
```

---

### captain workflow run

Execute a workflow by ID.

```
captain workflow run <WORKFLOW_ID> <INPUT>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<WORKFLOW_ID>` | Workflow UUID. Obtain from `captain workflow list`. |
| `<INPUT>` | Input text to pass to the workflow. |

**Example:**

```bash
captain workflow run abc123 "Analyze this code for security issues"
```

---

## Trigger Commands

All trigger commands require a running daemon.

### captain trigger list

List all event triggers.

```
captain trigger list [--agent-id <ID>]
```

**Options:**

| Option | Description |
|---|---|
| `--agent-id <ID>` | Filter triggers by the owning agent's UUID. |

**Output columns:** TRIGGER ID, AGENT ID, ENABLED, FIRES, PATTERN.

---

### captain trigger create

Create an event trigger for an agent.

```
captain trigger create <AGENT_ID> <PATTERN_JSON> [--prompt <TEMPLATE>] [--max-fires <N>]
```

**Arguments:**

| Argument | Description |
|---|---|
| `<AGENT_ID>` | UUID of the agent that owns the trigger. |
| `<PATTERN_JSON>` | Trigger pattern as a JSON string. |

**Options:**

| Option | Default | Description |
|---|---|---|
| `--prompt <TEMPLATE>` | `"Event: {{event}}"` | Prompt template. Use `{{event}}` as a placeholder for the event data. |
| `--max-fires <N>` | `0` (unlimited) | Maximum number of times the trigger will fire. |

**Pattern examples:**

```bash
# Fire on any lifecycle event
captain trigger create <AGENT_ID> '{"lifecycle":{}}'

# Fire when a specific agent is spawned
captain trigger create <AGENT_ID> '{"agent_spawned":{"name_pattern":"*"}}'

# Fire on agent termination
captain trigger create <AGENT_ID> '{"agent_terminated":{}}'

# Fire on all events (limited to 10 fires)
captain trigger create <AGENT_ID> '{"all":{}}' --max-fires 10
```

---

### captain trigger delete

Delete a trigger by ID.

```
captain trigger delete <TRIGGER_ID>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<TRIGGER_ID>` | UUID of the trigger to delete. |

---

## Skill Commands

### captain skill list

List all installed skills.

```
captain skill list
```

**Output columns:** NAME, VERSION, TOOLS, DESCRIPTION.

Loads skills from `~/.captain/skills/` plus bundled skills compiled into the binary.

---

### captain skill install

Install a skill from a local directory, git URL, or frozen marketplace-compatible source when configured.

```
captain skill install <SOURCE>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<SOURCE>` | Local directory path, git URL, or compatibility source. |

**Behavior:**

- **Local directory:** Looks for `skill.toml` in the directory. If not found, checks for OpenClaw-format skills (SKILL.md with YAML frontmatter) and auto-converts them.
- **Remote compatibility:** Compatibility source paths are frozen outside the active core release. Installed skills still pass through SHA256 verification and prompt injection scanning when verification metadata is available.

**Example:**

```bash
# Install from local directory
captain skill install ./my-skill/

# Install from a compatibility source when configured
captain skill install web-search

# Install an OpenClaw-format skill
captain skill install ./openclaw-skill/
```

---

### captain skill remove

Remove an installed skill.

```
captain skill remove <NAME>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<NAME>` | Name of the skill to remove. |

**Example:**

```bash
captain skill remove web-search
```

---

### captain skill search

Search installed, bundled, generated, and frozen marketplace-compatible skill metadata.

```
captain skill search <QUERY>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<QUERY>` | Search query string. |

**Example:**

```bash
captain skill search "docker kubernetes"
```

---

### captain skill create

Interactively scaffold a new skill project.

```
captain skill create
```

**Behavior:**

Prompts for:
- Skill name
- Description
- Runtime (`python`, `node`, or `wasm`; defaults to `python`)

Creates a directory under `~/.captain/skills/<name>/` with:
- `skill.toml` -- manifest file
- `src/main.py` (or `src/index.js`) -- entry point with boilerplate

**Example:**

```bash
captain skill create
# Skill name: my-tool
# Description: A custom analysis tool
# Runtime (python/node/wasm) [python]: python
```

---

## Channel Commands

### captain channel list

List configured channels and their status.

```
captain channel list
```

**Output columns:** CHANNEL, ENV VAR, STATUS.

Checks `config.toml` for channel configuration sections and environment variables for required tokens. Status is one of: `Ready`, `Missing env`, `Not configured`.

The command reports configured active channels and can also show frozen
compatibility entries retained by an older configuration. A listed
compatibility entry is not an active supported setup path.

---

### captain channel setup

Interactive setup wizard for a channel integration.

```
captain channel setup [<CHANNEL>]
```

**Arguments:**

| Argument | Description |
|---|---|
| `<CHANNEL>` | Channel name. If omitted, displays an interactive picker. |

**Supported setup channels:** `telegram`, `discord`, `signal`, `email`.

Each wizard:
1. Displays step-by-step instructions for obtaining credentials.
2. Prompts for tokens/credentials.
3. Saves tokens to `~/.captain/.env` with owner-only permissions.
4. Appends the channel configuration block to `config.toml` (prompts for confirmation).
5. Warns to restart the daemon if one is running.

**Example:**

```bash
# Interactive picker
captain channel setup

# Direct setup
captain channel setup telegram
captain channel setup discord
captain channel setup signal
captain channel setup email
```

---

### captain channel test

Send a test message through a configured channel.

```
captain channel test <CHANNEL>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<CHANNEL>` | Channel name to test. |

Requires a running daemon. Sends `POST /api/channels/<channel>/test`.

**Example:**

```bash
captain channel test telegram
```

---

### captain channel enable

Enable a channel integration.

```
captain channel enable <CHANNEL>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<CHANNEL>` | Channel name to enable. |

In daemon mode: sends `POST /api/channels/<channel>/enable`. Without a daemon: prints a note that the change will take effect on next start.

---

### captain channel disable

Disable a channel without removing its configuration.

```
captain channel disable <CHANNEL>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<CHANNEL>` | Channel name to disable. |

In daemon mode: sends `POST /api/channels/<channel>/disable`. Without a daemon: prints a note to edit `config.toml`.

---

## Config Commands

### captain config show

Display the current configuration file.

```
captain config show
```

Prints the contents of `~/.captain/config.toml` with the file path as a header comment.

---

### captain config edit

Open the configuration file in your editor.

```
captain config edit
```

Uses `$EDITOR`, then `$VISUAL`, then falls back to `notepad` (Windows) or `vi` (Unix).

---

### captain config get

Get a single configuration value by dotted key path.

```
captain config get <KEY>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<KEY>` | Dotted key path into the TOML structure. |

**Example:**

```bash
captain config get default_model.provider
# groq

captain config get api_listen
# 127.0.0.1:50051

captain config get memory.decay_rate
# 0.05
```

---

### captain config set

Set a configuration value by dotted key path.

```
captain config set <KEY> <VALUE>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<KEY>` | Dotted key path. |
| `<VALUE>` | New value. Type is inferred from the existing value (integer, float, boolean, or string). |

**Warning:** This command re-serializes the TOML file, which strips all comments.

**Example:**

```bash
captain config set default_model.provider anthropic
captain config set default_model.model claude-sonnet-4-20250514
captain config set api_listen "0.0.0.0:50051"
```

Changing `api_listen` away from loopback requires Captain authentication. For
remote access, also terminate TLS at a reverse proxy; never expose an
unauthenticated daemon directly.

---

### captain config set-key

Save an LLM provider API key to `~/.captain/.env`.

```
captain config set-key <PROVIDER>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<PROVIDER>` | Provider name (e.g. `groq`, `anthropic`, `openai`, `gemini`, `deepseek`, `openrouter`, `together`, `mistral`, `fireworks`, `perplexity`, `cohere`, `xai`, `brave`, `tavily`). |

**Behavior:**

- Prompts interactively for the API key.
- Saves to `~/.captain/.env` as `<PROVIDER_NAME>_API_KEY=<value>`.
- Runs a live validation test against the provider's API.
- File permissions are restricted to owner-only on Unix.

**Example:**

```bash
captain config set-key groq
# Paste your groq API key: gsk_...
# [ok] Saved GROQ_API_KEY to ~/.captain/.env
# Testing key... OK
```

---

### captain config delete-key

Remove an API key from `~/.captain/.env`.

```
captain config delete-key <PROVIDER>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<PROVIDER>` | Provider name. |

**Example:**

```bash
captain config delete-key openai
```

---

### captain config test-key

Test provider connectivity with the stored API key.

```
captain config test-key <PROVIDER>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<PROVIDER>` | Provider name. |

**Behavior:**

- Reads the API key from the environment (loaded from `~/.captain/.env`).
- Hits the provider's models/health endpoint.
- Reports `OK` (key accepted) or `FAILED (401/403)` (key rejected).
- Exits with code 1 on failure.

**Example:**

```bash
captain config test-key groq
# Testing groq (GROQ_API_KEY)... OK
```

---

## Quick Chat

### captain chat

Quick alias for starting a chat session.

```
captain chat [<AGENT>] [--plain]
```

**Arguments:**

| Argument | Description |
|---|---|
| `<AGENT>` | Optional agent name or UUID. |
| `--plain` | Use line-based scrollback mode for SSH or terminals that should not run the full-screen TUI. |

**Behavior:**

- **Daemon mode:** Finds the agent by name or ID among running agents. If no agent name is given, uses the first available agent. If no agents exist, suggests `captain agent new`.
- **Standalone mode (no daemon):** Boots an in-process kernel and auto-spawns an agent from templates. Searches for an agent matching the given name, then falls back to `assistant`, then to the first available template.
- **Durable sessions:** Full-screen and `--plain` modes create detached SQLite
  sessions, so another TUI, Web Control or Desktop can list and resume the same
  transcript without switching Telegram or another client's active context.
  Use `/history` to list/open history, `/resume <UUID|unique-prefix|title>` to
  reopen a conversation with its owning agent, and `/new` to start fresh while
  preserving the previous transcript.
- **Live subscription status:** full-screen mode uses the same five-second
  local snapshot watcher and active-model quota classification as the primary
  TUI. Plain mode stays line-oriented and uses `captain status` for the
  exhaustive provider report.

This is the simplest way to start chatting -- it works with or without a daemon.

**Example:**

```bash
# Chat with the default agent
captain chat

# Chat with a specific agent by name
captain chat coder

# Chat with a specific agent by UUID
captain chat a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

---

## MCP Server

### captain mcp

Start an MCP (Model Context Protocol) server over stdio.

```
captain mcp
```

**Behavior:**

- Exposes running Captain agents as MCP tools via JSON-RPC 2.0 over stdin/stdout with Content-Length framing.
- Each agent becomes a callable tool named `captain_agent_<name>` (hyphens replaced with underscores).
- Connects to a running daemon via HTTP if available; otherwise boots an in-process kernel.
- Protocol version: `2024-11-05`.
- Maximum message size: 10MB (security limit).

**Supported MCP methods:**

| Method | Description |
|---|---|
| `initialize` | Returns server capabilities and info. |
| `tools/list` | Lists all available agent tools. |
| `tools/call` | Sends a message to an agent and returns the response. |

**Tool input schema:**

Each agent tool accepts a single `message` (string) argument.

**Integration with Claude Desktop / other MCP clients:**

Add to your MCP client configuration:

```json
{
  "mcpServers": {
    "captain": {
      "command": "captain",
      "args": ["mcp"]
    }
  }
}
```

---

## Daemon Auto-Detect

The CLI uses a two-step mechanism to detect a running daemon:

1. **Read `daemon.json`:** On startup, the daemon writes `~/.captain/daemon.json` containing the listen address (e.g. `127.0.0.1:50051`). The CLI reads this file to learn where the daemon is.

2. **Health check:** The CLI sends `GET http://<listen_addr>/api/health` with a 2-second timeout. If the health check succeeds, the daemon is considered running and the CLI uses HTTP to communicate with it.

If either step fails (no `daemon.json`, stale file, health check timeout), the CLI falls back to in-process mode for commands that support it. Commands that require a daemon (workflows, triggers, channel test/enable/disable, terminal) will exit with an error and a helpful message.

**Daemon lifecycle:**

```
captain start          # Starts daemon, writes daemon.json
                        # Other CLI instances detect daemon.json
captain status         # Connects to daemon via HTTP
Ctrl+C                  # Daemon shuts down, daemon.json removed

captain doctor --repair  # Cleans up stale daemon.json from crashes
```

---

## Environment File

Captain loads `~/.captain/.env` into the process environment on every CLI invocation. System environment variables take priority over `.env` values.

The `.env` file stores API keys and secrets:

```bash
GROQ_API_KEY=gsk_...
ANTHROPIC_API_KEY=sk-ant-...
GEMINI_API_KEY=AIza...
TELEGRAM_BOT_TOKEN=123456:ABC-DEF...
```

Manage keys with the `config set-key` / `config delete-key` commands rather than editing the file directly, as these commands enforce correct permissions.

---

## Exit Codes

| Code | Meaning |
|---|---|
| `0` | Success. |
| `1` | General error (invalid arguments, failed operations, missing daemon, parse errors, spawn failures). |
| `130` | Interrupted by second `Ctrl+C` (force exit). |

---

## Examples

### First-time setup

```bash
# 1. Set your API key
export GROQ_API_KEY="gsk_your_key_here"

# 2. Initialize Captain
captain init --quick

# 3. Start the daemon
captain start
```

### Daily usage

```bash
# Quick chat (auto-spawns agent if needed)
captain chat

# Chat with a specific agent
captain chat coder

# Check what's running
captain status

# Open the web terminal
captain terminal
```

### Agent management

```bash
# Spawn from a template
captain agent new assistant

# Spawn from a custom manifest
captain agent spawn ./agents/custom-agent/agent.toml

# List running agents
captain agent list

# Chat with an agent by UUID
captain agent chat <UUID>

# Kill an agent
captain agent kill <UUID>
```

### Workflow automation

```bash
# Create a workflow
captain workflow create ./review-pipeline.json

# List workflows
captain workflow list

# Run a workflow
captain workflow run <WORKFLOW_ID> "Review the latest PR"
```

### Event triggers

```bash
# Create a trigger that fires on agent spawn
captain trigger create <AGENT_ID> '{"agent_spawned":{"name_pattern":"*"}}' \
  --prompt "New agent spawned: {{event}}" \
  --max-fires 100

# List all triggers
captain trigger list

# List triggers for a specific agent
captain trigger list --agent-id <AGENT_ID>

# Delete a trigger
captain trigger delete <TRIGGER_ID>
```

### Skill management

```bash
# Search skills and compatibility metadata
captain skill search "code review"

# Install a skill
captain skill install code-reviewer

# List installed skills
captain skill list

# Create a new skill
captain skill create

# Remove a skill
captain skill remove code-reviewer
```

### Channel setup

```bash
# Interactive channel picker
captain channel setup

# Direct channel setup
captain channel setup telegram

# Check channel status
captain channel list

# Test a channel
captain channel test telegram

# Enable/disable channels
captain channel enable discord
captain channel disable signal
```

### Configuration

```bash
# View config
captain config show

# Get a specific value
captain config get default_model.provider

# Change provider
captain config set default_model.provider anthropic
captain config set default_model.model claude-sonnet-4-20250514
captain config set default_model.api_key_env ANTHROPIC_API_KEY

# Manage API keys
captain config set-key anthropic
captain config test-key anthropic
captain config delete-key openai

# Open in editor
captain config edit
```

### MCP integration

```bash
# Start MCP server for Claude Desktop or other MCP clients
captain mcp
```

### Diagnostics

```bash
# Run all diagnostic checks
captain doctor

# Auto-repair issues
captain doctor --repair

# Machine-readable diagnostics
captain doctor --json
```

### Shell completions

```bash
# Generate and install completions for your shell
captain completion bash >> ~/.bashrc
captain completion zsh > "${fpath[1]}/_captain"
captain completion fish > ~/.config/fish/completions/captain.fish
```

---

## Supported LLM Providers

The following providers are recognized by `captain config set-key` and `captain doctor`:

| Provider | Environment Variable | Default Model |
|---|---|---|
| Groq | `GROQ_API_KEY` | `llama-3.3-70b-versatile` |
| Gemini | `GEMINI_API_KEY` or `GOOGLE_API_KEY` | `gemini-2.5-flash` |
| DeepSeek | `DEEPSEEK_API_KEY` | `deepseek-chat` |
| Anthropic | `ANTHROPIC_API_KEY` | `claude-sonnet-4-20250514` |
| OpenAI | `OPENAI_API_KEY` | `gpt-4o` |
| OpenRouter | `OPENROUTER_API_KEY` | `openrouter/google/gemini-2.5-flash` |
| Together | `TOGETHER_API_KEY` | -- |
| Mistral | `MISTRAL_API_KEY` | -- |
| Fireworks | `FIREWORKS_API_KEY` | -- |
| Perplexity | `PERPLEXITY_API_KEY` | -- |
| Cohere | `COHERE_API_KEY` | -- |
| xAI | `XAI_API_KEY` | -- |

Additional search/fetch provider keys: `BRAVE_API_KEY`, `TAVILY_API_KEY`.
