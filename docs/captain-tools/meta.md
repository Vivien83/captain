# Meta family

> **Status:** audited (D.14).
> See [`README.md`](README.md) for the index and drift policy.
> Tool name list pinned in [`captain_runtime::captain_docs::META_FAMILY_TOOLS`](../../crates/captain-runtime/src/captain_docs.rs).

## Tools

The meta family is Captain's reflexive layer — the tools that operate on Captain itself rather than on the user's domain data. Today it covers the wall-clock, approximate location context, canvas presentations, unified capability routing (`capability_search`), controlled native capability authoring (`capability_forge`), the RTFM surface that lets Captain reread its own manual (`captain_docs`), deferred-tool discovery (`tool_search`), the learning-review queue, and the system-bug register.

### `system_time`

Returns the current date and time, in three flavours, in one call.

| Field | Required | Notes |
|---|---|---|
| (none) | — | No parameters. |

Returns:

```
{
  "utc": "2026-04-29T08:42:11Z",
  "local": "2026-04-29T10:42:11+02:00",
  "unix_epoch": 1745916131,
  "timezone": "CET",
  "utc_offset": "+01:00"
}
```

Use this **before** any scheduling tool that takes an absolute date (`cron_create({"kind":"at"...})`) or any `memory_save` whose object embeds a date. The LLM cannot reliably infer "today" — calling `system_time` once is cheaper than computing wrong.

For relative arithmetic (`X minutes from now`) it is fine to do the math in the prompt; for "tomorrow at 9am" or anything anchored to the calendar, ground first.

### `system_update`

Updates Captain itself to the latest published release: version check,
download, checksum verification, atomic binary swap, then a daemon restart
through the platform service manager. Shells out to `captain update` so the
CLI stays the single source of truth for the whole recipe.

| Field | Required | Notes |
|---|---|---|
| `check_only` | no | `true` = only report whether a newer version exists; nothing is installed. Default `false`. |

The real update always requires an explicit user approval (forced, independent
of the configurable approval policy) because it replaces the running binary
and restarts the daemon. Warn the user that the session will be briefly
interrupted before calling it. Inside a Docker container the tool refuses and
explains that the image must be rebuilt/pulled instead. The updater's output
is appended to `~/.captain/update.log`.

### `location_get`

Return Captain's configured coarse location context when available.

| Field | Required | Notes |
|---|---|---|
| (none) | — | No parameters. |

Use this when a task depends on local context (weather, travel, nearby services) and the user has not specified a location in the current message. If it returns nothing useful, ask the user or use a location present in memory/config; do not infer a private address.

### `canvas_present`

Render an HTML panel as a saved canvas artifact for the user. The HTML is sanitised (no scripts, no event handlers, no external assets without CSP allowance) and saved to the agent's workspace.

| Field | Required | Notes |
|---|---|---|
| `html` | yes | The panel body. |
| `title` | no | Window title; defaults to "Captain canvas". |

Returns the saved file path. Use this for rich data visualisations, structured reports, or custom UI Captain assembles for one query. **For plain text replies, just answer in chat** — `canvas_present` is overkill for prose.

### `capability_search`

Resolve which active Captain capability should handle a task before guessing or giving up. This is the first stop when the agent is unsure whether the answer is a builtin tool, an active `.captain` CapSpec, an installed skill, a connected MCP tool, or a `captain_docs` family.

| Field | Required | Notes |
|---|---|---|
| `query` | yes | Capability keywords or `select:name1,name2` for exact lookup. |
| `sources` | no | Optional filter: `builtin`, `capfile`, `skill`, `mcp`, `docs`. `capspec` is accepted as an alias for `capfile`. Defaults to all active sources. |
| `max_results` | no | Default 8, clamped to 30. |
| `include_schemas` | no | Default true. Include input schemas when the candidate is callable. |

Returns candidates shaped for immediate routing:

```
{
  "query": "ssh alias recovery",
  "results": [
    {
      "source": "builtin",
      "name": "ssh_exec",
      "status": "core_visible",
      "usage": "Call this builtin tool directly...",
      "input_schema": { "type": "object", "properties": { ... } },
      "metadata": { "core": true }
    },
    {
      "source": "docs_family",
      "name": "ssh",
      "usage": "Call captain_docs with this family...",
      "metadata": { "snippet": "..." }
    }
  ]
}
```

Decision workflow:

1. Call `capability_search` when the capability surface is unclear.
2. If the best result is `docs_family`, call `captain_docs({family, query})`.
3. If the best result is a deferred builtin and the schema is not already visible enough, call `tool_search({"query":"select:<name>"})`.
4. If the best result is `capfile_tool`, call that active native capability directly. Its primitive steps still pass through the caller's normal grants and central ToolRunner policies.
5. If the best result is `skill_tool` or `mcp_tool`, call that tool directly.
6. Surfaces frozen for the current product phase (Hands, A2A, peers, fleets) are not proposed by default. Prefer explicit builtin tools, CapSpecs, skills, MCP tools, projects, or sub-agents in the active core.

Learning link: successful runs that used `capability_search`, `skill_search`, `tool_search`, or `captain_docs` are summarized in the end-of-run learning signal. The Haiku reflection pass should retain only reusable, generic capability routes; it must skip private aliases, secrets, one-off file paths, and user-specific infrastructure names.

### `capability_forge`

Validate, inspect, list, or propose a readable native `.captain` capability.
This is a deferred builtin: discover it through
`tool_search({"query":"select:capability_forge"})` when the user explicitly
asks Captain to make a workflow native, or when a reusable workflow has been
clearly established.

| Field | Required | Notes |
|---|---|---|
| `action` | yes | Exactly `list`, `inspect`, `validate`, or `propose`. There is deliberately no approval action. |
| `scope` | no | `effective`, `all`, `global`, or `project`. A proposal defaults to the active project when a workspace exists, otherwise global. |
| `name` | inspect: yes | Optional for validate/propose; if provided it must exactly match the source `name`. |
| `source` | validate/propose: yes | Complete strict TOML `.captain` source, capped and compiled before any write. |
| `include_source` | no | For inspect only. Returns the exact versioned source when explicitly requested. |

`propose` is reserved for the principal Captain agent. It writes the source
durably, reloads the registered scope immediately, records an audit entry, and
returns a structured capability card with source hash, status, permissions,
revision history, and `next_action`. A first strictly read-only revision can be
`operational` immediately. Write, shell, network, SSH, memory mutation, secret,
remote, or destructive authority returns `pending_approval` or
`update_pending_approval`; Captain must say that human approval of the exact
pending hash is still required.

The tool cannot approve, reject, roll back, or delete a capability. Those are
authenticated operator actions. Never claim `ready` when the returned status
is pending, invalid, rejected, or disabled, and never ask another agent to
circumvent this boundary. Control and the TUI expose direct exact-hash and
uncertain-run decisions. When Telegram has an allowlisted user and
`default_chat_id`, it also surfaces durable pending cards automatically; button
clicks resolve in the kernel before any agent turn. Captain should report the
pending state and wait for one of those human surfaces, not simulate a click or
invent an approval command.

### `captain_docs`

Search the family files under `docs/captain-tools/` for Captain's structured, AI-facing documentation about its own tools. **Use this before guessing, before asking the user how Captain works, and before abandoning a task because a tool failed.**

| Field | Required | Notes |
|---|---|---|
| `query` | yes | Multi-word AND search, case-insensitive. Include the tool name plus the strongest error keywords. |
| `family` | no | Optional family filter: `file`, `shell-process`, `network`, `browser`, `ssh`, `memory`, `skill`, `channel`, `agent-coordination`, `scheduling`, `config-secret`, `mcp`, `knowledge`, `session-workspace`, `meta`, `project`, `multimedia`, `runtime-changelog`. |
| `max_results` | no | Default 5, clamped to 14. |

Returns snippets with the matched family name. When `family` is provided, Captain receives the family body so it can reason from the canonical contract rather than a short hint.

Recovery query pattern:

```
captain_docs({
  "family": "ssh",
  "query": "ssh_exec alias not found recovery"
})
```

Decision rule for agents: if an error concerns a Captain tool, first read the exact error. If the next action is not obvious, call `captain_docs` with `{tool_name} + {error keywords}`. Ask the user only after docs, memory/config/knowledge checks, and safe retries have failed or the next step needs permission.

When `family` is explicit, `captain_docs` returns the full family guide plus a generated **Live Tool Schemas** section sourced from the running `builtin_tool_definitions()` registry. If prose and schema disagree, use the live schema and treat the prose as stale.

### `tool_search` *(added in TS.1)*

Discover and load the schema of any builtin tool that isn't in Captain's CORE prompt. Captain keeps a small CORE set always visible; the other builtin tools (`browser_*`, `image_*`, `text_to_speech`, `secret_write`, `fleet_*`, `project_*`, `milestone_*`, `cron_list`, `hand_*`, `schedule_*`, `knowledge_add_*`, `agent_kill`, …) are deferred and only enter the prompt when this tool surfaces them.

| Field | Required | Notes |
|---|---|---|
| `query` | yes | Free-text keywords (lower-cased, whitespace-split) **or** `select:name1,name2` for exact-name lookup. |
| `max_results` | no | Default 5, clamped to `[1, 20]`. |

Returns:

```
{
  "results": [
    {
      "name": "text_to_speech",
      "description": "Convert text to spoken audio…",
      "input_schema": { "type":"object", "properties":{ … } }
    },
    …
  ]
}
```

If no deferred builtin tool matches, the response still returns an empty `results` array and adds a `hint` field. Treat that hint as a routing cue: `tool_search` does **not** search installed skill tools or dynamic MCP tools. For skills, use the visible skill instructions or `skill_execute`; for external integrations, inspect the `Connected Tool Servers (MCP)` prompt section or configure the relevant MCP server; for builtin behaviour or recovery rules, query `captain_docs`.

Ranking is purely lexical — case-insensitive substring match, name×2 + description×1, sorted descending then by name as a stable tie-breaker. CORE tools are excluded from the candidate set on purpose: if Captain already has `file_read`, surfacing it again wastes tokens.

The two-turn workflow:

1. Captain identifies a need (`"je veux parler à voix haute"`, `"naviguer une URL"`).
2. `tool_search({"query":"voix haute"})` returns up to 5 deferred candidates with full input schemas.
3. Next turn, Captain calls the chosen tool directly — its schema is now in-context.

Use the `select:` form when Captain remembers the exact name from a previous interaction or from `captain_docs`. It bypasses ranking and de-duplicates cleanly:

```
tool_search({"query": "select:text_to_speech,browser_click"})
```

This pattern (Claude Code style) replaces the dynamic Tool RAG top-K filter that used to truncate the visible builtin list. The trade-off is a fixed two-turn latency for non-CORE tools, in exchange for a deterministic CORE prompt and no more "no access to X" hallucinations on tools the LLM never saw.

### Learning review

`learning_review_list` and `learning_review_decide` expose Captain's post-hoc review queue: tool failures, ask_user misuse, repeated work, and proposed behaviour corrections.

The learning bus keeps raw tool outcomes in order, but plain first-time successes/failures are classifier buffer events only. Durable review is triggered by repeated failures, retry-success recovery, workflow summaries, explicit user corrections, or conversation-level reflection. This keeps Captain adaptive without spending a reflection call on every ordinary tool result.

| Tool | Use |
|---|---|
| `self_improvement_review({limit?})` | Read-only overview of pending learning approvals, system bugs, skill refinements and skill proposals. Use after long/tool-heavy work, repeated failures, or when the user asks what Captain learned. |
| `learning_review_list({limit?})` | Inspect pending learning items before changing prompt/docs/skills. Tool output masks secret-looking strings and redacts local host paths. |
| `learning_review_decide({id, approve})` | `approve:true` commits the item through the memory writer; `approve:false` marks it denied. Tool output masks secret-looking strings and redacts local host paths. |

Use these when Captain notices a repeated failure pattern or when the user asks why Captain behaved badly. Do not silently accept learning items that encode user-specific secrets, private infrastructure names, or one-off paths; store durable user facts through `memory_save` and reusable procedures through `scaffold_skill`.

`self_improvement_review` is the safe first step for controlled auto-improvement. It does not mutate anything. It shows pending memory review items, open system bugs, pending skill refinements, pending skill proposals, the visual feedback contract, and the next approved action surface. Critical changes (skills, config, goals, routing, prompts, global behaviour) must remain visible proposals until the user approves them.

Feedback is mandatory. In approval mode, a queued learning emits `MemoryQueued` and the current chat renders `💭 apprentissage à valider` with the review id. Telegram approval buttons use the dedicated learning commands (`/learn_approve`, `/learn_reject`) so the review item is resolved through `learning_review_decide`. In auto mode, the commit emits `MemoryStored` and renders `🧠 mémorisé`. Repeatable-workflow proposals emit `SkillProposalQueued`; the current chat renders `🛠️ skill proposé`, and Telegram buttons resolve through `/skill_approve` / `/skill_reject` into `skill_proposal_decide`. Existing-skill improvements emit `SkillRefinementQueued`; Telegram buttons resolve through `/skill_refine_approve` / `/skill_refine_reject`, and file-backed skills include an automatic pre-improvement snapshot for `skill_refinement_restore`. If the learning changes future behaviour, Captain must also tell the user what changed and how it will act differently next time; if the preference is ambiguous, ask one short clarification before saving. If the user asks "qu'as-tu appris ?", answer from those visible events, `learning_review_list`, `skill_proposal_list`, or `skill_refinement_list`, not from guesses.

Generated-skill approvals keep the audit path internal. `skill_proposal_decide(approve:true)` returns a logical `written` state instead of the generated file path, and all learning/proposal review surfaces apply the same public-safe output projection before display or `self_improvement_review`.

### System bug register

`system_bug_report`, `system_bug_list`, and `system_bug_update` are Captain's self-diagnostic register. Use them when Captain detects a reproducible defect in its own system: missing or misleading tool, repeated internal failure, security gap, performance issue, MCP install weakness, channel bug, scheduler defect, stale documentation, or skill behaviour that needs repair.

| Tool | Use |
|---|---|
| `system_bug_report({title, description, category, severity, evidence?, suggested_fix?, source?})` | Create a categorized bug/weakness item. Secrets are refused and local paths in stored text fields are redacted before storage/output. |
| `system_bug_list({status?, category?, severity?, limit?})` | Inspect known issues before fixing or reporting one. Output is public-safe even for legacy registry items. |
| `system_bug_update({id, status?, category?, severity?, note?, suggested_fix?})` | Mark fixed/reported/duplicate, add notes, or reclassify. Output is public-safe and the registry is normalized on store. |

Categories: `tool`, `scheduler`, `channel`, `memory`, `security`, `performance`, `mcp`, `skill`, `docs`, `ui`, `unknown`.
Severities: `low`, `medium`, `high`, `critical`.
Statuses: `open`, `investigating`, `fixed`, `wont_fix`, `duplicate`, `reported`.

This register is not a replacement for fixing the bug. It is the durable "do not forget this defect" surface that lets Captain either self-improve after approval or produce a precise report for a developer. Its stored text fields are operator-safe: raw secrets are blocked and local host paths become `<local-path>` before the item reaches memory, lists, or `self_improvement_review`. Older registry items are also projected through the same public-safe output boundary before display.

## Sandbox

- `system_time` reads the daemon's wall clock; it does not touch the filesystem or the network.
- `location_get` reads configured/coarse location context only; it must not geolocate the user through network probes.
- `canvas_present` writes the rendered HTML under the agent's workspace (`workspace/canvas/<timestamp>.html`). The path resolution goes through the same multi-root sandbox as `file_write`.
- `capability_search` is read-only. It inspects live active builtin definitions, workspace-aware and hot-reloaded CapSpec definitions, the installed SkillRegistry, connected MCP tool registries, and the `captain_docs` family index. It does not execute candidate tools. CapSpec results use source `capfile_tool` and status `active_native`.
- `capability_forge` validates before writing, accepts only its four safe actions, and reserves proposals for the principal Captain agent. Project roots reject symbolic `.captain` ancestors before creating anything. Sensitive capability activation remains outside the agent tool and requires an authenticated exact-hash operator decision.
- `captain_docs` reads **only** files under `docs/captain-tools/` — that path is in Captain's authorised root. Ordinary agents do not have access; the tool errors out with a clear message rather than spilling the doc to a worker that should not see it.
- On real runtime update, the daemon records the current binary fingerprint in system KV and injects a one-turn "Mise a jour runtime reelle" notice. This notice proves only that the fingerprint changed. Read `captain_docs({family:"runtime-changelog", query:"update runtime"})` before explaining what changed, then treat current live schemas/docs as authoritative. Do not rely on `git log`, stale assumptions, or old sessions after an install/restart.

## Limites

- `system_time` reflects the daemon's clock. If the host clock drifts, every cron / `at` / `memory_save` event timestamp drifts too. There is no NTP sync built in — rely on the OS.
- The `timezone` field is best-effort: on Linux the daemon reads `/etc/localtime`, on macOS `systemsetup -gettimezone` may not be set. The `utc_offset` is always correct.
- `canvas_present` HTML is sanitised through `ammonia`; some legitimate styling (CSS in `<style>`, complex SVG defs) gets stripped. For complex visualisations write static files via `file_write` and link from chat.
- `canvas_present` does not stream — the whole HTML body sits in the response. Keep panels under ~256 KB or surface as a file link instead.
- `learning_review_decide` affects future behaviour, not the current failed turn. Pair accepted items with a concrete doc, prompt, memory, or skill change when appropriate.
- `system_bug_report` is for Captain's own product defects, not user-domain tasks. Do not store private hostnames, secrets, tokens, or one-off personal paths in titles/descriptions.

## Exemples

### Golden path — ground a scheduled prompt

```
1. system_time({})
   → {"local": "2026-04-29T10:42:11+02:00", "timezone": "CET", ...}
2. cron_create({
     "name": "9am ping",
     "prompt": "Bonjour…",
     "kind": {"kind":"at","at":"2026-04-30T09:00:00+02:00"},
     "one_shot": true
   })
```

### Golden path — render a report

```
canvas_present({
  "title": "Ops report",
  "html": "<h1>Service status</h1><ul><li>...</li></ul>"
})
→ {"path": ".../canvas/2026-04-29T084500.html"}
```

### Golden path — choose the right capability before guessing

```
1. capability_search({
     "query": "generate a spoken audio summary from text",
     "sources": ["builtin", "skill", "mcp", "docs"]
   })
   → [{"source":"builtin","name":"text_to_speech","status":"deferred_builtin", ...}]
2. tool_search({"query":"select:text_to_speech"})
   → exact builtin schema, if the schema was not already visible enough
3. text_to_speech({...})
```

Use this route whenever the user asks for a capability and Captain is not certain whether it lives in a builtin tool, installed skill, connected MCP server, Hand, or docs family.

### Anti-pattern — using ask_user as a docs lookup

```
ask_user({"question": "Comment fonctionne edit_file?"})
→ (anti-pattern — captain_docs("edit_file") will reach the same answer
without bothering the human; the reflection job flags this for review.)
```

The audit logs annotate `ask_user` calls whose subject matches a known tool name as RTFM-bypass candidates so the user can correct Captain's reflex once and for all.
