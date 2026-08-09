# API Reference

Captain exposes a REST API, WebSocket endpoints, and SSE streaming when the daemon is running. The default listen address is `http://127.0.0.1:50051`.

All responses include security headers (CSP, X-Frame-Options, X-Content-Type-Options, HSTS) and are protected by a GCRA cost-aware rate limiter with per-IP token bucket tracking and automatic stale entry cleanup. Captain implements defense-in-depth runtime protections including audit chaining, heuristic content guards, WASM dual metering, Ed25519 manifest signing, SSRF protection, host subprocess environment scrubbing, and secret zeroization. The native host backend reports `os_isolation: false`; Docker and WASM are separate explicit isolation backends. Content guards classify reviewed shell and URL patterns; they do not provide provenance tracking or prove arbitrary code safe.

## Table of Contents

- [Authentication](#authentication)
- [Agent Endpoints](#agent-endpoints)
- [Workflow Endpoints](#workflow-endpoints)
- [Trigger Endpoints](#trigger-endpoints)
- [Memory Endpoints](#memory-endpoints)
- [Learning Endpoints](#learning-endpoints)
- [Channel Endpoints](#channel-endpoints)
- [Approval Endpoints](#approval-endpoints)
- [Template Endpoints](#template-endpoints)
- [System Endpoints](#system-endpoints)
- [Live Run Endpoints](#live-run-endpoints)
- [Artifact Endpoints](#artifact-endpoints)
- [Model Catalog Endpoints](#model-catalog-endpoints)
- [Provider Configuration Endpoints](#provider-configuration-endpoints)
- [Native Capability Endpoints](#native-capability-endpoints)
- [Skills Endpoints](#skills-endpoints)
- [MCP Protocol Endpoints](#mcp-protocol-endpoints)
- [Audit & Security Endpoints](#audit--security-endpoints)
- [Usage & Analytics Endpoints](#usage--analytics-endpoints)
- [Session Management Endpoints](#session-management-endpoints)
- [WebSocket Protocol](#websocket-protocol)
- [SSE Streaming](#sse-streaming)
- [OpenAI-Compatible API](#openai-compatible-api)
- [Error Responses](#error-responses)

---

## Authentication

When an API key is configured in `config.toml`, every endpoint outside the
reviewed public allowlist requires a Bearer token:

```
Authorization: Bearer <your-api-key>
```

### Setting the API Key

Add to `~/.captain/config.toml`:

```toml
api_key = "your-secret-api-key"
```

### No Authentication

If `api_key` is empty and browser authentication is disabled, the API is
accessible without authentication. Captain refuses a non-loopback daemon bind
without an API key; this local development mode is not a public deployment
configuration.

### Public Endpoints (No Auth Required)

- `GET /`, which boots Control but does not expose operational API data
- `GET /assets/*`, `/logo.svg`, `/favicon.ico`, `/manifest.json`, and `/sw.js`
- `GET /api/health`, which returns only `status` and `version`
- `GET /api/version`
- `GET /api/auth/check`
- `POST /api/auth/login` and `/api/auth/logout`
- `POST /hooks/agents/{agent_id}/ingress`, which bypasses global auth only for
  an exact UUID route and enforces its own per-agent Bearer token and rate limit

Every other route is private, including `/terminal`, `/config`, frozen
agent-to-agent discovery/task compatibility surfaces, detailed health/status,
agents, sessions, approvals, budgets, logs, models/providers, and the GitHub
Copilot OAuth start/poll flow. Artifact inventory, previews, downloads, and
every `/api/tool-runs` route are also private. Agent-to-agent discovery reveals
manifests and task state; the
OAuth flow can persist a provider credential, so neither is treated as
self-authenticating. Control returns to its login screen when any protected
request reports `401`.

### Browser Sessions

Browser login issues an HttpOnly HMAC-SHA256 session token. Its key is a
per-install 32-byte random `auth.session_secret` generated and durably
persisted by Captain at first boot; it is never derived from the daemon API key
or password hash. The token carries the current managed `session_epoch`.
Changing the password through setup or `web_credentials_update` advances that
epoch, so an older token fails verification even when it has not expired.
New passwords use salted Argon2id PHC strings. A successful login against a
legacy SHA-256 hash atomically migrates the persisted value before Captain
issues the session. Five failed attempts arm a dedicated per-IP and
per-username exponential backoff; an immediate sixth attempt returns `429`.
Each process retains at most 4,096 IP and 4,096 normalized-username records.
Capacity cleanup may reuse only a record whose retry delay is no longer active.
If every slot is actively blocked, Captain keeps those blocks and applies a
logged five-second global `429` backoff with `Retry-After`. This bounded state
resets with the daemon, so Internet-facing deployments must enforce an
additional login limit at the reverse proxy, firewall, or edge.

The browser exchanges its HttpOnly cookie for a 30-second single-use ticket
before opening WebSocket or SSE. Tickets are bound to the exact path, effective
client IP, and current session epoch. Protected routes do not accept API keys
or session tokens from `?token=`.

`GET /api/config/raw` and other config-display surfaces remove
`auth.password_hash` and `auth.session_secret` from their response. Raw config
writes preserve the current managed values server-side and reject a supplied
signing key or any direct epoch change.

---

## Agent Endpoints

### GET /api/agents

List all running agents.

**Response** `200 OK`:

```json
[
  {
    "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
    "name": "hello-world",
    "state": "Running",
    "created_at": "2025-01-15T10:30:00Z",
    "model_provider": "groq",
    "model_name": "llama-3.3-70b-versatile",
    "reasoning": {
      "supported": false,
      "configured_effort": null,
      "effective_effort": null,
      "source": "unsupported"
    },
    "context_window_tokens": 131072
  }
]
```

### GET /api/agents/{id}

Returns detailed information about a single agent.

**Response** `200 OK`:

```json
{
  "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "name": "hello-world",
  "state": "Running",
  "created_at": "2025-01-15T10:30:00Z",
  "session_id": "s1b2c3d4-...",
  "model": {
    "provider": "groq",
    "model": "llama-3.3-70b-versatile"
  },
  "reasoning": {
    "provider": "groq",
    "model": "llama-3.3-70b-versatile",
    "supported": false,
    "source": "unsupported",
    "override_valid": true,
    "options": [],
    "reported_by_provider": false
  },
  "context_window_tokens": 131072,
  "capabilities": {
    "tools": ["file_read", "file_list", "web_fetch"],
    "network": []
  },
  "description": "A friendly greeting agent",
  "tags": []
}
```

### POST /api/agents

Spawn a new agent from a TOML manifest.

**Request Body** (JSON):

```json
{
  "manifest_toml": "name = \"my-agent\"\nversion = \"0.1.0\"\ndescription = \"Test agent\"\nauthor = \"me\"\nmodule = \"builtin:chat\"\n\n[model]\nprovider = \"groq\"\nmodel = \"llama-3.3-70b-versatile\"\n\n[capabilities]\ntools = [\"file_read\", \"web_fetch\"]\nmemory_read = [\"*\"]\nmemory_write = [\"self.*\"]\n",
  "agent_api": {
    "provision_ingress_token": true,
    "egress_callback_url": "https://service.example.com/captain/callback",
    "generate_callback_secret": true
  }
}
```

`agent_api` is optional. When omitted, Captain still provisions the ingress
bearer token and returns it once. Supplying `egress_callback_url` during spawn
also configures signed callbacks so the agent API can become fully in/out ready
immediately.

**Response** `201 Created`:

```json
{
  "agent_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "name": "my-agent",
  "agent_api_provisioning": {
    "protocol": "agent-as-service.v1",
    "status": "ready",
    "ingress": {
      "status": "ready",
      "ingress_url": "/hooks/agents/a1b2c3d4-e5f6-7890-abcd-ef1234567890/ingress",
      "auth_scheme": "Authorization: Bearer $TOKEN",
      "token_env": "CAPTAIN_AGENT_API_TOKEN_A1B2C3D4_E5F6_7890_ABCD_EF1234567890",
      "token": "<returned-once-bearer-token>"
    },
    "egress": {
      "status": "ready",
      "configure_url": "/api/agents/a1b2c3d4-e5f6-7890-abcd-ef1234567890/api/egress/configure",
      "test_url": "/api/agents/a1b2c3d4-e5f6-7890-abcd-ef1234567890/api/egress/test",
      "callback_secret": "<returned-once-generated-secret>"
    },
    "operator_actions": []
  },
  "agent_api_config_status": {
    "state": "ready",
    "can_receive": true,
    "can_send_callbacks": true
  }
}
```

If no egress callback URL is provided, `agent_api_provisioning.status` and
`agent_api_config_status.state` are `ingress_ready`, not `ready`; the response
states that Captain cannot infer the external callback URL and includes the
exact egress configure action.

### PUT /api/agents/{id}/update

Update an agent's configuration at runtime.

**Request Body**:

```json
{
  "description": "Updated description",
  "system_prompt": "You are a specialized assistant.",
  "tags": ["updated", "v2"]
}
```

**Response** `200 OK`:

```json
{
  "status": "updated",
  "agent_id": "a1b2c3d4-..."
}
```

### PUT /api/agents/{id}/mode

Set an agent's operating mode. `Stable` mode pins the current model and freezes the skill registry. `Normal` mode restores default behavior.

**Request Body**:

```json
{
  "mode": "Stable"
}
```

**Response** `200 OK`:

```json
{
  "status": "updated",
  "mode": "Stable",
  "agent_id": "a1b2c3d4-..."
}
```

### POST /api/agents/{id}/message

Send a message to an agent and receive the complete response.

**Request Body**:

```json
{
  "message": "What files are in the current directory?",
  "session_id": "3a1e6f4c-06ad-4bd4-9c79-c1e2fbf39d0d"
}
```

`session_id` is optional. When supplied, Captain validates that the persisted
session belongs to the target agent and executes the turn against that session
without changing the agent's globally active session. This is the safe contract
for independent browser tabs and external clients.

**Response** `200 OK`:

```json
{
  "response": "Here are the files in the current directory:\n- Cargo.toml\n- README.md\n...",
  "input_tokens": 142,
  "output_tokens": 87,
  "iterations": 1
}
```

When either Captain's internal rolling guard or the configured provider's
subscription allowance blocks the turn, this endpoint returns `429` rather
than a generic `500`. The payload and `Retry-After` header identify the actual
owner and reset when known:

```http
HTTP/1.1 429 Too Many Requests
Retry-After: 600
Content-Type: application/json

{
  "error": "Quota horaire Captain atteint pour l'agent captain: 228733 / 200000 tokens.",
  "code": "captain_agent_hourly_token_quota",
  "quota": {
    "code": "captain_agent_hourly_token_quota",
    "scope": "agent_hourly_tokens",
    "agent_id": "792a2b4e-20bd-495f-bcfa-819818c15911",
    "used": 228733,
    "limit": 200000,
    "unit": "tokens",
    "window_seconds": 3600,
    "resets_at": "2026-07-18T12:00:00Z",
    "retry_after_seconds": 600,
    "message": "Quota horaire Captain atteint pour l'agent captain: 228733 / 200000 tokens."
  }
}
```

For a Codex subscription limit, `scope` is `provider_subscription`,
`provider` is `codex`, and the window/reset values come from Codex's live
account, response-header, or stream signal. Captain does not infer them from
local token usage.

### Agent-as-service API

Each running agent can expose a dedicated external HTTP integration surface.
Use these routes when an external service, webhook, or backend needs to call one
specific agent directly.

The normal operator flow is:

1. Create the agent with `POST /api/agents`. Captain provisions the ingress
   token by default and returns it once in `agent_api_provisioning`.
2. `GET /api/agents/{id}/api/manifest` to read the integration contract.
3. `POST /hooks/agents/{id}/ingress` from the external service with
   `Authorization: Bearer <token>`.
4. Configure signed callbacks during creation with `agent_api.egress_callback_url`,
   or later with `POST /api/agents/{id}/api/egress/configure`, then verify
   delivery with `POST /api/agents/{id}/api/egress/test`.

`ready` means ingress and egress callbacks are configured. `ingress_ready` means
the agent can receive authenticated HTTP calls, but outbound callbacks still
need configuration before the integration is fully in/out ready.

### GET /api/agents/{id}/api

Returns the operator status for the per-agent API surface, including readiness,
token environment name, ingress URL, callback status, and queue health. Secrets
and callback URLs are redacted.

### GET /api/agents/{id}/api/manifest

Returns the full external integration contract:

- ingress URL, auth scheme, idempotency key, payload schema, and limits;
- egress event names, HMAC signature headers, and callback operations;
- readiness state and concrete operator actions.

### POST /api/agents/{id}/api/token/rotate

Generates and stores a new ingress bearer token for the agent API.

**Response** `200 OK`:

```json
{
  "rotation": {
    "status": "rotated",
    "agent_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
    "token_env": "CAPTAIN_AGENT_API_TOKEN_A1B2C3D4_E5F6_7890_ABCD_EF1234567890",
    "token": "<returned-once-bearer-token>",
    "stored_in": "secrets.env",
    "warning": "Token is returned once. Store it in the external service and use Authorization: Bearer <token>."
  }
}
```

### POST /hooks/agents/{id}/ingress

External REST/webhook ingress for one agent turn. This route uses the per-agent
bearer token, not the web session cookie.

**Request Body**:

```json
{
  "request_id": "external-unique-id-123",
  "message": "Ask this agent to do one concrete task.",
  "sender_id": "external-service:user-or-job-id",
  "sender_name": "External Service",
  "metadata": {
    "source": "external-service"
  }
}
```

**Response** `200 OK`:

```json
{
  "status": "completed",
  "response": "Agent response text",
  "request_id": "external-unique-id-123",
  "egress": {
    "attempted": true,
    "delivered": true
  }
}
```

### POST /api/agents/{id}/api/egress/configure

Configures outbound signed callback delivery for an agent API. The callback
secret is stored in `secrets.env` and normal status responses do not reveal it.

### POST /api/agents/{id}/api/egress/test

Sends a signed diagnostic callback with event `agent_api.test`.

### GET /api/agents/{id}/api/events

Returns recent per-agent API audit events for ingress, egress, duplicates,
failures, and retries without raw tokens or message payloads.

### GET /api/agents/{id}/api/egress

Returns pending and dead-lettered callback deliveries for the agent.

### POST /api/agents/{id}/api/egress/{queue_id}/retry

Retries one queued or dead-lettered callback delivery immediately.

### GET /api/agents/{id}/session

Returns the agent's conversation history.

**Response** `200 OK`:

```json
{
  "session_id": "s1b2c3d4-...",
  "agent_id": "a1b2c3d4-...",
  "message_count": 4,
  "context_window_tokens": 131072,
  "estimated_context_tokens": 1250,
  "messages": [
    {
      "role": "User",
      "content": "Hello"
    },
    {
      "role": "Assistant",
      "content": "Hello! How can I help you?"
    }
  ]
}
```

`context_window_tokens` is the effective capacity of the owning agent's
currently configured model, resolved from the live catalog. It is not a usage
counter. `estimated_context_tokens` approximates the stored transcript's
current occupancy; cumulative provider usage is reported separately.

### DELETE /api/agents/{id}

Terminate an agent and remove it from the registry.

**Response** `200 OK`:

```json
{
  "status": "killed",
  "agent_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890"
}
```

---

## Workflow Endpoints

### GET /api/workflows

List all registered workflows.

**Response** `200 OK`:

```json
[
  {
    "id": "w1b2c3d4-...",
    "name": "code-review-pipeline",
    "description": "Automated code review workflow",
    "steps": 3,
    "created_at": "2025-01-15T10:30:00Z"
  }
]
```

### POST /api/workflows

Create a new workflow definition.

**Request Body** (JSON):

```json
{
  "name": "code-review-pipeline",
  "description": "Review code changes with multiple agents",
  "steps": [
    {
      "name": "analyze",
      "agent_name": "coder",
      "prompt": "Analyze this code for potential issues: {{input}}",
      "mode": "sequential",
      "timeout_secs": 120,
      "error_mode": "fail",
      "output_var": "analysis"
    },
    {
      "name": "security-check",
      "agent_name": "security-auditor",
      "prompt": "Review this code analysis for security vulnerabilities: {{analysis}}",
      "mode": "sequential",
      "timeout_secs": 120,
      "error_mode": "skip"
    },
    {
      "name": "summarize",
      "agent_name": "writer",
      "prompt": "Write a concise code review summary based on: {{analysis}}",
      "mode": "sequential",
      "timeout_secs": 60,
      "error_mode": "fail"
    }
  ]
}
```

**Step configuration options:**

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Step name |
| `agent_id` | string | Agent UUID (use either this or `agent_name`) |
| `agent_name` | string | Agent name (use either this or `agent_id`) |
| `prompt` | string | Prompt template with `{{input}}` and `{{output_var}}` placeholders |
| `mode` | string | `"sequential"`, `"fan_out"`, `"collect"`, `"conditional"`, `"loop"` |
| `timeout_secs` | integer | Timeout per step (default: 120) |
| `error_mode` | string | `"fail"`, `"skip"`, `"retry"` |
| `max_retries` | integer | For `"retry"` error mode (default: 3) |
| `output_var` | string | Variable name to store output for later steps |
| `condition` | string | For `"conditional"` mode |
| `max_iterations` | integer | For `"loop"` mode (default: 5) |
| `until` | string | For `"loop"` mode: stop condition |

**Response** `201 Created`:

```json
{
  "workflow_id": "w1b2c3d4-..."
}
```

### GET /api/workflows/{id}

Get one workflow definition, including its serialized steps. Invalid UUIDs
return `400`; unknown workflow ids return `404`.

**Response** `200 OK`:

```json
{
  "id": "w1b2c3d4-...",
  "name": "code-review-pipeline",
  "description": "Review code changes with multiple agents",
  "steps": [],
  "created_at": "2026-07-12T10:30:00Z"
}
```

### PUT /api/workflows/{id}

Replace a workflow definition. The request uses the same `name`,
`description`, and `steps` contract as `POST /api/workflows`.

**Response** `200 OK`:

```json
{
  "status": "updated",
  "workflow_id": "w1b2c3d4-..."
}
```

### DELETE /api/workflows/{id}

Remove one workflow definition. Invalid UUIDs return `400`; unknown workflow
ids return `404`.

**Response** `200 OK`:

```json
{
  "status": "removed",
  "workflow_id": "w1b2c3d4-..."
}
```

### POST /api/workflows/{id}/run

Execute a workflow.

**Request Body**:

```json
{
  "input": "Review this pull request: ..."
}
```

**Response** `200 OK`:

```json
{
  "run_id": "r1b2c3d4-...",
  "output": "Code review summary:\n- No critical issues found\n...",
  "status": "completed"
}
```

### GET /api/workflows/{id}/runs

List execution history strictly scoped to the requested workflow. The id is
validated before lookup and results are returned newest-first.

**Response** `200 OK`:

```json
[
  {
    "id": "r1b2c3d4-...",
    "workflow_name": "code-review-pipeline",
    "state": "completed",
    "steps_completed": 3,
    "started_at": "2025-01-15T10:30:00Z",
    "completed_at": "2025-01-15T10:32:15Z",
    "output": "Code review summary...",
    "error": null
  }
]
```

---

## Trigger Endpoints

### GET /api/triggers

List all triggers. Optionally filter by agent.

**Query Parameters:**
- `agent_id` (optional): Filter by agent UUID

**Response** `200 OK`:

```json
[
  {
    "id": "t1b2c3d4-...",
    "agent_id": "a1b2c3d4-...",
    "pattern": {"lifecycle": {}},
    "prompt_template": "Event: {{event}}",
    "enabled": true,
    "fire_count": 5,
    "max_fires": 0,
    "created_at": "2025-01-15T10:30:00Z"
  }
]
```

### POST /api/triggers

Create a new event trigger.

**Request Body**:

```json
{
  "agent_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "pattern": {
    "agent_spawned": {
      "name_pattern": "*"
    }
  },
  "prompt_template": "A new agent was spawned: {{event}}. Review its capabilities.",
  "max_fires": 0
}
```

**Supported pattern types:**

| Pattern | Description |
|---------|-------------|
| `{"lifecycle": {}}` | All lifecycle events |
| `{"agent_spawned": {"name_pattern": "*"}}` | Agent spawn events |
| `{"agent_terminated": {}}` | Agent termination events |
| `{"all": {}}` | All events |

**Response** `201 Created`:

```json
{
  "trigger_id": "t1b2c3d4-...",
  "agent_id": "a1b2c3d4-..."
}
```

### PUT /api/triggers/{id}

Update an existing trigger's configuration.

**Request Body**:

```json
{
  "prompt_template": "Updated template: {{event}}",
  "enabled": false,
  "max_fires": 10
}
```

**Response** `200 OK`:

```json
{
  "status": "updated",
  "trigger_id": "t1b2c3d4-..."
}
```

### DELETE /api/triggers/{id}

Remove a trigger.

**Response** `200 OK`:

```json
{
  "status": "removed",
  "trigger_id": "t1b2c3d4-..."
}
```

---

## Memory Endpoints

### GET /api/memory/agents/{id}/kv

List all key-value pairs for an agent.

**Response** `200 OK`:

```json
{
  "kv_pairs": [
    {"key": "preferences", "value": {"theme": "dark"}},
    {"key": "state", "value": {"step": 3}}
  ]
}
```

### GET /api/memory/agents/{id}/kv/{key}

Get a specific key-value pair.

**Response** `200 OK`:

```json
{
  "key": "preferences",
  "value": {"theme": "dark"}
}
```

**Response** `404 Not Found` (key does not exist):

```json
{
  "error": "Key 'preferences' not found"
}
```

### PUT /api/memory/agents/{id}/kv/{key}

Set a key-value pair. Creates or overwrites.

**Request Body**:

```json
{
  "value": {"theme": "dark", "language": "en"}
}
```

**Response** `200 OK`:

```json
{
  "status": "stored",
  "key": "preferences"
}
```

### DELETE /api/memory/agents/{id}/kv/{key}

Delete a key-value pair.

**Response** `200 OK`:

```json
{
  "status": "deleted",
  "key": "preferences"
}
```

---

## Learning Endpoints

### GET /api/learning/status

Return the shared, public-safe operational state of Skill Learning V2. Counts,
worker identity, and timestamps come from one SQLite snapshot. `bound_model`
is present only after the worker successfully binds that exact proposer.

**Response** `200 OK`:

```json
{
  "schema_version": 1,
  "enabled": true,
  "mode": "approval",
  "state": "recovering",
  "recovery": "automatic_retry_active",
  "expected_model": {"provider": "codex", "model": "gpt-5.6-sol"},
  "worker": {
    "phase": "running",
    "bound_model": {"provider": "codex", "model": "gpt-5.6-sol"},
    "started_at_unix_ms": 1784700000000,
    "heartbeat_at_unix_ms": 1784700100000,
    "heartbeat_age_ms": 2000,
    "last_scan_at_unix_ms": 1784700095000,
    "last_progress_at_unix_ms": 1784700080000,
    "last_error_scope": null
  },
  "jobs": {
    "pending": 1, "running": 0, "retry_wait": 1,
    "uncertain": 0, "dead": 0,
    "oldest_actionable_at_unix_ms": 1784700080000,
    "next_retry_at_unix_ms": 1784700120000,
    "last_activity_at_unix_ms": 1784700100000,
    "last_error_code": "provider_busy"
  },
  "notifications": {
    "pending": 0, "delivering": 0, "retry_wait": 0, "dead": 0,
    "oldest_actionable_at_unix_ms": null,
    "next_retry_at_unix_ms": null,
    "last_activity_at_unix_ms": 1784700090000
  },
  "workflows": {
    "total": 5, "processing": 1, "awaiting_decision": 1,
    "active": 3, "attention": 0,
    "last_activity_at_unix_ms": 1784700100000
  },
  "generated_at_unix_ms": 1784700102000
}
```

No percentage is returned: opaque model work has no observable fractional
progress. A heartbeat older than the runtime threshold yields `stalled`; a
model binding failure yields `degraded`.

### GET /api/learning/workflows

Return the durable Skill/CapSpec/Automation/refinement projection consumed by
TUI, Control Web, Desktop, and operator cards. Optional `limit` is clamped to
500 for the HTTP surface.

### POST /api/learning/workflows/{operator_token}/decide

Apply one exact authenticated operator action using `decision_version`,
`action`, and an authenticated `surface`. Stale revisions return `409`; there
is no agent-facing positive-decision tool.

The generic memory-learning endpoints remain separate:

| Endpoint | Purpose |
|---|---|
| `GET /api/learning/committed` | Recent committed memory learnings |
| `GET /api/learning/review` | Memory candidates awaiting review |
| `POST /api/learning/review/{id}/decide` | Human decision for one memory candidate |
| `GET /api/learning/metrics` | Memory journal and review aggregate |

---

## Channel Endpoints

### GET /api/channels

List configured channel adapters and their status. The active core channel
surface is Telegram, Discord, Signal, and Email. Other channel adapters may
remain compiled or configurable for compatibility, but they are frozen out of
normal setup and bridge startup until the core is production-grade.

The response is also the authoritative guided-client schema. `fields` describes
generic channel forms. Email uses `setup_type: "email_accounts"`,
`account_fields`, and an `account_summary` with public-safe per-account
readiness. Secret values and credential references are never returned.

**Response** `200 OK`:

```json
{
  "channels": [
    {
      "name": "telegram",
      "display_name": "Telegram",
      "setup_type": "form",
      "configured": true,
      "ready": true,
      "has_required_secrets": true,
      "missing_required_fields": [],
      "operator_actions": [],
      "fields": []
    },
    {
      "name": "email",
      "display_name": "Email",
      "setup_type": "email_accounts",
      "configured": true,
      "ready": false,
      "operational_state": "partial",
      "account_summary": {
        "total_accounts": 2,
        "ready_accounts": 1,
        "default_account": "work"
      },
      "account_fields": []
    }
  ],
  "total": 4,
  "configured_count": 2,
  "active_only": true
}
```

Readiness states are `ready`, `partial`, `locked`, `disabled`, `invalid`, and
`not_configured`. A client must use the returned field keys and types rather
than translating environment-variable labels back into request properties.

### POST /api/channels/{name}/configure

Configure a generic active channel with the exact fields returned by the list
schema:

```json
{
  "fields": {
    "bot_token_env": "<secret>",
    "allowed_users": "123456789"
  }
}
```

Email patches one named mailbox and preserves every omitted field plus all
other accounts:

```json
{
  "account": {
    "alias": "work",
    "username": "captain@example.com",
    "password": "<app-password>",
    "imap_host": "imap.example.com",
    "smtp_host": "smtp.example.com",
    "folders": ["INBOX"],
    "allowed_senders": ["me@example.com", "@example.org"]
  },
  "make_default": true
}
```

`password` is accepted only as request input, zeroized after use, and persisted
through the credential resolver. It is never written to TOML or returned. An
existing account with a ready credential can be patched without sending the
password again. The success status is `configured`; if persistence succeeded
but hot reload failed it is `configured_reload_failed` with an actionable
`note`. Both are persisted outcomes, but only `activated: true` proves the live
adapter started.

### POST /api/channels/{name}/test

Test configuration and optionally send a transport message. Email accepts the
default as `email` or a named path `email:<alias>` and also accepts
`account_alias` in the body. Path and body aliases must agree.

```json
{
  "recipient": "operator@example.com"
}
```

The Email probe performs real IMAP login/folder checks and SMTP authentication;
a recipient additionally requests one real delivery. Network work is bounded,
but can take longer than token-only channel tests. This endpoint retains HTTP
200 for a completed negative probe and returns `{"status":"error",...}`;
clients must inspect the JSON status rather than treating HTTP success alone as
channel readiness.

### DELETE /api/channels/{name}/configure

Remove one active channel configuration and its locally managed credentials,
then hot reload. Externally managed secret sources are not deleted. Email
removal is serialized with account updates and removes every local per-account
credential after the channel block is removed. The response reports
`cleanup_warning_count`; a nonzero count requires operator inspection.

---

## Approval Endpoints

Tool approvals are authenticated operator decisions. Interactive session and
durable choices are bound to the exact agent, tool, and BLAKE3 action digest;
they never widen the administrator-owned `[approval].allow_always` list. Raw
commands are shown only on the pending request and are never persisted in the
durable rule file. Captain computes the digest from the complete untruncated
tool input; the bounded display preview is never the authorization key.

### GET /api/approvals

Returns pending requests plus the current durable exact-action rules. Rule
objects expose the digest, effect, creator, optional bounded reason, and rule
ID, but not the raw action.

```json
{
  "approvals": [
    {
      "id": "53a7a698-05aa-4933-92dd-c8c7af86e62b",
      "agent_id": "captain",
      "agent_name": "captain",
      "tool_name": "shell_exec",
      "action_summary": "deploy the verified release",
      "risk_level": "critical",
      "status": "pending"
    }
  ],
  "total": 1,
  "rules": [
    {
      "id": "e56de4c6-4956-47e0-b918-934c3b69d18e",
      "effect": "deny",
      "agent_id": "captain",
      "tool_name": "shell_exec",
      "action_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "created_by": "channel:123456",
      "reason": "Use the staging host"
    }
  ],
  "rules_total": 1
}
```

### POST /api/approvals/{id}/approve

Approves this occurrence only.

### POST /api/approvals/{id}/approve_session

Approves this exact action for this agent until daemon restart or an explicit
session-approval cache clear.

### POST /api/approvals/{id}/approve_always

Creates a durable, revocable allow rule for this exact action.

### POST /api/approvals/{id}/reject

### POST /api/approvals/{id}/reject_session

### POST /api/approvals/{id}/reject_always

All rejection routes accept an optional bounded operator reason:

```json
{ "reason": "Use the staging host instead" }
```

`reject` keeps an empty request body for compatibility. `reject_always`
requires a non-empty reason. Every reason is whitespace-normalized, limited to
280 characters, checked for secret-like material, returned to the blocked
tool, and recorded in the audit decision. The agent is told not to retry the
same action unchanged.

### DELETE /api/approvals/rules/{id}

Revokes one durable exact-action rule. Revocation is audited. Invalid input is
`400`, a missing or already-resolved request/rule is `404`, an expired request
is `410`, and persistence failures remain `500` rather than being reported as
an operator rejection.

### POST /api/approvals/clear_session

Clears every in-memory exact-action session decision. Durable rules are not
affected.

---

## Template Endpoints

### GET /api/templates

List available agent templates from the agents directory.

**Response** `200 OK`:

```json
{
  "templates": [
    {
      "name": "hello-world",
      "description": "A friendly greeting agent",
      "path": "/home/user/.captain/agents/hello-world/agent.toml"
    },
    {
      "name": "coder",
      "description": "Expert coding assistant",
      "path": "/home/user/.captain/agents/coder/agent.toml"
    }
  ],
  "total": 30
}
```

### GET /api/templates/{name}

Get a specific template's manifest and raw TOML.

**Response** `200 OK`:

```json
{
  "name": "hello-world",
  "manifest": {
    "name": "hello-world",
    "description": "A friendly greeting agent",
    "module": "builtin:chat",
    "tags": [],
    "model": {
      "provider": "groq",
      "model": "llama-3.3-70b-versatile"
    },
    "capabilities": {
      "tools": ["file_read", "file_list", "web_fetch"],
      "network": []
    }
  },
  "manifest_toml": "name = \"hello-world\"\nversion = \"0.1.0\"\n..."
}
```

---

## System Endpoints

### GET /api/health

Public health check. Does not require authentication. Returns only liveness and
the running version; it does not expose counters, database state, or agent
details.

**Response** `200 OK`:

```json
{
  "status": "ok",
  "version": "0.1.0-alpha.13"
}
```

The `status` field is `"ok"` when the database and audit integrity are
healthy. It is `"degraded"` when either check fails; no internal detail is
included in this public response.

### GET /api/health/detail

Full health check with all dependency status. Requires authentication. Unlike the public `/api/health`, this endpoint includes database connectivity and agent counts.

**Response** `200 OK`:

```json
{
  "status": "ok",
  "version": "0.1.0-alpha.13",
  "uptime_seconds": 3600,
  "failure_count": 4,
  "panic_count": 0,
  "restart_count": 0,
  "agent_count": 3,
  "database": "connected",
  "audit": {
    "valid": true,
    "status": "healthy",
    "active_epoch": 0,
    "active_epoch_valid": true,
    "invalid_epochs": [],
    "entry_count": 42,
    "tip_hash": "8f6d..."
  },
  "config_warnings": []
}
```

### GET /api/status

Detailed operator-safe kernel status. This is the shared source for
`captain status`, the TUI Status hub, and the Control web Status hub.

The response includes these stable groups in addition to basic version,
provider, uptime, path, deployment, and access fields:

| Field | Contract |
|---|---|
| `runtime_health` | Roll-up `state`, structured `issues`, and deduplicated `operator_actions` |
| `agents`, `active_runs`, `active_processes` | Registered agents and currently supervised work |
| `tool_runs` | Counts for running/completed/failed/cancelled/interrupted plus payload-free recent metadata, input digest, retained-output integrity fields, and retry lineage |
| `artifacts` | Immutable artifact/version counts, payload bytes, integrity health, invalid entries, recovered staging count, and hard size quotas; never payloads or managed paths |
| `workload` | Projects, goals, crons, triggers, and automation delivery/dead-letter state |
| `agent_api` | Agent-as-service egress queue/readiness summary |
| `budget` | Captain internal token/cost guards plus separately persisted provider-reported subscription windows and operator actions |
| `channels` | Active channel readiness and inbound queue summary |
| `consciousness` | Operational awareness state, signals, and actions |
| `streaming` | Active/completed stream timing telemetry |
| `disk`, `shutdown` | Free-space policy and graceful-drain state |
| `runtime_update` | Last successful release check, next 12-hour check, pending version, detached-install state, and notification retry/dead-letter counts |
| `deployment.readiness` | Cached local/public port, DNS, TLS, reverse-proxy routing, health, version-parity and freshness checks with bounded operator actions |
| `outbound_delivery` | Exact final-response transport state: pending, attempting, delivered, dead, possible duplicate, oldest pending age, and bounded last error |
| `native_voice`, `native_embeddings`, `media`, `tts` | Native capability readiness |

`consciousness.supervisor.failure_count` counts recoverable turn failures since
the daemon started. `panic_count` is reserved for actual caught task panics;
historical failures alone do not keep operational awareness in warning state.

**Representative response** `200 OK` (fields are intentionally abbreviated):

```json
{
  "status": "running",
  "version": "0.1.0-dev.<build>",
  "default_provider": "codex",
  "default_model": "gpt-5.5",
  "llm_driver_ready": true,
  "agent_count": 2,
  "active_run_count": 0,
  "process_count": 0,
  "runtime_health": {
    "state": "ok",
    "issue_count": 0,
    "issues": [],
    "operator_actions": []
  },
  "deployment": {
    "profile": "vps",
    "public_url": "https://captain.example.com",
    "https": true,
    "reverse_proxy": "caddy",
    "readiness": {
      "state": "ready",
      "checked_at": "2026-08-08T12:00:00Z",
      "duration_ms": 284,
      "next_check_at": "2026-08-08T12:05:00Z",
      "checks": [
        { "id": "dns", "status": "ok", "summary": "The public domain resolves safely" }
      ],
      "operator_actions": []
    }
  },
  "tool_runs": {
    "running": 0,
    "completed": 4,
    "failed": 0,
    "cancelled": 0,
    "interrupted": 0,
    "recent": []
  },
  "artifacts": {
    "healthy": true,
    "artifacts": 3,
    "versions": 7,
    "bytes": 18432,
    "invalid_entries": 0,
    "recovered_staging_entries": 0,
    "max_artifact_bytes": 52428800,
    "max_total_bytes": 536870912
  },
  "runtime_update": {
    "last_checked_at": "2026-07-20T08:00:00Z",
    "last_success_at": "2026-07-20T08:00:00Z",
    "next_check_at": "2026-07-20T20:00:00Z",
    "last_error": null,
    "consecutive_failures": 0,
    "pending_version": "0.1.0-alpha.13",
    "update_in_progress": false,
    "undelivered_notifications": 1,
    "dead_notifications": 0
  },
  "budget": {
    "total_tokens_used": 12000,
    "limited_agents": 2,
    "provider_subscriptions": {
      "state": "ok",
      "reported_by_provider": true,
      "contract": "official_provider_signals",
      "stale_after_seconds": 900,
      "items": [
        {
          "provider": "codex",
          "limit_id": "codex",
          "primary": {
            "used_percent": 28.0,
            "remaining_percent": 72.0,
            "remaining_source": "derived_from_provider_used_percent",
            "window_seconds": 18000,
            "resets_at": "2026-07-18T15:00:00Z"
          },
          "secondary": {
            "used_percent": 47.0,
            "remaining_percent": 53.0,
            "remaining_source": "derived_from_provider_used_percent",
            "window_seconds": 604800,
            "resets_at": "2026-07-22T10:00:00Z"
          },
          "spend_control": {
            "reached": false,
            "individual_limit": {
              "source": "monthly",
              "limit": "200.00",
              "used": "56.00",
              "remaining": "144.00",
              "used_percent": 28,
              "remaining_percent": 72,
              "remaining_source": "provider_reported",
              "resets_at": "2026-08-01T00:00:00Z"
            }
          },
          "source": "account_status",
          "alert_level": "normal",
          "stale": false
        }
      ],
      "reset_notifications": {
        "state": "active",
        "pending": 1,
        "delivering": 0,
        "retry_wait": 0,
        "delivered": 3,
        "suppressed": 0,
        "dead": 0,
        "uncertain": 0
      }
    }
  },
  "agents": []
}
```

Deployment probes run in a bounded daemon worker after startup and every five
minutes. `/api/status` only reads the crash-safe owner-only cache; polling
Status therefore never starts DNS or HTTP work. Public DNS is resolved under a
timeout, every selected address passes the outbound SSRF policy, the request is
pinned to that address with redirects and environment proxies disabled, and
the health body is capped at 64 KiB. The projection omits resolved addresses,
raw network errors, cache paths, schema identity, and its configuration
fingerprint. A changed configuration or Captain version invalidates the cache;
an overdue check becomes `degraded` instead of preserving a stale `ready`.

The example window durations are illustrative values returned by the provider,
not Captain defaults. With no official observation,
`budget.provider_subscriptions.state` is `unavailable` and
`reported_by_provider` is `false`; Captain never turns absence into an
"unlimited" claim. Observations older than 900 seconds are marked `stale`.
`reset_notifications` is a content-free projection of the durable Telegram
reset outbox. Its state is `ok`, `active`, `attention`, or `unavailable`.
`active` means at least one row is pending, delivering, or waiting for retry;
`attention` means a `dead` or `uncertain` delivery requires operator review.
An uncertain delivery is never automatically replayed because Telegram may
already have accepted it before Captain lost the receipt.

### GET /api/version

Build and version information.

**Response** `200 OK`:

```json
{
  "name": "captain",
  "version": "0.1.0-dev.<build>",
  "build_date": "<build date>",
  "git_sha": "<git sha>",
  "rust_version": "1.82.0",
  "platform": "linux",
  "arch": "x86_64"
}
```

### POST /api/shutdown

Initiate graceful shutdown. Agent states are preserved to SQLite for restore on
next boot. The server also drains its persistent Web terminal registry and
terminates each owned PTY child, so a daemon restart cannot inherit an orphaned
`captain chat` process.

**Response** `200 OK`:

```json
{
  "status": "shutting_down"
}
```

### GET /api/profiles

List available agent profiles (predefined configurations for common use cases).

**Response** `200 OK`:

```json
{
  "profiles": [
    {
      "name": "coder",
      "tier": "smart",
      "description": "Expert coding assistant"
    },
    {
      "name": "researcher",
      "tier": "frontier",
      "description": "Deep research and analysis"
    }
  ]
}
```

### GET /api/tools

List all available built-in and connected MCP tools. Entries are definitions,
not bare names. MCP entries include `"source": "mcp"`; built-ins omit it.

**Response** `200 OK`:

```json
{
  "tools": [
    {
      "name": "file_read",
      "description": "Read a file from the allowed workspace.",
      "input_schema": {
        "type": "object",
        "properties": {
          "path": {"type": "string"}
        },
        "required": ["path"]
      }
    }
  ],
  "total": 1
}
```

### GET /api/config

Retrieve current kernel configuration (secrets are redacted).

**Response** `200 OK`:

```json
{
  "data_dir": "/home/user/.captain/data",
  "default_provider": "codex",
  "default_model": "gpt-5.5",
  "listen_addr": "127.0.0.1:50051",
  "api_key_set": true,
  "channels_configured": 2,
  "mcp_servers": 1
}
```

### GET /api/peers

List OFP (Captain Protocol) wire peers and their connection status.

**Response** `200 OK`:

```json
{
  "peers": [
    {
      "node_id": "peer-1",
      "address": "192.168.1.100:4000",
      "state": "connected",
      "authenticated": true,
      "last_seen": "2025-01-15T10:30:00Z"
    }
  ]
}
```

### GET /api/sessions

List all persisted sessions across agents, newest first. Empty and historical
sessions are included until explicitly deleted.

**Response** `200 OK`:

```json
{
  "sessions": [
    {
      "session_id": "3a1e6f4c-06ad-4bd4-9c79-c1e2fbf39d0d",
      "agent_id": "a1b2c3d4-...",
      "agent_name": "captain",
      "label": "Organiser les documents administratifs du couple",
      "message_count": 12,
      "active": false,
      "created_at": "2025-01-15T10:30:00Z",
      "updated_at": "2025-01-15T10:42:00Z"
    }
  ]
}
```

When no explicit label exists, Captain derives a bounded label from the first
meaningful user message. Greetings and slash commands are skipped, and an
explicit label is never overwritten. `active` only identifies the owning
agent's default session; every other row remains directly restorable. This is
the source-independent catalog used by Web Control, TUI, CLI and Desktop.

At kernel boot, legacy TUI mirrors under
`$CAPTAIN_HOME/sessions/*/*.json` (`~/.captain` by default) are imported into
this catalog. Missing IDs are derived deterministically from the relative
source path and original timestamps are preserved. A successful import writes
a sibling `.json.imported` marker, so later boots do not overwrite or
resurrect a session already continued or explicitly deleted elsewhere.

### GET /api/agents/{id}/sessions

List every persisted session owned by one agent. Each row includes `active`,
which only marks the agent's default session; any other row remains directly
addressable with `session_id`.

### POST /api/agents/{id}/sessions

Create a persisted session.

```json
{
  "label": "Optional explicit label",
  "activate": false
}
```

`activate` defaults to `true` for backward compatibility. Use `false` for a
detached Web/API conversation: Captain creates the session but leaves the
agent's global active session unchanged.

```json
{
  "session_id": "3a1e6f4c-06ad-4bd4-9c79-c1e2fbf39d0d",
  "label": "Optional explicit label",
  "active": false
}
```

### GET /api/sessions/{id}

Load the public transcript and metadata for one persisted session without
switching the agent. A missing UUID returns `404`. The bundled clients expose
the same operation as `client.sessions.get(sessionId)` in JavaScript and
`client.sessions.get(session_id)` in Python.

The response uses the same transcript shape as the per-agent session endpoint,
including live `context_window_tokens` and approximate
`estimated_context_tokens`. If the owning agent is no longer registered, the
last capacity persisted with the session is returned.

### POST /api/agents/{id}/session/reset

Create and activate a fresh session. The previous session is summarized when
useful and remains persisted so it can be reopened. Use
`DELETE /api/sessions/{id}` or `DELETE /api/agents/{id}/history` only when
destructive deletion is intentional.

### DELETE /api/sessions/{id}

Delete a specific session and its conversation history.

**Response** `200 OK`:

```json
{
  "status": "deleted",
  "session_id": "s1b2c3d4-..."
}
```

---

## Live Run Endpoints

These operator endpoints require the normal daemon API key or an authenticated
Web session. API and local operator surfaces share one explicit runtime
projection rather than serializing the internal run record: raw tool input,
result text or preview, output file name, and managed filesystem path are never
returned. An input SHA-256 may be present so an operator can compare intent
without recovering that input.

### GET /api/tool-runs

List recent foreground and detached runs, newest first. `limit` defaults to 50
and must be between 1 and 200. Optional `status` accepts `running`, `completed`,
`failed`, `cancelled`, or `interrupted`.

```http
GET /api/tool-runs?status=running&limit=20
```

Each item includes identity, state, timestamps, elapsed time, caller and tool
use IDs when available, input digest, retry lineage, error/truncation flags,
and checksum/size/redaction metadata for retained output. It never includes the
result itself.

### GET /api/tool-runs/{run_id}

Return the same selective metadata for one bounded `toolrun-*` identifier.
Unknown valid identifiers return `404`; malformed identifiers return `400`.

### GET /api/tool-runs/{run_id}/tail

Read only the sanitized suffix of retained output. `max_lines` defaults to 80
and must be between 1 and 200. The returned `content` is also capped at 32 KiB
on a UTF-8 boundary. `content_truncated`, `content_withheld`, and `sanitized`
make the boundary explicit. If a defense-in-depth scan still recognizes secret
material after normal Live Run redaction, Captain returns a fixed withheld
marker instead of the text. Disk paths and internal read errors do not cross
the API boundary.

### POST /api/tool-runs/{run_id}/cancel

Cancel only a currently running task that still has a live runtime abort
handle. Validation and the `cancelled` transition are atomic. Foreground work
that Captain cannot actually interrupt and already-terminal work return `409`
without changing state. The request crosses the same kernel mutation boundary
as an in-process TUI cancellation. Successful and rejected valid-ID attempts
are appended to the hash-chained audit trail with their fixed operator surface
and `ToolInvoke` outcome; no raw input or result enters the audit event.

Live Run reads cost 3 GCRA tokens and cancellation costs 10. There is no route
that retries a run or sends its output to Telegram, a provider, or another
external destination.

Captain Control consumes only this private projection through the global top
bar's **Live Runs** drawer. It polls while open, filters terminal and active
states locally, renders tail content as text rather than HTML, and displays the
cancel action only when `cancellable` is true. Closing the drawer stops its
polling. It remains an operator drawer, not a seventh product hub.

The full Ratatui `/runs` overlay consumes the same response types for daemon
and in-process modes. It polls only while open, ignores stale tails for a
previous selection, renders tail bytes as terminal-safe text, and requires an
explicit `x` then `y` confirmation before cancellation. Standalone
`captain chat` and the embedded Web terminal only print a summary capped at
twelve selective metadata rows; they do not expose tail or cancellation.

---

## Artifact Endpoints

These authenticated operator endpoints expose immutable, user-facing output
versions without disclosing Captain's managed filesystem paths. Inventory and
version-list responses contain metadata only. Inspection, preview, and
download re-read the selected payload and verify both its byte count and
SHA-256 digest immediately before returning it.

### GET /api/artifacts

List the newest version of each artifact across registered agents. `limit`
defaults to 50 and must be between 1 and 200.

```http
GET /api/artifacts?limit=20
```

```json
{
  "items": [
    {
      "artifact_id": "550e8400-e29b-41d4-a716-446655440000",
      "version": 2,
      "agent_id": "captain",
      "session_id": "session-1",
      "title": "Deployment report",
      "filename": "deployment-report.md",
      "mime_type": "text/markdown",
      "preview_kind": "markdown",
      "size_bytes": 8192,
      "sha256": "<64 lowercase hex characters>",
      "created_at": "2026-08-08T12:00:00Z"
    }
  ],
  "status": {
    "healthy": true,
    "artifacts": 1,
    "versions": 2,
    "bytes": 12288,
    "invalid_entries": 0,
    "recovered_staging_entries": 0,
    "max_artifact_bytes": 52428800,
    "max_total_bytes": 536870912
  }
}
```

### GET /api/artifacts/{id}

Inspect the latest version, or one exact immutable version with
`?version=N`. Version zero is invalid. The response includes typed
`versions`, `download`, and optional `preview` links; a download-only format
returns `null` for `preview`.

### GET /api/artifacts/{id}/versions

List all immutable manifests for one artifact, newest first. This metadata
operation does not read payload bytes; consume a version through inspect,
preview, or download when integrity verification matters.

### GET /api/artifacts/{id}/versions/{version}/preview

Return a bounded, browser-safe representation of one verified version.

| Kind | Preview contract |
|---|---|
| text, Markdown | Valid UTF-8 is HTML-escaped into a passive `<pre>` document |
| HTML | Original UTF-8 bytes are isolated by CSP `sandbox` with no script authority and no network authority |
| raster image, PDF | Verified bytes are served inline with their declared MIME type |
| SVG or unknown/active format | `415 preview_unavailable`; use the download endpoint |

Text, Markdown, and HTML previews are limited to 2 MiB. The preview endpoint
uses `frame-ancestors 'self'`, `X-Frame-Options: SAMEORIGIN`, `no-referrer`,
`nosniff`, and private no-store caching so Control can embed it without
granting same-origin script authority to the artifact.

### GET /api/artifacts/{id}/versions/{version}/download

Return the exact verified bytes as an attachment. The response includes a
quoted SHA-256 `ETag`, UTF-8-safe `Content-Disposition`, `nosniff`,
`no-referrer`, private no-store caching, CSP `sandbox`, and
`frame-ancestors 'none'`. The store permits at most 50 MiB per version and
512 MiB total.

There is intentionally no artifact mutation or deletion endpoint in this
contract. Publishing creates a new immutable version; Captain neither rewrites
nor silently prunes an older version.

---

## Model Catalog Endpoints

Captain maintains a runtime model catalog. These endpoints allow you to browse
available models, check provider authentication status, and resolve model
aliases. Use `captain models providers`, `captain models list`, and
`captain models aliases` to verify the installed binary.

### GET /api/models

List the full model catalog. Returns all known models with their provider, tier, context window, and pricing information.

**Response** `200 OK`:

```json
{
  "models": [
    {
      "id": "claude-sonnet-4-20250514",
      "provider": "anthropic",
      "display_name": "Claude Sonnet 4",
      "tier": "frontier",
      "context_window": 200000,
      "input_cost_per_1m": 3.0,
      "output_cost_per_1m": 15.0,
      "supports_tools": true,
      "supports_vision": true,
      "supports_streaming": true
    },
    {
      "id": "gemini-2.5-flash",
      "provider": "gemini",
      "display_name": "Gemini 2.5 Flash",
      "tier": "smart",
      "context_window": 1048576,
      "input_cost_per_1m": 0.15,
      "output_cost_per_1m": 0.6,
      "supports_tools": true,
      "supports_vision": true,
      "supports_streaming": true
    }
  ],
  "total": 51
}
```

### GET /api/models/updates

Inspect the durable Codex live-catalog monitor. The monitor is active only when
at least one registered agent uses provider `codex` or `openai-codex`. It scans
15 seconds after daemon startup and then hourly. A pending item remains visible
until the user explicitly keeps the current model or completes a safe switch.

**Response** `200 OK`:

```json
{
  "provider": "codex",
  "active": true,
  "baseline_ready": true,
  "known_model_count": 5,
  "last_checked_at": "2026-07-13T12:00:00Z",
  "last_success_at": "2026-07-13T12:00:00Z",
  "last_error": null,
  "consecutive_failures": 0,
  "pending": [
    {
      "model_id": "codex/gpt-5.6",
      "display_name": "GPT-5.6 (Codex)",
      "discovered_at": "2026-07-13T12:00:00Z",
      "telegram_notified_at": "2026-07-13T12:00:02Z"
    }
  ],
  "recent_decisions": [],
  "agents": [
    {
      "agent_id": "00000000-0000-0000-0000-000000000001",
      "agent_name": "captain",
      "current_model": "codex/gpt-5.5"
    }
  ]
}
```

The first successful refresh is silent when no previous Codex cache exists. If
the live endpoint fails, Captain keeps the last valid catalog and reports the
error fields above; it does not manufacture a pending update.

### POST /api/models/updates/decision

Resolve one pending Codex model update. This endpoint never performs an
implicit switch.

Keep the active model:

```json
{
  "model_id": "codex/gpt-5.6",
  "decision": "keep"
}
```

Switch one agent with an explicit provider-portable session strategy:

```json
{
  "model_id": "codex/gpt-5.6",
  "decision": "switch",
  "agent_id": "00000000-0000-0000-0000-000000000001",
  "session_strategy": "compact_session"
}
```

`session_strategy` must be `new_session` or `compact_session`. A switch is
accepted only while `model_id` is still pending, and the ordinary model-switch
preflight must also pass for the selected agent.

**Response** `200 OK` for `keep`:

```json
{
  "status": "kept",
  "resolved": ["codex/gpt-5.6"],
  "message": "Current model retained; no automatic switch was performed."
}
```

**Errors:** `400` for an invalid agent, missing session strategy, or failed
model-switch preflight; `404` when the model is no longer pending; `500` when
the durable decision state cannot be read or written.

### GET /api/models/{id}

Get detailed information about a specific model.

**Response** `200 OK`:

```json
{
  "id": "llama-3.3-70b-versatile",
  "provider": "groq",
  "display_name": "Llama 3.3 70B",
  "tier": "fast",
  "context_window": 131072,
  "input_cost_per_1m": 0.59,
  "output_cost_per_1m": 0.79,
  "supports_tools": true,
  "supports_vision": false,
  "supports_streaming": true
}
```

**Response** `404 Not Found`:

```json
{
  "error": "Model 'unknown-model' not found in catalog"
}
```

### GET /api/models/aliases

List all model aliases. Aliases provide short names that resolve to full model IDs (e.g., `sonnet` resolves to `claude-sonnet-4-20250514`).

**Response** `200 OK`:

```json
{
  "aliases": {
    "sonnet": "claude-sonnet-4-20250514",
    "opus": "claude-opus-4-20250514",
    "haiku": "claude-3-5-haiku-20241022",
    "flash": "gemini-2.5-flash",
    "gpt4": "gpt-4o",
    "llama": "llama-3.3-70b-versatile",
    "deepseek": "deepseek-chat",
    "grok": "grok-4.5",
    "jamba": "jamba-1.5-large"
  },
  "total": 23
}
```

### GET /api/providers

List all known LLM providers and their authentication status. Auth status is detected by checking environment variable presence (never reads secret values).

**Response** `200 OK`:

```json
{
  "providers": [
    {
      "id": "anthropic",
      "display_name": "Anthropic",
      "auth_status": "configured",
      "api_key_env": "ANTHROPIC_API_KEY",
      "base_url": "https://api.anthropic.com",
      "key_required": true,
      "model_count": 3
    },
    {
      "id": "xai",
      "display_name": "xAI",
      "auth_status": "configured",
      "api_key_env": "XAI_API_KEY",
      "base_url": "https://api.x.ai/v1",
      "key_required": true,
      "model_count": 10,
      "auth_methods": ["api_key"],
      "oauth": {
        "bearer_recognized_by_api": true,
        "captain_login_available": false,
        "status": "external_only"
      }
    },
    {
      "id": "ollama",
      "display_name": "Ollama",
      "auth_status": "not_required",
      "api_key_env": "OLLAMA_API_KEY",
      "base_url": "http://localhost:11434",
      "key_required": false,
      "model_count": 0
    }
  ],
  "total": 20
}
```

---

## Provider Configuration Endpoints

Manage LLM provider API keys at runtime without editing config files or restarting the daemon.

### POST /api/providers/{name}/key

Set an API key for a provider. The key is stored securely and takes effect immediately.

**Request Body**:

```json
{
  "key": "sk-..."
}
```

**Response** `200 OK`:

```json
{
  "status": "saved",
  "provider": "anthropic"
}
```

### DELETE /api/providers/{name}/key

Remove the API key for a provider. Agents using this provider will fall back to the FallbackDriver or fail.

**Response** `200 OK`:

```json
{
  "status": "removed",
  "provider": "anthropic"
}
```

### POST /api/providers/{name}/test

Test provider connectivity. Most providers make a one-token completion. xAI
instead uses the non-billable `/v1/me` identity endpoint and, for API keys,
`/v1/api-key` to verify blocked/disabled state plus chat and selected-model
ACLs. An externally issued xAI OAuth bearer is identified by `/v1/me`, but
Captain does not expose an xAI OAuth login flow.

**Response** `200 OK`:

```json
{
  "status": "ok",
  "provider": "anthropic",
  "latency_ms": 245
}
```

Provider authentication failures retain HTTP `200` so every provider test has
the same result envelope:

```json
{
  "status": "error",
  "provider": "anthropic",
  "error": "Invalid API key"
}
```

An xAI success additionally returns `auth_method`, `credential_active`,
`inference_ready`, `zdr_status`, `permissions_verified`,
`chat_endpoint_allowed`, and `selected_model_allowed`. It never returns user,
team, key, or OAuth client identifiers.

---

## Native Capability Endpoints

Captain Forge compiles readable `*.captain` CapSpecs into typed native tools.
These authenticated operator endpoints manage their sources, exact-hash
approvals, revision history, and public-safe run metadata. Agent-facing
authoring is deliberately narrower: Captain may validate or propose a source,
but only an authenticated operator can approve, reject, roll back, or disable
it.

The Control web `Capabilities > Natives` view is the reference operator client
for these routes. It sends the full pending hash for every decision, requests
source only when explicitly opened, and does not ask the conversation model to
mediate an approval.

Scopes are `global`, `project`, `effective`, and `all`. `all` means global plus
the explicitly selected project, never every registered project. Mutations
require an explicit `global` or `project` scope. Project operations also
require a canonical `workspace`; reads default to `effective`, where an active
project definition overrides the global definition with the same name.

### GET /api/capabilities/native

List native capabilities. Optional query parameters are `scope` and
`workspace`. The default scope is `effective`.

```http
GET /api/capabilities/native?scope=effective&workspace=/srv/project
```

The response contains `status`, `ready`, `active_hash`, `pending_hash`, the
permission fingerprint, approval requirements, revision metadata, and the
next operator action. Runtime inputs, step arguments, outputs, and errors are
not exposed.

### GET /api/capabilities/native/{name}

Inspect one capability. Supports the same `scope` and `workspace` query
parameters. Add `include_source=true` only when the authenticated operator
explicitly needs the selected revision source; source text is omitted by
default. The `all` scope is invalid for a single-name inspection.

### POST /api/capabilities/native/validate

Compile a source without installing it or changing runtime state.

```json
{
  "name": "project-summary",
  "source": "format = 1\nname = \"project-summary\"\n..."
}
```

`name` is optional, but when present it must match the source name. A valid
response includes the source hash, permission fingerprint, input schema,
sanitized step metadata, and whether human approval would be required. Step
payloads are never echoed.

### POST /api/capabilities/native/install

Validate and durably install a global or project source.

```json
{
  "scope": "project",
  "workspace": "/srv/project",
  "name": "project-summary",
  "source": "format = 1\nname = \"project-summary\"\n..."
}
```

A first read-only revision can return `ready: true` immediately. A revision
that expands authority returns `human_action_required: true` and a
`pending_hash`; it cannot execute until that exact hash is approved.

### POST /api/capabilities/native/{name}/decision

Approve or reject the exact pending revision. A stale or mismatched hash is
rejected with `409 Conflict`.

```json
{
  "decision": "approve",
  "expected_hash": "exact-source-hash-from-the-pending-revision",
  "scope": "global"
}
```

`decision` is `approve` or `reject`. Project decisions also require
`workspace`. The audit actor is fixed by the server to `control-web`; request
bodies cannot impersonate another operator or an agent.

### POST /api/capabilities/native/{name}/rollback

Restore a known revision by exact source hash. Rollback preserves the complete
history and applies the ordinary approval boundary.

```json
{
  "target_hash": "exact-source-hash-from-revisions",
  "scope": "global"
}
```

### DELETE /api/capabilities/native/{name}

Remove the selected source and disable new runs without deleting revision or
run history. Pass `scope=global`, or `scope=project&workspace=/srv/project`.
Deleting a project override reveals the active global capability again.

### GET /api/capabilities/native/runs

List recent CapSpec runs, newest first. `limit` defaults to 100 and is clamped
to 1-500. The response exposes source hash, status, timestamps, and node states,
not decrypted runtime payloads.

### GET /api/capabilities/native/runs/{run_id}

Inspect public-safe metadata for one durable run, including each node's status,
attempt count, and current tool-use ID. Runtime inputs, outputs, errors, and the
encrypted authority snapshot remain private.

### POST /api/capabilities/native/runs/{run_id}/decision

Resolve one exact `uncertain` node without passing through the conversation
model. The caller must copy all three identity fields from the current run
projection:

```json
{
  "node_id": "publish",
  "expected_tool_use_id": "capspec-run-id:publish:2",
  "expected_attempt": 2,
  "decision": "retry"
}
```

`decision` is one of:

- `retry`, with neither `output` nor `reason`;
- `confirm_succeeded`, with the observed tool result in required field
  `output`; explicit JSON `null` is valid, but an absent field is not;
- `mark_failed`, with a non-empty `reason` and no `output`.

The run/node status, attempt, and tool-use ID are compared in the same SQLite
transaction that applies the decision. A duplicate or stale request returns
`409 Conflict`. Retry and confirmation first reload the encrypted authority
captured when the run started and intersect it with the caller agent's current
mode, grants, blocklist, environment boundary, execution policy, and lineage.
Current policy can revoke a resume but can never grant more authority than the
run originally had. An accepted retry or confirmation writes its resume intent
in the same durable transaction as the exact decision and returns immediately.
The kernel claims that intent and recovers an abandoned claim after restart;
ordinary interrupted runs have no such intent and are not auto-resumed.
`mark_failed` terminates the run without invoking a tool. The audit actor is
fixed to `control-web` by the server.

All request bodies reject unknown fields. Common errors are `400` for invalid
source/scope/workspace, `404` for an unknown capability, revision, or run,
`409` for an exact-hash conflict, and `500` for a durable storage or reload
failure.

---

## Skills Endpoints

Manage installed, bundled, and generated local skills. Remote marketplace
search and installation are deliberately absent from the active API because
publisher-backed integrity is unavailable. The former remote paths return
`404`; local installation is an operator CLI action.

### GET /api/skills

List all installed skills.

**Response** `200 OK`:

```json
{
  "skills": [
    {
      "name": "github",
      "version": "1.0.0",
      "runtime": "prompt_only",
      "description": "GitHub integration for issues, PRs, and repos",
      "bundled": true
    },
    {
      "name": "docker",
      "version": "1.0.0",
      "runtime": "prompt_only",
      "description": "Docker container management",
      "bundled": true
    }
  ],
  "total": 60
}
```

### POST /api/skills/uninstall

Remove an installed skill. Bundled skills cannot be uninstalled.

**Request Body**:

```json
{
  "name": "my-custom-skill"
}
```

**Response** `200 OK`:

```json
{
  "status": "uninstalled",
  "skill": "my-custom-skill"
}
```

### POST /api/skills/create

Create a new skill from a template.

**Request Body**:

```json
{
  "name": "my-skill",
  "runtime": "python",
  "description": "A custom skill"
}
```

**Response** `201 Created`:

```json
{
  "status": "created",
  "skill": "my-skill",
  "path": "/home/user/.captain/skills/my-skill"
}
```

## MCP Protocol Endpoints

Captain exposes Model Context Protocol (MCP) for external tool-server
interoperability. Frozen A2A compatibility routes are intentionally omitted
from the active public API guide.

### GET /api/mcp/servers

List configured and connected MCP servers with their available tools.

**Response** `200 OK`:

```json
{
  "servers": [
    {
      "name": "filesystem",
      "transport": "stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem"],
      "connected": true,
      "tools": [
        {
          "name": "mcp_filesystem_read_file",
          "description": "Read a file from the filesystem"
        },
        {
          "name": "mcp_filesystem_write_file",
          "description": "Write content to a file"
        }
      ]
    }
  ],
  "total": 1
}
```

### POST /mcp

MCP HTTP transport endpoint. Accepts JSON-RPC 2.0 requests and exposes Captain tools via the MCP protocol to external clients.

**Request Body** (JSON-RPC 2.0):

```json
{
  "jsonrpc": "2.0",
  "method": "tools/list",
  "id": 1
}
```

**Response** `200 OK`:

```json
{
  "jsonrpc": "2.0",
  "result": {
    "tools": [
      {
        "name": "file_read",
        "description": "Read a file's contents",
        "inputSchema": {
          "type": "object",
          "properties": {
            "path": {"type": "string"}
          }
        }
      }
    ]
  },
  "id": 1
}
```

## Audit & Security Endpoints

Captain maintains a linear, versioned SHA-256 hash chain for security-relevant
operations. Version-2 entries length-prefix every field. If the active epoch is
corrupt at boot, Captain seals it as invalid and opens an anchored recovery
epoch without rewriting historical entries.

### GET /api/audit/recent

Retrieve recent entries and the current integrity state.

**Query Parameters:**
- `n` or `limit` (optional): Number of entries (default: 50, max: 1000)

**Response** `200 OK`:

```json
{
  "entries": [
    {
      "seq": 1042,
      "epoch": 0,
      "hash_version": 2,
      "timestamp": "2026-07-29T10:30:00Z",
      "agent_id": "a1b2c3d4-...",
      "action": "AgentSpawn",
      "detail": "Agent created",
      "outcome": "ok",
      "prev_hash": "f6e5d4c3b2a1...",
      "hash": "a1b2c3d4e5f6..."
    }
  ],
  "total": 1043,
  "tip_hash": "a1b2c3d4e5f6...",
  "integrity": {
    "valid": true,
    "status": "healthy",
    "active_epoch": 0,
    "active_epoch_valid": true,
    "invalid_epochs": [],
    "entry_count": 1043,
    "tip_hash": "a1b2c3d4e5f6..."
  }
}
```

### GET /api/audit/verify

Verify the active epoch and report any sealed invalid history or runtime append
failure.

**Response** `200 OK`:

```json
{
  "valid": true,
  "status": "healthy",
  "message": "Audit hash chain verified in epoch 0",
  "entries": 1043,
  "tip_hash": "a1b2c3d4e5f6...",
  "active_epoch": 0,
  "active_epoch_valid": true,
  "invalid_epochs": []
}
```

A degraded response keeps `active_epoch_valid: true` when recovery succeeded,
lists the sealed invalid epochs, and includes a redacted `error`. There is no
repair endpoint: Captain never re-hashes altered historical content.

### GET /api/security

Security status overview showing the state of runtime security systems.

**Response** `200 OK`:

```json
{
  "core_protections": {
    "path_traversal": true,
    "ssrf_protection": true,
    "capability_system": true,
    "subprocess_environment_scrub": true,
    "subprocess_os_isolation": false
  },
  "execution": {
    "profile": "remote_operator",
    "backend": "host_process",
    "isolation_level": "environment_scrub",
    "os_isolation": false,
    "environment_scrub": true,
    "dangerous_command_guard": "normalized_lexical_heuristic",
    "configured_policy_mode": "full",
    "policy_mode": "allowlist",
    "critical_mode": "safe",
    "host_execution_allowed": true,
    "isolation_routing": "explicit_only",
    "explicit_isolation_backends": ["docker_exec", "wasm_agent"],
    "docker": {
      "enabled": true,
      "routing": "explicit_only",
      "runtime_availability": "checked_on_invocation",
      "network": "none",
      "read_only_root": true,
      "capabilities_dropped": true,
      "workspace_mount": "read_only",
      "untrusted_profile_ready": false,
      "violations": []
    }
  },
  "monitoring": {
    "audit_trail": {
      "enabled": true,
      "algorithm": "versioned SHA-256 hash chain",
      "entry_count": 1043,
      "integrity": "healthy",
      "active_epoch": 0,
      "active_epoch_valid": true,
      "invalid_epochs": []
    }
  },
  "secret_zeroization": true,
  "total_features": 16
}
```

The `execution` object is also present in authenticated
`GET /api/health/detail` and `GET /api/status`. `configured_policy_mode` is
operator intent; `policy_mode` is the profile-constrained effective mode.
`runtime_availability = "checked_on_invocation"` does not claim the Docker
daemon is currently reachable. `environment_scrub` is a host-process
protection, not an operating-system isolation claim, and explicit isolation
never falls back to the host.

---

## Usage & Analytics Endpoints

Track persisted token usage, costs, model utilization, Captain-owned guards,
and separately observed provider-owned subscription limits.

### GET /api/usage

Get lifetime scheduler counters for each registered agent.

**Response** `200 OK`:

```json
{
  "agents": [
    {
      "agent_id": "792a2b4e-20bd-495f-bcfa-819818c15911",
      "name": "captain",
      "total_tokens": 212000,
      "tool_calls": 18
    }
  ]
}
```

### GET /api/usage/summary

Get the persisted aggregate usage summary.

**Response** `200 OK`:

```json
{
  "total_input_tokens": 125000,
  "total_output_tokens": 87000,
  "total_cost_usd": 0.42,
  "call_count": 156,
  "total_tool_calls": 18
}
```

### GET /api/usage/by-model

Get usage breakdown by model.

**Response** `200 OK`:

```json
{
  "models": [
    {
      "model": "llama-3.3-70b-versatile",
      "total_cost_usd": 0.09,
      "total_input_tokens": 80000,
      "total_output_tokens": 55000,
      "call_count": 120
    }
  ]
}
```

### GET /api/usage/daily

Get the last seven daily usage buckets plus today's cost and the first stored
event date.

```json
{
  "days": [
    {"date": "2026-07-18", "cost_usd": 0.42, "tokens": 212000, "calls": 156}
  ],
  "today_cost_usd": 0.42,
  "first_event_date": "2026-07-01"
}
```

### GET /api/budget

Return Captain's global cost-budget snapshot and the latest persisted
provider-subscription observations. The top-level cost fields are
`hourly_spend`, `hourly_limit`, `hourly_pct`, `daily_*`, `monthly_*`,
`alert_threshold`, and `default_max_llm_tokens_per_hour`.

`provider_subscriptions` has stable states `unavailable`, `stale`, `ok`,
`warning`, `critical`, or `exhausted`. Each item carries the provider's limit
family, primary/secondary windows, optional plan/credits, observation source,
age, and alert level. Window durations and reset timestamps are live
provider-reported values. `used_percent` remains the official observation;
`remaining_percent` on rolling windows is its explicit derived complement and
is labelled `derived_from_provider_used_percent`. Codex `spend_control` keeps
the provider-reported limit, used amount, remaining amount, percentages, and
reset without converting them into a guessed subscription allowance.

The nested `reset_notifications` object reports the durable reset-card queue
without exposing its payload. Its counters are `pending`, `delivering`,
`retry_wait`, `delivered`, `suppressed`, `dead`, and `uncertain`; its derived
state is `ok`, `active`, `attention`, or `unavailable`. Reset detection requires
an advanced provider reset identity plus reported capacity replenishment. It is
not derived from a local countdown.

Ratatui Chat, the xterm Web terminal, Control web, and its retained desktop
compatibility wrapper poll this authenticated local endpoint every five
seconds while visible. Compact Chat bands name the active model, render gauges
for provider-wide windows and matching model-specific families, and summarize
other families as outside the active model. Status and Budget remain exhaustive
and render every supplied primary/secondary window independently. All surfaces
preserve the last valid observation across a transient daemon error. This UI
cadence does not call the provider; provider refresh remains daemon-owned and
persisted.

### PUT /api/budget

Update any supplied global fields: `max_hourly_usd`, `max_daily_usd`,
`max_monthly_usd`, `alert_threshold`, or
`default_max_llm_tokens_per_hour`. The body must be an object containing at
least one recognized field. Cost values must be finite, non-negative, and no
greater than `1_000_000_000`; `alert_threshold` must be in `[0,1]`; token
limits must fit a signed TOML integer. Invalid values return `400` rather than
being clamped.

The authenticated route serializes updates, validates the complete candidate,
atomically persists that exact snapshot to `config.toml`, and only then
publishes it to the live runtime. A write failure returns `500` and leaves the
previous live snapshot in force. Readers therefore observe either the complete
old budget or the complete new budget, never a field-by-field intermediate
state.

### GET /api/budget/agents

Return per-agent cost ranking plus durable rolling token usage. Each row
includes `tokens_used`, `tokens_reset_at`, and
`max_llm_tokens_per_hour`; restarting the daemon does not reset this ledger.

### GET /api/budget/agents/{id}

Return one agent's hourly/daily/monthly spend and its `tokens` object:

```json
{
  "agent_id": "792a2b4e-20bd-495f-bcfa-819818c15911",
  "agent_name": "captain",
  "tokens": {
    "used": 228733,
    "limit": 200000,
    "window_seconds": 3600,
    "resets_at": "2026-07-18T12:00:00Z",
    "pct": 1.143665
  }
}
```

### PUT /api/budget/agents/{id}

Update at least one of `max_cost_per_hour_usd`, `max_cost_per_day_usd`,
`max_cost_per_month_usd`, or `max_llm_tokens_per_hour`. Setting the token
limit to `0` is an explicit opt-out from Captain's internal token guard; it
does not change the provider subscription allowance. The same finite,
non-negative, upper-bound, and TOML-integer validation applies. A successful
token-limit update is published immediately to the scheduler used by the next
turn.

---

## Session Management Endpoints

### POST /api/agents/{id}/session/reset

Create and activate a fresh session for the agent. The previous session and
its transcript remain persisted and reopenable in the shared session catalog;
only an explicit history deletion is destructive.

**Response** `200 OK`:

```json
{
  "status": "reset",
  "agent_id": "a1b2c3d4-...",
  "new_session_id": "s5e6f7g8-..."
}
```

### POST /api/agents/{id}/session/compact

Trigger LLM-based session compaction. The agent's conversation is summarized by
an LLM, keeping only the most recent messages plus a generated summary. The
operation is serialized with other turns for that agent. Progress is available
on the agent WebSocket and daemon event stream using the canonical persisted
session ID.

**Response** `200 OK`:

```json
{
  "status": "compacted",
  "message": "Session compacted: 80 messages summarized, 20 kept"
}
```

**Response** `200 OK` (no compaction needed):

```json
{
  "status": "ok",
  "message": "Session does not need compaction (below threshold)"
}
```

### POST /api/agents/{id}/stop

Cancel the agent's current LLM run. Aborts any in-progress generation.

**Response** `200 OK`:

```json
{
  "status": "stopped",
  "message": "Agent run cancelled"
}
```

### PUT /api/agents/{id}/model

Switch an agent's LLM model at runtime.

**Request Body**:

```json
{
  "model": "claude-sonnet-4-20250514"
}
```

**Response** `200 OK`:

```json
{
  "status": "updated",
  "model": "claude-sonnet-4-20250514"
}
```

---

### GET /api/agents/{id}/reasoning

Read the agent's durable reasoning selection and the effective value for its
current model. `configured_effort: null` means **Auto**: Captain omits the
provider field instead of inventing a default. Auto is not the explicit
`none` effort. `source` identifies whether the effective value comes from an
agent override, the live model catalogue, or the provider default.

**Response** `200 OK`:

```json
{
  "provider": "codex",
  "model": "gpt-5.6-sol",
  "supported": true,
  "configured_effort": null,
  "effective_effort": "low",
  "source": "model_default",
  "override_valid": true,
  "options": [
    {"effort": "low", "description": null},
    {"effort": "medium", "description": null},
    {"effort": "high", "description": null},
    {"effort": "xhigh", "description": null},
    {"effort": "max", "description": null},
    {"effort": "ultra", "description": null}
  ],
  "reported_by_provider": true
}
```

### PUT /api/agents/{id}/reasoning

Persist one effort for the agent. The value must be one of the current model
catalogue options. The persisted value is open rather than a frozen Captain
enum, so new catalogue levels remain forward-compatible. Send `null` (or use
`auto` in an operator UI) to remove the override and return control to the
model/provider. An explicit `none`, when advertised, remains distinct from
Auto.

Codex `ultra` is a product-level mode rather than a wire-level API effort.
Captain preserves `ultra` in the agent configuration and status, sends `max`
to the Codex response endpoint, and enables bounded proactive delegation only
for the root agent. A sub-agent never inherits the proactive policy
recursively.

```json
{"effort": "ultra"}
```

The response is the same complete status object as `GET`. Reset example:

```json
{"effort": null}
```

---

## WebSocket Protocol

### Connecting

```
POST /api/auth/realtime-ticket
{"path":"/api/agents/{id}/ws"}

GET /api/agents/{id}/ws?ticket={single-use-ticket}
```

Browser clients mint the ticket with their authenticated HttpOnly session
cookie. It expires after 30 seconds and is consumed by the first upgrade
attempt. Technical clients that can set headers may use the normal
`Authorization` or `X-API-Key` header instead. A `token` query parameter never
authenticates the request. A valid request upgrades to a real-time
bidirectional agent chat; invalid agent IDs return `400`, missing agents
return `404`.

### Message Format

All messages are JSON-encoded strings.

### Client to Server

**Send a message:**

```json
{
  "type": "message",
  "content": "What is the weather like?"
}
```

Plain text (non-JSON) is also accepted and treated as a message.

**Command messages** use the explicit protocol shape below. Ratatui turns its
local slash syntax into the same kernel controls; Control web and Desktop also
expose reasoning as a direct selector backed by the REST endpoint above.

```json
{"type": "command", "command": "reasoning", "args": "high"}
```

| Command | Description |
|---------|-------------|
| `/new` | Start a new session; the previous session remains available in history |
| `/compact` | Trigger LLM session compaction |
| `/model <name>` | Switch the agent's model |
| `/stop` | Cancel current LLM run |
| `/usage` | Show token usage and cost |
| `/reasoning [auto\|level]` | Inspect or persist the real model reasoning effort |
| `/think` | Terminal-only visibility toggle for reasoning blocks already received; it does not change the model |
| `/context` | Show context-window pressure |
| `/verbose [off\|on\|full]` | Select streamed tool detail |
| `/queue` | Show whether the agent is processing |
| `/budget` | Show Captain's internal cost budget |

**Ping:**

```json
{
  "type": "ping"
}
```

### Server to Client

**Connection confirmed** (sent immediately on connect):

```json
{
  "type": "connected",
  "agent_id": "a1b2c3d4-..."
}
```

**Thinking indicator** (sent when agent starts processing):

```json
{
  "type": "thinking"
}
```

**Delivery verification phase** (ephemeral; never added to chat history):

```json
{
  "type": "phase",
  "phase": "verifying",
  "detail": null
}
```

The phase is one of `verifying`, `correcting`, `verification_verified`, or
`verification_incomplete`. A client should replace its current transient
status rather than append a message. `verification_verified` clears a fast
verification silently; `correcting` and `verification_incomplete` may be made
visible. These events contain no tool input, result, path, model draft, or
hidden reasoning.

**Text delta** (streaming token, sent as the LLM generates output):

```json
{
  "type": "text_delta",
  "content": "The weather"
}
```

**Suggested replies** (non-blocking choices for the next user message):

```json
{
  "type": "suggested_replies",
  "options": ["Concise", "Detailed"]
}
```

Clients may render these as clickable choices, but must keep normal free-text
input available. Selecting a choice sends the standard client message shape
(`{"type":"message","content":"..."}`), not `user_response`. An empty
`options` array clears previous suggestions. This event is distinct from
`ask_user`, which blocks an active tool call and expects `user_response`.

**Tool use started** (sent when the agent invokes a tool):

```json
{
  "type": "tool_start",
  "tool": "web_fetch"
}
```

**Complete response** (sent when agent finishes, contains final aggregated response):

```json
{
  "type": "response",
  "content": "The weather today is sunny with a high of 72F.",
  "input_tokens": 245,
  "output_tokens": 32,
  "iterations": 2,
  "cost_usd": 0.0012
}
```

**Error:**

```json
{
  "type": "error",
  "content": "Agent not found"
}
```

**Agent list update** (sent every 5 seconds with current agent states):

```json
{
  "type": "agents_updated",
  "agents": [
    {
      "id": "a1b2c3d4-...",
      "name": "hello-world",
      "state": "Running",
      "model_provider": "groq",
      "model_name": "llama-3.3-70b-versatile"
    }
  ]
}
```

**Compaction progress:**

```json
{
  "type": "compaction_progress",
  "progress": {
    "schema_version": 1,
    "operation_id": "7ecfd49c-...",
    "runtime_instance_id": "dc50a22b-...",
    "agent_id": "a1b2c3d4-...",
    "session_id": "3a1e6f4c-...",
    "phase": "chunking",
    "state": "running",
    "detail": "Processed chunk 2 of 4",
    "message_count": 180,
    "estimated_tokens": 42000,
    "context_window_tokens": 200000,
    "completed_units": 2,
    "total_units": 4,
    "unit": "chunks",
    "started_at_ms": 1784770000000,
    "updated_at_ms": 1784770001200
  }
}
```

The wire contract has no percentage field. Derive one only when both unit
counts are present and `total_units > 0`. During an opaque model call all three
unit fields are `null`; render indeterminate progress. Terminal states are
`succeeded`, `failed`, or `interrupted`.

**Pong** (response to ping):

```json
{
  "type": "pong"
}
```

### Connection Lifecycle

1. Client connects to `ws://host:port/api/agents/{id}/ws`.
2. Server sends `{"type": "connected"}`.
3. Client sends `{"type": "message", "content": "..."}`.
4. Server sends `{"type": "thinking"}`, then zero or more `{"type": "text_delta"}` events, then `{"type": "response"}`.
5. Server periodically sends `{"type": "agents_updated"}` every 5 seconds.
6. Client sends a Close frame or disconnects to end the session.

---

## SSE Streaming

### POST /api/agents/{id}/message/stream

Send a message and receive the response as a Server-Sent Events stream. This enables real-time token-by-token streaming.

**Request Body** (JSON):

```json
{
  "message": "Explain quantum computing",
  "session_id": "3a1e6f4c-06ad-4bd4-9c79-c1e2fbf39d0d"
}
```

The optional `session_id` has the same isolated-session semantics as the
non-streaming message endpoint. `POST /api/agents/{id}/message/answer` accepts
the same `session_id` alongside `content`, so simultaneous pending questions
for different sessions cannot consume one another's answer channel.

**SSE Event Stream:**

```
event: chunk
data: {"content":"Quantum","done":false}

event: chunk
data: {"content":" computing","done":false}

event: chunk
data: {"content":" is a type","done":false}

event: tool_use
data: {"tool":"web_search"}

event: tool_result
data: {"tool":"web_search","input":{"query":"quantum computing basics"}}

event: suggested_replies
data: {"type":"suggested_replies","options":["Concise","Detailed"]}

event: done
data: {"done":true,"usage":{"input_tokens":150,"output_tokens":340}}
```

### SSE Event Types

| Event Name | Description |
|------------|-------------|
| `chunk` | Text delta from the LLM. `"done": false` indicates more tokens are coming. |
| `tool_use` | The agent is invoking a tool. Contains the tool name. |
| `tool_result` | A tool invocation has completed. Contains the tool name and input. |
| `suggested_replies` | Non-blocking choices. Send a selected value as a normal message; an empty list clears prior choices. |
| `ask_user` | Blocking operator question. Answer it through the session-scoped message answer endpoint. |
| `done` | Final event. Contains `"done": true` and token usage statistics. |

---

### GET /api/memory/events

Open the authenticated daemon-wide Server-Sent Events stream used by live
operator surfaces. Context compaction is emitted as a named event whose data is
the `progress` object shown in the WebSocket example above:

```text
event: compaction_progress
data: {"schema_version":1,"operation_id":"7ecfd49c-...","phase":"summarizing","state":"running","completed_units":null,"total_units":null,"unit":null}
```

Filter by `agent_id` and `session_id` before rendering. Repeated updates for the
same `operation_id` replace one live operation; they are not independent jobs.
The persisted session timeline remains the recovery source after reconnect or
restart.

---

## OpenAI-Compatible API

Captain exposes an OpenAI-compatible API for drop-in integration with tools that support the OpenAI API format (Cursor, Continue, Open WebUI, etc.).

### POST /v1/chat/completions

Send a chat completion request using the OpenAI message format.

**Request Body**:

```json
{
  "model": "captain:coder",
  "messages": [
    {"role": "system", "content": "You are a helpful assistant."},
    {"role": "user", "content": "Hello!"}
  ],
  "stream": false,
  "temperature": 0.7,
  "max_tokens": 1024
}
```

**Model resolution** (the `model` field maps to an Captain agent):

| Format | Example | Behavior |
|--------|---------|----------|
| `captain:<name>` | `captain:coder` | Find agent by name |
| UUID | `a1b2c3d4-...` | Find agent by ID |
| Plain string | `coder` | Try as agent name |
| Any other | `gpt-4o` | Falls back to first registered agent |

**Image support** --- messages can include image content parts:

```json
{
  "model": "captain:analyst",
  "messages": [
    {
      "role": "user",
      "content": [
        {"type": "text", "text": "Describe this image"},
        {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBOR..."}}
      ]
    }
  ]
}
```

**Response (non-streaming)** `200 OK`:

```json
{
  "id": "chatcmpl-a1b2c3d4-...",
  "object": "chat.completion",
  "created": 1708617600,
  "model": "coder",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "Hello! How can I help you today?"
      },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 25,
    "completion_tokens": 12,
    "total_tokens": 37
  }
}
```

**Streaming** --- Set `"stream": true` for SSE:

```
data: {"id":"chatcmpl-...","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}

data: {"id":"chatcmpl-...","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"!"},"finish_reason":null}]}

data: {"id":"chatcmpl-...","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":25,"completion_tokens":12,"total_tokens":37}}

data: [DONE]
```

### GET /v1/models

List available models (agents) in OpenAI format.

**Response** `200 OK`:

```json
{
  "object": "list",
  "data": [
    {
      "id": "captain:coder",
      "object": "model",
      "created": 1708617600,
      "owned_by": "captain"
    },
    {
      "id": "captain:researcher",
      "object": "model",
      "created": 1708617600,
      "owned_by": "captain"
    }
  ]
}
```

---

## Error Responses

All error responses use a consistent JSON format:

```json
{
  "error": "Description of what went wrong"
}
```

### HTTP Status Codes

| Code | Meaning |
|------|---------|
| `200` | Success |
| `201` | Created (spawn agent, create workflow, create trigger, install skill) |
| `400` | Bad request (invalid UUID, missing required fields, malformed TOML/JSON) |
| `401` | Unauthorized (missing or invalid `Authorization: Bearer` header) |
| `404` | Not found (agent, workflow, trigger, template, model, skill, or KV key does not exist) |
| `429` | GCRA request throttling, Captain internal quota, or provider subscription quota; inspect `code`, `quota.scope`, and `Retry-After` |
| `500` | Internal server error (agent loop failure, database error, driver error) |

### Request IDs

Every response includes an `x-request-id` header with a UUID for tracing:

```
x-request-id: 550e8400-e29b-41d4-a716-446655440000
```

Use this value when reporting issues or correlating requests in logs.

### Security Headers

Every response includes security headers:

| Header | Value |
|--------|-------|
| `Content-Security-Policy` | Same-origin scripts only on normal responses; exact artifact preview/download routes preserve their stricter sandbox policy |
| `X-Frame-Options` | `DENY`, except `SAMEORIGIN` for the exact sandboxed artifact preview route |
| `X-Content-Type-Options` | `nosniff` |
| `X-XSS-Protection` | `0` (legacy mutation filter disabled; CSP and sanitization are authoritative) |
| `Strict-Transport-Security` | `max-age=63072000; includeSubDomains` |
| `X-Request-Id` | Unique UUID per request |

### Rate Limiting

The GCRA (Generic Cell Rate Algorithm) rate limiter provides cost-aware token
bucket throttling with per-IP tracking and automatic stale entry cleanup.
Different endpoints consume different token costs. That transport-level limit
returns a simple `429`:

```
HTTP/1.1 429 Too Many Requests
Retry-After: 60

{"error": "Rate limit exceeded"}
```

The `Retry-After` header indicates the remaining wait in seconds.

Agent message endpoints also use `429` for enforced resource quotas, but return
the structured `code` and `quota` object shown above. Captain-owned scopes and
`provider_subscription` are deliberately distinct; clients must not assume
that changing a Captain budget can reset an allowance owned by Codex.

---

## Endpoint Summary

Operator-maintained endpoint summary. Treat the running daemon and route tests
as source of truth when validating exact route availability.

| Method | Path | Description |
|--------|------|-------------|
| **System** | | |
| GET | `/` | Authenticated six-hub Control web UI |
| GET | `/api/health` | Health check (no auth, redacted) |
| GET | `/api/health/detail` | Full health check (auth required) |
| GET | `/api/status` | Kernel status |
| GET | `/api/version` | Version info |
| POST | `/api/shutdown` | Graceful shutdown |
| GET | `/api/profiles` | List agent profiles |
| GET | `/api/tools` | List available tools |
| GET | `/api/config` | Configuration (secrets redacted) |
| GET | `/api/peers` | List OFP wire peers |
| **Live Runs** | | |
| GET | `/api/tool-runs` | List selective metadata for recent tool runs |
| GET | `/api/tool-runs/{run_id}` | Inspect one run without raw input or result |
| GET | `/api/tool-runs/{run_id}/tail` | Read a sanitized tail capped at 200 lines and 32 KiB |
| POST | `/api/tool-runs/{run_id}/cancel` | Audit and cancel one active actually cancellable run |
| **Artifacts** | | |
| GET | `/api/artifacts` | List latest immutable artifact versions and store health |
| GET | `/api/artifacts/{id}` | Inspect one latest or exact version |
| GET | `/api/artifacts/{id}/versions` | List immutable versions newest-first |
| GET | `/api/artifacts/{id}/versions/{version}/preview` | Preview verified passive content in a browser sandbox |
| GET | `/api/artifacts/{id}/versions/{version}/download` | Download exact checksum-verified bytes |
| **Agents** | | |
| GET | `/api/agents` | List agents |
| POST | `/api/agents` | Spawn agent |
| GET | `/api/agents/{id}` | Get agent details |
| PUT | `/api/agents/{id}/update` | Update agent config |
| PUT | `/api/agents/{id}/mode` | Set agent mode (Stable/Normal) |
| GET | `/api/agents/{id}/reasoning` | Read configured and effective reasoning |
| PUT | `/api/agents/{id}/reasoning` | Set reasoning effort or reset to Auto |
| DELETE | `/api/agents/{id}` | Kill agent |
| POST | `/api/agents/{id}/message` | Send message (blocking) |
| POST | `/api/agents/{id}/message/stream` | Send message (SSE stream) |
| GET | `/api/agents/{id}/api` | Inspect per-agent external API status |
| GET | `/api/agents/{id}/api/manifest` | Get per-agent external integration contract |
| POST | `/api/agents/{id}/api/token/rotate` | Rotate/generate ingress bearer token |
| POST | `/hooks/agents/{id}/ingress` | External REST/webhook ingress for one agent turn |
| POST | `/api/agents/{id}/api/egress/configure` | Configure signed callback egress |
| POST | `/api/agents/{id}/api/egress/test` | Send signed callback diagnostic |
| GET | `/api/agents/{id}/api/events` | Inspect agent API audit events |
| GET | `/api/agents/{id}/api/egress` | Inspect callback queue/dead letters |
| POST | `/api/agents/{id}/api/egress/{queue_id}/retry` | Retry one callback delivery |
| GET | `/api/agents/{id}/session` | Get conversation history |
| GET | `/api/agents/{id}/ws` | WebSocket chat |
| POST | `/api/agents/{id}/session/reset` | Reset session |
| POST | `/api/agents/{id}/session/compact` | LLM-based compaction |
| POST | `/api/agents/{id}/stop` | Cancel current run |
| PUT | `/api/agents/{id}/model` | Switch model |
| **Workflows** | | |
| GET | `/api/workflows` | List workflows |
| POST | `/api/workflows` | Create workflow |
| GET | `/api/workflows/{id}` | Get workflow definition |
| PUT | `/api/workflows/{id}` | Replace workflow definition |
| DELETE | `/api/workflows/{id}` | Remove workflow definition |
| POST | `/api/workflows/{id}/run` | Run workflow |
| GET | `/api/workflows/{id}/runs` | List scoped runs newest-first |
| **Triggers** | | |
| GET | `/api/triggers` | List triggers |
| POST | `/api/triggers` | Create trigger |
| PUT | `/api/triggers/{id}` | Update trigger |
| DELETE | `/api/triggers/{id}` | Delete trigger |
| **Memory** | | |
| GET | `/api/memory/agents/{id}/kv` | List KV pairs |
| GET | `/api/memory/agents/{id}/kv/{key}` | Get KV value |
| PUT | `/api/memory/agents/{id}/kv/{key}` | Set KV value |
| DELETE | `/api/memory/agents/{id}/kv/{key}` | Delete KV value |
| **Channels** | | |
| GET | `/api/channels` | List active channels and frozen compatibility status |
| POST | `/api/channels/{name}/configure` | Configure or patch one active channel |
| DELETE | `/api/channels/{name}/configure` | Remove channel config and local credentials |
| POST | `/api/channels/{name}/test` | Run a live transport probe or test delivery |
| POST | `/api/channels/reload` | Reload channel config from disk |
| **Templates** | | |
| GET | `/api/templates` | List templates |
| GET | `/api/templates/{name}` | Get template |
| **Sessions** | | |
| GET | `/api/sessions` | List sessions |
| DELETE | `/api/sessions/{id}` | Delete session |
| **Model Catalog** | | |
| GET | `/api/models` | Full runtime model catalog |
| GET | `/api/models/updates` | Durable Codex catalog additions awaiting a decision |
| POST | `/api/models/updates/decision` | Keep the current Codex model or apply an explicit safe switch |
| GET | `/api/models/{id}` | Model details |
| GET | `/api/models/aliases` | List model aliases |
| GET | `/api/providers` | Provider list with auth status |
| **Usage and budgets** | | |
| GET | `/api/usage` | Per-agent lifetime scheduler counters |
| GET | `/api/usage/summary` | Persisted aggregate usage |
| GET | `/api/usage/by-model` | Persisted usage grouped by model |
| GET | `/api/usage/daily` | Seven-day usage buckets |
| GET | `/api/budget` | Global Captain budget and provider-reported subscriptions |
| PUT | `/api/budget` | Update global Captain budget limits |
| GET | `/api/budget/agents` | Per-agent cost and rolling token ranking |
| GET, PUT | `/api/budget/agents/{id}` | Inspect or update one agent's budget |
| **Provider Config** | | |
| POST | `/api/providers/{name}/key` | Set provider API key |
| DELETE | `/api/providers/{name}/key` | Remove provider API key |
| POST | `/api/providers/{name}/test` | Test provider connectivity |
| **Native Capabilities** | | |
| GET | `/api/capabilities/native` | List effective, global, project, or all CapSpecs |
| GET | `/api/capabilities/native/{name}` | Inspect one CapSpec and its revision metadata |
| POST | `/api/capabilities/native/validate` | Compile a CapSpec without installing it |
| POST | `/api/capabilities/native/install` | Durably install or propose a CapSpec revision |
| POST | `/api/capabilities/native/{name}/decision` | Approve or reject the exact pending hash |
| POST | `/api/capabilities/native/{name}/rollback` | Restore a known revision by exact hash |
| DELETE | `/api/capabilities/native/{name}` | Disable new runs while retaining history |
| GET | `/api/capabilities/native/runs` | List public-safe durable run metadata |
| GET | `/api/capabilities/native/runs/{run_id}` | Inspect one public-safe durable run |
| POST | `/api/capabilities/native/runs/{run_id}/decision` | Resolve an exact uncertain node and resume or fail the run |
| **Skills** | | |
| GET | `/api/skills` | List installed/bundled/generated skills |
| POST | `/api/skills/uninstall` | Uninstall skill |
| POST | `/api/skills/create` | Create new skill |
| **MCP** | | |
| GET | `/api/mcp/servers` | MCP server connections |
| POST | `/mcp` | MCP HTTP transport (JSON-RPC 2.0) |
| **Audit & Security** | | |
| GET | `/api/audit/recent` | Recent audit logs |
| GET | `/api/audit/verify` | Verify hash-chain and epoch integrity |
| GET | `/api/security` | Security status |
| **Usage & Analytics** | | |
| GET | `/api/usage` | Usage statistics |
| GET | `/api/usage/summary` | Usage summary with quota |
| GET | `/api/usage/by-model` | Usage by model breakdown |
| **OpenAI Compatible** | | |
| POST | `/v1/chat/completions` | OpenAI-compatible chat |
| GET | `/v1/models` | OpenAI-compatible model list |
