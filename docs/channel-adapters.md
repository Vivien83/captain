# Channel Adapters

Captain's active external messaging tier is deliberately small: Telegram,
Discord, Signal, and Email. These channels use the same durable session,
memory, agent, policy, and observability contracts as CLI, TUI, and Web.

Other adapter implementations may remain in source for compatibility, but
they are frozen and intentionally omitted from this ready-to-use guide.

## Set Up a Channel

Run the guided setup instead of editing secrets into `config.toml`:

```bash
captain channel setup telegram
captain channel setup discord
captain channel setup signal
captain channel setup email
```

The wizard explains the provider-specific prerequisites, stores secret values
outside the public configuration, writes the channel configuration, and tells
you when the daemon must be restarted.

Then verify the runtime, not just the presence of a token:

```bash
captain channel list
captain channel test <telegram|discord|signal|email>
captain status
captain doctor --full
```

`captain channel list` can display frozen compatibility entries found in an
older configuration. Their presence does not make them an active supported
setup path.

## Active Channels

| Channel | Runtime transport | Primary prerequisite |
|---|---|---|
| Telegram | Bot API long polling | Bot token from BotFather |
| Discord | Gateway WebSocket | Discord bot token and required intents |
| Signal | Local signal-cli service | Registered Signal account and local service |
| Email | IMAP and SMTP | Mailbox credentials and server settings |

Follow the setup wizard for the exact fields required by the installed Captain
version. Never paste a bot token, mailbox password, or callback secret into a
manifest, repository, issue, or chat message.

### Telegram Rich Messages

Telegram is Rich-first for normal Captain replies. Bot API 10.2 preserves GFM
tables, lists, code, and collapsible details; exact plain-text commands and raw
dumps keep their literal transport. Private chats form a response in an
ephemeral draft and then persist one final message, while groups use persistent
Rich messages directly.

Consecutive independent tools share one editable activity board. A dependent
tool opens the next board after the prior wave produces progress or a result.
Long private turns show one ephemeral operational draft only after real
inactivity, so the durable transcript does not fill with heartbeat notices.

Interactive `ask_user` prompts are stateful Rich cards. Buttons and freeform
replies unblock the active turn; answer and expiry edits remove the inline
keyboard. Captain Forge approvals and uncertain-run recovery use a separate
control-plane card: the adapter acknowledges the click, the bridge resolves it
before session dispatch, and the kernel validates the exact durable identity
without asking the model. Pending cards are rebuilt from state after restart;
accepted and stale decisions edit the original message and remove its buttons.
User-visible errors are sanitized before rendering. If a Telegram
server explicitly lacks the Rich endpoints, Captain caches that result and
uses the existing HTML/plain fallback. Ambiguous network or server failures do
not trigger a second send that could duplicate a message.

Captain release notices use the same model-independent control plane. The
daemon checks the compatible official release channel after startup and every
12 hours, then durably queues one Rich card for `default_chat_id`. Its buttons
carry a compact candidate token and exact decision version. Before applying an
update, deferral, or refusal, the kernel verifies the current candidate, exact
chat, and an explicitly listed numeric Telegram user; `allowed_users = ["*"]`
is not sufficient for this privileged action. Stale or malformed callbacks are
answered directly and never enter a session or model turn. Host updates verify
the archive checksum in a detached process. Docker/manual procedures retain a
24-hour reminder until the new runtime is actually observed.

## Access Policy

Restrict inbound users before exposing a channel. For active inbound adapters,
an empty allowlist is deny-by-default; `allowed_users = ["*"]` deliberately
opens the channel.

Example policy:

```toml
[channels.telegram]
bot_token_env = "TELEGRAM_BOT_TOKEN"
default_agent = "captain"
allowed_users = ["123456789"]

[channels.telegram.overrides]
dm_policy = "allowed_only"
group_policy = "mention_only"
rate_limit_per_user = 10
threading = true
output_format = "telegram_html"
```

Useful shared controls:

| Field | Purpose |
|---|---|
| `default_agent` | Agent that receives messages without a more specific route |
| `allowed_users` | Platform user IDs permitted to interact |
| `dm_policy` | Respond, allowed-only, or ignore direct messages |
| `group_policy` | All, mention-only, commands-only, or ignore group messages |
| `rate_limit_per_user` | Per-user message cap per minute; `0` disables the cap |
| `threading` | Reply in a thread when the platform supports it |
| `output_format` | Platform-safe output formatting |

Policy checks happen before an inbound message reaches the model, so rejected
messages do not consume LLM tokens.

## Routing and Sessions

Each inbound conversation maps to a durable Captain session. `/new` creates a
fresh session and preserves the previous transcript in the global history.
Sessions created from Telegram, Discord, Signal, Email, Web, TUI, CLI, or API
remain visible and reopenable from the other surfaces.

Set `default_agent` for the usual route. Use explicit routing rules only when a
channel or identity must reach a specialist. Keep routing deterministic: a
message should not silently switch provider, model, or agent because another
channel is active.

## Reliability

The active adapters share graceful shutdown, reconnect backoff, message-size
splitting, rate limiting, policy enforcement, and structured runtime status.
Long-running channel work is detached from the adapter receive loop so one
slow tool call does not block unrelated messages. A daemon restart preserves
durable sessions and memory; in-flight work is reported as interrupted or
recoverable rather than silently lost.

## Troubleshooting

1. Run `captain channel list` and confirm the channel is configured and ready.
2. Run `captain channel test <channel>` to verify outbound delivery.
3. Run `captain status` and `captain doctor --full` for adapter, memory, model,
   and daemon health.
4. Inspect `captain logs daemon` for authentication, connection, allowlist, or
   rate-limit errors.
5. Restart Captain after changing a token, endpoint, or channel block.

If inbound messages arrive but receive no response, check `allowed_users`, DM
and group policies, the routed agent state, and its current budget. If replies
stall behind a tool, inspect detached tool runs in Status instead of repeatedly
resending the message.

See [Configuration](configuration.md), [CLI Reference](cli-reference.md#channel-commands),
and the embedded [channel tool contract](captain-tools/channel.md) for the
versioned implementation details.
