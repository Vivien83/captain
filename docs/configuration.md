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
```

Run `captain setup` to generate authentication material and complete provider
login. Do not copy this example over an existing production file.

## Secrets

`config.toml` should contain secret references, not secret values. Captain can
use the OS keyring, encrypted vault, restricted `secrets.env`, or an
environment variable named by a `*_env` field.

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
```

The password hash and daemon API key are provisioned by setup and stored in the
secret path, not copied into this example.

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
critical-command handling. Keep destructive actions behind explicit human
approval and inspect the effective agent capabilities before broadening them.

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
