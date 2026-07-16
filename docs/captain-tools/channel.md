# Channel family

> **Status:** audited (D.8).
> See [`README.md`](README.md) for the index and drift policy.
> Tool name list pinned in [`captain_runtime::captain_docs::CHANNEL_FAMILY_TOOLS`](../../crates/captain-runtime/src/captain_docs.rs).

## Tools

Captain talks to the outside world through a per-channel adapter pool managed by `BridgeManager`. The active core setup surface is Telegram, Discord, Signal, and Email; non-core channels may remain compiled for compatibility but are frozen out of setup and normal bridge startup until the core is Hermes-level. The tools below let an agent push a message to any active channel, rotate adapter config without disturbing the others, and manage Telegram topic routing.

### `channel_delivery_batch`

Grouped outbound delivery. Sends up to ten channel messages/attachments in one
tool call by delegating each item to `channel_send`. Use it for multi-recipient
or summary-plus-file deliveries across active channels where separate tool calls
would add latency and context churn.

Each item in `deliveries` accepts the same fields as `channel_send`. The batch
is sequential and `stop_on_error=true` stops after the first failed delivery.

### `channel_send`

Send a message to a specific channel. The default outbound verb whenever the user asks Captain to deliver content somewhere.

| Field | Required | Notes |
|---|---|---|
| `channel` | yes | Active channel name as it appears in `[channels.<name>]` (`telegram`, `discord`, `signal`, `email`). |
| `recipient` | no | Recipient platform-id. Omit or pass an empty string to use the channel default when configured. |
| `message` | yes | Plain text. The bridge auto-selects the channel's output format (`TelegramHtml`, Discord/plain, Email subject/body) so cross-platform pasting works. |
| `file_path` | no | Optional attachment relative to the workspace; the bridge handles MIME detection. |
| `thread_id` | no | Telegram forum-topic id for thread-aware routing. |

Returns `{"status": "sent", "channel": "...", "recipient": "..."}` on success.

When the user asks Captain to deliver a result somewhere, the **chat reply must repeat the delivered content verbatim** — never a meta-summary like "Message envoyé sur Telegram avec les résultats." The tool's success status is the implicit confirmation.

### `channel_reconfigure` (A.2)

Hot-reload **one** channel adapter after editing its `config.toml` section. Use **spontaneously** when the user says `change le bot Telegram`, `mets le nouveau token X`, `reconnecte Discord avec ces paramètres`.

| Field | Required | Notes |
|---|---|---|
| `channel` | yes | Channel name as it appears in `[channels.<name>]`. |

Validation reads the live `config.toml` first; an unknown name returns a verbose error listing the configured channels, and a configured non-core name returns a frozen-channel error listing the active set. On success the kernel publishes `SystemEvent::IntegrationConfigured { name }`, the API listener picks it up and calls `BridgeManager::reload_channel(name, new_adapter)` (A.1) — only that channel restarts.

The expected workflow for an existing channel is `config_read` -> `secret_write` if a credential changes -> `config_write` for scalar fields -> `channel_reconfigure({channel})`. For a full Telegram install or replacement, prefer `config_setup({integration:"telegram", ...})` because it can write arrays such as `allowed_users` correctly. **Don't restart the daemon** — it would disconnect every other channel for several seconds.

### Adding a brand-new channel without a daemon restart

Captain can wire a channel that was **never booted** (no `[channels.<name>]` section at startup, or section present but token absent). The flow is identical to a config rotation, just starting from zero:

1. `config_read` — confirm there is no live section yet, or that the current one is incomplete.
2. `secret_write({key: "<NAME>_BOT_TOKEN", value: "..."})` — store the token. The runtime mirrors the value into `std::env` immediately, so the next bridge spin sees it without a daemon restart.
3. `config_setup({integration:"<name>", ...})` (preferred) **or** `config_write` for the few scalar fields the integration needs (`bot_token_env`, `default_channel_id`, `allowed_users`, ...). For Discord the minimal useful section includes `bot_token_env = "DISCORD_BOT_TOKEN"` and a non-empty `allowed_users` list.
4. `channel_reconfigure({channel: "<name>"})` — publishes `IntegrationConfigured`. The hot-reload listener re-reads `secrets.env`, rebuilds the bridge from the fresh config, and inserts the new adapter into `kernel.channel_adapters`. Other channels keep dispatching.
5. Verify by reading the channel context block in your next prompt — the new channel should now read `ACTIVE` (live adapter) instead of `CONFIGURED` (TOML only).
6. `channel_send({channel: "<name>", recipient: "...", message: "..."})` — works directly. The system-wide default channel does not change unless you set `default_agent` for the new section; leave it absent if you only want the channel to be addressable, not promoted.

The same flow brings up Discord, Signal, or Email. Non-core channels such as Slack, Matrix, IRC, and WhatsApp are intentionally frozen from the active setup surface and normal bridge startup for now. If an old `config.toml` still contains those sections, Captain logs them as frozen and does not start them by default.

### Channel readiness API

`GET /api/channels` is the operator status surface for channel setup. For each active core channel it returns:

- `ready`: all required runtime inputs are present.
- `missing_required_fields`: missing env/config fields such as `TELEGRAM_BOT_TOKEN` or `allowed_users`.
- `operator_actions`: concrete setup steps to make the channel ready.
- `security_state`: `locked`, `allowlist`, or `allow_all_explicit`.

Telegram, Discord, Signal, and Email are deny-by-default. A token, API URL, or mailbox password alone is not enough: set `allowed_users` for Telegram/Discord/Signal, set `allowed_senders` for Email, or use `["*"]` only when intentionally allowing everyone.

`GET /api/status` also carries a compact `channels` summary for day-to-day diagnostics: configured, ready and locked channel names, plus the same missing-field actions used by `captain status`.

`GET /api/status.channels.inbound_queue` reports operator-safe counters for active, pending, recovered in-flight, dead-lettered, and interjected inbound channel sessions. It includes only aggregate counts, recovery budget, dead-letter retention, oldest dead-letter age, operator actions, and per-channel totals; it deliberately omits chat ids, user ids, message text, and thread ids. `captain status --verbose` prints active, queued, inflight retry, dead-letter, and interjected counts when a channel turn is active or a follow-up is queued/injected. A plain text follow-up can be interjected only into the active streaming turn for the exact same channel session; another chat, user, or topic targeting the same agent is queued separately. Once the outbound stream has closed or the active interjection buffer is full, follow-ups are queued as the next turn instead of being accepted into a loop that cannot safely read them. `/queue <message>` forces the stripped message into the next turn, while `/steer <message>` explicitly tries the active stream first and then falls back to queueing. If no active stream accepts a follow-up, the first queued follow-up sends a short visible acknowledgment, then acknowledgments are debounced per session for 30 seconds so rapid bursts do not spam the channel.

Pending inbound follow-ups are durably stored in `channel_inbound_queue.json` under the Captain home directory. The file is bounded and written through a `.tmp` file then atomically renamed. On bridge start, each active channel drains its own recovered pending messages before accepting new follow-ups. A recovered item remains durable as `inflight` until the dispatcher finishes it, so a crash during recovery retries the message instead of silently dropping it. After repeated unfinished recoveries, Captain moves the entry to a durable dead letter so status shows operator action is needed instead of retrying forever. Dead letters are timestamped, status exposes only their age, and stale dead letters are pruned after 24 hours when the queue loads. After review, `DELETE /api/channels/inbound-queue/dead-letters` clears handled dead letters without returning message content; add `?channel=telegram` or another active channel name to clear one channel. The CLI wrapper is `captain channel inbound dead-letters clear --channel telegram`; `captain channels ...` is accepted as an alias. The action is audited with counts only and stays content-free: review channel logs and ask the affected user to resend after the underlying issue is fixed.

### Telegram-first operating rules

Most end-user traffic is expected to come from Telegram. Treat Telegram as a primary UX surface, not as a logging sink.

- Send the actual answer/content, not a meta confirmation. Never add a second message like "Rappel envoyé sur Telegram".
- Normal final answers are Rich-first and preserve GFM tables, lists, code, and collapsible details. Keep each message readable on mobile; for long reports, lead with the conclusion and structure details instead of flattening them into ASCII.
- Consecutive independent tool starts share one live activity board. A progress or result event closes that parallel wave; a dependent tool belongs to the next board. Do not narrate each tool in a second message.
- Private long turns refresh one ephemeral operational draft after 20 seconds of real inactivity and before Telegram's 30-second draft TTL. Text, tool activity, and visible edits reset that timer. Do not manually add duplicate "still working" messages unless the next action genuinely changed.
- `ask_user` questions are stateful Rich cards. A button or freeform reply must reach the active turn before the card is confirmed; answered and expired cards clear their keyboard. Never tell the user a choice was recorded merely because a callback arrived.
- Explicitly unsupported Rich endpoints use the cached HTML/plain fallback. Network ambiguity and 5xx failures must not be retried through another send path because Telegram may already have accepted the request.
- During a long channel turn, exact mobile messages like `Stop`, `annule`, or `/stop` plus recognized slash commands (`/approve`, `/reject`, `/status`, `/new`, etc.) bypass the active-session queue and run immediately. Normal plain text follow-ups for the same channel/chat/user/thread are first offered to the active stream as context interjections; `/steer <message>` forces that explicit intent, and `/queue <message>` skips active interjection to preserve the message for the next turn. If no active stream accepts a follow-up, it is queued, the first queued follow-up gets a short debounced acknowledgment, rapid text bursts are appended into one pending turn, and the bridge drains that pending turn before releasing the session.
- When a cron/goal is created from Telegram, set explicit Telegram delivery/escalation when the tool supports it; do not rely on a hidden home-channel guess.
- In groups, respect topics: use `thread_id` for one-off sends and `telegram_set_topic` only for durable agent/Hand routing.
- Before changing Telegram config, inspect `channels.telegram` and prefer `config_setup` over hand-editing TOML.

### Telegram forum topics

`telegram_set_topic` and `telegram_get_topic` manage persistent routing between an agent/Hand name and a Telegram forum topic id.

| Tool | Use |
|---|---|
| `telegram_set_topic({agent_name, topic_id})` | Persist the topic id used for that agent's outbound Telegram messages. |
| `telegram_get_topic({agent_name})` | Inspect the current mapping before changing it. |

Use these only for Telegram groups with forum topics enabled. For a one-off threaded message, pass `thread_id` / `topic_id` directly to `channel_send` instead.

## Sandbox

- **No raw token access for config** — bot tokens / API keys live under `~/.captain/secrets.env` or integration vault keys. The channel config stores env/key references, not the token itself.
- **Channel setup secret boundary** — `POST /api/channels/{name}/configure` writes secret form values to `secrets.env`, stores only env-var pointers such as `EMAIL_PASSWORD` in `config.toml`, and rejects multiline secret values.
- **Attachment URL caveat** — inbound Telegram media URLs currently contain the bot token in the Bot API file URL. Treat these URLs as secret-bearing: use them only for immediate processing, never echo them to users, docs, logs, memory, or other channels.
- **Outbound secret guard** — `channel_send` refuses text, captions, URLs and text attachments that look like raw API keys/tokens/passwords. If a message needs to mention a credential, refer to the vault key name (`GEMINI_API_KEY`) or a masked confirmation, never the literal value.
- **Discord mention guard** — Discord outbound payloads set `allowed_mentions` explicitly: user mentions are allowed, but `@everyone`, `@here`, and role pings are disabled for agent-generated messages.
- **`config.toml` writes should go through config tools** — use `config_setup` / `config_write` first. Direct `file_write` or `edit_file` is a fallback for TOML shapes the tools cannot express.
- **RBAC / allowlists (B.8)** — Telegram, Discord, and Signal user allowlists route through the shared RBAC helper: `allowed_users = []` means **deny all** and `allowed_users = ["*"]` explicitly allows everyone. Email uses the same deny-by-default contract with `allowed_senders = []` and supports explicit sender addresses/domains or `["*"]`. Guild, channel and tenant lists remain routing filters with their own documented semantics.

#### RBAC coverage matrix (audit 2026-05-04)

`is_authorized` is enforced **at the adapter parse path**, before
`dispatch_message` -> `handle_command` ever sees the message. That means
sensitive slash commands (`model`, `compact`, `stop`, `usage`, `think`,
`new`/`clear`) are protected only on adapters that wire the helper.

| Adapter | RBAC enforced | Sensitive commands gated |
|---|---|---|
| telegram | ✅ `parse_telegram_message` + `callback_query` filter | yes |
| discord | ✅ `parse_discord_message` filter | yes |
| signal | ✅ adapter parse filter | yes |
| email | ✅ adapter sender filter via `allowed_senders` | yes |
| whatsapp | ✅ adapter parse filter, but channel is frozen from the active surface | yes when explicitly started outside the core policy |
| slack, matrix, irc, mattermost, rocketchat, teams, zulip, xmpp, nostr, line, viber, wecom, messenger, googlechat, bluesky, webhook | ❌ no `is_authorized` call in parse path | **no — any sender can run sensitive commands** |

**Captain-side rule**: when adding RBAC to a new adapter, plug
`crate::rbac::is_authorized(&allowed_users, &user_id)` inside the adapter's
parse function (mirror Telegram/Discord). Don't try to enforce it in
`bridge::handle_command` — by then the parse layer already accepted the
message and the sender context may not be precise enough.
- **Telegram callbacks share RBAC** — inline-button `callback_query` updates are gated by the same `allowed_users` list as text messages.
- **Per-channel watch (A.1)** — `BridgeManager` keeps one private `watch::Sender<bool>` per adapter. `channel_reconfigure` signals only that channel; the others keep dispatching messages.

## Limites

- `channel_send` retries transient channel failures with jittered backoff. Ambiguous read/write timeouts are not retried because the platform may already have accepted the message.
- Inbound channel sessions are serialized per channel/chat/user/thread. Captain keeps one active agent turn and one pending follow-up slot for that session. Plain text follow-ups are injected into an accepted active stream when possible; `/steer <message>` is the explicit form, `/queue <message>` is the explicit next-turn form, and `@agent` routing plus media stay in the normal routing/queue path. A newer non-text pending message replaces the prior pending item, while pending text messages are appended in arrival order. Follow-up acknowledgments are visible but rate-limited to one per 30 seconds for the same session.
- Pending follow-up turns survive daemon restart through `channel_inbound_queue.json`. Recovered entries are drained per active channel and removed from the durable queue only when dispatch completes; if Captain crashes mid-dispatch, the recovered follow-up is retried on the next start and remains visible as `inflight` in status. If the same recovered entry repeatedly fails to complete, it becomes a durable dead letter visible in status.
- Telegram Rich replies are split conservatively at the native 32,768-byte ceiling; the legacy HTML/plain fallback keeps its smaller platform-safe chunks. Discord uses its own 2 KB limit. Chunks are sent in order but each one is a separate platform message.
- `channel_reconfigure` validates the channel name against `config.toml` live. A typo (`telegrm`) returns the configured-names list — fix and retry rather than guessing.
- `channel_send` and `channel_reconfigure` reject non-core channels in the runtime tool layer. If `slack`, `matrix`, `whatsapp`, etc. appears in config or old docs, treat it as frozen and use Telegram, Discord, Signal, or Email instead.
- The reload primitive expects `config.toml` to already be on disk before being called. Captain's typical sequence is **config_write/config_setup** then **channel_reconfigure** in the same turn; calling `channel_reconfigure` on a stale config restarts the adapter with the old token.
- Only the targeted channel restarts. Adapter-specific state (Telegram poll cursor, Discord gateway session) is reset for that channel only — others keep their cursors.
- `channel_reconfigure` is not idempotent over rapid fire — calling it three times in a row spawns three replacements (the bridge serialises them, so the third one wins). Avoid repeated invocations within the same turn.
- `telegram_set_topic` validates neither the remote topic existence nor its title; it stores the id Captain was given. Send a test message with `channel_send` if the mapping matters immediately.
- Telegram inline-keyboard `model_switch` plans expire after **5 minutes** in the bridge cache. If the user clicks an old button after the TTL, the callback returns `Ce choix de switch a expiré (5 min). Relance /model <modèle>.` — Captain must re-open a fresh `/model X` plan instead of asking the user to click again. Last-wins concurrency is also enforced: a second `/model` call from the same agent invalidates the first agent's pending plan.
- Telegram startup currently calls `deleteWebhook(drop_pending_updates=true)` before long polling. This avoids webhook/polling conflicts but can discard queued messages during restarts; avoid unnecessary restarts and prefer targeted reconfigure.

## Exemples

### Golden path — send a reply to Telegram default chat

```
channel_send({
  "channel": "telegram",
  "recipient": "",
  "message": "Status check complete: service is healthy."
})
→ {"status": "sent", "channel": "telegram", "recipient": "..."}
```

### Golden path — rotate the Telegram bot token

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
2. channel_send({"channel": "telegram", "recipient": "", "message": "Bot rotated."})
```

### Error case — reconfigure on a typo

```
channel_reconfigure({"channel": "telegrm"})
→ Err("channel 'telegrm' is not declared under [channels.*] in <home>/.captain/config.toml — known: telegram, discord, slack").
```

The error lists what's actually configured so the next call lands.
