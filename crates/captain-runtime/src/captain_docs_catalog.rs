//! Static catalogue for Captain self-documentation.
//!
//! Keep this data isolated from the search/rendering logic so `captain_docs`
//! stays focused on runtime behavior while preserving the public constants
//! used by discovery and tests.

/// All Captain tool families, slug + the phase that owns the audit prose.
///
/// Slugs are the canonical filename stem under `docs/captain-tools/`.
/// Order matches the audit plan (D.1..D.14) so the index reads top-down.
pub const FAMILIES: &[(&str, &str)] = &[
    ("file", "D.1"),
    ("shell-process", "D.2"),
    ("network", "D.3"),
    ("browser", "D.4"),
    ("ssh", "D.5"),
    ("memory", "D.6"),
    ("skill", "D.7"),
    ("channel", "D.8"),
    ("agent-coordination", "D.9"),
    ("scheduling", "D.10"),
    ("config-secret", "D.11"),
    ("mcp", "D.17"),
    ("knowledge", "D.12"),
    ("session-workspace", "D.13"),
    ("meta", "D.14"),
    ("project", "D.15"),
    ("multimedia", "D.16"),
    ("document", "D.19"),
    ("runtime-changelog", "D.18"),
];

/// D.1 — Tools that belong to the `file` family. Audit prose lives in
/// `docs/captain-tools/file.md`. The CI regression `file_doc_covers_every_tool`
/// fails if any of these names is missing from the markdown body, so a
/// rename or addition forces the doc and the code to ship together.
pub const FILE_FAMILY_TOOLS: &[&str] = &[
    "file_inspect_batch",
    "file_read",
    "file_write",
    "file_list",
    "glob",
    "grep",
    "edit_file",
    "multi_edit",
    "apply_patch",
];

/// D.2 — Tools that belong to the `shell-process` family. Audit prose lives
/// in `docs/captain-tools/shell-process.md`. Covers one-shot shell + code
/// execution, persistent process management, structured language wrappers,
/// and Docker-sandboxed escape hatches.
pub const SHELL_PROCESS_FAMILY_TOOLS: &[&str] = &[
    "shell_exec",
    "execute_code",
    "tool_run_start",
    "tool_run_status",
    "tool_run_result",
    "tool_run_cancel",
    "tool_run_list",
    "process_start",
    "process_poll",
    "process_write",
    "process_kill",
    "process_list",
    "docker_exec",
    "cargo",
    "npm",
    "pip",
];

/// D.3 — Tools that belong to the `network` family. Audit prose lives in
/// `docs/captain-tools/network.md`. Covers outbound HTTP plus search-engine
/// abstraction; the SSRF allowlist contract is documented next to each.
pub const NETWORK_FAMILY_TOOLS: &[&str] = &[
    "web_research_batch",
    "web_download",
    "web_fetch",
    "web_search",
];

/// D.4 — Tools that belong to the `browser` family. Audit prose lives in
/// `docs/captain-tools/browser.md`. Headless Chrome remote-debug protocol
/// driver: navigation, interaction, capture, and the JS escape hatch.
pub const BROWSER_FAMILY_TOOLS: &[&str] = &[
    "browser_batch",
    "browser_navigate",
    "browser_click",
    "browser_type",
    "browser_keys",
    "browser_select",
    "browser_hover",
    "browser_screenshot",
    "browser_read_page",
    "browser_close",
    "browser_scroll",
    "browser_wait",
    "browser_run_js",
    "browser_back",
    "browser_status",
    "browser_network_log",
    "browser_observe",
    "browser_diagnostics",
    "screenshot",
];

/// D.5 — Tools that belong to the `ssh` family. Audit prose lives in
/// `docs/captain-tools/ssh.md`. Embedded russh / russh-sftp; keys are
/// resolved by alias from the Captain vault.
pub const SSH_FAMILY_TOOLS: &[&str] =
    &["ssh_health_check", "ssh_exec", "ssh_upload", "ssh_download"];

/// D.6 — Tools that belong to the `memory` family. Audit prose lives in
/// `docs/captain-tools/memory.md`. Captain-native declarative save vs
/// key-value store, plus durable retraction; the local continuity journal and
/// MemPalace semantic-index contract are pinned next to each.
pub const MEMORY_FAMILY_TOOLS: &[&str] = &[
    "memory_context_batch",
    "memory_save",
    "memory_recall",
    "memory_store",
    "memory_forget",
];

/// D.7 — Tools that belong to the `skill` family. Audit prose lives in
/// `docs/captain-tools/skill.md`. Per-skill subprocess sandbox + the
/// extensibility verbs that let Captain ship new skills mid-session.
pub const SKILL_FAMILY_TOOLS: &[&str] = &[
    "skill_search",
    "skill_view",
    "skill_check",
    "skill_execute",
    "scaffold_skill",
    "workflow_learning_list",
    "skill_refinement_propose",
    "skill_refinement_list",
    "skill_refinement_decide",
    "skill_refinement_update",
    "skill_refinement_restore",
];

/// D.8 — Tools that belong to the `channel` family. Audit prose lives in
/// `docs/captain-tools/channel.md`. The outbound message verb plus the
/// hot-reload primitive that lets Captain rotate adapter config without
/// dropping other channels (A.1+A.2).
pub const CHANNEL_FAMILY_TOOLS: &[&str] = &[
    "channel_delivery_batch",
    "channel_send",
    "channel_reconfigure",
    "telegram_set_topic",
    "telegram_get_topic",
];

/// D.9 — Tools that belong to the `agent-coordination` family. Audit prose
/// lives in `docs/captain-tools/agent-coordination.md`. Spawn/list/kill,
/// orchestration, fleet management, task queue, and the user-facing
/// ask_user.
pub const AGENT_COORDINATION_FAMILY_TOOLS: &[&str] = &[
    "agent_spawn",
    "agent_send",
    "agent_list",
    "agent_kill",
    "agent_status",
    "agent_caps",
    "agent_watch",
    "agent_delegate",
    "agent_correct",
    "agent_find",
    "fleet_create_manager",
    "fleet_list_managers",
    "fleet_close_manager",
    "fleet_set_mission",
    "fleet_configure_autoscale",
    "fleet_metrics",
    "peer_list",
    "task_post",
    "task_claim",
    "task_complete",
    "task_list",
    "event_publish",
    "hand_list",
    "hand_activate",
    "hand_status",
    "hand_deactivate",
    "scaffold_hand",
    "a2a_discover",
    "a2a_send",
    "ask_user",
];

/// D.10 — Tools that belong to the `scheduling` family. Audit prose lives
/// in `docs/captain-tools/scheduling.md`. Cron jobs, low-level schedules,
/// and the autopilot Goal lifecycle.
pub const SCHEDULING_FAMILY_TOOLS: &[&str] = &[
    "cron_create",
    "cron_list",
    "cron_update",
    "cron_cancel",
    "reminder_set",
    "schedule_create",
    "schedule_list",
    "schedule_delete",
    "file_trigger_register",
    "file_trigger_list",
    "file_trigger_set_enabled",
    "file_trigger_remove",
    "todo_create",
    "todo_list",
    "todo_complete",
    "todo_reopen",
    "todo_delete",
    "goal_create",
    "goal_list",
    "goal_pause",
    "goal_resume",
    "goal_status",
    "goal_delete",
    "goal_list_suggestions",
    "goal_apply_suggestion",
    "goal_reject_suggestion",
];

/// D.11 — Tools that belong to the `config-secret` family. Audit prose
/// lives in `docs/captain-tools/config-secret.md`. config.toml + secrets.env
/// CRUD with backup-and-roundtrip protection on every mutation.
pub const CONFIG_SECRET_FAMILY_TOOLS: &[&str] = &[
    "config_read",
    "config_write",
    "web_credentials_update",
    "config_setup",
    "config_schema",
    "self_configure",
    "model_switch_plan",
    "model_switch_apply",
    "codex_auth_status",
    "codex_tool_probe",
    "codex_login_start",
    "codex_login_poll",
    "secret_read",
    "secret_write",
];

/// D.17 — MCP installation and recovery playbook.
pub const MCP_FAMILY_TOOLS: &[&str] = &[
    "mcp_catalog_search",
    "mcp_integration_install",
    "mcp_status",
];

/// D.12 — Tools that belong to the `knowledge` family. Audit prose lives
/// in `docs/captain-tools/knowledge.md`. Knowledge graph (entities +
/// relations + structured query), distinct from the diary-style memory
/// triples in D.6.
pub const KNOWLEDGE_FAMILY_TOOLS: &[&str] = &[
    "knowledge_add_entity",
    "knowledge_add_relation",
    "knowledge_query",
];

/// D.13 — Tools that belong to the `session-workspace` family. Audit prose
/// lives in `docs/captain-tools/session-workspace.md`. Cross-session recall
/// over checkpoint.md files + the multi-root sandbox grant.
pub const SESSION_WORKSPACE_FAMILY_TOOLS: &[&str] = &[
    "session_recall",
    "workspace_add",
    "session_tool_call_summary",
];

/// D.14 — Tools that belong to the `meta` family. Audit prose lives in
/// `docs/captain-tools/meta.md`. The reflexive layer Captain uses to look
/// at itself — current time, UI panels, `capability_search` (CR.1) for
/// cross-surface routing, `capability_forge` for controlled native capability
/// authoring, `captain_docs` (C.2) for RTFM-style lookups, and `tool_search`
/// (TS.1) for exact deferred builtin schemas.
pub const META_FAMILY_TOOLS: &[&str] = &[
    "system_time",
    "system_update",
    "canvas_present",
    "capability_search",
    "capability_forge",
    "captain_docs",
    "self_improvement_review",
    "system_bug_report",
    "system_bug_list",
    "system_bug_update",
    "tool_search",
    "location_get",
    "learning_review_list",
    "learning_review_decide",
];

/// D.15 — Tools that belong to the `project` family. Audit prose lives in
/// `docs/captain-tools/project.md`. Active project, task, milestone and
/// checkpoint management for long-running work.
pub const PROJECT_FAMILY_TOOLS: &[&str] = &[
    "project_create",
    "project_list",
    "project_get",
    "project_archive",
    "project_delete",
    "project_resume",
    "project_task_create",
    "project_task_list",
    "project_task_update",
    "milestone_create",
    "milestone_list",
    "milestone_complete",
    "milestone_progress",
    "checkpoint_save",
];

/// D.16 — Tools that belong to the `multimedia` family. Audit prose lives in
/// `docs/captain-tools/multimedia.md`. This family covers BOTH input verbs
/// (analyze / describe / transcribe — image, audio, video → text or metadata)
/// AND output verbs (generate / synthesize — text → image or audio), spanning
/// all three media types: image, audio, and video.
pub const MULTIMEDIA_FAMILY_TOOLS: &[&str] = &[
    "media_pipeline",
    "image_analyze",
    "image_generate",
    "media_describe",
    "media_transcribe",
    "text_to_speech",
    "speech_to_text",
    "video_analyze",
];

/// D.19 — Native document generation. Audit prose lives in
/// `docs/captain-tools/document.md`.
pub const DOCUMENT_FAMILY_TOOLS: &[&str] =
    &["document_pipeline", "document_create", "document_extract"];

/// D.18 — Agent-facing runtime changelog. No builtin tool belongs to this
/// family; it is the canonical public-safe release note surface Captain
/// reads after a real runtime update notice.
pub const RUNTIME_CHANGELOG_FAMILY_TOOLS: &[&str] = &[];

/// C.2 — Audit prose for every family, bundled at compile-time so
/// `captain_docs` can read it without a filesystem lookup at runtime.
/// Entries match `FAMILIES` slug for slug; CI ensures this stays in sync
/// via `family_docs_match_families`.
pub const FAMILY_DOCS: &[(&str, &str)] = &[
    ("file", include_str!("../../../docs/captain-tools/file.md")),
    (
        "shell-process",
        include_str!("../../../docs/captain-tools/shell-process.md"),
    ),
    (
        "network",
        include_str!("../../../docs/captain-tools/network.md"),
    ),
    (
        "browser",
        include_str!("../../../docs/captain-tools/browser.md"),
    ),
    ("ssh", include_str!("../../../docs/captain-tools/ssh.md")),
    (
        "memory",
        include_str!("../../../docs/captain-tools/memory.md"),
    ),
    (
        "skill",
        include_str!("../../../docs/captain-tools/skill.md"),
    ),
    (
        "channel",
        include_str!("../../../docs/captain-tools/channel.md"),
    ),
    (
        "agent-coordination",
        include_str!("../../../docs/captain-tools/agent-coordination.md"),
    ),
    (
        "scheduling",
        include_str!("../../../docs/captain-tools/scheduling.md"),
    ),
    (
        "config-secret",
        include_str!("../../../docs/captain-tools/config-secret.md"),
    ),
    ("mcp", include_str!("../../../docs/captain-tools/mcp.md")),
    (
        "knowledge",
        include_str!("../../../docs/captain-tools/knowledge.md"),
    ),
    (
        "session-workspace",
        include_str!("../../../docs/captain-tools/session-workspace.md"),
    ),
    ("meta", include_str!("../../../docs/captain-tools/meta.md")),
    (
        "project",
        include_str!("../../../docs/captain-tools/project.md"),
    ),
    (
        "multimedia",
        include_str!("../../../docs/captain-tools/multimedia.md"),
    ),
    (
        "document",
        include_str!("../../../docs/captain-tools/document.md"),
    ),
    (
        "runtime-changelog",
        include_str!("../../../docs/captain-tools/runtime-changelog.md"),
    ),
];

/// Return the live tool names assigned to a docs family.
///
/// Keep this next to `FAMILIES`/`FAMILY_DOCS` so captain_docs can append
/// generated schemas to the prose. The Markdown explains judgment and
/// recovery rules; the generated block is the exact runtime contract.
pub fn family_tools(slug: &str) -> Option<&'static [&'static str]> {
    match slug {
        "file" => Some(FILE_FAMILY_TOOLS),
        "shell-process" => Some(SHELL_PROCESS_FAMILY_TOOLS),
        "network" => Some(NETWORK_FAMILY_TOOLS),
        "browser" => Some(BROWSER_FAMILY_TOOLS),
        "ssh" => Some(SSH_FAMILY_TOOLS),
        "memory" => Some(MEMORY_FAMILY_TOOLS),
        "skill" => Some(SKILL_FAMILY_TOOLS),
        "channel" => Some(CHANNEL_FAMILY_TOOLS),
        "agent-coordination" => Some(AGENT_COORDINATION_FAMILY_TOOLS),
        "scheduling" => Some(SCHEDULING_FAMILY_TOOLS),
        "config-secret" => Some(CONFIG_SECRET_FAMILY_TOOLS),
        "mcp" => Some(MCP_FAMILY_TOOLS),
        "knowledge" => Some(KNOWLEDGE_FAMILY_TOOLS),
        "session-workspace" => Some(SESSION_WORKSPACE_FAMILY_TOOLS),
        "meta" => Some(META_FAMILY_TOOLS),
        "project" => Some(PROJECT_FAMILY_TOOLS),
        "multimedia" => Some(MULTIMEDIA_FAMILY_TOOLS),
        "document" => Some(DOCUMENT_FAMILY_TOOLS),
        "runtime-changelog" => Some(RUNTIME_CHANGELOG_FAMILY_TOOLS),
        _ => None,
    }
}
