# MCP family

> **Status:** audited (D.17).
> See [`README.md`](README.md) for the index and drift policy.
> This family is a playbook, not a builtin-tool group: MCP servers are discovered dynamically and exposed as `mcp_{server}_{tool}` tools after connection.

## Tools

Captain can extend itself with external Model Context Protocol servers. Prefer the typed MCP tools below before falling back to shell/config edits:

| Surface | Use |
|---|---|
| `capability_search({query, sources:["mcp","docs","builtin"]})` | First check whether the needed capability is already available. |
| `captain_docs({family:"mcp", query:"install"})` | Read this playbook before inventing an install path. |
| `mcp_catalog_search` | Search bundled MCP templates and required env vars without shelling out. |
| `mcp_integration_install` | Install a bundled template, store credentials through the vault/resolver, and hot-reload MCP. |
| `mcp_status` | Verify configured/connected MCP servers and visible `mcp_*` tools. |
| `captain integrations <query>` / `captain add <id>` via `shell_exec` | Fallback only if the typed tools are unavailable. |
| `secret_write` / `secret_read` | Store and verify required API keys without exposing raw values. |
| `config_read` / `config_write` / direct TOML fallback | Configure `[[mcp_servers]]` only when the extension registry cannot express the server. |
| `system_bug_report` | Record missing templates, repeated MCP failures, or docs/config gaps. |

### Autonomous install flow

1. **Discover first.** Call `capability_search` with the user's task. If a connected `mcp_...` tool already matches, use it directly.
2. **Search bundled templates.** Call `mcp_catalog_search({"query":"<service or capability>"})`. If a template exists, install through `mcp_integration_install` instead of hand-writing config.
3. **Classify the server.** If no template exists, identify transport (`stdio` or `sse`), command, args, env vars, required credentials, and expected tool names. Do not guess missing credentials.
4. **Store credentials in the vault.** Raw keys go only through `secret_write` or a first-class setup helper. Config stores env-var names, never secret literals.
5. **Patch config narrowly.** For direct config, add one `[[mcp_servers]]` entry with a stable, generic `name`, explicit `timeout_secs`, exact transport fields, the env-var names the subprocess needs, and `auth_token_env` for authenticated SSE.
6. **Reload or restart only what is needed.** `mcp_integration_install` attempts hot-reload for packaged integrations. If raw `[[mcp_servers]]` was edited manually, explain when a daemon restart is required before using the new tools.
7. **Verify from Captain's view.** Call `mcp_status`, re-run `capability_search`, or inspect the newly namespaced `mcp_{server}_...` tools. Do not declare success from config write alone.
8. **Learn the reusable route.** Save generic lessons with `memory_save`; if the install required a reusable workflow, propose a skill or integration template. If the flow exposed a product gap, call `system_bug_report`.

### Direct `[[mcp_servers]]` shape

Use this only when the bundled extension registry cannot install the server.

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

For remote servers:

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

For remote SSE, Captain sends `Authorization: Bearer <value of auth_token_env>` on MCP requests. If the server cannot enforce that token, do not expose it outside loopback/private development.

## Sandbox

- **Credential SSOT** — credentials resolve from `~/.captain/secrets.env` first, then legacy `vault.enc`, then `.env`, then process env. `secret_write` updates the first source and is visible to MCP resolution without daemon restart.
- **No false ready** — an MCP integration is `ready` only when required credentials are actually resolvable from the credential chain. A key passed in a tool call does not count unless it was persisted successfully.
- **SSE auth boundary** — `auth_token_env` is the supported bearer-token path for remote MCP SSE. Never put the token in the URL, TOML, docs, command args, memory, or logs.
- **No private defaults** — never encode user-specific hostnames, account ids, local aliases, or private infrastructure names in prompts, templates, docs, skills, or bug reports. Use generic placeholders and derive real values from the current user's config.
- **No blind shell execution** — inspect the install command, package name, and required env before running it. Prefer known package managers and official package names.
- **Dynamic tool names** — MCP tools appear only after the server connects. Names are normalized as `mcp_{server}_{tool}` with hyphens replaced by underscores.
- **MemPalace SSOT** — for Captain memory, prefer the first-class `memory_save`, `memory_recall`, and `memory_forget` tools. Raw MemPalace MCP write tools may be hidden when the mirror owns writes.

## Limites

- Raw `[[mcp_servers]]` edits may require a daemon restart before tools appear; bundled integrations can be hot-reloaded by the extension system.
- `config_write` cannot append arrays/tables safely. Use a typed integration installer when possible; otherwise make the smallest TOML edit and parse-check immediately.
- Installing a package is not proof of capability. Captain must verify that the MCP handshake succeeds and that expected tools are visible.
- SSE `auth_token_env` protects Captain's client request. The remote MCP server must also validate the bearer token; Captain cannot secure a server that ignores authentication.
- Some MCP servers need OAuth or browser login. If no safe non-interactive flow exists, ask the user for the authorization step instead of trying to bypass it.
- Do not persist one-off troubleshooting commands as skills unless the workflow is generic and likely reusable.

## Exemples

### Golden path — bundled MCP integration

```
1. capability_search({"query":"manage GitHub issues","sources":["mcp","builtin","docs"]})
   -> no connected GitHub MCP tool
2. mcp_catalog_search({"query":"github"})
   -> bundled template exists
3. mcp_integration_install({"id":"github","credentials":{"GITHUB_PERSONAL_ACCESS_TOKEN":"<user secret>"}})
4. mcp_status({})
5. capability_search({"query":"create GitHub issue","sources":["mcp"]})
   -> mcp_github_create_issue visible
```

### Golden path — custom stdio MCP server

```
1. captain_docs({"family":"mcp","query":"direct mcp server config"})
2. secret_write({"key":"SERVICE_API_KEY","value":"<user secret>"})
3. Add one [[mcp_servers]] entry with env = ["SERVICE_API_KEY"]
4. Parse-check config, reload/restart as required
5. capability_search({"query":"service desired action","sources":["mcp"]})
```

### Golden path — authenticated remote MCP server

```
1. secret_write({"key":"SERVICE_MCP_API_KEY","value":"<random shared token>"})
2. Configure the remote MCP server to require Authorization: Bearer <same token>
3. Add [[mcp_servers]] with auth_token_env = "SERVICE_MCP_API_KEY"
4. Restart/reload as required
5. capability_search({"query":"service remote tool","sources":["mcp"]})
```

### Recovery — install gap becomes product learning

```
system_bug_report({
  "title":"Missing bundled MCP template for a reusable service",
  "category":"mcp",
  "severity":"medium",
  "description":"Captain had to hand-configure a generic stdio MCP server because no extension template existed.",
  "suggested_fix":"Add an integration template with command, args, required env vars, health check, and docs."
})
```
