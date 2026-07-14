# Captain Tools — RTFM Index

This directory holds the structured documentation Captain consults via the
`captain_docs(query, family?)` tool (added in C.2). Each `*.md` file under
this folder describes one tool family — Action, Sandbox, Limits,
Examples — and is the only doc Captain is allowed to surface to itself
when it needs to remember how a tool behaves before reaching for
`ask_user`.

## Why this exists

Without `captain_docs`, Captain's only options when it isn't sure how a
tool behaves are (1) hallucinate the schema, (2) ask the user. Both are
worse than letting it reread its own manual. The doc is intentionally
**code-adjacent** and version-controlled here, not in MEMORY/, so a live tool
definition change and the matching family file ship in the same PR.

## Drift protection

The `captain_docs` test suite compares every family file in this directory
against the live `builtin_tool_definitions()` registry so:

- a tool added in code without a doc entry fails CI,
- a doc entry referencing a tool that no longer exists fails CI.

Any change to a definition under `crates/captain-runtime/src/tools/` must ship
with the matching family documentation. Any new family means: append a
`(slug, "phase tag")` to
[`captain_runtime::captain_docs::FAMILIES`](../../crates/captain-runtime/src/captain_docs.rs)
**and** create the matching markdown file here.

## Families

| Slug | Audit | What lives here |
|------|-------|------------------|
| [`file`](file.md)                                 | D.1  | `file_inspect_batch`, `file_read`, `file_write`, `file_list`, `glob`, `grep`, `edit_file`, `multi_edit`, `apply_patch` |
| [`shell-process`](shell-process.md)               | D.2  | `shell_exec`, `execute_code`, `process_start`, `process_poll`, `process_write`, `process_kill`, `process_list`, `docker_exec`, `cargo`, `npm`, `pip` |
| [`network`](network.md)                           | D.3  | `web_research_batch`, `web_download`, `web_fetch`, `web_search` |
| [`browser`](browser.md)                           | D.4  | `browser_batch`, `browser_navigate`, `browser_click`, `browser_type`, `browser_keys`, `browser_select`, `browser_hover`, `browser_screenshot`, `browser_read_page`, `browser_close`, `browser_scroll`, `browser_wait`, `browser_run_js`, `browser_back`, `browser_status`, `browser_network_log`, `browser_observe`, `browser_diagnostics`, `screenshot` |
| [`ssh`](ssh.md)                                   | D.5  | `ssh_health_check`, `ssh_exec`, `ssh_upload`, `ssh_download` |
| [`memory`](memory.md)                             | D.6  | `memory_context_batch`, `memory_save`, `memory_recall`, `memory_store`, `memory_forget` |
| [`skill`](skill.md)                               | D.7  | `skill_search`, `skill_execute`, `scaffold_skill`, `skill_proposal_list`, `skill_proposal_decide`, skill refinement tools |
| [`channel`](channel.md)                           | D.8  | `channel_delivery_batch`, `channel_send`, `channel_reconfigure`, Telegram topic tools |
| [`agent-coordination`](agent-coordination.md)     | D.9  | `agent_spawn`, `agent_send`, `agent_list`, `agent_kill`, `agent_status`, `agent_watch`, `agent_delegate`, `agent_correct`, `agent_find`, fleet tools, task tools, Hands, A2A, `ask_user` |
| [`scheduling`](scheduling.md)                     | D.10 | `cron_create`, `cron_list`, `cron_update`, `cron_cancel`, `reminder_set`, schedule tools, goal lifecycle and suggestions |
| [`config-secret`](config-secret.md)               | D.11 | `model_switch_plan`, `model_switch_apply`, `codex_auth_status`, `codex_tool_probe`, `codex_login_start`, `codex_login_poll`, `config_read`, `config_write`, `web_credentials_update`, `config_setup`, `config_schema`, `self_configure`, `secret_read`, `secret_write` |
| [`mcp`](mcp.md)                                   | D.17 | MCP install/recovery playbook: capability discovery, integrations registry, vault-backed env, direct `[[mcp_servers]]` fallback |
| [`knowledge`](knowledge.md)                       | D.12 | `knowledge_add_entity`, `knowledge_add_relation`, `knowledge_query` |
| [`session-workspace`](session-workspace.md)       | D.13 | `session_recall`, `workspace_add` |
| [`meta`](meta.md)                                 | D.14 | `system_time`, `location_get`, `canvas_present`, `capability_search`, `captain_docs`, `self_improvement_review`, `tool_search`, learning review and system-bug tools |
| [`project`](project.md)                           | D.15 | project, task, milestone, and checkpoint tools |
| [`multimedia`](multimedia.md)                     | D.16 | `media_pipeline`, `image_analyze`, `image_generate`, `media_describe`, `media_transcribe`, `text_to_speech`, `speech_to_text`, `video_analyze` |
| [`document`](document.md)                         | D.19 | `document_pipeline`, `document_create`, `document_extract` |
| [`runtime-changelog`](runtime-changelog.md)       | D.18 | public-safe, versioned runtime changes Captain reads after install/restart |

> **Status:** audited — family bodies are enforced against
> `captain_runtime::captain_docs::*_FAMILY_TOOLS` by the `captain_docs`
> test suite.
