# Config + Secret family

> **Status:** audited (D.11).
> See [`README.md`](README.md) for the index and drift policy.
> Tool name list pinned in [`captain_runtime::captain_docs::CONFIG_SECRET_FAMILY_TOOLS`](../../crates/captain-runtime/src/captain_docs.rs).

## Tools

Every persistent mutation in this family must use the safe rail: **read current state -> backup -> atomic write -> roundtrip verify -> rollback on failure**. `config_write`, `web_credentials_update`, `secret_write`, and `config_setup` are the preferred tools; direct file edits are only a fallback when the typed tool cannot express the needed TOML shape.

### Config

#### `config_read`

Read a TOML key path from `~/.captain/config.toml`.

| Field | Required | Notes |
|---|---|---|
| `path` | yes | Dotted key path (`channels.telegram.bot_token_env`, `scheduling.timezone`). |

Returns the deserialised value (string / number / table) or `null` when the key is absent.

#### `config_write`

Set a TOML key path. Backed by a snapshot of `config.toml` before the write; on roundtrip failure the snapshot is restored.

| Field | Required | Notes |
|---|---|---|
| `path` | yes | Dotted key path. |
| `value` | yes | String input. The kernel infers only booleans (`true`/`false`), integers and floats; every other value is written as a TOML string. |

The write is **atomic** (write-temp + rename) and followed by a `load_config(...)` to confirm the file still parses. If parsing fails, the snapshot is restored before the call returns.

Use `config_write` for scalar global settings: model names, booleans, numeric budgets, env-var names, default chat ids. Do **not** use it for arrays/tables such as `allowed_users = ["..."]`; use `config_setup` when available, or a direct TOML edit plus parse check when no typed helper exists.

#### `config_setup`

High-level helper for a complete integration install. It validates credentials, stores secrets, patches `config.toml` while preserving comments, optionally runs a live test, and emits the hot-reload event for the integration.

| Field | Required | Notes |
|---|---|---|
| `integration` | yes | Canonical name: `telegram`, `tts_elevenlabs`, `tts_openai`, `stt_whisper`. ElevenLabs STT reuses `ELEVENLABS_API_KEY` from `tts_elevenlabs` when present. |
| `credentials` | yes | Integration-specific object. Telegram: `{bot_token, default_chat_id, allowed_users?[]}`. `tts_elevenlabs`: `{api_key, voice_id?, model_id?}` with `model` accepted as a compatibility alias. `tts_openai`: `{api_key, voice?, model?, format?}`. `stt_whisper`: `{api_key, provider?}`. |
| `run_test` | no | `true` runs the integration's live validation before reporting success. |

Prefer `config_setup` over a manual `secret_write` + `config_write` chain whenever the user is activating or replacing a full integration. It can write arrays/tables correctly where `config_write` cannot.

For TTS, `config_setup` is the source-of-truth rail: it writes `[tts].enabled`,
`[tts].provider`, provider credentials, and provider-specific voice/model fields
together. Do not let memory or a tool argument override a configured TTS
provider when the user has selected one in config.

#### `config_schema`

Returns the TOML schema for `config.toml` (allowed sections, fields, types). Useful when Captain needs to know what a section expects before writing it.

No parameters.

#### High-signal runtime keys

When the user asks about runtime behavior, do not hesitate or invent a new knob. These keys are first-class `config.toml` settings:

| Need | Preferred key(s) | Notes |
|---|---|---|
| Long analysis stops with `Max iterations exceeded` | `agent_loop.max_iterations` | Default `90`; recommend `180` or `240` for heavy analysis. Runtime clamps to `1..1000`. Read before writing and verify after write. |
| Session checkpoints | `checkpoints.enabled`, `checkpoints.model`, `checkpoints.inactivity_secs`, `checkpoints.emit_learning_review` | Defaults use Codex background models. With provider `codex`, runtime normalizes through the live Codex model list/cache before static fallback. |
| Native speech-to-text | `media.audio_transcription`, `media.audio_provider`, `media.audio_model` | Native install uses `local-whisper` + `whisper-small` with no API key. API providers can still be pinned explicitly. |
| Native text-to-speech | `tts.enabled`, `tts.provider`, `tts.local_native.preferred_engine`, `tts.local_native.fallback_engine` | Native install uses `local-native`; Kokoro is preferred and Piper is fallback by default. |
| ElevenLabs TTS | `tts.provider`, `tts.elevenlabs.api_key_env`, `tts.elevenlabs.voice_id`, `tts.elevenlabs.model_id` | Store the real key through `secret_write` or `config_setup`, never in config. |
| Web terminal exposure | `web_terminal.enabled`, `web_terminal.allow_raw_shell`, `deployment.public_url`, `api_listen` | On VPS, direct IP access requires `api_listen = "0.0.0.0:50051"` or a reverse proxy. |
| Learning autonomy | `learning.mode`, `learning.autonomy_aggressiveness`, `learning.reflection_model` | Aggressiveness is clamped to `0.25..3.0`; approval/auto/off still controls final writes. Default model follows the Codex subscription path. |
| Skill synthesis | `skills.mode`, `skills.pattern_threshold`, `skills.proposer_model`, `skills.generated_dir` | Use approval mode unless the user explicitly wants fully automatic skill writes. Default proposer follows the Codex subscription path. |

After any config surface changes in code, update `docs/configuration.md`, `captain.toml.example`, and this section so the runtime docs stay aligned with what Captain can actually write.

#### `web_credentials_update`

Rotate the login used by the web terminal. This is the native rail for natural
language requests such as "change mon mot de passe web", "renomme l'utilisateur
admin" or "génère-moi un nouveau mot de passe terminal".

| Field | Required | Notes |
|---|---|---|
| `username` | no | New web username. ASCII letters/digits/`._-`, 2-64 chars. |
| `password` | no | New web password. It is hashed before writing; never repeat it back if the user supplied it. |
| `generate_password` | no | `true` generates a strong password and returns it once in the tool result. |
| `session_ttl_hours` | no | Web session lifetime, 1-8760 hours. New installs default to 72 hours. |

The tool writes `[auth]` in `config.toml`, forces `auth.enabled = true`, creates
a config backup, validates the roundtrip parse, and emits the config hot-reload
event. The web auth layer reads `config.toml` live, so new credentials work
without restarting Captain. Password rotation invalidates old browser sessions
because the session signature is bound to both `api_key` and `password_hash`.

#### `self_configure`

Lets the caller agent reconfigure itself without touching `config.toml`. The target is the current agent context; the tool does not accept `agent_id`.

| Field | Required | Notes |
|---|---|---|
| `model` | no | New model id. |
| `provider` | no | Provider to pair with `model`. |
| `description` | no | Agent description. |
| `system_prompt` | no | Agent system prompt. Use small patches, not a wholesale rewrite. |
| `routing` | no | `{simple_model, medium_model, complex_model, simple_threshold, complex_threshold}`. |
| `fallback_models` | no | Array of `{provider, model}` fallbacks. |

### Model Switch and Codex OAuth

#### `model_switch_plan`

Prepare a safe global model/provider switch for the principal Captain agent. This is read-only: it checks the requested target, active session risk, auth state, tool/function calling support, streaming/vision capabilities, and whether a new or compacted session is required.

Use this before every change to `default_model.provider` or `default_model.model`. Do not write those fields directly with `config_write`.

#### `model_switch_apply`

Apply a previously prepared model switch after the user chooses the session strategy (`new_session` or `compact_session`). The tool updates the global default model/provider and performs the requested session migration.

Never call it without a fresh successful `model_switch_plan`, and never bypass a blocked plan by editing config manually.

#### `codex_auth_status`

Check whether Codex OAuth is installed, authenticated, expired, or missing. Codex OAuth is separate from OpenAI API-key auth; a working `OPENAI_API_KEY` does not prove that `codex/*` models can run.

#### `codex_tool_probe`

Run a live read-only Codex Responses probe with a fake `probe_pass` tool, then report whether the candidate model emitted a structured tool call. Use it before promoting a Codex model as Captain's main agent when tool-calling reliability is in doubt.

| Field | Required | Notes |
|---|---|---|
| `model` | no | Codex model to test, with or without the `codex/` prefix. Defaults to the first available Codex model, then `gpt-5.5`. |

The probe requires a valid Codex OAuth credential and does not mutate config, sessions, secrets, files, or external systems. If it returns `missing_auth`, complete `codex_login_start` / `codex_login_poll` or `captain login codex` first. If it returns `no_tool_call` or `error`, do not promote that model until the probe passes.

#### `codex_login_start`

Start the Codex OAuth device-code flow from the current conversation. Use it when the user wants a Codex model and `codex_auth_status` or `model_switch_plan` reports missing/expired auth.

#### `codex_login_poll`

Poll the Codex OAuth flow started by `codex_login_start` until the user approves it, it succeeds, or it expires. After success, run `model_switch_plan` again before applying a Codex switch.

### Secrets

#### `secret_read`

Read a secret by key from `~/.captain/secrets.env`. Use **spontaneously** when the user mentions an alias / service that needs an API key (`chargement clé Stripe`, `je veux utiliser le token GitHub que j'ai déjà mis`).

| Field | Required | Notes |
|---|---|---|
| `key` | yes | Env-style key (`OPENAI_API_KEY`, `STRIPE_TEST_KEY`). |

Returns a masked confirmation string, or "not found" if missing. The LLM should use `secret_read` to check whether a key exists, not to extract and paste the raw value. Use first-class tools/integrations/skills with env injection when raw credentials are needed for execution.

#### `secret_write`

Set a secret in `~/.captain/secrets.env`. The key/value must be single-line. The kernel now backs up existing `secrets.env`, writes through a temp file, enforces `0600` permissions on Unix, and roundtrip-checks that the key can be read back.

| Field | Required | Notes |
|---|---|---|
| `key` | yes | Env-style key. |
| `value` | yes | The secret. |

A successful `secret_write` updates the stored value only. If a running adapter already holds the old secret, call its reload tool after updating the config pointer.

Important: never load `~/.captain/secrets.env` with `source`, `.`, `set -a`, or
another raw shell import. The file is Captain's canonical credential store, not
a shell profile; some legacy or integration entries can use logical identifiers
that are not valid shell variable names. Use `secret_read`, typed integration
tools, or a skill with `[requirements.env_inject]` instead.

### Credential Resolution Chain

Runtime credential lookup is intentionally one rail:

1. `~/.captain/secrets.env` (canonical, updated by `secret_write` / typed installers).
2. `~/.captain/vault.enc` (legacy compatibility).
3. `~/.captain/.env` (legacy/bootstrap compatibility).
4. Process environment.

Do not reason about MCP/API credentials as a separate vault. If an integration install reports `setup`, first check whether the required key is present through `secret_read`, then persist missing values with `secret_write` or the typed installer credentials object.

### No Literal Secret Sinks

Once a value is a credential, treat the raw literal as toxic. It may be passed only to `secret_write` / `config_setup` credential fields. It must never be copied into generated files, scripts, patches, shell commands, HTTP headers/bodies, process stdin, channel messages, memory, docs or logs.

Runtime guards refuse high-risk sinks (`file_write`, `edit_file`, `multi_edit`, `apply_patch`, `execute_code`, `shell_exec`, `docker_exec`, `process_*`, `web_fetch`, `web_download`, `channel_send`) when their model-controlled content contains a value that looks like an API key/token/password. The error is intentional feedback for the model: switch to the vault flow instead of retrying with the raw value.

Tool errors include a `[tool_error]` JSON block with `code`, `retryable`, `severity`, `next_action` and `docs_query`. Treat it as instructions for recovery: do the next action, consult `captain_docs` when requested, and do not ask the user before checking the available native rails.

Correct pattern:

1. Store or rotate: `secret_write({"key":"GEMINI_API_KEY","value":"<user-provided secret>"})`.
2. Verify later: `secret_read({"key":"GEMINI_API_KEY"})` only confirms masked presence.
3. Execute with a native integration or a skill whose manifest declares `[requirements.env_inject]` so the vault injects `GEMINI_API_KEY` at runtime.
4. Generated code may reference `os.getenv("GEMINI_API_KEY")`, `process.env.GEMINI_API_KEY`, or `$GEMINI_API_KEY` only when that runtime injection exists. It must never contain the real key.

## Sandbox

- **File access** — `~/.captain/` is an authorised root for the principal Captain agent so it can maintain its own sessions, config, docs and state. Raw credential stores are the exception: `.env`, `secrets.env`, `secrets-backups/` and `vault.enc` are blocked from generic file tools and must go through `secret_read`, `secret_write`, `config_setup`, `ssh_*` or integration-specific tools.
- **0600 perms** — `secrets.env` is created with mode `0600` on Unix; the file system owner-only check is part of the sanity test the kernel runs at boot.
- **Backup snapshots** — `config_write` writes to `~/.captain/config-backups/config.toml.<timestamp>` and keeps the newest 20. `web_credentials_update` writes to `~/.captain/config-backups/config.toml.web-auth.<timestamp>`. `secret_write` writes to `~/.captain/secrets-backups/secrets.env.<timestamp>` and keeps the newest 20.
- **Roundtrip verification** — after every mutation the file is re-parsed (`captain_kernel::config::load_config` or the secrets-env mini parser). A parse error triggers an immediate rollback to the snapshot **before** the call returns to the LLM.
- **No secret-in-config rule** — `config_write` now refuses obvious raw credential literals, but this is only a guardrail. Real credentials live in `secrets.env` via `secret_write` or `config_setup`, never in `config.toml`.
- **Hot-reload** — `config_setup` publishes the integration-configured event for supported integrations. A manual `secret_write` alone only updates `secrets.env`; call the relevant reload tool (`channel_reconfigure` for channels) when the running adapter already holds the old value.

## Autonomous config playbook

When changing Captain's own behavior, choose the narrowest tool:

1. **Need your own model/provider changed?** Use `model_switch_plan` first, then ask the user to choose `new_session` or `compact_session`, then call `model_switch_apply`. Never write `default_model.provider` / `default_model.model` directly: Claude, Codex and OpenAI do not share identical tool-call history formats, so a provider switch is a session migration.
2. **Need your own prompt/routing/fallbacks changed?** Use `self_configure`, then inspect the agent state if available. This is per-agent, not global. If `model` or `provider` is included, `self_configure` requires the same explicit `session_strategy` safety rail.
3. **Need a scalar global setting changed?** Call `config_schema`, then `config_read(path)`, then `config_write(path, value)`, then read back the same path.
4. **Need the public assistant name or answer style changed?** Use `config_write("assistant.display_name", "...")` or `config_write("assistant.style", "balanced|concise|professional|developer|friendly|classic")`. The internal `captain` slug is not renamed by this.
5. **Need the web terminal login changed?** Use `web_credentials_update`, not `config_write("auth.password_hash", ...)`: the native rail hashes, backs up, validates and hot-reloads.
6. **Need a secret stored or rotated?** Use `secret_write` for the secret and `config_write` only for the env-var pointer. Never place the raw value in config.
7. **Need Telegram/TTS/STT installed or replaced?** Use `config_setup({integration, credentials, run_test:true})` when possible; it handles arrays, vault keys, TOML patches and hot reload.
8. **Need to call an API with a stored key?** Prefer a first-class integration/tool. If none exists, create or use a skill with `env_inject`; do not create an ad-hoc script containing the key.
9. **Need an array/table not covered by a typed helper?** Read `config_schema`, inspect the current TOML, make the smallest direct TOML edit, parse-check immediately, then call the specific reload tool if one exists.

## Limites

- `config_write` cannot delete a key — pass an empty string / `null` is interpreted as "set to that value", not "remove". Removal still requires `file_read` + `edit_file` + roundtrip on `config.toml` directly.
- `config_write` cannot write arrays or inline tables from JSON; a value like `["123"]` is written as a string unless a typed helper such as `config_setup` handles that integration.
- `secret_read` / `secret_write` are **not** the right tool for OAuth refresh tokens that expire — those have their own per-driver token cache (e.g., `CopilotTokenCache`). Use this family for static credentials only.
- `secret_write` rejects multiline values and leading/trailing whitespace because `secrets.env` is a line-based format.
- `model_switch_apply` refuses target models that do not advertise tool/function calling and refuses providers whose driver cannot initialize. If the user wants to switch despite that, report the blocker instead of bypassing the rail with raw config edits.
- `config_schema` mirrors the kernel's struct definition at compile time; if the running daemon is older than the schema you're reading, fields the schema lists may not yet exist.
- `config_setup` covers only registered integrations. Unknown integrations require either a new setup module or a skill/tool dedicated to that service.

## Exemples

### Golden path — set a per-channel field, then hot-reload

```
1. config_setup({
     "integration": "telegram",
     "credentials": {
       "bot_token": "...",
       "default_chat_id": "123456",
       "allowed_users": ["123456"]
     },
     "run_test": true
   })
2. channel_send({"channel": "telegram", "recipient": "", "message": "Telegram is ready."})
```

### Error case — rolled-back write that broke parsing

```
config_write({"path": "channels.telegram.allowed_users", "value": "[\"123\"]"})
→ success, but the value is a string, not an allowlist array. Use config_setup or a TOML edit instead.
```

The roundtrip guard means the file on disk is exactly what it was before the call.

### Error case — unknown key on secret_read

```
secret_read({"key": "MISSING_KEY"})
→ {"value": null}
```

`null` (not an error) means "I have no record" — Captain should follow up with `ask_user` if the key is genuinely needed.
