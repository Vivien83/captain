# Troubleshooting Captain

Use live diagnostics before editing state. Captain's defaults, configured
providers, active channels, and service manager vary by installation.

## First Response

```bash
captain --version
captain status
captain doctor --full
captain service status
captain logs daemon
```

The unauthenticated health probe is intentionally small:

```bash
curl http://127.0.0.1:50051/api/health
```

Use `captain status --verbose` or the authenticated Status hub for details.
Avoid diagnosing a current install from an old session summary or changelog
entry; verify the live binary and daemon first.

## Installation

### `captain` is not found

The release installer normally places Captain in `~/.captain/bin` and updates
the user path. Start a fresh shell, then inspect:

```bash
command -v captain
ls -l "$HOME/.captain/bin/captain"
```

For a manual shell session:

```bash
export PATH="$HOME/.captain/bin:$PATH"
```

### Reinstall the public alpha

GitHub's `/releases/latest` route excludes prereleases. Pin the alpha tag:

```bash
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.9/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.9 bash
```

No GitHub token is required for the official public repository. A token is
only relevant when `CAPTAIN_GITHUB_REPO` points to a private fork or mirror.

### macOS or Windows blocks first launch

The macOS alpha binary is ad-hoc signed but not Apple-notarized. The Windows
CLI is not Authenticode-signed. Verify the archive against its published
SHA-256 sidecar, then use the operating system's explicit approval flow. Do not
disable platform security globally.

## Daemon and Port

### Daemon does not start

```bash
captain service status
captain logs daemon
captain doctor --full
```

Check the configured listen address and whether port `50051` is already in
use. On macOS/Linux:

```bash
lsof -nP -iTCP:50051 -sTCP:LISTEN
```

Captain binds to `127.0.0.1:50051` by default. A non-loopback binding requires
authentication. Remote access must also use HTTPS through a reverse proxy.

### Restart is stuck

Current releases bound HTTP WebSocket/SSE draining and channel-adapter draining
separately. If the process remains after those windows, capture status and logs
before forcing it down:

```bash
captain status --verbose
captain logs daemon
captain service restart
```

Report the process state and the final log lines; do not delete daemon state as
a first response.

## Providers and Models

```bash
captain auth status
captain auth doctor
captain models providers
captain models current
captain models test
```

For Codex through a ChatGPT subscription:

```bash
captain auth login codex
```

For API-key providers, keep credentials in the environment, `secrets.env`, or
Captain's vault, and make `api_key_env` reference the variable name. Do not put
the secret value directly in `config.toml`.

HTTP `401` usually means a missing, expired, or wrong credential. An HTTP
`429` from an agent message endpoint is now structured: inspect `code`,
`quota.scope`, `quota.provider`, `quota.resets_at`, and the `Retry-After`
header instead of treating every `429` alike. `agent_hourly_tokens` is
Captain's durable rolling guard and can be inspected with
`captain agent caps <agent>`. `provider_subscription` is an allowance owned by
the provider; wait for its reported reset or make an explicit model/provider
decision. Captain does not retry an exhausted subscription or silently select
a fallback. If Status says `unavailable`, verify the Codex session and network;
do not read it as unlimited. Use `captain models list` rather than a fixed model
list from documentation.

Codex catalog notifications report availability only. Captain never changes
the active model until you explicitly keep or switch it and choose a new or
compacted session.

## Control and API Authentication

Control is at `http://127.0.0.1:50051/`; the expert terminal is at
`http://127.0.0.1:50051/terminal`. Setup writes one-time initial credentials to
`~/.captain/initial-credentials.txt`.

API requests use the configured bearer key:

```bash
curl -H "Authorization: Bearer $CAPTAIN_API_KEY" \
  http://127.0.0.1:50051/api/status
```

Do not paste API keys into issues or logs. If browser login loops, verify the
daemon clock, auth configuration, cookie origin, and reverse-proxy headers.
Keep the browser origin and Captain URL on HTTPS when deployed remotely.

## Sessions and Pending Questions

Every persisted conversation is independently reopenable across Control, TUI,
CLI, API, and the frozen Desktop wrapper:

```text
/history
/resume <UUID|unique-prefix|title>
/new
```

`/new` preserves the previous transcript. A session disappears only after an
explicit history deletion. If a restored session looks wrong, record its UUID,
owner agent, and source surface before continuing.

An `ask_user` request should be removed when answered, completed, cancelled,
or disconnected. If a stale prompt remains, capture the agent/session IDs and
status output; starting unrelated turns should not be required to clear it.

## Agents and Tools

```bash
captain agent list
captain agent caps <agent>
captain agent api <agent> --manifest
```

If an agent cannot use a tool, inspect its effective capabilities instead of
granting `*` immediately. Confirm the provider/model, budget, tool allowlist,
tool blocklist, network scope, shell policy, and parent-agent restrictions.

Detached tool calls are runtime work, not a blocked chat turn. Their status,
result, cancellation, and dependency ordering remain available to the agent
and in operational surfaces. Interrupt the active run with `/stop`; do not
assume a review-window message means the remote process has hung.

For an external agent API, ingress can be ready immediately while egress is
still pending. Captain cannot infer the external callback URL. Supply it, then
use the generated signed-callback test before declaring the agent fully ready.

## Channels

The active channel surface is Telegram, Discord, Signal, and Email. Each
adapter is deny-by-default and needs an explicit user allowlist.

```bash
captain channel list
captain channel setup telegram
captain status --verbose
captain logs daemon
```

For Telegram, verify the bot token, send `/start`, and ensure the user/chat is
allowed. For Discord, enable the required message-content intent and verify the
bot permissions. Long-tail adapters remain frozen compatibility surfaces and
should not be the first production integration.

## Docker

The public alpha image supports Linux AMD64 and ARM64:

```bash
docker pull ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.9
docker run --rm ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.9 --version
docker logs captain
```

The container must publish `50051`, set
`CAPTAIN_LISTEN=0.0.0.0:50051`, and mount `/root/.captain` on durable storage.
The moving prerelease channel is `:alpha`; use the immutable tag for a
reproducible deployment.

## Backup, Restore, and Reset

```bash
captain snapshot create --reason before-change
captain snapshot list
captain snapshot restore <id>
```

Factory reset is deliberately destructive but creates a recovery snapshot by
default:

```bash
captain reset --factory
```

Do not use `rm -rf ~/.captain` as routine troubleshooting. It bypasses the
snapshot, service shutdown, and preservation safeguards.

## After an Abrupt Shutdown

After a power cut or forced process termination, let launchd/systemd restart
Captain and inspect the live state before changing files:

```bash
captain service status
captain status
captain doctor --full
```

Committed memory, sessions, configuration, queues, and control-plane state use
the durable storage contract described in
[Architecture](architecture.md#sqlite-architecture). Work that was still in
flight can be marked interrupted or require a safe retry. Do not delete
`captain.db-wal`, `captain.db-shm`, temporary state files, or session data by
hand; collect a snapshot and sanitized diagnostics if integrity or recovery
still fails.

## Performance

Use live measurements rather than fixed resource estimates:

```bash
captain status --verbose
captain doctor --full
```

High memory or latency can come from local embeddings/media, large sessions,
concurrent agents, browser processes, provider latency, or channel retries.
Compact long sessions, reduce concurrency, and inspect active work before
removing persisted data.

## Data Egress

State is stored locally under `~/.captain`, but configured operations can send
data to model providers, channels, web/SSH targets, MCP servers, agent API
callbacks, and other explicitly invoked network tools. Review capabilities and
destinations; local storage does not mean every tool call stays offline.

## Report a Problem

- General bugs and feature requests: GitHub Issues.
- Security vulnerabilities: follow [SECURITY.md](../SECURITY.md) and use
  GitHub's private vulnerability reporting.

Include the Captain version, operating system/architecture, sanitized command,
live status, and bounded log excerpt. Remove tokens, usernames, hostnames,
private paths, IP addresses, and conversation content before posting.
