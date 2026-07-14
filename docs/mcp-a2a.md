# MCP and A2A

Captain keeps two interoperability paths, but they do not have the same product
status:

- **MCP is active** for connecting governed external tool servers and exposing
  Captain through a local stdio server.
- **A2A is frozen compatibility** for explicitly approved external agents. It
  remains compiled and documented, but Captain does not propose it by default.
  Use local subagents or the per-agent ingress/egress API for normal Captain
  orchestration.

## MCP Client

Prefer Captain's integration registry and capability discovery. It can install
packaged integrations, store required credentials through the secret rail, and
hot-reload supported servers. Use raw config only when the registry cannot
represent the server.

Custom stdio server:

```toml
[[mcp_servers]]
name = "service-name"
timeout_secs = 30
env = ["SERVICE_API_KEY"]

[mcp_servers.transport]
type = "stdio"
command = "npx"
args = ["-y", "package-name"]
```

Authenticated remote SSE server:

```toml
[[mcp_servers]]
name = "service-name"
timeout_secs = 60
auth_token_env = "SERVICE_MCP_API_KEY"
env = []

[mcp_servers.transport]
type = "sse"
url = "https://example.com/mcp/sse"
```

Store secret values in `secrets.env` or Captain's vault. Config contains only
environment-variable names. Remote MCP must enforce the bearer token resolved
from `auth_token_env`; do not expose an unauthenticated server outside a local
development boundary.

After adding a raw config entry, restart the daemon when required and verify
from Captain's view. Connected tools are normalized as
`mcp_{server}_{tool}`. A config write alone is not proof that the server is
ready.

The agent-facing install and recovery contract lives in
[Captain MCP tools](captain-tools/mcp.md). Configuration fields live in
[Configuration](configuration.md#mcp-servers).

## MCP Server

Run Captain as a stdio MCP server for a local client:

```bash
captain mcp
```

The command uses the running daemon when available and can fall back to an
in-process kernel. Keep stdio local to the process boundary and grant only the
tools the target agent should use.

Captain also mounts an authenticated HTTP JSON-RPC endpoint at `POST /mcp`.
Do not publish it directly to the Internet. Use Captain authentication, HTTPS,
network restrictions, capability scopes, and request limits. See
[API Reference](api-reference.md#mcp--a2a-protocol-endpoints) for request
shapes.

## A2A Compatibility

The inbound compatibility routes are:

```text
GET  /.well-known/agent.json
GET  /a2a/agents
POST /a2a/tasks/send
GET  /a2a/tasks/{id}
POST /a2a/tasks/{id}/cancel
```

Outbound discovery and task management use the authenticated `/api/a2a/*`
routes documented in the API reference. A2A crosses a trust boundary: do not
send secrets, raw user data, local files, or memory to an external agent unless
the user explicitly approved that destination and payload.

For local Captain agents, use `agent_spawn`, `agent_delegate`, and `agent_send`.
For an external service that needs a stable per-agent contract, use
`captain agent api <agent> --manifest`: Captain provisions authenticated ingress
and can emit signed callbacks once the operator supplies the external callback
URL.

## Operational Checks

```bash
captain status --verbose
captain doctor --full
captain agent api <agent> --manifest
```

Within Captain, use `mcp_status` and capability discovery to verify connected
servers and effective tools. Do not infer readiness from a process existing or
from an old session summary.

## Security Rules

- Keep credentials out of TOML, URLs, command arguments, memory, and logs.
- Treat every remote MCP or A2A endpoint as untrusted until authenticated and
  explicitly approved.
- Scope tools and network destinations per agent; avoid wildcard grants.
- Use TLS for any remote HTTP/SSE connection.
- Preserve audit correlation IDs when investigating a cross-system call.
- Prefer active core paths over frozen A2A compatibility when both can solve
  the same problem.
