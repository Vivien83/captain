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
allow_unauthenticated_loopback = false
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

The encrypted vault keeps its master key in the native platform credential
store and verifies every new write by reading it back. It never prints a
generated key. Hosts without an unlocked credential store must inject an
explicit base64 32-byte `CAPTAIN_VAULT_KEY`; otherwise vault initialization or
unlock fails closed. The obsolete obfuscated `.keyring` file is migrated only
after the native copy is verified, then removed. Conflicting copies require
operator intervention.

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

When `captain setup --profile vps` receives `CAPTAIN_DOMAIN` or
`CAPTAIN_PUBLIC_URL`, it accepts only a path-free HTTPS DNS hostname and writes
`deployment.public_url`. The API is forced back to `127.0.0.1` while preserving
an explicitly customized port, and the generated Caddy upstream uses that same
port. The host release installer can activate this configuration through the
transactional managed-Caddy path described in
[GitHub + VPS Install](deployment/github-vps-install.md).

Once the daemon runs, it verifies the effective deployment every five minutes:
local API port and health, public DNS, TLS, public port, reverse-proxy routing,
public health, and exact Captain version parity. The crash-safe snapshot lives
under `data/health/deployment-readiness.json`, is private to the Captain owner,
and is invalidated by any relevant configuration or binary-version change.
Inspect its public-safe projection with `captain doctor --full`,
`captain status --json | jq '.deployment.readiness'`, or Control's Status hub.

```toml
api_listen = "0.0.0.0:50051"

[auth]
enabled = true
allow_unauthenticated_loopback = false
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

If both browser auth and the daemon API key are absent, protected routes return
an actionable authentication error instead of becoming public. The only
credentialless mode is the explicit
`auth.allow_unauthenticated_loopback = true` development escape hatch. It
accepts only the actual loopback client; a local reverse proxy declared for a
public deployment does not turn remote clients into loopback requests.
`captain setup` and `web_credentials_update` always reset this flag to `false`.
Older configurations that explicitly contained `auth.enabled = false` are
migrated once to the explicit loopback flag so upgrades preserve intentional
local-only behavior without preserving an implicit fail-open default.

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
after five failures and is capped at 15 minutes. The two in-memory maps retain
at most 4,096 keys each and never evict an entry while its block is active. If
all slots are actively blocked, Captain applies a logged five-second global
backoff instead of forgetting one. This process-local defense resets on daemon
restart; public deployments still need upstream login rate limiting.

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

[channels.email]
default_account = "work"

[[channels.email.accounts]]
alias = "work"
enabled = true
imap_host = "imap.example.com"
imap_port = 993
smtp_host = "smtp.example.com"
smtp_port = 587
username = "captain@example.com"
password_env = "CAPTAIN_EMAIL_WORK_12AB34CD_PASSWORD"
poll_interval_secs = 30
folders = ["INBOX"]
allowed_senders = ["me@example.com", "@example.org"]
default_agent = "captain"
```

An empty inbound allowlist is deny-by-default. Use `allowed_users = ["*"]`
only for a deliberate public bot. Long-tail channel sections can remain in old
config files for compatibility but are frozen and are not documented as ready
setup paths.

Use `captain channel setup email` or the schema-driven Channels screen instead
of constructing the Email table manually. Guided setup derives an injective
credential key per alias and writes the password through the credential
resolver, never into TOML. Re-running setup for another alias preserves the
existing accounts. Hosts must not contain a URL scheme; IMAP always uses
implicit TLS, SMTP port 465 uses implicit TLS, and other SMTP ports require
STARTTLS. Account aliases are stable lowercase identifiers used by
`email:<alias>`.

`allowed_senders` accepts only `*`, an exact address, or an explicit
`@domain`. Empty means locked. `folders` defaults to `INBOX`, and the poll
interval is bounded from 5 to 3600 seconds. A legacy scalar `[channels.email]`
block remains readable as one account named `default`; all new writes use
`[[channels.email.accounts]]`.

## Execution and Approvals

### Hub devices

The Hub/Client/Node rail is available by default, while every enrollment
window starts closed and expires automatically. Disable the rail entirely only
when this installation must never accept Clients or Nodes:

```toml
[pairing]
hub_enabled = true
enabled = false # legacy mobile pairing routes
```

Open a bounded enrollment window with `captain devices pair`; this requires
the existing authenticated operator API. Approved devices remain revocable
and can reconnect after the enrollment window closes.

Execution policy controls command availability, timeouts, output limits, and
critical-command handling. The typed default is
`profile = "personal_workstation"` with `mode = "allowlist"`. Guided local
setup deliberately writes `personal_workstation` plus `full` for a trusted
single-user workstation; this is visible configuration, not an inferred
fallback. Set the deployment profile explicitly for remotely operated or
untrusted workloads. `critical_mode = "open"` is an opt-in that allows a
recognized command only after content-bound operator approval; `paranoid`
requests approval for every shell-affecting operation.

```toml
[exec_policy]
profile = "remote_operator" # personal_workstation | remote_operator | untrusted_execution
mode = "allowlist"          # deny | allowlist | full
critical_mode = "safe"
```

Profiles only remove authority:

- `personal_workstation` applies the configured mode;
- `remote_operator` constrains both `allowlist` and legacy `full` settings to
  effective allowlist semantics;
- `untrusted_execution` denies agent-controlled host processes. Explicit
  `docker_exec` and WASM agents remain available when configured.

The daemon policy and each per-agent policy are intersected before tools are
advertised or executed. An agent manifest cannot broaden the deployment
profile, command mode, allowlists, blocklists, limits, or critical policy.

Host execution clears and reconstructs the child environment, but it is not an
operating-system sandbox. `captain status`, `captain security`, full doctor,
`GET /api/status`, `GET /api/health/detail`, and `GET /api/security` report
the profile, configured and effective policy, host permission,
`backend = "host_process"`, `isolation_level = "environment_scrub"`, and
`os_isolation = false`. Docker routing is always `explicit_only`. A failed or
disabled Docker rail never falls back to host execution. When Docker is enabled
under `untrusted_execution`, Captain requires network `none`, a read-only root,
no added capabilities, and finite CPU, memory, and PID limits.

```bash
captain agent caps <agent>
captain status
captain doctor --full
```

Use `captain config schema` for the typed `[approval]` and `[exec_policy]`
fields supported by the installed version. Do not copy permissive examples
from an older release or enable broad host access as a convenience.

### Adaptive delivery verification

Delivery verification is a runtime invariant, not a switch that broadens
authority. Pure conversation and read-only inspection finish without a second
model pass. Once a turn has effectful receipts, Captain evaluates their order,
scope, status, and evidence strength at the next useful milestone and before
delivery. A successful mutation alone is not proof of its post-condition; the
relevant check must be newer than the mutation. Pending detached work remains
pending until its terminal result is inspected.

Captain may request at most two targeted correction rounds. If evidence is
still missing, a budget or iteration limit prevents another check, or an
external effect is uncertain, the final result says what remains unverified.
Captain never replays that effect merely to manufacture proof. Durable records
contain only requirement codes, receipt order, digests, states, and timestamps;
they never contain hidden reasoning, raw tool input, or raw output. An abrupt
restart converts an unfinished verification lease to `interrupted` and
re-evaluates current state without replay.

The states `verifying`, `correcting`, `verification_verified`, and
`verification_incomplete` are ephemeral presentation signals. They do not add
messages to session history. Telegram shows a Rich verification card only for
a correction, incomplete delivery, or a verification that lasts more than
three seconds.

Alpha 12 also contains the disabled-by-default core for exact approval
suggestions:

```toml
[approval.suggestions]
enabled = false
minimum_approvals = 3
observation_window_hours = 720
dismissal_cooldown_hours = 168
```

When disabled, Captain records no learning observation. When explicitly
enabled, only repeated one-time approvals for the same agent, canonical tool,
complete action digest, and Low or Medium risk can become a suggestion. The
default requires three approvals within thirty days. A denial, session-scoped
choice, durable choice, or escalation to High/Critical clears that candidate.
A changed digest starts a distinct candidate and can never merge with the
first. High and Critical actions are never learned.

A suggestion grants no authority and does not alter prompts, model routing, or
tool visibility. Accepting one must be a separate operator action; it creates
the existing exact, revocable approval rule rather than broadening
`allow_always`. Dismissal grants nothing and starts a seven-day cooldown. The
core stores only identifiers, timestamps, risk, and the action digest in the
owner-only `approval-suggestions.json`; it stores no raw input, preview,
description, or model output. Corruption or persistence failure opens only the
suggestion circuit breaker and never blocks or upgrades the operator's current
one-time decision. If power is lost after an accepted exact rule commits but
before its candidate is removed, boot reconciles the two stores and removes the
stale suggestion without changing the rule.

This checkpoint is an internal ALPHA12 contract. Operator list, accept, and
dismiss controls are not yet claimed as available by the current source docs;
keep the feature disabled outside development until those authenticated
surfaces land.

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
