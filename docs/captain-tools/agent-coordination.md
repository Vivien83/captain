# Agent coordination family

> **Status:** audited (D.9).
> See [`README.md`](README.md) for the index and drift policy.
> Tool name list pinned in [`captain_runtime::captain_docs::AGENT_COORDINATION_FAMILY_TOOLS`](../../crates/captain-runtime/src/captain_docs.rs).

## Tools

Captain is the principal agent — it orchestrates ad-hoc workers and is the only path the user has when Captain genuinely needs a human in the loop. Manager fleets, Hands, peers, and A2A remain compiled but are frozen out of the active discovery path until the core is stable.

### Direct agent control

#### `agent_spawn`

Create a new agent from a TOML manifest string.

| Field | Required | Notes |
|---|---|---|
| `manifest_toml` | yes | TOML body matching `AgentManifest` (name, description, model, tools, …). |

Canonical minimal manifest for `agent_spawn`:

```toml
name = "veille-technologique"
description = "Agent specialise dans la veille technologique."
module = "builtin:chat"
tool_allowlist = ["web_research_batch", "web_fetch", "memory_recall", "memory_save"]

[model]
provider = "codex"
model = "gpt-5.5"
system_prompt = "Tu es un agent de veille technologique. Utilise des sources reelles, cite-les, et signale les incertitudes."
```

Important: `model` is a TOML table, not a string. Do not write
`model = "codex:gpt-5.5"` or `model = "gpt-5.5"`. Do not use
`[tools] allow = [...]` for the child surface; `tools` is a map of per-tool
configs. Use top-level `tool_allowlist = [...]` or `[capabilities] tools = [...]`.

Returns `{agent_id, agent_name}`. The new agent inherits `parent_id = caller_agent_id` for lineage tracking.

Security note: every sub-agent manifest must declare an explicit non-wildcard
`tool_allowlist` or `capabilities.tools`. A profile alone is not enough for
`agent_spawn`. Captain automatically adds the minimal discovery set
`capability_search`, `skill_search`, `tool_search`, `captain_docs`, and
`system_time`; any other tool must be named deliberately. When the parent is
scoped, the child must stay inside the parent's tool set, except for that
mandatory discovery set. If a worker needs a tool outside its allowlist, it
should ask Captain for an extension instead of trying to work around the missing
capability.

#### `agent_send`

Forward a message to another agent and get the response.

| Field | Required | Notes |
|---|---|---|
| `agent_id` | yes | UUID of the target agent. |
| `message` | yes | What to ask. |

Synchronous: blocks until the receiving agent's loop completes one full turn.

### External agent API (agent-as-service)

Local agent-to-agent calls use `agent_send` or `agent_delegate`. External HTTP
clients do **not** need a custom bridge: Captain exposes a dedicated
agent-as-service API for each agent.

For any running agent returned by `agent_list`, the external integration
contract is:

- `agent_spawn` / `POST /api/agents` — creation now follows
  `agent-as-service.v1`: Captain provisions the ingress bearer token by default,
  returns it once with the created agent details, and reports egress readiness.
  Provide `agent_api.egress_callback_url` at creation time when the service must
  be fully in/out ready immediately. Without it, Captain reports
  `ingress_ready` and explicitly says that it cannot infer the external
  callback URL for outbound events.
- `GET /api/agents/{id}/api/manifest` — operator manifest describing ingress,
  egress, auth, callback signature, readiness, and example payloads.
- `POST /api/agents/{id}/api/token/rotate` — rotate/generate the ingress bearer
  token. The token is returned once and then stored in `secrets.env`.
- `POST /hooks/agents/{id}/ingress` — external HTTP webhook / REST ingress. Use
  `Authorization: Bearer <token>` and JSON body
  `{request_id,message,sender_id?,sender_name?,metadata?}`. Captain runs one
  agent turn and returns `{status,response,egress,usage,...}`.
- `POST /api/agents/{id}/api/egress/configure` — configure outbound callback
  delivery with `callback_url` and `callback_secret`.
- `POST /api/agents/{id}/api/egress/test` — send a signed diagnostic callback.
- `GET /api/agents/{id}/api/events?n=50` — inspect recent ingress/egress audit
  events for that agent.
- `GET /api/agents/{id}/api/egress` — inspect pending/dead-letter callback
  deliveries.
- `POST /api/agents/{id}/api/egress/{queue_id}/retry` — retry one queued
  callback now.

Readiness vocabulary is strict: `ready` means ingress and signed egress are
both configured. `ingress_ready` means the bearer ingress works, but callbacks
still require `/api/agents/{id}/api/egress/configure`.

Example for an external service calling a specialized agent:

```bash
AGENT_ID="5a2454b7-d8bc-4902-b903-93a12bad10d3"
BASE="http://127.0.0.1:50051"

curl -sS -X POST "$BASE/hooks/agents/$AGENT_ID/ingress" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "request_id": "watch-2026-06-29-001",
    "message": "Fais une veille technologique sur les nouveaux runtimes agents.",
    "sender_id": "external-service:veille",
    "sender_name": "Veille API",
    "metadata": {"source": "external-api"}
  }'
```

Outbound callbacks are signed with HMAC-SHA256:

- header `x-captain-agent-id`: agent id
- header `x-captain-event`: `agent_api.completed`, `agent_api.failed`, or
  `agent_api.test`
- header `x-captain-signature`: `sha256=<hex_hmac_sha256>`
- signature input: raw JSON request body bytes

If the user asks "comment communiquer avec cet agent par API externe", answer
with the agent id from `agent_list`, then these endpoints. Do not say that no
HTTP endpoint exists; the dedicated ingress is
`POST /hooks/agents/{id}/ingress`.

#### `agent_list`

List every running agent, with name, state, model, tags. No parameters.

#### `agent_kill`

Terminate an agent loop and free its session.

| Field | Required | Notes |
|---|---|---|
| `agent_id` | yes | UUID of the target. |

#### `agent_status`

Inspect what an agent is doing right now (current tool call, context size, last event).

| Field | Required | Notes |
|---|---|---|
| `agent_id` | yes | UUID of the target. |

#### `agent_caps`

Full capability + budget report for an agent (yourself or another): declared and effective tools/network/shell/memory scopes, resource quotas, and hourly/daily/monthly cost + token budget usage vs limit. The agent-facing equivalent of the CLI's `captain agent caps` — use this instead of `shell_exec`-ing the CLI, since the `captain` binary is not reachable from `shell_exec`'s sandbox.

| Field | Required | Notes |
|---|---|---|
| `agent_id` | yes | UUID of the target (yours or another agent's). |

#### `agent_watch`

Tail an agent's recent events (tool calls, messages, state transitions).

| Field | Required | Notes |
|---|---|---|
| `agent_id` | yes | UUID of the target. |

#### `agent_delegate`

Assign a scoped task to an agent with a token budget. Captain's primary
delegation primitive — use this rather than `agent_send` when you want a
bounded sub-task with a persisted task record.

Current runtime contract: this is synchronous today. Captain posts a task,
runs one worker turn under a scoped run budget, completes the task with the
worker response, and returns the measured usage. Use `task_post` / `task_claim`
for true fire-and-forget queueing.

| Field | Required | Notes |
|---|---|---|
| `agent_id` | yes | Target. |
| `task` | yes | Task description. |
| `max_tokens` | yes | Scoped run budget. It can stop further tool steps once reached; one LLM call can still overshoot before Captain can interrupt it. |

#### `agent_correct`

Inject a correction message mid-flight while another agent is running. Useful when watching a fleet worker drift.

| Field | Required | Notes |
|---|---|---|
| `agent_id` | yes | Target. |
| `message` | yes | The correction. |

#### `agent_find`

Lookup agents by name / role / tag. Returns the matching list.

| Field | Required | Notes |
|---|---|---|
| `query` | yes | Free-text query. |

### Fleet management (Managers + Workers)

Frozen surface for the current Captain Core Excellence phase. Keep these tools documented for compatibility and recovery, but do not prefer them over local sub-agents, task queue, projects, or explicit user-approved orchestration.

A **Manager** is a long-lived agent with its own domain, budget and worker pool. Workers are scaled up/down based on queue depth and idle time (see `fleet_configure_autoscale`).

- `fleet_create_manager({name, domain, model, budget_tokens})` — spin up a Manager.
- `fleet_list_managers()` — current Managers + fleet info.
- `fleet_close_manager({manager_id})` — close a Manager and all its workers.
- `fleet_set_mission({manager_id, mission})` — persist a mission string the Manager carries across reboots.
- `fleet_configure_autoscale({manager_id, ...})` — min/max workers, idle timeout, queue threshold.
- `fleet_metrics({manager_id})` — queue depth, avg latency, cost per worker.

### Discovery, Hands, and A2A

Frozen surface for the current Captain Core Excellence phase. These tools stay documented because existing installs may have them, but `capability_search`, `tool_search`, and Live Tool Schemas omit them by default.

- `peer_list()` — inspect known peer Captain instances or compatible runtimes.
- `event_publish({topic, payload})` — publish an internal coordination event when a workflow needs decoupled listeners.
- `hand_list()` — list available curated Hands and whether they are active.
- `hand_activate({hand_id, config?})` — start a specialized Hand when a task maps to a known capability package.
- `hand_status({hand_id})` — inspect uptime, activity and health before reusing a Hand.
- `hand_deactivate({instance_id})` — stop a Hand instance and free resources.
- `scaffold_hand({id, name, description, category?, icon?, tools?})` — create a new Hand package when a reusable autonomous role is clearer than a one-off skill.
- `a2a_discover({url})` — inspect an external A2A-compatible agent card.
- `a2a_send({message, agent_url?, agent_name?, session_id?})` — send work to an external A2A agent. Use internal `agent_send` for local Captain agents.

Decision rule: prefer `agent_spawn`, `agent_delegate`, task queue tools, projects, skills, or MCP before touching this frozen surface. Use A2A only for an explicitly approved external agent with a URL or prior discovery record.

### Task queue

A simple producer/consumer queue spanning agents. Producers `task_post` work, consumers `task_claim` and `task_complete`. Captain typically delegates routine work this way rather than spawning ad-hoc agents.

- `task_post({title, description, assigned_to?, created_by?})` — enqueue a task. Returns `task_id`.
- `task_claim({agent_id})` — pop the next task assigned (or unassigned) for the calling agent. Returns `null` when the queue is empty.
- `task_complete({task_id, result})` — mark done with the deliverable.
- `task_list()` — full queue snapshot.

### Human-in-the-loop

#### `ask_user`

Surface a question to the user when no other tool can resolve it. **Last resort** — every other coordination/discovery/RTFM tool should be exhausted first.

| Field | Required | Notes |
|---|---|---|
| `question` | yes | The question to ask. |
| `options` | no | Optional set of suggested answers for fast UI replies. |

The question lands on the user's home channel (or the originating channel if
the conversation has one). The agent loop blocks until the user responds. Once
an answer, timeout, or channel-closed fallback is recorded, the runtime emits
the corresponding tool completion event so chat/TUI/web surfaces must clear the
`ask_user` running state instead of leaving it active.

## Sandbox

- **Spawn lineage** — every spawned agent records `parent_agent_id`,
  `root_agent_id`, `subagent_depth`, and `is_subagent=true` so a rogue chain
  can be traced and killed by the Manager.
- **Depth-aware tool policy** — lineaged sub-agents do not see or execute
  admin/scheduling tools reserved for principal agents. Leaf-depth workers also
  lose spawn/kill tools to prevent deep delegation chains.
- **Budget gating** — `agent_delegate` applies `max_tokens` as a scoped run
  budget. It no longer mutates the worker's hourly quota. The budget can stop
  further tool steps once reached, and the returned JSON includes
  `used_tokens` plus `budget_exceeded`.
- **Approval surface** — `ask_user` queues onto the same approval system used by sensitive tools. It's not a security boundary, but it's auditable: the question and answer end up in the session log.
- **Cross-agent message authority** — agents can `agent_send` to each other; the receiver enforces its own `allowed_tools` (B.4) so no privilege escalation by routing through a sibling.

## Limites

- `agent_send` is synchronous — Captain blocks while the target agent runs.
- `agent_delegate` is also synchronous in the current runtime, but records and
  completes a task around the worker turn. Use `task_post` when the caller must
  continue immediately.
- `agent_delegate` budget is scoped to the delegated run, but it is still not a
  hard pre-call meter: a single LLM request can overshoot before Captain can
  observe usage and interrupt the next tool step.
- `agent_correct` is best-effort — a correction injected mid-tool-call only takes effect on the next tool decision; it does not abort the in-flight tool call.
- Frozen fleet autoscale ticks every `CAPTAIN_AUTOSCALE_TICK_SECS` seconds (default 30). Newly-bursting workloads will see a cold-start delay of up to one tick.
- `task_claim` returns `null` rather than blocking; loop with a short sleep when polling. There is no built-in long-poll.
- `ask_user` blocks until a reply arrives or the runtime timeout/channel fallback
  records a synthetic answer. A resolved `ask_user` is no longer an active tool
  and must not keep an operator spinner alive.
- `event_publish` is fire-and-forget; if a consumer must acknowledge work, use `task_post`/`task_claim` or `agent_send`.
- Frozen A2A calls cross a trust boundary. Do not send secrets, raw user data, or local file contents unless the user explicitly authorized that remote agent.

## Exemples

### Golden path — delegate a sub-task with a budget

```
1. agent_spawn({"manifest_toml": "[agent]\nname=\"summarizer\"\n..."})
   → {"agent_id": "...", "agent_name": "summarizer"}
2. agent_delegate({
     "agent_id": "...",
     "task": "Summarize the last 30 days of issues from repo X.",
     "max_tokens": 20000
   })
   → {"result": "Top issues: ..."}
3. agent_kill({"agent_id": "..."})
```

### Golden path — task queue between two agents

```
task_post({"title": "Fetch slot", "description": "...", "assigned_to": "scraper-1"})
→ {"task_id": "t-001"}
(agent scraper-1)
task_claim({"agent_id": "scraper-1"})
→ {"task_id": "t-001", "title": "...", "description": "..."}
... work ...
task_complete({"task_id": "t-001", "result": "Slot booked"})
```

### Frozen reference — specialized Hand

```
hand_list()
→ [{"id":"researcher","status":"inactive",...}]
hand_activate({"hand_id":"researcher"})
→ {"instance_id":"...","agent_id":"..."}
hand_status({"hand_id":"researcher"})
→ {"status":"active","last_activity":"..."}
```

### Error case — ask_user used before exhausting RTFM

```
ask_user({"question": "Que fait l'outil glob?"})
→ (anti-pattern — Captain should call captain_docs("glob") first; the
audit logs flag this as RTFM-bypass for review.)
```

The `ask_user` is technically allowed but is the last resort — when Captain hits this with a question that `captain_docs` could have answered, the reflection pipeline marks the session for review.
