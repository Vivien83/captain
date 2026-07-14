# Project family

> **Status:** audited (D.15).
> See [`README.md`](README.md) for the index and drift policy.
> Tool name list pinned in [`captain_runtime::captain_docs::PROJECT_FAMILY_TOOLS`](../../crates/captain-runtime/src/captain_docs.rs).

## Tools

The project family gives Captain durable structure for long-running work: a project record, task rows, milestones, progress snapshots, and explicit checkpoints. Use it when a user goal will span multiple turns, sessions, or agents.

### Projects

- `project_create({name, goal?, status?})` — create a durable project and return its id/slug.
- `project_list({include_archived?, query?})` — list known projects, newest
  first, in a compact agent-safe projection. Use `query` when the user gives a
  partial name, slug, or reference such as `projet1`.
- `project_get({id_or_slug})` — inspect one project with its current metadata.
- `project_archive({id_or_slug})` — hide a completed or abandoned project without deleting history.
- `project_delete({id})` — permanently delete a project and its goals. Not reversible; confirm explicitly with the user first. Prefer `project_archive` unless the user asks for permanent removal.
- `project_resume({id_or_slug})` — reactivate an archived or paused project.

Decision rule: create or resume a project when the user is not asking for a one-shot answer, but for an outcome Captain should keep context for.

CORE visibility rule: `project_list` and `project_get` are read-only CORE tools
because project continuity is part of Captain's memory surface. Mutating project
tools remain outside CORE and should be discovered or used only when the user
actually asks to change project state.

Continuity rule: resolve user references against durable project slugs and
names before treating a number as a menu choice. For example, `projet1` can be
the project slug/name, not option 1. Ask `project_list({"query":"..."})` first
when the reference is partial, then `project_get` or `project_resume` once the
slug is known. Do not start by searching the filesystem/workspace for a project
status question when the project is already visible in the Projects store.
`project_list` intentionally returns a compact projection with identity, goal,
status, runtime state, worker counts, progress, and next actions so the result
remains usable in Telegram/API contexts without being swallowed by context
compaction.

For common status wording such as `où en est le projet ...`, the kernel may
answer directly from recent Projects state before invoking the LLM. This is
intentional: durable project status should not depend on the model choosing the
right tool over filesystem probing.

The runtime prompt also injects a bounded `Recent Projects` section for recent
non-terminal projects. This is a continuity rail, not a replacement for
MemPalace: project state remains sourced from the Projects store, while
MemPalace keeps durable learned facts and memories.

Every development project should follow the project lifecycle:
`OBSERVE -> THINK -> PLAN -> BUILD -> EXECUTE -> VERIFY -> LEARN`. The web
Projects surface and REST project launch route persist this lifecycle and seed a
task for each phase.

Development projects also have a live runtime record stored in project metadata
under `runtime` with protocol `captain.project_runtime.v2`. It includes the run
status, current lifecycle phase, progress, manager agent, same-provider
parallelism policy, real worker/sub-agent state, worker results, and a bounded
operational timeline. Use `/api/projects/{id}/runtime` to read it and
`/runtime/start`, `/runtime/pause`, `/runtime/resume`, or `/runtime/takeover`
to control a live run from the web UI, TUI, or external tooling.
The runtime response also includes `operator_status`: a compact, secret-free
status block for operators and integrations. It reports whether the run is
`running`, waiting for a user answer, ready to resume after an answered question
or approved tool request, stale after restart, paused, blocked, failed, done, or
ready. It includes worker counts, the last runtime event, pending question
count, resume reason, and concrete API actions such as answering a question or
resuming a stale run.
The global `/api/status` workload also aggregates projects that need operator
attention under `workload.projects.attention`, and `captain status` prints them
as `Project Attention`. Attention rows are priority-sorted before the bounded
status list is returned, so pending user answers, tool requests, resume-ready
projects, and stale active runtimes stay visible first. `attention_count`
remains the full count even when only the top rows are returned. If an older
daemon omits that count and returns more than eight rows, the CLI still reports
how many rows are hidden locally. In verbose mode, the CLI prints the first
concrete action endpoint and its `body_hint` payload, so operator answers and
tool decisions can be prepared directly from status. Pending questions also show
a bounded question preview and the first options before the answer action.
Pending or denied tool requests show the phase, tools, reason or denial reason,
and repeated-denial marker before the tool-request or resume action. The first
action reason is printed as a bounded `reason:` line so the operator can see why
that endpoint is recommended. The API `operator_status` itself also uses
allowlisted question and tool-request views: it keeps the fields needed for an
operator decision and omits stored answers, agent/run/worker ids, nested
previous requests, runtime metadata, raw payloads, paths, tokens, and secrets.
Its top-level runtime status, phase, resume reason, worker-count buckets, last
event and action body hints are also finite and bounded, so malformed runtime
metadata cannot turn status into an open-ended payload surface.
The `/api/projects/{id}/runtime` response itself is projected the same way:
`project`, `runtime`, and transcript events are allowlisted for operator use.
It omits raw project metadata, workspace/runtime blobs, worker prompts or task
bodies, stored question answers, event `data`, worker result payloads, paths,
tokens, and secrets.
The runtime payload no longer embeds a project-chat `agent_id`; project chat
continues through the normal web/agent chat routing instead of widening this
read-only runtime surface.
General project list/detail responses use the same allowlist: they keep project
identity, lifecycle, derived source/workspace status, counters, and the
sanitized runtime preview, but do not return the raw `metadata` object or raw
runtime snapshot.
Direct project creation normalizes input before durable storage. `POST
/api/projects` trims and bounds `name`, trims and allowlists `slug`, and trims
and bounds optional `goal`. Invalid values return static validation errors and
do not create a partial project.
Direct project edits follow the same boundary on input. `PATCH
/api/projects/{id}` accepts only typed operator fields (`name`, `goal`,
`status`, `deadline`) and rejects free-form `metadata`; runtime/source/workspace
metadata is managed by the dedicated launch and runtime endpoints. Those typed
fields are normalized before storage: `name` and `goal` are trimmed and reject
empty or oversized values, and invalid `status` returns a static allowlisted
error rather than echoing request text.
Direct project goal create/update routes normalize input before durable
storage too. Goal `id`, `name`, `description`, `check_command`, and
`recovery_command` are trimmed and bounded; blank recovery commands clear the
recovery command, and invalid values return static validation errors without
creating or mutating a partial goal. Command bodies remain internal operational
data for the goal runner and are not echoed through operator-safe project views.
Project goal path ids use the same boundary. `goal_id` values on update,
pause, resume, and delete are trimmed and validated with the creation id rule
before any goal-store lookup. Invalid ids return a static validation error, and
missing or cross-project goals return `project goal not found` without echoing
the submitted goal id.
Task updates apply the same status boundary. `PATCH /api/project-tasks/{id}`
accepts only `todo`, `doing`, `blocked`, `review`, `done`, or `cancelled`; an
invalid status returns a static validation error and leaves the task unchanged.
Task create/update text fields are normalized before durable storage: `title`
is trimmed, required and capped, while `description` is trimmed and capped.
Invalid task text returns a static validation error and leaves an existing task
unchanged.
Project task identifiers use the same boundary before mutation. `task_id` path
values on update/delete and `parent_id` values on create/update are trimmed,
bounded, and allowlisted before any task store call. Invalid ids return a
static validation error, and missing tasks return `project task not found`
without echoing the submitted id.
Task collection project ids use the same boundary before list/create store
access. Invalid project ids return a static validation error without echoing the
submitted id, path, or token text, and invalid creates do not leave partial task
rows.
Milestone creation follows the same input boundary. `name` is trimmed, required
and capped; `deliverables` are trimmed, empty entries are ignored, and
size/count violations return static validation errors without creating a partial
milestone.
Project milestone path ids use the same boundary before store access. Project
ids on milestone list/create/progress and milestone ids on complete are trimmed,
bounded, and allowlisted. Invalid ids return a static validation error, and a
missing milestone returns `project milestone not found` without echoing the
submitted id.
Direct checkpoint creation is summary-only for HTTP operators. `summary` and
`session_id` are trimmed and bounded, and any non-empty `state` payload is
rejected; structured resume state is written only by internal runtime
checkpoints. Checkpoint project ids on list/create are trimmed, bounded, and
allowlisted before store access, and list `limit` must be an integer from 1 to
100. Invalid values return static validation errors without echoing submitted
ids, paths, tokens, or limit text.
Runtime Project tools use the same small boundary before kernel access. Tool
inputs for `project_*`, `project_task_*`, `milestone_*`, and `checkpoint_save`
trim and bound ids, slugs, task statuses, and short text fields; invalid values
return static errors without echoing raw paths or tokens and without creating
partial project, task, milestone, or checkpoint rows.
Lifecycle phase changes are also allowlisted to `observe`, `think`, `plan`,
`build`, `execute`, `verify`, or `learn`; an invalid phase returns a static
validation error and leaves existing lifecycle metadata unchanged.
The Projects web page follows that boundary too: it does not read legacy
`project.metadata` fallbacks for lifecycle, runtime, or worker task text. During
a daemon/front-end version skew, the page prefers an empty or default view over
re-opening raw metadata blobs.
Project goal views expose only whether check/recovery commands are configured,
not the command bodies. Web goal editing preserves configured commands when the
operator leaves command prompts blank, replaces them only when a new command is
entered, and asks before clearing a configured recovery command.
`/api/projects/environment` is also path-free: it exposes platform/source
readiness only, not `workspaces_dir` or the default project root. The Projects
web page no longer calls it; leaving the local folder empty lets the server use
its configured default without putting the absolute path in the browser.
The GitHub repository list used by Projects is also allowlisted. It exposes
repository identity, privacy, default branch, and update time only; clone URLs,
browser URLs, SSH URLs, and git transport URLs stay out of the browser. When a
GitHub project launches, the web page sends `github_full_name` and branch, and
the server derives the HTTPS clone URL internally when it needs one. The launch
path also validates `github_full_name` as a strict `owner/repo` identifier,
ignores legacy `github_clone_url` values, and stores only bounded source
metadata instead of raw GitHub payloads.
GitHub account status follows the same boundary: Projects receives only the
account login and bounded id, not profile URLs, names, emails, avatar URLs,
plans, or other raw GitHub user fields. Repository listing failures return a
status error without echoing GitHub's raw response body.
The in-process `captain status` fallback applies the same finite projection when
it reconstructs Project Attention from persisted runtime metadata during daemon
upgrades or downtime.
Verbose status also shows runtime progress and worker status counts when
available. The CLI also exposes the matching
operator actions as `captain project list|status|workers|questions|replay|context|task|milestone|goal|timeline|checkpoints|answer|tool-request|start|resume|pause|takeover`,
so the endpoint/body shown by status can be discovered, inspected, or executed
without hand-written curl. Runtime status and replay share the same action
formatter: if the API exposes multiple operator actions, for example `pause`
and `takeover`, the CLI prints each next command and prefers the project slug
over the internal id when available. Dynamic project ids, ask ids, and runtime
phases in printed next commands are shell-quoted, matching the copyable-command
guard used by the TUI, so a weird runtime identifier cannot change the command
boundary. Runtime `--json` output is also projected
through an operator-safe view: project identity, compact runtime status,
attention details, last event, command result flags, and next commands are kept,
while raw runtime metadata, worker prompts/tasks, transcripts, stored answers,
chat agent ids, workspace paths, tokens, and secrets are omitted. `captain
project list --attention` prints only the projects likely to need operator
review, using a sanitized projection rather than raw metadata or workspace
paths.
`captain project context <project>` prints the durable Hermes-style reprise
context without starting the runtime: project id/slug/status, latest checkpoint,
bounded tasks, bounded goals, milestone progress, and next CLI actions. It reads
`/api/projects/{id}/resume`, accepts id or slug, and sanitizes the output so raw
checkpoint state, task descriptions, goal commands, metadata, workspace paths,
tokens, and secrets are not printed. Its next actions now point to the full
reprise loop: `replay`, `status`, `workers`, `questions`, `timeline --follow`,
and `checkpoints`.
The API resume response itself is also allowlisted: checkpoint `state`, task
descriptions/assignees/metadata, goal descriptions/check/recovery commands,
recent checks, suggestions, and milestone deliverable payloads are not returned
by `/api/projects/{id}/resume`.
The direct project resource APIs use the same operator-safe boundary. Task,
goal, milestone, milestone progress, checkpoint, and project launch responses
keep only ids, status, counters, bounded names/summaries, and timestamps; they
do not return task descriptions, assignees, goal commands, recent check logs,
suggestions, milestone deliverables, checkpoint `state`, raw launch payloads, or
`rules_file` paths.
Project list/detail and runtime project views also omit workspace/source paths:
`workspace_path`, `workspace.path`, `workspace.default_root`, `source.path`, and
`source.local_path` remain internal runtime metadata rather than API/web display
fields. The Projects page shows workspace readiness and repository identity
instead of local filesystem paths.
`captain project workers <project>` prints the live runtime worker/sub-agent
state from `/api/projects/{id}/runtime`, with optional `--phase` filtering. It
exposes only project state, worker id/role/phase/status, agent id, tool-name
counts, timing, cleanup state, bounded summary, and requested tool names. It
never prints worker prompts, phase task bodies, dependencies, raw tool inputs or
outputs, tool request reasons, event `data`, runtime metadata, workspace paths,
tokens, or secrets.
`captain project questions <project>` prints pending project `ask_user`
questions from the same runtime endpoint, with optional `--phase`, `--all`, and
`--json`. It shows the bounded question, bounded options, phase, worker role,
status, delivery/timing fields, and the exact `captain project answer` next
command for pending questions. It never prints stored answers, agent ids, run
ids, worker ids, raw timeline payloads, runtime metadata, workspace paths,
tokens, or secrets.
`captain project replay <project>` prints a bounded Hermes-style reprise
capsule from `/api/projects/{id}/runtime`. It combines runtime state,
transcript/session counters, recent transcript events, worker summaries, pending
questions with answer commands, and next operator actions. It accepts
`--events`, `--workers`, and `--json`, and omits raw event `data`, worker
prompts, phase task bodies, dependencies, raw tool payloads, stored answers,
agent/run ids, runtime metadata, workspace paths, tokens, and secrets.
`captain project timeline <project> --follow` keeps that operator-safe timeline
open and prints only new runtime events as they arrive. It polls the runtime
transcript, deduplicates by event id or stable event fields, and keeps the same
sanitization as the bounded timeline view. Live follow is text-only; omit
`--follow` when a one-shot `--json` timeline is needed.
`captain project task list|create|update|delete` wraps the durable project task
API for Hermes-style execution state. `list` resolves a project id or slug,
supports status filtering, and prints bounded task identity/status/title only.
`create` and `update` send the requested description, parent, priority or
status to the daemon but echo only sanitized task fields; `delete` requires
`--yes`. The CLI never prints task descriptions, assignee ids, metadata, or
other free-form task payloads by default.
`captain project milestone list|create|complete|progress` wraps the durable
milestone API for outcome checkpoints. `list`, `create`, and `progress` resolve
a project id or slug; `complete` operates on the milestone id. The CLI echoes
only milestone identity, bounded name, status, due/completion times,
deliverable count, and aggregate progress, never raw deliverable text,
metadata, workspace paths, tokens, or secrets.
`captain project goal list|create|update|pause|resume|delete` wraps the
project-scoped continuous goal API for Hermes-style project monitoring. It sends
check and recovery commands to the daemon when creating or updating goals, but
text and JSON output only echo safe operator fields: id, bounded name, status,
interval, failure counters, LLM budget, and timestamps. The CLI never prints raw
goal descriptions, check commands, recovery commands, escalation targets, recent
checks, suggestions, logs, metadata, paths, tokens, or secrets. `delete`
requires `--yes`.
`captain project checkpoints <project>` prints the recent durable checkpoint
history that Hermes-style reprise depends on. The command accepts an id or slug,
resolves it through the runtime project endpoint, then reads the checkpoint
history and returns only id, created time, session id, and bounded summary. It
never prints raw checkpoint `state`, workspace paths, tokens, or metadata.
The same CLI also exposes durable lifecycle cleanup with
`captain project archive <project>` and `captain project unarchive <project>`.
`unarchive` restores the project record to `active` without starting the live
runtime; use `captain project start` or `resume` separately when runtime work
should continue. JSON output for those lifecycle commands is sanitized to id,
slug, name, status, update time, and next action, not raw metadata or workspace
paths.
The same aggregate feeds
`/api/status.consciousness`
project counters so repeated tool denials, stale runtimes, failed phases and
resume-ready projects also appear as operational awareness signals. Agent runs
also receive a compact `[OPERATIONAL AWARENESS]` project summary from persisted
runtimes so they can prioritize those blockers without seeing raw project
transcripts.

Runtime timeline entries are operational summaries: decisions, tool actions,
blockers, worker state, verification outcomes, and learning candidates. They
are not hidden chain-of-thought. V2 workers are actual child agents spawned by
Captain. `OBSERVE` and `THINK` can run in parallel; `PLAN`, `BUILD`, `EXECUTE`,
`VERIFY`, and `LEARN` are dependency-gated. A completed worker result is the
source of truth for whether a phase actually ran. Worker summaries should be
short handoff blocks (`STATUS`, `SUMMARY`, `CHANGED_FILES`, `VERIFY`, `NEXT`);
if a provider returns raw tool transcripts, Captain stores a readable fallback
summary generated from execution metadata instead of leaking the transcript into
the project surfaces.

Runtime workers execute in the real project workspace so shell and file tools
operate against the repo/folder the user selected. Captain does not scaffold
identity files inside that project workspace for runtime workers. If a paused
run is resumed, completed phases are skipped, stale `running` workers are
recovered, and blocked/failed phases require manual review or a fresh start.
After a phase completes successfully, Captain stops the child worker agent after
storing its result. The runtime keeps the `agent_id`, `summary`, usage metadata,
`cleanup_status`, and timeline evidence for traceability, but the worker should
not remain visible as an active daemon agent. Blocked or failed workers are
retained for review instead of being cleaned up automatically.

Each runtime worker has a concrete `tool_allowlist` and matching
`capabilities.tools`; the authorized list is also stored as worker metadata.
Workers always keep `capability_search`, `skill_search`, `tool_search`,
`captain_docs`, and `system_time` for discovery and recovery. If a phase needs
another tool, the worker must stop with `STATUS: blocked`, `TOOL_REQUEST`, and
`REASON` so Captain can approve or deny the extension explicitly. Runtime
operator status exposes that case as `tool_request_pending` with the requested
tools, reason, phase, and resume action instead of hiding it behind a generic
blocked state.
Respond with `POST /api/projects/{id}/runtime/tool-request` using
`decision:"approve"` or `decision:"deny"` and the phase. Approval marks the
phase `resume_pending`, adds the approved tools to the next worker allowlist for
that phase, and requires a normal runtime resume; `Start` also honors that
resume marker instead of resetting the run. Operator status and `captain status`
then describe the resume as an approved tool request, not as a stored user
answer. Denial keeps the phase blocked with a recorded decision, and operator
status reports `tool_request_denied` with the denied tools and decision reason
instead of collapsing it into a generic blocked run. When a worker is resumed
after review, its prompt includes the prior approval/denial decisions so it can
avoid re-requesting a denied tool and choose another path or return the
smallest manual next action. `Resume` also reopens the denied phase as ready so
the worker can actually relaunch with that context instead of remaining stuck
behind the old blocked worker status. If the relaunched worker still asks for
the same denied tool, Captain keeps the request denied, records a
`worker.tool_request.denied_repeat` timeline event, and does not create a fresh
operator approval prompt. Operator status and `captain status` surface that
case as a repeated denial so the next action is review/alternate plan, not
approving the same tools again by reflex.

Development projects are workspace-backed. `/api/projects/launch` accepts
`source_type:"local"` with a `local_path`/`repo_path`, or
`source_type:"github"` with `github_full_name`, branch and local path. Older
API clients may still send an explicit `github_clone_url`, but Captain ignores
that value for project launch and derives its own HTTPS clone URL from the
validated repository identity. If no path is supplied, Captain uses the
configured `workspaces_dir`, falling back to the Captain home workspaces
directory; never assume a macOS/Desktop path. GitHub projects are cloned into
that workspace when possible, then represented as a normal project with source
metadata and workspace path.
Launch input is validated before any workspace folder is prepared or project
row is created. `goal`, optional `name`/`slug`, branches, `autonomy_level`,
acceptance criteria, and optional goal guard commands are trimmed and bounded.
Dangling guard recovery/interval values, unsafe guard commands, and oversized
criteria return static validation errors without creating a partial workspace
or project.
Active project selection has the same input boundary. `/api/active-project/{agent_id}`
and the `/project <slug>` slash command trim and bound the agent id and slug,
allow only project slugs that match the stored project slug format, and update
the active-project registry only after the project resolves. Invalid values and
missing projects return static messages rather than echoing the raw slug.
Direct project path identifiers use the same rule. Project `id_or_slug` values
on list/detail mutation, runtime, answer, and tool-request routes are trimmed,
bounded, and limited to lowercase letters, digits, and hyphens before any
lookup. Invalid identifiers fail with a static validation error, and missing
projects return `project not found` without echoing the submitted path segment.

GitHub project sources use `GITHUB_TOKEN` from the Captain secret store. The web
Projects page exposes GitHub setup directly: it can save, validate, list account
repositories, and disconnect the token through the `/api/projects/github/*`
routes. Do not store GitHub tokens in project metadata, checkpoints, memory, or
docs.

Long-running `goal_create` goals can be scoped to a project with `project_id` or
`project_slug`. Use project-scoped goals for health checks, delivery guards, and
other background objectives tied to one project. Keep global goals only for
system-wide objectives.

The web/API project surface supports the full project-goal lifecycle:
create, pause, resume, edit, and delete. Editing a project goal goes through
`PATCH /api/projects/{id}/goals/{goal_id}` and reuses the normal goal
validation path, including critical-command refusal and minimum interval
checks. When the check command changes, Captain resets the goal's consecutive
failure counter because the old failure streak no longer describes the new
guard.

Project goal checks run from the project's workspace when the goal is scoped to
a project. Prefer concise relative commands such as `test -f main.py` or the
project's own test command; Captain resolves them against the recorded project
workspace instead of the daemon's current directory. When a paused or escalated
goal is corrected and becomes active again, Captain starts a fresh goal loop so
the guard resumes without requiring a daemon restart.

### Project tasks

- `project_task_create({project_id, title, description?, status?})` — add a task to a project.
- `project_task_list({project_id, status?})` — list tasks for planning or handoff.
- `project_task_update({task_id, status?, title?, description?})` — mark progress or correct stale task text.

Use tasks for execution state, not memory facts. If the user corrects durable knowledge, use `memory_forget` / `memory_save` instead.

### Milestones and checkpoints

- `milestone_create({project_id, title, due_at?})` — create an outcome checkpoint.
- `milestone_list({project_id})` — inspect planned milestones.
- `milestone_complete({milestone_id})` — mark a milestone done.
- `milestone_progress({milestone_id, note})` — append progress without completing it.
- `checkpoint_save({project_id, title?, summary})` — persist a high-signal state snapshot after meaningful work.

Checkpoints are Captain's compact memory for project state: decisions made, files touched, blockers, verification status, and next concrete step.

Captain also runs a proactive milestone alert loop at kernel boot. Any active
project milestone due in less than 24 hours and not completed is sent once to
the configured Telegram `default_chat_id`; the sent marker is stored in
structured memory so daemon restarts do not spam the user. If Telegram is not
configured or the adapter is inactive, the alert is skipped and retried on the
next scan instead of being marked as delivered.

## Sandbox

- Project data lives in Captain's kernel store under `~/.captain`; tools validate ids/slugs through the kernel rather than letting the model write raw state files.
- Project launch authorizes the chosen workspace for the principal Captain agent through the same `workspace_add` path rail when it is outside Captain home. If authorization fails, the project still records the path and the error so the user can decide what to grant next.
- Older project records may only contain a path. If Captain cannot read that folder, call `workspace_add` with the user-approved path first.
- Checkpoints should summarize facts already visible in the session or project state. Do not use them to stash secrets, raw tokens, or copied private logs.

## Limites

- Project status is not a scheduler. For autonomous repeated work, combine a project with `goal_create` or `cron_create`.
- Task status values are finite; invalid statuses return a validation error. Read the error and retry with the allowed value rather than inventing a new label.
- `project_archive` is reversible via `project_resume`; it is not a hard delete. `project_delete` is the hard delete and cannot be undone — use it only when the user explicitly asks for permanent removal.
- Checkpoints are only useful if concise. Large transcripts belong in session history; checkpoints should fit in a skim.
- Milestone progress notes do not update task status automatically. Update the related task separately when needed.

## Exemples

### Golden path — structure a multi-session fix

```
project_create({
  "name": "Improve tool autonomy",
  "goal": "Make Captain recover from tool failures without giving up."
})
→ {"id":"p-...","slug":"improve-tool-autonomy"}

project_task_create({
  "project_id": "p-...",
  "title": "Audit core tools for recovery docs"
})

checkpoint_save({
  "project_id": "p-...",
  "summary": "Core prompt now has failure_recovery and all core tools have WHEN/WHY/SKIP docs. Tests: prompt_builder passed."
})
```

### Error case — project folder inaccessible

```
project_get({"id_or_slug":"example-service"})
→ {"workspace":"/srv/example-service"}
file_read({"path":"/srv/example-service/README.md"})
→ Err("path outside workspace")
workspace_add({"path":"/srv/example-service"})
```

The project record tells Captain where to work, but the sandbox grant is still explicit.
