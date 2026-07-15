# Agents

Captain can create persistent specialized agents from installed templates or
from a TOML manifest. The installed catalog is discovered at runtime, so this
guide intentionally does not pin a template count, provider, or model list.

## Create an Agent

Use the interactive catalog when an installed template already matches the
role:

```bash
captain agent new
captain agent new <template>
```

Use a manifest when the role, model, tools, or budget must be explicit:

```bash
captain agent spawn ./agents/veille-technologique/agent.toml
```

Canonical minimal manifest:

```toml
name = "veille-technologique"
description = "Agent specialise dans la veille technologique."
module = "builtin:chat"
tool_allowlist = ["web_research_batch", "web_fetch", "memory_recall", "memory_save"]

[model]
provider = "codex"
model = "gpt-5.5"
system_prompt = "Utilise des sources reelles, cite-les et signale les incertitudes."
```

`model` is a TOML table, not a string. These forms are invalid:

```toml
model = "codex:gpt-5.5"
model = "gpt-5.5"
```

Use a top-level `tool_allowlist` or `[capabilities] tools = [...]`. Do not use
`[tools] allow = [...]`: `tools` is a map of per-tool configuration. Every
child agent must have an explicit, non-wildcard tool allowlist. Captain adds
only the minimal discovery tools automatically.

## Creation Contract

In daemon mode, successful creation is persistent and follows
`agent-as-service.v1`. Captain returns:

- the agent ID, name, state, provider, and model;
- effective capability and budget information;
- an ingress URL and bearer token, returned once;
- manifest, event, egress configuration, and egress test URLs;
- a strict readiness state and any remaining operator action.

`ingress_ready` means external callers can send work to the agent. `ready`
means both ingress and signed outbound callbacks are configured. Captain cannot
infer the callback URL of an external system. To make the agent fully in/out
ready at creation, provide `agent_api.egress_callback_url` and its callback
secret in the creation request or manifest configuration.

Inspect the current integration sheet at any time:

```bash
captain agent api <agent>
captain agent api <agent> --manifest
captain agent api <agent> --rotate-token
```

Token rotation prints a new secret once. Store it as a secret and do not place
it in source control, logs, or chat transcripts.

## External Ingress and Egress

An external service calls the dedicated ingress endpoint; no custom bridge is
required:

```bash
curl -sS -X POST "$CAPTAIN_URL/hooks/agents/$AGENT_ID/ingress" \
  -H "Authorization: Bearer $CAPTAIN_AGENT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "request_id": "watch-001",
    "message": "Prepare a sourced technology watch.",
    "sender_id": "external-service"
  }'
```

Configure and test outbound delivery through the URLs returned by
`captain agent api <agent> --manifest`. Callbacks are HMAC-SHA256 signed and
failed deliveries remain observable and retryable in the agent egress queue.

For the complete HTTP payloads and signature contract, see
[API Reference](api-reference.md#agent-as-service-api).

## Operate an Agent

```bash
captain agent list
captain agent caps <agent>
captain agent chat <agent-id>
captain agent api <agent> --manifest
captain agent kill <agent-id>
```

Use `captain agent caps` before expanding an agent's tools or budget. Prefer a
small allowlist and a bounded quota that matches the role. A disposable worker
can be tagged `ephemeral`; Captain may reap an inactive ephemeral agent, while
long-lived service agents must not use that tag.

Inside Captain, use `agent_delegate` for a bounded task and `agent_send` for a
direct synchronous message. Parallel delegation is appropriate only for
independent work. If one task needs another task's result, run them in
dependency order or model the dependency in a workflow.

## Troubleshooting

```bash
captain agent list --json
captain agent caps <agent> --json
captain agent api <agent> --json
captain status
captain doctor --full
```

Common failures:

- TOML reports `expected struct ModelConfig`: replace a string `model` with the
  `[model]` table shown above.
- TOML reports a tool type mismatch: use `tool_allowlist = [...]`, not a string
  array under `[tools]`.
- API state remains `ingress_ready`: configure the external callback URL and
  secret, then run the egress test endpoint.
- A tool is unavailable: inspect effective capabilities and update the
  manifest explicitly instead of granting a wildcard.

See also [CLI Reference](cli-reference.md#agent-commands),
[API Reference](api-reference.md#agent-endpoints), and the embedded
[agent-coordination contract](captain-tools/agent-coordination.md).
