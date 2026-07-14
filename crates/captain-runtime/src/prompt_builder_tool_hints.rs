/// Map a tool name to a one-line description hint.
pub fn tool_hint(name: &str) -> &'static str {
    TOOL_HINTS
        .iter()
        .find_map(|(tool, hint)| (*tool == name).then_some(*hint))
        .unwrap_or("")
}

const TOOL_HINTS: &[(&str, &str)] = &[
    // Files
    ("file_read", "read file contents"),
    ("file_write", "create or overwrite a file"),
    ("file_list", "list directory contents"),
    ("edit_file", "replace exact text in a file"),
    ("multi_edit", "apply multiple exact replacements"),
    ("apply_patch", "apply precise multi-file patch hunks"),
    ("file_delete", "delete a file"),
    ("file_move", "move or rename a file"),
    ("file_copy", "copy a file"),
    ("file_search", "search files by name pattern"),
    ("grep", "search inside file contents"),
    ("glob", "find files by path pattern"),
    (
        "document_create",
        "create a native PDF, DOCX, HTML, or Markdown document",
    ),
    (
        "document_extract",
        "extract text from a PDF or document source",
    ),
    ("document_pipeline", "create and optionally send a document"),
    // Web
    (
        "web_research_batch",
        "run grouped web searches and compact fetches",
    ),
    ("web_search", "search the web for information"),
    ("web_fetch", "fetch a URL and get its content as markdown"),
    ("web_download", "download a source file into the workspace"),
    // Browser
    ("browser_navigate", "open a URL in the browser"),
    ("browser_click", "click an element on the page"),
    ("browser_type", "type text into an input field"),
    ("browser_keys", "press keyboard keys in the browser"),
    ("browser_screenshot", "capture a screenshot"),
    ("browser_read_page", "extract page content as text"),
    ("browser_close", "close the browser session"),
    ("browser_scroll", "scroll the page"),
    ("browser_wait", "wait for an element or condition"),
    ("browser_evaluate", "run JavaScript on the page"),
    ("browser_run_js", "run JavaScript on the page"),
    ("browser_select", "select a dropdown option"),
    ("browser_hover", "hover over an element"),
    ("browser_back", "go back to the previous page"),
    (
        "browser_status",
        "inspect browser session and profile state",
    ),
    (
        "browser_network_log",
        "inspect recent browser network events",
    ),
    // Shell
    ("shell_exec", "execute a shell command"),
    ("execute_code", "run a short code snippet"),
    ("shell_background", "run a command in the background"),
    // Memory
    (
        "memory_context_batch",
        "retrieve memory and prior-session context together",
    ),
    ("memory_save", "save a structured durable memory"),
    ("memory_store", "save a key-value pair to memory"),
    ("memory_recall", "search memory for relevant context"),
    ("memory_forget", "remove obsolete or incorrect memory facts"),
    ("memory_delete", "delete a memory entry"),
    ("memory_list", "list stored memory keys"),
    // Projects
    (
        "project_list",
        "list durable projects with compact runtime state",
    ),
    ("project_get", "inspect one durable project by slug"),
    // Agents
    ("agent_send", "send a message to another agent"),
    ("agent_spawn", "create a new agent"),
    ("agent_list", "list running agents"),
    ("agent_kill", "terminate an agent"),
    // Media
    ("image_describe", "describe an image"),
    ("image_generate", "generate an image from a prompt"),
    ("audio_transcribe", "transcribe audio to text"),
    ("speech_to_text", "transcribe audio locally with native STT"),
    ("tts_speak", "convert text to speech"),
    ("text_to_speech", "generate local voice audio from text"),
    // Docker
    ("docker_exec", "run a command in a container"),
    ("docker_build", "build a Docker image"),
    ("docker_run", "start a Docker container"),
    // Scheduling
    ("cron_create", "schedule a recurring task"),
    ("cron_list", "list scheduled tasks"),
    ("cron_delete", "remove a scheduled task"),
    // Processes
    (
        "process_start",
        "start a long-running process (REPL, server)",
    ),
    ("process_poll", "read stdout/stderr from a running process"),
    ("process_write", "write to a process's stdin"),
    ("process_kill", "terminate a running process"),
    ("process_list", "list active processes"),
    // SSH
    ("ssh_exec", "run a remote command via a vault SSH alias"),
    ("ssh_upload", "upload a workspace file over native SFTP"),
    ("ssh_download", "download a remote file over native SFTP"),
    // Channels / user
    (
        "ask_user",
        "ask the user only when clarification is required",
    ),
    ("channel_send", "send a proactive outbound channel message"),
    (
        "channel_reconfigure",
        "hot-reload one configured channel adapter",
    ),
    // Config / secrets / workspace
    ("config_read", "read Captain's live config"),
    ("config_write", "safely update Captain config"),
    ("config_schema", "inspect config schema"),
    ("self_configure", "apply a self-configuration plan"),
    ("model_switch_plan", "prepare a safe model/provider switch"),
    (
        "model_switch_apply",
        "apply an approved model/provider switch",
    ),
    ("codex_auth_status", "check Codex OAuth readiness"),
    ("codex_tool_probe", "probe real Codex tool-call support"),
    ("codex_login_start", "start Codex OAuth device-code login"),
    ("codex_login_poll", "poll and finish Codex OAuth login"),
    ("secret_read", "check a secret without exposing it"),
    ("secret_write", "store a secret in the vault"),
    ("workspace_add", "authorize an extra workspace root"),
    // Knowledge / sessions / scheduling
    ("knowledge_query", "query Captain's knowledge graph"),
    ("session_recall", "search prior session checkpoints"),
    ("goal_create", "create an autonomous ongoing goal"),
    (
        "cron_update",
        "update an existing cron without recreating it",
    ),
    ("scaffold_skill", "create a reusable skill scaffold"),
    (
        "skill_refinement_propose",
        "propose an improvement to an existing skill",
    ),
    ("skill_refinement_list", "list skill improvement proposals"),
    (
        "skill_refinement_decide",
        "reject or record human-approved skill improvement decisions",
    ),
    ("skill_refinement_update", "mark skill improvement progress"),
    (
        "skill_refinement_restore",
        "restore a skill from its pre-improvement snapshot",
    ),
    // Meta
    (
        "capability_search",
        "resolve which active tool, skill, MCP server, or docs family to use",
    ),
    (
        "skill_search",
        "discover relevant procedural skills by family",
    ),
    ("skill_view", "load one exact skill's workflow context"),
    (
        "captain_docs",
        "search Captain's internal tool documentation",
    ),
    (
        "self_improvement_review",
        "review pending self-improvement items",
    ),
    (
        "system_bug_report",
        "record a Captain system bug or autonomy gap",
    ),
    ("system_bug_list", "list known Captain system bugs"),
    ("system_bug_update", "update a known Captain system bug"),
    ("tool_search", "discover hidden/deferred Captain tools"),
    ("system_time", "read the daemon's current time"),
    ("canvas_present", "render a dashboard HTML panel"),
];
