# Configuration

Captain stores operator configuration in `$CAPTAIN_HOME/config.toml`
(`~/.captain/config.toml` by default). Use the setup wizard and typed CLI for
normal changes; edit TOML directly only when the setting has no guided surface.

```bash
captain setup
captain config show
captain config get <key>
captain config set <key> <value>
captain config edit
captain config schema
captain doctor --full
```

`captain config schema` is the exact contract for the installed binary. This
guide covers the operational settings most users need and intentionally omits
frozen compatibility sections and volatile provider catalogs.

## Minimal Configuration

Captain is Codex-first and uses managed MemPalace by default:

```toml
home_dir = "~/.captain"
data_dir = "~/.captain/data"
api_listen = "127.0.0.1:50051"
log_level = "info"
language = "en"
timezone = "UTC"

[api]
allowed_origins = []

[default_model]
provider = "codex"
model = "gpt-5.5"
api_key_env = ""

[memory]
backend = "mempalace"

[auth]
enabled = true
username = "admin"
password_hash = ""
session_cookie_secure = "auto"
```

Run `captain setup` to generate authentication material and complete provider
login. At first boot Captain also writes a unique 32-byte
`auth.session_secret` and a managed `auth.session_epoch`; they are omitted from
the example because operators must never copy or edit them. Do not copy this
example over an existing production file.

## Secrets

`config.toml` should contain secret references, not secret values. Production
deployments can reference read-only files mounted by Docker, Kubernetes,
systemd credentials, or another secret manager through
`$CAPTAIN_HOME/secret-sources.toml`.

```toml
version = 1

[sources.OPENAI_API_KEY]
type = "file"
path = "/run/secrets/openai_api_key"

[sources.TELEGRAM_BOT_TOKEN]
type = "file"
path = "/run/secrets/telegram_bot_token"
```

Keys use `A-Z`, `0-9`, and underscore and are bounded to 128 bytes. Only
absolute file paths are accepted. Captain deliberately does not execute
commands or provider-specific secret-manager clients from this registry. Keep
the registry non-writable by group or others; each source must be a regular,
UTF-8, single-line file no larger than 64 KiB. Group/world-writable sources are
rejected. Group/world-readable sources remain usable with an explicit warning
because read-only container secret mounts commonly use mode `0444`.

An external mapping is **authoritative**. If its file is missing, unreadable,
unsafe, empty, or malformed, Captain reports that credential unavailable and
does not fall back to a stale value in `secrets.env`, `vault.enc`, `.env`, or
the process environment. Resolver-backed consumers read source file contents
live. A consumer that caches a credential must be reloaded; use
`channel_reconfigure` or the channel reload API for channel adapters. Boot
credentials such as `CAPTAIN_DAEMON_API_KEY`, and every edit to the registry
itself, require a daemon restart.

For durable per-agent callbacks, a temporarily unavailable authoritative URL
or signing secret is reported as `unavailable` and the outbound event remains
queued for retry. Captain never downgrades it to an unsigned send or silently
classifies the callback as unconfigured.

Inspect readiness without exposing values or individual source paths:

```bash
captain vault sources
captain vault sources --json
captain doctor --full
```

Without an external mapping, Captain continues through the restricted local
`secrets.env`, encrypted vault, legacy `.env`, and process environment. Writes
to an externally managed key are refused; rotate the mounted file instead.
This applies to fixed provider/channel names and to the deterministic
per-agent API token/callback keys shown by that agent's API manifest.

Never commit or paste:

- API keys and OAuth tokens;
- channel bot tokens;
- daemon bearer tokens;
- webhook or agent callback secrets;
- password hashes copied from a live installation;
- SSH private keys.

Use the guided secret and provider commands where available:

```bash
captain auth login codex
captain config set-key <provider>
captain config test-key <provider>
captain auth status
```

## Network and Authentication

Loopback is the safe local default:

```toml
api_listen = "127.0.0.1:50051"

[api]
allowed_origins = []
```

Binding to `0.0.0.0` exposes Captain beyond the local machine and requires
authentication. On a VPS, keep Captain behind HTTPS and a reviewed reverse
proxy, restrict firewall access, and use generated credentials. Do not disable
auth to make a remote setup easier.

```toml
api_listen = "0.0.0.0:50051"

[auth]
enabled = true
username = "admin"
session_ttl_hours = 72
session_cookie_secure = "auto"
```

Browser access is fail-closed independently of the daemon API key. By default,
CORS accepts only `http://localhost:{api_port}`,
`http://127.0.0.1:{api_port}`, and the IPv6 loopback equivalent. Methods and
request headers use an explicit reviewed list. A separate trusted web
application can be added with an exact origin:

```toml
[api]
allowed_origins = ["https://console.example.com"]
```

Entries must be complete `http` or `https` origins without credentials, paths,
queries, fragments, or wildcards. `deployment.public_url` is also treated as
an explicit trusted origin for a declared reverse-proxy deployment. Captain
derives its request `Host` allowlist from loopback, the concrete listen
address, these configured origins, and `deployment.public_url`; every other or
malformed `Host` is rejected with `400` before routing. Changing this policy
requires a daemon restart.

The password hash and daemon API key are provisioned by setup and stored in the
secret path, not copied into this example. Browser session signatures use only
the independent Captain-managed `session_secret`, never the daemon API key or
password hash. Every token also carries `session_epoch`; changing the password
through setup or `web_credentials_update` increments it and rejects all older
sessions. `captain config show`, `/config`, and `GET /api/config/raw` redact
both managed secret fields. Raw config writes preserve them but cannot replace
the signing key or roll the epoch back.

Passwords are stored as salted Argon2id PHC strings. A legacy SHA-256 hash is
accepted only for one successful compatibility login, then atomically replaced
by Argon2id without changing the session epoch. Login failures are tracked
separately per client IP and normalized username; exponential backoff starts
after five failures and is capped at 15 minutes.

`session_cookie_secure = "auto"` adds `Secure` when `deployment.public_url`
uses HTTPS or a trusted loopback reverse proxy supplies
`X-Forwarded-Proto: https`. Use `"always"` to require secure cookies
unconditionally. `"never"` exists only for explicit local HTTP development.
Browser WebSocket and SSE clients obtain 30-second, path/IP/epoch-bound
single-use tickets; API keys and session tokens are never accepted from a URL
query string.

## Model Provider

Use the live catalog before changing a model:

```bash
captain models providers
captain models list
captain config set default_model.provider codex
captain config set default_model.model gpt-5.5
captain models test
```

For API-key providers, set `api_key_env` to the intended environment variable
or use Captain's secret commands. A model change on an active conversation may
require a new session or a provider-portable compaction; Captain must ask
instead of switching silently.

## Memory

The production path is:

```toml
[memory]
backend = "mempalace"
```

Official installers and containers manage the pinned MemPalace runtime inside
Captain's private home and verify it before every active local kernel boot.
Check both the native runtime and durable synchronization journal:

```bash
captain memory doctor --json
captain doctor --full
captain status
```

Do not point production Captain at a manually installed Python package. The
local SQLite journal remains the durable source during a MemPalace outage and
resynchronizes when the managed backend recovers.

## Active Channels

The ready external channels are Telegram, Discord, Signal, and Email. Configure
them with their wizards:

```bash
captain channel setup telegram
captain channel setup discord
captain channel setup signal
captain channel setup email
```

Example policy shape:

```toml
[channels.telegram]
bot_token_env = "TELEGRAM_BOT_TOKEN"
default_agent = "captain"
allowed_users = ["123456789"]

[channels.telegram.overrides]
dm_policy = "allowed_only"
group_policy = "mention_only"
rate_limit_per_user = 10
```

An empty inbound allowlist is deny-by-default. Use `allowed_users = ["*"]`
only for a deliberate public bot. Long-tail channel sections can remain in old
config files for compatibility but are frozen and are not documented as ready
setup paths.

## Execution and Approvals

Execution policy controls command availability, timeouts, output limits, and
critical-command handling. New installations use `mode = "full"` with
`critical_mode = "safe"`: routine host commands are available, while
recognized catastrophic commands fail closed. `critical_mode = "open"` is an
explicit opt-in that allows a recognized command only after content-bound
operator approval; `paranoid` requests approval for every shell-affecting
operation.

```toml
[exec_policy]
mode = "full"
critical_mode = "safe"
```

Host execution clears and reconstructs the child environment, but it is not an
operating-system sandbox. `captain status`, `captain security`, full doctor,
`GET /api/status`, `GET /api/health/detail`, and `GET /api/security` report
`backend = "host_process"`, `isolation_level = "environment_scrub"`, and
`os_isolation = false`. Use the explicit Docker or WASM backend when untrusted
code requires OS-level isolation. Keep destructive actions behind explicit
human control and inspect the effective agent capabilities before broadening
them.

```bash
captain agent caps <agent>
captain status
captain doctor --full
```

Use `captain config schema` for the typed `[approval]` and `[exec_policy]`
fields supported by the installed version. Do not copy permissive examples
from an older release or enable broad host access as a convenience.

## Web, Browser, and Media

Common sections include `[web]`, `[web_terminal]`, `[browser]`, `[media]`,
`[tts]`, and `[voice_call]`. Their defaults depend on the installed release and
optional local components. Start from setup, inspect the schema, then enable
only the capability you can test:

```bash
captain doctor --full
captain status
```

Images remain on the active multimodal model. Captain does not require or
silently select a secondary Mistral Vision provider.

## Projects, Automation, and Learning

Projects, workflows, triggers, crons, learning, checkpoints, and skills have
typed config sections, but most day-to-day changes belong in Control or their
CLI commands. Use raw TOML for deployment policy, not as a substitute for
runtime state.

The six active product hubs are Chat, Projects, Automation, Learning,
Capabilities, and Status. Hands, fleets, A2A, marketplace, long-tail channels,
and Desktop packaging remain frozen compatibility surfaces.

No configuration switch reopens remote skill marketplaces. Their clients fail
before I/O, the HTTP routes and TUI actions are absent, and the CLI accepts only
a reviewed existing local directory.

## Includes and Reload

`include = [...]` can merge relative TOML fragments before the main file. Keep
fragments inside the trusted configuration directory and avoid splitting
secrets into ordinary readable files.

Some settings can reload; others require a bounded daemon restart. After any
manual edit:

```bash
captain config show
captain doctor --full
captain status
```

If the daemon reports a restart requirement, use `captain restart` rather than
killing processes by name.

## Maintained Full Example

[`captain.toml.example`](../captain.toml.example) is a parse-tested source
example shipped in release bundles. It includes advanced and compatibility
fields required for reproducible development. For an operator, the generated
schema and current local config remain authoritative:

```bash
captain config schema
captain config show
```

See [Getting Started](getting-started.md), [Security](security.md),
[Model Providers](providers.md), and [Channel Adapters](channel-adapters.md).
