# Scheduling family

> **Status:** audited (D.10).
> See [`README.md`](README.md) for the index and drift policy.
> Tool name list pinned in [`captain_runtime::captain_docs::SCHEDULING_FAMILY_TOOLS`](../../crates/captain-runtime/src/captain_docs.rs).

## Tools

Captain has three scheduling layers, from highest level to lowest:

1. **Goals** — autopilot loops that the kernel re-evaluates on its own (R.2.1). Goals can be paused, resumed and accept LLM-generated suggestions to refine themselves.
2. **Crons** — recurring or one-shot prompts dispatched on a clock or interval, optionally delivered to a channel. Captain's everyday "remind me at 9am tomorrow" surface.
3. **Schedules** — the raw scheduler primitive that powers crons. Use directly only when the cron wrapper is too restrictive.

### Goals (autopilot)

#### `goal_create`

Create a long-running autopilot goal with a check command, optional recovery command, escalation policy, and LLM reflection budget. Use a goal when Captain must **maintain** a state over time, not just run a prompt on a clock.

| Field | Required | Notes |
|---|---|---|
| `id` | yes | Stable id, 3..64 chars, ASCII alphanumeric plus `-` or `_`. |
| `name` | yes | Short label. |
| `description` | yes | Natural-language objective. |
| `interval_secs` | yes | Seconds between checks, minimum 10. |
| `check_command` | yes | Shell command. Exit 0 means healthy/satisfied. |
| `recovery_command` | no | Shell command attempted after a failed check. |
| `escalation_threshold` | no | Consecutive failed checks before user escalation. Default 3. |
| `max_llm_calls_per_hour` | no | Sliding-window cap for goal reflection. Default 20, hard max 1000. |
| `escalation_channel` | no | `{channel, recipient}`. For Telegram-heavy users, set this explicitly. |

Returns the new goal's id.

#### `goal_list`, `goal_status`, `goal_pause`, `goal_resume`, `goal_delete`

CRUD on the goal store. `goal_pause` flips the goal to `Paused` (loop skips it); `goal_resume` flips it back to `Active`. `goal_status({id})` returns the current state plus `consecutive_fails` and `last_check_at`.

#### `goal_list_suggestions`, `goal_apply_suggestion`, `goal_reject_suggestion`

The reflection pipeline emits **suggestions** — proposed refinements such as changing interval, threshold, or recovery command. The user (or Captain) reviews and applies/rejects them.

| Field | Required | Notes |
|---|---|---|
| `id` | yes | Owning goal. |
| `suggestion_id` | yes (apply/reject) | UUID prefix of the suggestion. |

### Goal vs cron decision

- Use `goal_create` when the requested outcome is continuous: "keep X healthy", "watch Y and recover", "alert me after repeated failures".
- Use `cron_create` when the requested action is scheduled: reminders, reports, recurring prompts, one-shot future actions.
- Use `reminder_set` for quick relative reminders only.
- Use raw `schedule_*` only when `cron_create` cannot express the schedule shape.

### Crons

Operator surfaces:

- Agent/tool path: `cron_create`, `cron_list`, `cron_update`, `cron_cancel`.
- Web path: `/crons` for native cron CRUD and status, backed by
  `/api/cron/jobs`.
- Legacy raw schedules remain under Triggers only when the underlying
  `schedule_*` primitive is needed.

#### `cron_create`

Schedule a prompt to be dispatched on a clock. Captain's everyday reminder/recurring-task surface.

| Field | Required | Notes |
|---|---|---|
| `name` | yes | Human label. |
| `schedule` | yes | One of `at` (`{kind:"at", at:"<ISO-8601 future date>"}`), `every` (`{kind:"every", every_secs:N}`), or `cron` (`{kind:"cron", expr:"<cron expression>", tz:"<IANA TZ>"}`). |
| `action` | yes | Usually `{kind:"agent_turn", message:"...", timeout_secs:N}`; `timeout_secs` is an inactivity/review window, not a wall-clock cap. Use `system_event` only for internal events. |
| `delivery` | no | `{kind:"channel", channel:"telegram", to:"<recipient>"}` to surface the result on a channel, `{kind:"last_channel"}` when appropriate, or `{kind:"webhook", url:"https://..."}` for public webhook delivery. |
| `one_shot` | no | `true` for a single fire (must combine with `kind:at`). |

The tool description spells out the choice tree. Always resolve the **current date** with `shell_exec date` (or `system_time`) before passing an absolute `at`; the LLM frequently hallucinates "tomorrow" without grounding.

#### `cron_list`

List active crons (id, name, next_run_at, recurrence). No parameters.

#### `cron_update`

Modify an existing cron in place. Use this when the user asks to move a reminder, change the recurring cadence, edit the message/action, change delivery, pause/resume, or switch one-shot behaviour. Do **not** cancel/create just to modify a job: `cron_update` preserves the job id, owner, created_at, last_run, last_status and run history.

| Field | Required | Notes |
|---|---|---|
| `job_id` | yes | Full UUID or non-ambiguous prefix from `cron_list` / `cron_create`. |
| `name` | no | New label. |
| `schedule` | no | New schedule object. Call `system_time` before calendar-relative updates. |
| `action` | no | New action object. Never include raw secrets. |
| `delivery` | no | New delivery object. |
| `enabled` | no | `false` pauses without deletion; `true` re-enables and recomputes `next_run`. |
| `one_shot` | no | Toggle removal after next successful execution. |

#### `cron_cancel`

Cancel a cron by id prefix.

| Field | Required | Notes |
|---|---|---|
| `job_id` | yes | Full UUID from `cron_list` / `cron_create`. |

#### `reminder_set`

Create a lightweight one-shot reminder without constructing a full cron payload.

| Field | Required | Notes |
|---|---|---|
| `delay_minutes` | yes | Relative delay from the daemon's current clock. |
| `message` | yes | Reminder text. |

Use `reminder_set` for quick relative reminders ("in 20 minutes"). Use `cron_create` for absolute dates, recurring schedules, named jobs, or channel delivery options.

### Raw schedules

`schedule_create`, `schedule_list`, `schedule_delete` operate on the underlying scheduler without the cron wrapper's defaulting. Use these only when you need fields not exposed by `cron_create` (custom backoff, jitter, multiple recipients).

### Cross-session todos

A **todo** is the lightest persistence surface in this family: a one-line item that survives daemon restarts and conversation compactions but does not run a check, fire on a schedule, or attach to a project. Use it when the user says "n'oublie pas de…", "rappelle-moi de regarder X" without a specific time, or "je dois encore m'occuper de Y". Heavier intents belong elsewhere:

- A specific time / recurrence → `cron_create`.
- A continuous loop to maintain (health, tail, watchdog) → `goal_create`.
- Project-scoped work with sub-tasks and a DAG → `project_task_*`.

#### `todo_create`

| Field | Required | Notes |
|---|---|---|
| `title` | yes | One-line summary, non-empty after trim. |
| `body` | no | Free-form details. Markdown allowed. |

Returns the new row including its UUID. Storage is a global SQLite table — no project FK, no agent FK — matching the cross-session intent.

#### `todo_list`

`status: "open"` (default) returns the live list, ordered newest first. `"done"` returns the done list ordered by `completed_at` desc. `"all"` is open-then-done with the same intra-bucket order.

#### `todo_complete`, `todo_reopen`, `todo_delete`

`todo_complete` flips `done = true` and stamps `completed_at`. `todo_reopen` is the inverse (idempotent on an already-open todo). `todo_delete` is irreversible — the done list is *not* an audit journal; treat it as a buffer the user can prune.

### File-change triggers (filesystem watchers)

A **file-change trigger** wakes Captain when something on disk changes. Use it instead of polling with `cron_create` when the user wants Captain to react to *real* events: a config touched, a screenshot dropped into an inbox folder, a Markdown file saved by an external tool. The OS watch is debounced and rate-limited; if a watcher fires more than 10 times in 60s it auto-pauses to break agent → file_write → trigger → agent loops.

#### `file_trigger_register`

Arm a watcher on one or more paths. The trigger persists across daemon restarts.

| Field | Required | Notes |
|---|---|---|
| `paths` | yes | Array of absolute or relative paths. A path that does not exist yet is accepted: the watcher arms on the closest existing ancestor. |
| `events` | no | Subset of `["create", "modify", "remove", "rename", "any"]`. Default `["any"]`. `modify` covers writes/append; `rename` includes move-to and move-from. |
| `recursive` | no | When a path is a directory, also watch its descendants. Default `true`. |
| `prompt_template` | no | Template rendered into the agent prompt at fire-time. Variables: `{path}`, `{kind}`, `{previous_path}`. Default `"File {kind}: {path}"`. |
| `debounce_ms` | no | Bursts within this window collapse to one fire. Clamped to `[200, 60000]`. Default `800`. |
| `enabled` | no | When `false`, persists the trigger but does not arm the OS watcher. Default `true`. |

Returns `{trigger_id, agent_id}`.

#### `file_trigger_list`

List file-change triggers visible to the agent. Default `scope: "self"` returns only triggers the calling agent owns. `scope: "all"` returns every persisted trigger (debug/admin path). The response carries `enabled` so paused or auto-disabled triggers are visible.

#### `file_trigger_set_enabled`, `file_trigger_remove`

`file_trigger_set_enabled({trigger_id, enabled})` toggles arming without losing the persisted definition — handy for temporary pauses (vacation, tests). `file_trigger_remove({trigger_id})` deletes the trigger and stops the watcher; irreversible.

### Native webhook/event API surfaces

Captain exposes native HTTP surfaces for systems that need to wake it from the
outside or observe what it is doing without asking the model to call a tool.

#### Inbound wake hooks

- `POST /hooks/wake` publishes a `webhook.wake` custom event into the kernel
  event bus. Event triggers can match it, and the event is visible through
  `GET /api/events`.
- `POST /hooks/agent` sends one isolated turn to a named agent or agent id.

Both routes require `[webhook_triggers] enabled = true` and a bearer token read
from `token_env`. The token must stay in the environment/secret store, not in
the config file.

#### Outbound webhooks

- `GET /api/webhooks/outbound` returns the currently loaded outbound webhook
  configuration and endpoint summaries.
- `POST /api/webhooks/outbound/endpoints`, `PUT
  /api/webhooks/outbound/endpoints/{name}`, and `DELETE
  /api/webhooks/outbound/endpoints/{name}` edit `config.toml` and return
  `restart_required: true` because the dispatcher reads config at daemon boot.
- `POST /api/webhooks/outbound/test` validates URL/signature behavior and can
  run in `dry_run` mode without sending a network request.

Outbound targets must be public HTTP(S) URLs. Localhost, private IPs,
link-local IPs, and cloud metadata hosts are rejected before storage or
delivery.

## Sandbox

- **Goals run shell checks** — `check_command` and `recovery_command` are validated against critical-pattern guards at creation and suggestion-apply time. Still keep them narrow, idempotent, and cheap.
- **Cron prompts hit the same bridge as user messages** — when delivery is `channel:telegram`, the cron-spawned conversation goes through `channel_send` and respects every channel-side rate limit.
- **Cron agent/workflow timeout is inactivity-based** — `timeout_secs` cancels a cron agent turn or workflow step only after that many seconds without model/tool/phase stream activity. Omit it for the 600s default, or set up to 7200s for planned long work. Long active/healthy jobs must keep running; the timeout is a review window, not a kill deadline.
- **Cron delivery reliability** — transient channel/webhook transport failures retry with capped jittered backoff. If the job completed but delivery failed, cron detail exposes `last_delivery_error`, `redelivery_queue`, and bounded `dead_letters`; `/api/status.workload.automation.delivery` and `captain status` expose aggregate failed/retry/dead-letter counts. Inspect those before recreating or modifying the job. Redelivery payloads are stored as sidecar files so the cron job store stays readable.
- **Webhook SSRF guard** — cron webhooks are outbound network sinks. `cron_create`, `cron_update`, and runtime delivery reject localhost, private IPs and cloud metadata hosts. Use channel delivery or a public HTTPS endpoint instead of internal network URLs.
- **Persistence** — goals live in `~/.captain/goals.json`; crons/schedules live in the scheduler store. Read for diagnostics only; writes go through the tools above.
- **Failure escalation** — goals declare an `escalation_threshold`. Once the consecutive-fail counter crosses the threshold the goal flips to `Escalated`, then surfaces a channel notice when `escalation_channel` or a home channel is available.
- **File-trigger sandbox** — `file_trigger_register` rejects any path inside `KernelHandle::blocked_workspace_paths()` — `~/.ssh/`, `~/.gnupg/`, `~/.captain/secrets.env`, `~/.captain/secrets-backups/`, `~/.captain/vault.enc`, `~/.captain/.env*`. Symlinks are resolved before the check, so a symlink shortcut into a protected zone fails.
- **File-trigger persistence** — triggers live in `~/.captain/file_change_triggers.json`. At boot the kernel re-canonicalises every persisted path: any path that has vanished while the daemon was down is auto-disabled and a `warn!` line names the casualty. A trigger that fires more than 10 times per 60s window is auto-paused (`enabled = false`) to break feedback loops; the user must re-enable it explicitly.
- **Todo store** — todos live in the `todos` table inside `~/.captain/data/captain.db` alongside `project_tasks` and the rest of the kernel state. The schema (`id, title, body, done, created_at, completed_at`) is migration v19; a fresh install adds it the first time the kernel boots, an existing install upgrades on the next start. Captain has no separate per-agent partition — todos are global.

## Limites

- `cron_create` with `kind:cron` accepts a 5-field cron expression (`m h dom mon dow`). Six-field "with seconds" expressions are rejected.
- The kernel polls the schedule queue on a single ticker; sub-second precision is not guaranteed. The actual fire time is within ±1 s of the configured tick.
- `goal_pause` does not interrupt an in-flight check — it only prevents the next dispatch.
- Suggestions expire after 7 days like skill proposals; an aged-out suggestion id returns an explicit error.
- Many short-interval goals can create system load even when `max_llm_calls_per_hour` is low, because shell checks still run. Prefer health endpoints, cheap commands, and intervals that match real urgency.
- Recovery commands must be idempotent. A goal may retry after repeated failures; avoid commands that mutate broad state unless the check/recovery contract is very clear.
- `cron_update` accepts a non-ambiguous id prefix; if two crons share a prefix it returns an "ambiguous" error rather than picking one. `cron_cancel` currently expects a full UUID.
- `reminder_set` is relative only. For "tomorrow at 9" call `system_time` first and use `cron_create` with an ISO-8601 `at`.
- The default delivery target is the user's home channel (set with `set_home_channel`). Without a home, cron fires that don't carry an explicit `delivery` field log a warning and skip the dispatch.
- Time zones: `at` ISO-8601 strings carry their own offset. Without offset they are interpreted as the daemon's local TZ (`config.toml [scheduling] timezone` if set, otherwise system TZ).
- File-change triggers debounce server-side, but the underlying `notify` watcher cannot guarantee one OS-level event per logical write: editor tools (Vim, VS Code) routinely produce a Remove + Create pair on save. Match `events: ["any"]` if you want to catch every editor flavour, or `["modify", "create"]` if you only care about new content.

## Exemples

### Golden path — schedule a one-shot reminder

```
1. shell_exec({"command": "date -u +'%Y-%m-%dT%H:%M:%SZ'"})
   → "2026-04-29T08:42:11Z"
2. cron_create({
     "name": "morning ping",
     "prompt": "Bonjour ! Voici les nouveautés de la nuit…",
     "kind": {"kind":"at","at":"2026-04-30T07:00:00+02:00"},
     "one_shot": true,
     "delivery": {"kind":"channel","channel":"telegram"}
   })
   → {"id": "c-9f1...", "next_run_at": "2026-04-30T05:00:00Z"}
```

### Golden path — Goal lifecycle

```
goal_create({
  "id": "service-health",
  "name": "Service health",
  "description": "Keep the monitored service healthy and escalate on repeated failures.",
  "interval_secs": 300,
  "check_command": "curl -fsS https://example.com/health",
  "recovery_command": "systemctl restart example-service",
  "escalation_threshold": 3,
  "max_llm_calls_per_hour": 10,
  "escalation_channel": {"channel": "telegram", "recipient": "123456"}
})
goal_status({"id":"service-health"})
→ {"state":"Active","consecutive_fails":0,"last_check_at":"..."}
... three failed checks ...
→ {"state":"Escalated","consecutive_fails":3,...}
goal_pause({"id":"service-health"})
... fix ...
goal_resume({"id":"service-health"})
```

### Error case — cron expression with seconds is rejected

```
cron_create({"name":"x","prompt":"...","kind":{"kind":"cron","expr":"0 0 12 * * ?"}})
→ Err("cron expression: expected 5 fields (m h dom mon dow), got 6 — drop the seconds field").
```

The error tells Captain exactly what to drop.

### Golden path — react to a screenshot dropped into an inbox

```
file_trigger_register({
  "paths": ["/Users/me/Desktop/inbox/"],
  "events": ["create", "modify"],
  "recursive": false,
  "prompt_template": "Inbox event {kind}: {path}. Decide if this should be archived, summarised, or escalated."
})
→ {"trigger_id":"f1a-...","agent_id":"agent-..."}
```

Now any `mv ~/Downloads/foo.png ~/Desktop/inbox/` will fire a single agent prompt (debounced 800ms) instead of polling the directory on a cron.

### Error case — file trigger refused on a protected path

```
file_trigger_register({"paths":["/Users/me/.ssh/known_hosts"]})
→ Err("refused file-change trigger path /Users/me/.ssh/known_hosts: inside protected zone /Users/me/.ssh").
```

The sandbox names the violated prefix so Captain can choose a different watch root or surface a clear refusal to the user.

### Golden path — capture and complete a todo across sessions

```
todo_create({"title": "lire le rapport hermes #12326"})
→ {"id":"3f6a-...","title":"lire le rapport hermes #12326","done":false,...}

# … days later, in a fresh session after a daemon restart …
todo_list({})
→ [{"id":"3f6a-...","title":"lire le rapport hermes #12326","done":false,...}]

todo_complete({"id":"3f6a-..."})
→ {"id":"3f6a-...","done":true,"completed_at":1717248000000,...}
```

The id and the body survive the restart because the row lives in `captain.db`, not in the agent's working memory.
