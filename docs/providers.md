# Model Providers

Captain is Codex-first. Other providers remain available, but the installed
runtime catalog is the source of truth: provider and model inventories can
change independently of this guide.

```bash
captain models providers
captain models list
captain models aliases
captain models current
captain models test
```

Do not choose a model from a copied catalog, an old README, or a fixed price
table. Run these commands against the binary that will execute the work.

## Codex with a ChatGPT Subscription

Codex is the default Captain path and does not require an OpenAI API key:

```bash
captain auth login codex
captain auth status
captain models test
```

The device login works on a headless VPS: open the displayed URL on another
device and enter the code. Captain reuses the resulting Codex subscription
credentials without copying them into `config.toml`.

When at least one registered agent uses Codex, Captain refreshes the official
catalog shortly after startup and then once per hour. The request declares
`client_version=1.0.0` as the catalog protocol capability; this is not the
Captain release version.

Newly listed models are discovered but never activated automatically. Control
and configured Telegram delivery present an explicit decision. **Conserver**
keeps the current model. **Basculer** requires the agent plus a safe session
choice: **Nouvelle session** or **Résumé compact**. A failed refresh preserves
the last valid catalog and remains visible through:

```bash
curl -sS -H "Authorization: Bearer $CAPTAIN_API_KEY" \
  http://127.0.0.1:50051/api/models/updates
```

## Context Window Authority

Every turn resolves the configured provider/model against the live runtime
catalog. That model entry supplies the effective context capacity used by
pre-loop compaction, post-loop compaction, session metadata, API responses,
and the TUI `ctx used/window` meter. A model switch changes the budget without
requiring a Captain restart, and reopening a session refreshes its capacity
from the model currently configured on its owning agent.

For Codex, Captain reads the official cache's active `context_window` field.
`max_context_window` is an upper bound for an explicit Codex override, not the
default active window, so Captain does not promote it to the normal budget.
When an older cache omits active metadata, Captain uses a conservative bounded
fallback instead of assuming the largest advertised ceiling. Inspect the
installed truth with `captain models list` or `GET /api/models`; do not copy a
context size from an old release note.

Session token totals are billing/usage counters, not current context pressure.
The TUI uses the latest provider-reported prompt usage and adds only unreported
stream output. Session APIs expose `context_window_tokens` for capacity and
`estimated_context_tokens` for approximate transcript occupancy.

## API-Key Providers

Captain also supports native and OpenAI-compatible providers. Set only the
credential required by the provider you intentionally configured. Common
credentials include:

| Provider family | Credential |
|---|---|
| Anthropic | `ANTHROPIC_API_KEY` |
| OpenAI API | `OPENAI_API_KEY` |
| Google Gemini | `GEMINI_API_KEY` or `GOOGLE_API_KEY` |
| Groq | `GROQ_API_KEY` |
| Mistral | `MISTRAL_API_KEY` |
| OpenRouter | `OPENROUTER_API_KEY` |

The live `captain models providers` output lists every provider recognized by
the installed binary and whether its authentication is ready. Local
OpenAI-compatible servers can be configured with a provider URL; verify them
with `captain models test` before assigning production work.

Keep secrets in the OS keyring, Captain secret store, `secrets.env`, or
environment variables with restricted permissions. Never put a raw API key in
an agent manifest, repository, issue, or chat transcript.

## Select a Model

The guided setup is the safest path:

```bash
captain setup
```

For an explicit default:

```bash
captain config set default_model.provider codex
captain config set default_model.model gpt-5.5
captain models current
captain models test
```

For a specialized agent, `model` is a TOML table:

```toml
[model]
provider = "codex"
model = "gpt-5.5"
system_prompt = "Work within the declared role and report uncertainty."
```

Do not write `model = "codex:gpt-5.5"`. Inspect the live catalog before using
an example model ID; examples document structure, not perpetual availability.

## Switching a Running Agent

A provider or model change must not silently reinterpret an existing
conversation. Captain preflights compatibility, then requires one of the safe
session strategies when needed:

- start a new session and preserve the old transcript;
- compact the current session into a provider-portable summary;
- cancel the switch.

Use Control or the model commands surfaced by the installed CLI. Verify the
result with `captain models current`, `captain agent list`, and
`captain agent caps <agent>`.

## Configured Model Authority

Every normal agent turn uses the provider and model declared on that agent.
Captain does not classify a request as simple, medium, or complex to substitute
another model, and it does not change model because a conversation became long
or moved between CLI, Web, Desktop, API, or a messaging channel.

When work benefits from a different model or role, Captain creates or delegates
to an explicit specialist sub-agent. Changing the current agent itself remains
an explicit model-switch decision with a safe session strategy.

Fallback providers are optional, failure-only continuity settings. Captain
never infers them from credentials present on the host. Leave
`fallback_providers` and an agent's `fallback_models` empty for strict
single-model execution. If fallbacks are configured deliberately, test each
provider first and keep the ordered chain bounded. A fallback is not task
routing and must not bypass capability, budget, or image requirements.

Images and prompted browser screenshots stay on the active conversation model.
Captain never auto-spawns a secondary Vision agent or changes provider to
analyze pixels. If the selected model is text-only, the request fails with an
actionable model-selection message.

## Budgets and Usage

Provider readiness does not imply an unlimited budget. Inspect live usage and
agent limits:

```bash
captain status
captain agent caps <agent>
captain doctor --full
```

Captain keeps two independent contracts visible:

- **Captain internal guard:** each agent's durable rolling
  `max_llm_tokens_per_hour`, plus configured cost limits. Restarting the daemon
  does not reset the rolling token ledger.
- **Provider subscription (reported):** allowance windows owned and reported
  by the configured provider. These values cannot be reconstructed reliably
  from Captain's local usage counters.

For Codex, Captain reads the authenticated account-usage endpoint associated
with the configured official base (`/backend-api/wham/usage` for ChatGPT or
`/api/codex/usage` for the Codex service). It refreshes that status immediately
after daemon startup and every five minutes, and supplements it with dynamic
`x-codex-*` response headers and `codex.rate_limits` stream events from real
model calls. Durations, percentages, reset timestamps, plan labels, credits,
and additional metered families are stored exactly as reported. Captain does
not hard-code a five-hour or weekly allowance.

`captain status --verbose`, the Status hub, `GET /api/status`, and
`GET /api/budget` show these provider observations separately. `unavailable`
means that no current official observation exists; it never means unlimited.
Data older than fifteen minutes is marked `stale`. A provider-reported
exhaustion is not retried or silently routed to a fallback provider.

During interactive use, full-screen Ratatui Chat polls only Captain's local
snapshot every five seconds. Its compact bottom band names the active model
first and renders gauges only for provider-wide windows and limit families
whose reported identifier or name matches that model. Additional model-specific
families are summarized as outside the active model, with warning or critical
pressure still surfaced; the exhaustive report remains in Status and Budget.
The xterm Web terminal launches that same standalone Ratatui chat. Control web
renders the equivalent responsive contract from `/api/budget`, and the frozen
desktop compatibility wrapper inherits that exact Control asset when built.
These interface polls do not increase calls to the provider: the daemon remains
the only owner of account refresh, response observation, persistence,
staleness, and exhaustion decisions.

Provider prices and nominal plan entitlements remain absent from this guide
because they change. Use live provider observations and the provider's current
billing page instead of a copied static table.

## Troubleshooting

1. Run `captain auth status` and `captain models providers`.
2. Confirm the configured provider and model exist in `captain models list`.
3. Run `captain models test` before diagnosing agent behavior.
4. Use `captain doctor --full` for credentials, network, catalog, and daemon
   health.
5. Inspect `GET /api/models/updates` when Codex discovery is degraded.

See [Configuration](configuration.md), [CLI Reference](cli-reference.md), and
[API Reference](api-reference.md#model-catalog-endpoints).
