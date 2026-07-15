struct ToolDoc {
    name: &'static str,
    doc: &'static str,
}

const TOOL_DOCS: &[ToolDoc] = &[
    ToolDoc { name: "file_read", doc: "\
WHEN: inspect exact file contents before editing, explaining, or debugging.
WHY: typed sandboxed read; safer than shelling out to cat.
SKIP: directory listings (file_list/glob) or huge generated outputs better summarized by grep."
    },

    ToolDoc { name: "memory_store", doc: "\
    WHEN: low-level scratch state or legacy key-value compatibility is explicitly needed.
    WHY: writes a flat key/value record; kept for old skills and temporary coordination.
    SKIP: durable facts, preferences, corrections, or lessons — use memory_save for MemPalace long-term memory."
    },

    ToolDoc { name: "memory_save", doc: "\
WHEN: store a structured durable fact after the user confirms or clearly states it.
WHY: commits first to Captain's durable local journal, then synchronizes the MemPalace semantic index.
SKIP: speculative inferences, temporary task notes, or replacements whose exact old triple has not been retracted with memory_forget first."
    },

    ToolDoc { name: "memory_recall", doc: "\
WHEN: before starting any task, when user references past conversation, when you need context.
WHY: semantic search — you don't need the exact key; content similarity is enough.
SKIP: trivial acknowledgements, obvious greetings, cases where the answer is in the current turn."
    },

    ToolDoc { name: "memory_context_batch", doc: "\
WHEN: a user references earlier exchanges, asks for remembered decisions, or the answer needs several remembered facts/sessions.
WHY: read-only grouped recall across durable memory and prior-session summaries; higher signal than guessing from filesystem state.
SKIP: single exact durable fact where memory_recall is enough, or current-turn facts already visible in context."
    },

    ToolDoc { name: "memory_forget", doc: "\
WHEN: user corrects a stored fact, says to forget something, or memory is clearly obsolete.
WHY: preserves the audit trail, removes the fact from active context, and queues a durable MemPalace invalidation.
SKIP: broad wipes. Prefer the exact old subject/predicate/object; after success, save a replacement only when the user supplied one."
    },

    ToolDoc { name: "project_list", doc: "\
WHEN: user asks about a project, mentions a project name/slug, asks where work stands, or says refs like projet1.
WHY: durable Projects store is the source of truth for project identity/status/runtime before filesystem search.
SKIP: non-project questions or once project_get already supplied the needed exact project state."
    },

    ToolDoc { name: "project_get", doc: "\
WHEN: you have a known project slug/id and need exact durable project details before answering or resuming.
WHY: reads one Projects record from the source of truth without exposing raw metadata.
SKIP: ambiguous project references; call project_list with query first."
    },

    ToolDoc { name: "file_list", doc: "\
WHEN: discover direct children of a known directory.
WHY: cheap typed listing through the sandbox.
SKIP: recursive search (glob/grep) or reading file contents (file_read)."
    },

    ToolDoc { name: "edit_file", doc: "\
WHEN: make a small targeted replacement in an existing file.
WHY: preserves unrelated content and fails closed if context is ambiguous.
SKIP: new files or whole-file rewrites (file_write), complex multi-hunk patches (apply_patch)."
    },

    ToolDoc { name: "multi_edit", doc: "\
WHEN: apply several exact replacements to one file as one logical change.
WHY: keeps related edits atomic and avoids repeated read/write churn.
SKIP: uncertain context, generated rewrites, or edits spanning multiple files."
    },

    ToolDoc { name: "apply_patch", doc: "\
WHEN: perform precise multi-file code edits with reviewable hunks.
WHY: best for code changes; exact context prevents accidental drift edits.
SKIP: binary files, generated output, or cases where a typed domain tool exists."
    },

    ToolDoc { name: "grep", doc: "\
WHEN: search file contents for a string, symbol, error, or TODO.
WHY: fast evidence gathering before reading whole files.
SKIP: file-name discovery (glob) or semantic memory/knowledge lookup."
    },

    ToolDoc { name: "glob", doc: "\
WHEN: find files by path/name pattern before reading or editing.
WHY: cheaper than broad shell find and stays inside the workspace.
SKIP: searching inside files (grep) or when the path is already known."
    },

    ToolDoc { name: "agent_send", doc: "\
WHEN: an agent is already running and you need its expertise or response on a sub-task.
WHY: synchronous message with response — use for delegation with result expected.
SKIP: if no such agent exists (use agent_spawn first) or the task is single-tool. \
Check agent_list first."
    },

    ToolDoc { name: "agent_list", doc: "\
WHEN: before delegating, reusing a worker, or checking running background agents.
WHY: prevents duplicate workers and reveals existing capabilities.
SKIP: direct single-tool work that does not benefit from another agent."
    },

    ToolDoc { name: "agent_spawn", doc: "\
WHEN: multi-step task (>3 tool calls), repetitive loops, or work that deserves a cheaper model.
WHY: isolation — child has fresh context, cheaper model, doesn't pollute your cycles.
SKIP: single-call tasks, conversational turns, tasks where you already have the context loaded. \
Prefer reusing an existing worker via agent_list before spawning."
    },

    ToolDoc { name: "ask_user", doc: "\
WHEN: the next step needs user preference, missing private info, or permission.
WHY: avoids guessing when ambiguity is genuinely user-owned.
SKIP: tool docs, current state, memories, config, or recoverable errors you can inspect yourself."
    },

    ToolDoc { name: "knowledge_query", doc: "\
WHEN: checking what you already know about a person, org, project, concept, or location.
WHY: graph-backed semantic + entity search — faster and cheaper than web_search for known facts.
SKIP: if the fact is current-events (use web_search) or transient state (use memory_recall)."
    },

    ToolDoc { name: "knowledge_add_entity", doc: "\
WHEN: you discover a new entity (person, org, project, service, tool, location) worth tracking.
WHY: builds the long-term knowledge graph — compounds value across sessions.
SKIP: if entity exists (query first to avoid duplicates) or is trivially re-discoverable."
    },

    ToolDoc { name: "shell_exec", doc: "\
WHEN: inspecting the live system (date, OS, git, processes, files), running one-shot commands.
WHY: ground truth — your training data is stale, the shell reflects current reality.
SKIP: if a typed API tool exists (file_read over cat, file_list over ls, web_fetch over curl). \
Avoid for long-running processes (use process_start), anything requiring a raw secret literal, \
or sourcing `~/.captain/secrets.env`. Use secret_read/native integrations/skill env_inject for credentials."
    },

    ToolDoc { name: "execute_code", doc: "\
WHEN: run a short Python, Node, or Bash snippet for computation or parsing.
WHY: structured code execution with timeout and streaming output.
SKIP: long-running servers (process_start), host inspection better done with shell_exec, \
or API calls that require embedding a raw key. Use a native tool or a skill with env_inject."
    },

    ToolDoc { name: "ssh_exec", doc: "\
WHEN: operate on a remote host through a stored SSH vault alias.
WHY: native SSH uses Captain's vault and known-host checks; no shell ssh config guesswork.
SKIP: local commands, unknown aliases without first using docs/error hints, or destructive remote ops."
    },

    ToolDoc { name: "ssh_upload", doc: "\
WHEN: copy a small local workspace file to a remote host via a vault alias.
WHY: native SFTP keeps key material inside Captain.
SKIP: large transfers, missing remote directories, or files outside authorized workspace roots."
    },

    ToolDoc { name: "ssh_download", doc: "\
WHEN: fetch a small remote file into the workspace for inspection or editing.
WHY: native SFTP avoids exposing private keys to shell commands.
SKIP: large logs/datasets; use remote shell filtering or rsync-style workflows instead."
    },

    ToolDoc { name: "file_write", doc: "\
WHEN: creating or replacing a file whose content you fully control.
WHY: atomic write — safer than echo >/cat redirection.
SKIP: partial edits on existing files (read first, then write), binary content, \
destination outside workspace_path without explicit user confirmation."
    },

    ToolDoc { name: "document_create", doc: "\
WHEN: user asks for a polished deliverable: PDF, DOCX, report, synthesis, invoice, memo, or brief.
WHY: native structured renderer creates the artifact directly and returns a shareable path.
SKIP: raw scratch notes (file_write), complex brand/layout publishing that needs a dedicated skill \
or external renderer, or sending the artifact (use channel_send after creation)."
    },

    ToolDoc { name: "document_extract", doc: "\
WHEN: read a downloaded PDF/report/text-like document before summarizing, comparing, or citing it.
WHY: extracts evidence from source files inside the workspace; prevents guessing from filenames.
SKIP: image-only/scanned PDFs without OCR, or creating deliverables (document_create/document_pipeline)."
    },

    ToolDoc { name: "file_delete", doc: "\
WHEN: explicitly asked to remove a file, or cleaning up temp artifacts you created.
WHY: irreversible — confirm scope matches exactly what the user authorized.
SKIP: deleting files you did not create or did not inspect. If user intent is ambiguous, ask."
    },

    ToolDoc { name: "web_search", doc: "\
WHEN: current events, library docs, prices, versions — facts that change over time.
WHY: breadth — better than a single URL when you don't know the source.
SKIP: for a specific known URL (use web_fetch directly) or facts already in knowledge/memory."
    },

    ToolDoc { name: "web_fetch", doc: "\
WHEN: you have a specific URL whose content you need verbatim.
WHY: renders to markdown, lighter than a browser, ideal for articles and API docs.
SKIP: if the page requires JS rendering (use browser_navigate) or you don't have a URL yet."
    },

    ToolDoc { name: "web_download", doc: "\
WHEN: a source is a PDF, CSV, report, dataset, or file that must be saved before analysis.
WHY: downloads with SSRF/size guards and returns a workspace path + checksum for follow-up extraction.
SKIP: ordinary HTML pages readable with web_fetch, or files whose contents you cannot inspect afterward."
    },

    ToolDoc { name: "cron_create", doc: "\
WHEN: task must recur on a schedule (daily, hourly, weekly) without user prompting.
WHY: delegates to the scheduler — you are not running continuously.
SKIP: one-shot delayed tasks (use process_start with sleep or a queued job), \
tasks that require live user context each time."
    },

    ToolDoc { name: "goal_create", doc: "\
WHEN: the user wants an ongoing objective Captain should pursue autonomously.
WHY: creates a supervised goal loop instead of relying on a single chat turn; checks can print \
CAPTAIN_PROGRESS=<token> to detect stalled convergence.
SKIP: simple reminders, one-shot tasks, or goals without a clear success or progress condition."
    },

    ToolDoc { name: "speech_to_text", doc: "\
WHEN: transcribe a user-provided audio file, voice note, or recording.
WHY: native STT keeps voice workflows inside Captain without guessing from filenames.
SKIP: non-audio files, already-transcribed content, or cases where the user asks for TTS."
    },

    ToolDoc { name: "text_to_speech", doc: "\
WHEN: the user asks Captain to produce spoken audio from text.
WHY: uses configured TTS directly and returns an audio artifact for sending or playback.
SKIP: transcription tasks (speech_to_text), long documents that need summarization first, or normal text replies."
    },

    ToolDoc { name: "channel_send", doc: "\
WHEN: proactively notify the user outside the current chat flow.
WHY: routes through configured channels and returns delivery status.
SKIP: normal replies in the active conversation, or when no recipient/default channel is configured."
    },

    ToolDoc { name: "channel_reconfigure", doc: "\
WHEN: channel config or secrets changed and one adapter must hot-reload.
WHY: restarts only the named channel and validates against live config.
SKIP: unrelated config edits or guessing channel names; fix typos using the error's known list."
    },

    ToolDoc { name: "config_read", doc: "\
WHEN: inspect Captain's live configuration before changing behaviour or diagnosing setup.
WHY: config is ground truth for enabled tools, channels, models, and workspace roots.
SKIP: secrets (secret_read) or external service state."
    },

    ToolDoc { name: "model_switch_plan", doc: "\
WHEN: the user wants to change Captain's main provider/model, or diagnose whether a switch is safe.
WHY: read-only rail that checks target availability, auth, tool support, active session risk, and migration strategy.
SKIP: direct config_write for default_model; for current model only, use config_read."
    },

    ToolDoc { name: "model_switch_apply", doc: "\
WHEN: model_switch_plan succeeded and the user chose a session strategy.
WHY: applies the global default model/provider through the migration rail, preserving or clearing context intentionally.
SKIP: no prior plan, missing explicit user choice, blocked auth/capability checks, or casual per-agent experiments."
    },

    ToolDoc { name: "codex_auth_status", doc: "\
WHEN: checking whether Codex OAuth is ready before selecting a codex/* model.
WHY: validates the Codex auth cache separately from OpenAI API-key auth.
SKIP: Anthropic/OpenAI API-key setup, or when model_switch_plan already reports Codex auth readiness."
    },

    ToolDoc { name: "codex_tool_probe", doc: "\
WHEN: validating that a real Codex OAuth model can emit structured tool calls before promoting it as Captain's main agent model.
WHY: catches the common Codex failure mode where the model narrates action but never sends a function call.
SKIP: casual auth checks; use codex_auth_status first when auth readiness is unknown."
    },

    ToolDoc { name: "codex_login_start", doc: "\
WHEN: Codex OAuth is missing or expired and the user wants to authenticate from the current chat.
WHY: starts the device-code login flow without asking the user to leave Captain.
SKIP: if Codex is already authenticated; use codex_auth_status first when unsure."
    },

    ToolDoc { name: "codex_login_poll", doc: "\
WHEN: after codex_login_start, to complete the device-code OAuth flow once the user has approved it.
WHY: finishes and persists the Codex token so model_switch_plan can validate codex/* targets.
SKIP: before login_start or after timeout without restarting the flow."
    },

    ToolDoc { name: "secret_read", doc: "\
WHEN: verify whether a required secret exists, without exposing its raw value.
WHY: returns masked presence so Captain can diagnose setup safely.
SKIP: when the user asks for the secret value; never reveal credentials."
    },

    ToolDoc { name: "secret_write", doc: "\
WHEN: the user provides a credential or token to store for later tool use.
WHY: keeps secrets in Captain's vault instead of memory, files, or prompts.
SKIP: guessed credentials, public config values, or secrets you cannot name precisely. \
After storing, never copy the raw value into a script/command; use env_inject or a native integration."
    },

    ToolDoc { name: "session_recall", doc: "\
WHEN: user references previous sessions or Captain needs past decisions/results.
WHY: searches persisted session checkpoints beyond current context.
SKIP: current-turn facts, durable user preferences (memory_recall), or project config."
    },

    ToolDoc { name: "workspace_add", doc: "\
WHEN: user asks Captain to access a new folder or project root.
WHY: expands the sandbox persistently with validation and protected-path blocks.
SKIP: protected locations, files outside user intent, or paths not needed for the task."
    },

    ToolDoc { name: "scaffold_skill", doc: "\
WHEN: the user asked for a skill, or a solved workflow has been visibly proposed and approved.
WHY: turns repeated manual tool sequences into a durable capability.
SKIP: one-off tasks, unclear workflows, unapproved global behaviour changes, or secrets not yet stored in the vault."
    },

    ToolDoc { name: "system_time", doc: "\
WHEN: scheduling, relative dates, or any answer depends on the daemon's current clock.
WHY: avoids stale model dates and timezone mistakes.
SKIP: when the current date/time is already explicitly provided in the prompt."
    },

    ToolDoc { name: "cron_update", doc: "\
WHEN: the user wants to modify an existing cron/reminder/report schedule.
WHY: preserves the job id, owner and execution history instead of cancel/create churn.
SKIP: deleting a job entirely (cron_cancel) or creating a new unrelated schedule (cron_create)."
    },

    ToolDoc { name: "skill_execute", doc: "\
WHEN: a loaded skill matches the task — prefer skill logic over ad-hoc tool calls.
WHY: skills encode proven workflows, API endpoints, and pitfalls the agent otherwise trips on.
SKIP: if no skill matches or the skill's prerequisites are not met. \
Never improvise a skill's procedure — call the skill."
    },

    ToolDoc { name: "skill_refinement_propose", doc: "\
WHEN: using a skill reveals a reusable improvement, missing precondition, error recovery, or version bump.
WHY: captures a controlled v1→v2→v3 refinement proposal, creates a rollback snapshot for file-backed skills, and surfaces approval without mutating the skill silently.
SKIP: one-off failures, unclear fixes, or changes involving secrets not stored in the vault."
    },

    ToolDoc { name: "skill_refinement_list", doc: "\
WHEN: before editing a skill or after self_improvement_review shows pending refinements.
WHY: keeps skill evolution deliberate and avoids duplicate proposals.
SKIP: creating a brand-new skill (use scaffold_skill / skill_proposal tools)."
    },

    ToolDoc { name: "skill_refinement_decide", doc: "\
WHEN: an explicit human/API/channel review rejects a proposed skill refinement; positive approval is not available from tool-call self-evaluation.
WHY: gates critical durable skill changes before file edits happen and avoids proxy-reward self-approval.
SKIP: approving your own proposal or applying the patch itself; after human approval plus patch/test, record completion with skill_refinement_update."
    },

    ToolDoc { name: "skill_refinement_update", doc: "\
WHEN: a refinement has been patched, tested, versioned, reported, or reclassified.
WHY: closes the feedback loop so Captain knows which skill improvement landed.
SKIP: approval decisions (skill_refinement_decide) or new proposals (skill_refinement_propose)."
    },

    ToolDoc { name: "skill_refinement_restore", doc: "\
WHEN: the user asks to roll back an applied skill improvement or a refinement made behaviour worse.
WHY: restores the file-backed skill from the automatic pre-improvement snapshot.
SKIP: generated new-skill proposals or skills without a snapshot."
    },

    ToolDoc { name: "mcp_setup", doc: "\
WHEN: the user needs a capability exposed by an MCP server not yet installed.
WHY: one-time setup — tokens cached in the credential vault for reuse.
SKIP: if the MCP server is already installed (check mcp list), or if a builtin tool covers it."
    },

    ToolDoc { name: "process_start", doc: "\
WHEN: launching a long-running process (REPL, dev server, watcher, daemon).
WHY: stays alive across multiple tool calls — you can process_poll and process_write.
SKIP: one-shot commands (use shell_exec). Always process_kill when done to free resources."
    },

    ToolDoc { name: "captain_docs", doc: "\
WHEN: unsure how a Captain tool behaves, an error needs recovery, or two tools overlap.
WHY: internal AI-facing manual — faster and safer than asking the user or guessing.
SKIP: external facts, user preferences, or live config values (use web_search, memory_recall, config_read)."
    },

    ToolDoc { name: "capability_search", doc: "\
WHEN: you need to decide which active capability/tool/skill/MCP/docs family should handle a task.
WHY: unified resolver across Captain's builtin tools, installed skills, connected MCP tools, and docs.
SKIP: when the exact visible tool is obvious, or when you already need the full docs body (captain_docs)."
    },

    ToolDoc { name: "skill_search", doc: "\
WHEN: the task matches a reusable procedure, project workflow, debugging, code review, release, or sub-agent handoff.
WHY: discovers SKILL.md guidance by family without loading every skill into the visible tool prompt.
SKIP: one-shot facts or exact builtin schemas (use tool_search/captain_docs/capability_search as appropriate)."
    },

    ToolDoc { name: "skill_view", doc: "\
WHEN: skill_search found a relevant candidate and you need the exact SKILL.md workflow before acting.
WHY: loads one skill's actionable context without flooding the prompt with the whole skill index.
SKIP: broad discovery (use skill_search), exact builtin schemas (tool_search), or Captain manual text (captain_docs)."
    },

    ToolDoc { name: "self_improvement_review", doc: "\
WHEN: after a long/tool-heavy task, repeated failure, Security blocked recovery, or user asks what Captain learned.
WHY: read-only dashboard of pending memory learnings, system bugs, and skill proposals with approval guidance.
SKIP: ordinary one-shot turns; do not use it as a substitute for doing the user's current task."
    },

    ToolDoc { name: "system_bug_report", doc: "\
WHEN: you identify a reproducible Captain product bug, missing capability, doc/tool mismatch, security gap, or repeated internal failure.
WHY: records the defect in a persistent, categorized self-diagnostic register.
SKIP: one-off user mistakes, external service outages, or raw logs containing secrets."
    },

    ToolDoc { name: "system_bug_list", doc: "\
WHEN: before fixing or reporting an internal Captain defect, or during self_improvement_review follow-up.
WHY: avoids rediscovering the same bug and shows status/category/severity.
SKIP: ordinary user-domain tasks unrelated to Captain's own system."
    },

    ToolDoc { name: "system_bug_update", doc: "\
WHEN: a known Captain bug has been verified, fixed, reported, deduplicated, or reclassified.
WHY: keeps the self-diagnostic register actionable instead of accumulating stale defects.
SKIP: creating a new defect (use system_bug_report)."
    },

    ToolDoc { name: "tool_search", doc: "\
WHEN: the needed Captain capability is not visible in the current tool list or schema context.
WHY: loads deferred builtin tool definitions so you can act instead of claiming no access.
SKIP: when a visible core tool already covers the task, when captain_docs is the better behaviour reference, or when the capability is clearly from a skill/MCP server."
    },

];

pub fn tool_doc(name: &str) -> Option<&'static str> {
    TOOL_DOCS
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.doc)
}
