# Skill family

> **Status:** audited (D.7).
> See [`README.md`](README.md) for the index and drift policy.
> Tool name list pinned in [`captain_runtime::captain_docs::SKILL_FAMILY_TOOLS`](../../crates/captain-runtime/src/captain_docs.rs).

## Tools

A **skill** is a packaged unit of agent capability — Markdown front-matter plus repeatable instructions or executable blocks. There are two creation paths:

- `scaffold_skill` creates a working draft under the current agent workspace: `skills/<name>/SKILL.md`.
- explicit human/API/channel approval of a skill proposal materialises reflection-generated proposals under the configured generated-skills root, usually `~/.captain/skills/generated/<name>.md` when `[skills] generated_dir = "skills/generated"`. Tool calls may reject proposals, but cannot approve with `approve=true`.

The tools below cover discovery, invocation, creation, the proposal queue that lets Captain turn repeated work into reusable capability, and the refinement loop that lets Captain improve an existing skill after real use.

### `skill_search`

Discover procedural skills by family and keywords before inventing a workflow or claiming the capability is missing.

| Field | Required | Notes |
|---|---|---|
| `query` | no | Workflow keywords such as `debug failing test`, `project planning`, or `release review`. Can be empty when `family` is provided. |
| `family` | no | One of `software-development`, `project-management`, `review-release`, `platform-devops`, `data-ai`, `product-design`, `business-tools`, `security-compliance`, `general-automation`. |
| `max_results` | no | Default 8, clamped to 30. |
| `include_context` | no | Default false. When true, includes a short SKILL.md excerpt so Captain can apply the workflow immediately. |
| `include_families` | no | Default true. Includes the family catalog and current counts. |

Use `skill_search` when a task is procedural: development planning, debugging, code review, project orchestration, release checks, DevOps playbooks, or a generated workflow. With no `query` and no `family`, it returns the minimal skill index. It returns installed and bundled skills with their family, source, `file_backed`, required/provided tools, and usage guidance. It does not expose local skill paths; file-backed skills are inspected through `skill_view` and relative `file_path` values only. Generated `.md` skills remain under the configured generated-skills root, but their frontmatter `family` / `family:<id>` tag makes them discoverable here.

### `skill_view`

Load one exact skill after `skill_search`, without injecting the whole skill catalog.

| Field | Required | Notes |
|---|---|---|
| `name` | yes | Exact skill name returned by `skill_search`. |
| `include_context` | no | Default true. When false, returns metadata and linked files only. |
| `max_context_chars` | no | Default 8000, clamped between 500 and 20000. |
| `file_path` | no | Load a linked file inside the skill, such as `references/api.md`, `templates/config.yaml`, `scripts/check.py`, or `assets/example.json`. Path traversal and symlink escapes are blocked. |

The first `skill_view({name})` call returns metadata, required/provided tools, `file_backed`, the skill workflow context, `linked_files` grouped by `references`, `templates`, `scripts`, `assets`, and `other` when the skill is file-backed, plus a `validation` object. `validation.status` is `ok`, `warn`, or `limited`; warnings identify missing runtime entries, missing support files referenced by the skill text, blocked path escapes, and required env/tool checks without exposing absolute local paths. When the skill has linked scripts, bash/sh blocks, executable runtime config, or validation warnings, `validation.preflight_recommended` is true and `validation.preflight_tool_call` points to `skill_check`. A second call with `file_path` loads one supporting file on demand. This mirrors the operationally simple pattern: short index first, exact workflow second, supporting details only when needed.

Approved generated skills are reloaded into the active daemon registry immediately after the human/API/channel approval writes the file. A restart is not required before the next `skill_search`. Their frontmatter records promotion provenance (`owner: agent`, `locked: false`, `approved: true`, `verified_by`, `approved_by`, `success_rate`) so the durable skill carries the validation verdict instead of relying on a transient chat decision.

### `skill_check`

Run a static, no-side-effect preflight on one installed skill before using a brittle, scripted, or recently changed workflow.

| Field | Required | Notes |
|---|---|---|
| `name` | yes | Exact skill name returned by `skill_search` / `skill_view`. |
| `run_static_tests` | no | Default true. Runs available static tests without executing the skill. |
| `max_shell_blocks` | no | Default 20, clamped between 1 and 50. |

`skill_check` returns `pass`, `warn`, or `fail`. It includes the same file-backed validation used by `skill_view`, promotes blocking validation issues to failures, and runs `bash -n` on shell runtime entries or bash/sh fenced blocks without executing commands. A `fail` means do not run the skill yet; fix or refine it first.

CLI smoke: `captain skill search <query-or-family>` searches the same local installed/generated/bundled skill surface for quick operator checks. It is not a marketplace search.

API/CLI workflow rule: for third-party APIs, SaaS providers, DevOps platforms,
custom CLIs, OpenAPI specs, Postman collections, SDKs, or MCP integrations,
Captain should call `skill_search({query, include_context:true})` before
building an ad-hoc shell/code path, unless an exact loaded skill or typed tool
already covers the workflow. When a workflow succeeds, its exact endpoint or CLI
syntax, required parameters, credential handling, safety level, and verification
steps belong in a skill proposal/refinement so the next run is direct.

### `skill_execute`

Run a `.md` skill capability declared as a `### capability` heading with a bash/sh fenced block.

| Field | Required | Notes |
|---|---|---|
| `skill` | yes | Skill name as registered or present under `skills/<name>/SKILL.md`. |
| `capability` | yes | Capability heading to execute, for example `login`, `list_slots`, or `create_event`. |
| `args` | no | JSON object injected as environment variables into the script. |

Before spawning bash, Captain runs a no-side-effect syntax preflight with `bash -n` on the selected capability. If the script is invalid, the capability is not executed and the tool returns a JSON block with `status:"blocked"`, `is_error:true`, the syntax error, and a `next_action` that points back to `skill_check` / skill refinement. Valid capabilities then execute normally and return stdout.

### `scaffold_skill`

Create a new skill mid-session. Use **spontaneously** when:

1. you've solved a **repeatable workflow** and notice it will recur (a sequence of API calls, a recurring check),
2. an **integration is missing** and is blocking you (Slack, Notion, calendar, monitoring, payment),
3. a **capability would help other agents** (verification, scraping, transformation).

| Field | Required | Notes |
|---|---|---|
| `name` | yes | Lowercase, kebab-case (`status-checker`). |
| `description` | yes | One-sentence what-it-does. |
| `capabilities` | no | Array of capability slugs (`["check_slots", "book_slot"]`). |

Generates `skills/<name>/SKILL.md` under the current workspace with YAML front-matter and a bash stub for each capability. The scaffold is marked as human-authored and locked (`owner: human`, `locked: true`, `approved: true`, `verified_by: human`) because it is only appropriate when the user asked for it or explicitly approved the durable change. For spontaneous discoveries, start with `self_improvement_review` / `skill_proposal_list` and make the proposal visible before writing files.

### `skill_proposal_list`

List pending skill proposals — drafts queued by the reflection pipeline that the user has not yet approved.

| Field | Required | Notes |
|---|---|---|
| `limit` | no | Maximum pending proposals to return. Clamped to 1-50, default 50. |

Returns pending proposals with `id`, name, description, trigger hint, tool sequence, argument-schema hint, confidence, and discovery family. Tool output is capped to the requested review window, masks secret-looking strings, redacts local host paths, and omits internal audit fields such as pattern hashes, agent ids, origin channels, decision timestamps, and generated paths.

### `learning_review_list`

List pending non-critical memory candidates queued by the approval learning mode.

| Field | Required | Notes |
|---|---|---|
| `limit` | no | Maximum pending learnings to return. Clamped to 1-50, default 50. |

Returns only the review fields needed for a decision: `id`, `wing`, `room`, `subject`, `predicate`, `object`, and `confidence`. Tool output is capped to the requested review window, masks secret-looking strings, redacts local host paths, and omits internal audit fields such as agent ids, source labels, write ids, and decision timestamps.

### `learning_review_decide`

Approve or reject a pending learning candidate.

| Field | Required | Notes |
|---|---|---|
| `id` | yes | Review item id from `learning_review_list`. |
| `approve` | yes | Boolean. `true` commits the learning; `false` marks it denied. |

Decision output is a logical public projection: `status`, `id`, and `memory` state on approval. It does not expose the kernel write payload, source channel, agent id, write id, local path, or secret-looking text.

### `skill_proposal_decide`

Reject a pending proposal from an agent tool call, or record an approval that came through an explicit human/API/channel review surface. Approved proposals materialise into installed skills.

| Field | Required | Notes |
|---|---|---|
| `id` | yes | The proposal's UUID prefix (8 chars suffice). |
| `approve` | yes | Boolean. `false` marks it denied from the tool. `true` is accepted only on explicit human/API/channel review paths after external validation; tool-call approval is blocked even when no agent id is present. |

Prefix lookup uses the bounded `skill_proposal_list` review window, so decisions come from the visible list or a full id for older proposals. Approval output is an allowlisted public projection: it reports `status`, `id`, and the logical `written` state only. Generated paths, debug fields, and legacy path aliases remain internal audit data. If no deterministic verifier is available, the verifier is the explicit human/API/channel review; silence never promotes a proposal.

### Skill refinement loop

Existing skills must be questioned after use. When a skill works but reveals a missing precondition, brittle parsing, better tool route, recoverable error, missing `env_inject`, stale docs, or version bump, create a visible refinement proposal. Do not silently edit the skill.

| Tool | Use |
|---|---|
| `skill_refinement_propose({skill, finding, suggested_change, evidence?, current_version?, proposed_version?, risk?, source?, channel?})` | Queue a controlled improvement proposal for an existing skill and snapshot the current file-backed skill before any mutation. |
| `skill_refinement_list({skill?, status?, risk?, limit?})` | Inspect pending/approved/applied/restored refinements before editing a skill. |
| `skill_refinement_decide({id, approve, note?})` | Reject from a tool call, or record explicit human/API/channel approval through the review surface. Tool-call approval is blocked. Approval authorizes a later patch; it does not patch by itself. |
| `skill_refinement_update({id, status?, risk?, proposed_version?, note?})` | After patch/test/reporting, mark progress, especially `status:"applied"`; after verified rollback, `status:"restored"` is valid too. |
| `skill_refinement_restore({id, note?})` | Restore a file-backed skill from the automatic pre-improvement snapshot; creates a pre-restore backup first. |

Versioning contract: when a refinement changes behaviour, bump the skill version (`0.1.0 → 0.2.0`, `v1 → v2`, etc.) in the skill manifest/front matter when that format exposes a version. Record `current_version` and `proposed_version` in the proposal when known. `skill_refinement_propose` emits `SkillRefinementQueued`; Telegram shows inline approve/reject buttons via `/skill_refine_approve` and `/skill_refine_reject`. Positive approval must come from that explicit review surface or the API/channel equivalent, not from agent self-evaluation. After applying and testing, call `skill_refinement_update` with `status:"applied"` and a short note. If a rollback is needed, use `skill_refinement_restore`; restored items can be found with `skill_refinement_list({status:"restored"})`.

Output contract: `skill_refinement_*` keeps rollback locators internal and path-free. Tool output only reports logical snapshot state (`available`, `kind`, `reason`, `created_at`) plus the refinement journal fields; it does not publish local skill, snapshot, backup, restored paths, or snapshot ids. Stored text fields, including skill/source/channel/versions, findings, evidence, and notes, reject raw secret-looking values and redact local host paths before storage. Legacy registry items are still projected through the public-safe output boundary before list/decide/update/restore responses or `self_improvement_review`. Restore errors also avoid echoing legacy `skill` values when the target skill is missing from the registry.

### Skill curator (background pass)

A built-in cron `skill_curator` runs daily at 03:00 Europe/Paris with `delivery: "none"` (silent — no `channel_send` is fired). The cron sends Captain a structured prompt asking to:

1. List installed skills and their usage metadata (last-used, success/failure counts, current version).
2. Identify candidates : skills idle > 30 days, skills with failure_rate > 50 % over the last 10 runs, or semantic duplicates.
3. For each candidate, queue a `skill_refinement_propose` proposal (consolidation, deprecation, or merge) — never modify a skill silently.
4. Write a per-run report to `~/.captain/data/curator-reports/<YYYY-MM-DD>.md` via `file_write`. One line if no candidate.
5. **Do not** message the user proactively. The proposals surface through the existing `self_improvement_review` rail and the Telegram `🛠️ skill proposé` cards.

The Curator is a Captain-driven pass over the existing `skill_refinement_*` rail (commits Codex Phase 6). It does not bypass the approval contract: every change still requires an explicit human/API/channel decision before `skill_refinement_decide(approve=true)` can be recorded. To temporarily disable the pass, set `enabled = false` on the cron rather than deleting it (otherwise it gets re-created at the next boot — the builtin is idempotent).

### Autonomous learning flow

Captain has two complementary learning paths:

- **Immediate fact/lesson**: use `memory_save` for a durable fact, preference, error/success lesson, or concise solution.
- **Repeatable workflow**: use `scaffold_skill` when the same tool sequence or integration pattern will be reused and the user requested or approved the durable skill.
- **Reflection proposal**: use `skill_proposal_list` after long or tool-heavy work. Reject noisy ones with `skill_proposal_decide({"id":"...", "approve":false})`; high-confidence proposals must be promoted through explicit human/API/channel review after objective validation.
- **Skill diff gate**: generated proposals are automatically compared against bundled, user, and generated skills before review. If the workflow already exists, Captain must refine that skill instead of creating a near-duplicate.
- **Linked-file boundary**: supporting Markdown under `references/`, `templates/`, `scripts/`, and `assets/` is context for the owning skill, not a separate skill. The duplicate gate ignores those files as standalone entries.
- **Family contract**: every generated proposal carries a `family`. The LLM may choose it, but Captain normalizes it against the fixed taxonomy and writes it into the generated SKILL.md frontmatter plus a `family:<id>` tag.
- **Visible proposal contract**: when the SkillSynthesizer queues a proposal, Captain emits `SkillProposalQueued`. The active chat must show a natural-language skill proposal with the proposal id, purpose, future trigger, observed rationale, family, observed tools, and confidence. Telegram surfaces inline buttons that resolve via `/skill_approve` and `/skill_reject`. User-facing proposal text must use the configured user language, including trigger hints such as `Quand`.
- **Concrete workflow gate**: a skill proposal must carry reusable procedural evidence before it is shown to the user. The evidence can be an observed tool trace, or concrete natural-language steps from a documented discovery (API endpoint, CLI command, debugging path, project convention, recovery procedure, etc.). Proposals with underspecified descriptions, generic validation notes, or no concrete steps are rejected by policy rather than routed to Telegram.
- **Skill refinement**: after using a skill, proactively check whether it can be improved. If yes, use `skill_refinement_propose`; the current skill is snapshotted when file-backed, the proposal is routed to the preferred validation channel, then after explicit approval patch minimally, test, and `skill_refinement_update(status:"applied")`.
- **Memory review**: use `learning_review_list` / `learning_review_decide` for reflection-generated memory candidates that need approval before long-term write-through. `learning_review_list.limit` is clamped to 1-50, default 50, and output is capped to that review window.

Rule of thumb: if future Captain should **remember a fact**, write memory; if future Captain should **know how to do a procedure**, propose a skill and let explicit review promote it.

For API provider discoveries, "procedure" means more than a tool sequence. The
skill should include the official source used, base URL, auth scheme, endpoint
groups, required path/query/body parameters, destructive-operation gates, exact
read-only examples, expected response fields, and a smoke test. Never include a
raw token; declare `requirements.env_inject` or reference a typed integration.

## Sandbox

- **Per-skill secret injection (B.3)** — the manifest's `[requirements.env_inject]` map decides which entries from `~/.captain/secrets.env` cross over and under which target name. Other secrets stay invisible. A skill that asks for `OPENAI_API_KEY` will only see it if its manifest explicitly lists it.
- **env_clear whitelist (B.3 + B.1)** — every `skill_execute` spawn goes through `env_sandbox::apply_minimal_env` (PATH, HOME, TMPDIR/TMP/TEMP, LANG/LC_ALL, TERM) plus the manifest-declared secrets. The daemon's API keys never leak by accident.
- **Workspace isolation** — each skill runs with `current_dir` set to its own directory under `~/.captain/skills/<name>/`. File reads and writes outside that directory require an explicit Captain call from inside the skill (the skill cannot reach into the agent's workspace by itself).
- **`scaffold_skill` writes only to the agent's allowed workspace root** — the new SKILL.md lands in `skills/<name>/` below that workspace. The blocklist (`~/.ssh`, `~/.gnupg`) still applies.

## Limites

- The `skill_execute` runtime is decided by the manifest's `runtime.type` field: `python`, `node`, `shell`, `wasm` (not yet implemented), or `prompt_only` (no spawn — the body is injected into the system prompt). Mismatches return `RuntimeNotAvailable` rather than auto-fall-back.
- Each skill's child process inherits the daemon's `cwd = skill_dir` and the (B.3) injected env. There is **no** sudo escalation, no agent-forwarded SSH, no shared token cache: skills are isolated from each other at the env boundary.
- `scaffold_skill` does not run `skill_install` — it writes a draft that must still be implemented and discovered/reloaded by the skill registry.
- Proposals queued by the reflection pipeline live for 7 days then auto-expire. After expiry the LLM has to propose again from scratch.
- Skill refinements are durable review items. A `pending` item is not permission to edit; an `approved` item is permission to patch; `applied` means the patch/test already happened.
- The `pip_install` allowlist that gates `execute_code` does **not** apply to skills that bring their own dependency files. Keep dependencies minimal and prefer standard library/tool APIs.
- A scaffold is not the same as a finished integration. If credentials, rate limits, retries, or parsing matter, implement and test those before relying on `skill_execute`.

## Exemples

### Golden path — execute, then capture into a skill

```
1. shell_exec({"command": "curl -s https://api.example.com/slots | jq '.next'"})
   → "2026-05-02T08:00:00"
2. (after the third manual run, recognise the pattern)
   scaffold_skill({
     "name": "next-slot-finder",
     "description": "Returns the next available class slot from example.com",
     "capabilities": ["check_slot"]
   })
   → "Skill 'next-slot-finder' scaffolded at <workspace>/skills/next-slot-finder/SKILL.md..."
3. (next time the user asks)
   skill_execute({
     "skill": "next-slot-finder",
     "tool": "check_slot",
     "input": {}
   })
```

### Error case — tool self-approval

```
skill_proposal_decide({"id": "8a1b2c3d", "approve": true})
→ Err("skill_proposal_decide approve=true requires explicit human/API/channel approval after external validation; tool calls may only use approve=false.").
```

The approval guard prevents the agent from materialising self-evaluated or stale context as a permanent skill. Expiry or stale-context checks still belong on the human/API/channel promotion path.
