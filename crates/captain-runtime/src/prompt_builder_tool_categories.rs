/// Map a tool name to its category for grouping.
pub fn tool_category(name: &str) -> &'static str {
    match name {
        "file_read" | "file_write" | "file_list" | "edit_file" | "multi_edit" | "apply_patch"
        | "grep" | "glob" | "file_delete" | "file_move" | "file_copy" | "file_search" => "Files",

        "document_create" | "document_extract" | "document_pipeline" => "Documents",

        "web_search" | "web_fetch" | "web_download" | "web_research_batch" => "Web",

        "browser_navigate"
        | "browser_click"
        | "browser_type"
        | "browser_keys"
        | "browser_screenshot"
        | "browser_read_page"
        | "browser_close"
        | "browser_scroll"
        | "browser_wait"
        | "browser_evaluate"
        | "browser_select"
        | "browser_hover"
        | "browser_back"
        | "browser_run_js"
        | "browser_status"
        | "browser_network_log" => "Browser",

        "shell_exec" | "execute_code" | "shell_background" | "cargo" | "npm" | "pip" => "Shell",

        "ssh_exec" | "ssh_upload" | "ssh_download" => "SSH",

        "memory_context_batch"
        | "memory_save"
        | "memory_store"
        | "memory_recall"
        | "memory_forget"
        | "memory_delete"
        | "memory_list" => "Memory",

        "project_list"
        | "project_get"
        | "project_create"
        | "project_resume"
        | "project_archive"
        | "project_task_create"
        | "project_task_list"
        | "project_task_update"
        | "milestone_create"
        | "milestone_list"
        | "milestone_complete"
        | "milestone_progress"
        | "checkpoint_save" => "Projects",

        "agent_send" | "agent_spawn" | "agent_list" | "agent_kill" | "agent_status"
        | "agent_watch" | "agent_delegate" | "agent_correct" => "Agents",

        "ask_user" | "channel_send" | "channel_reconfigure" => "User and channels",

        "config_read" | "config_write" | "config_schema" | "self_configure"
        | "model_switch_plan" | "model_switch_apply" | "codex_auth_status" | "codex_tool_probe"
        | "codex_login_start" | "codex_login_poll" | "secret_read" | "secret_write"
        | "config_setup" | "workspace_add" => "Config and access",

        "knowledge_query" | "knowledge_add_entity" | "knowledge_add_relation" => "Knowledge",

        "session_recall" => "Sessions",

        "scaffold_skill"
        | "scaffold_hand"
        | "skill_execute"
        | "skill_refinement_propose"
        | "skill_refinement_list"
        | "skill_refinement_decide"
        | "skill_refinement_update"
        | "skill_refinement_restore" => "Skills",

        "cron_create" | "cron_list" | "cron_update" | "cron_cancel" | "goal_create"
        | "goal_list" | "goal_pause" | "goal_resume" | "goal_status" | "goal_delete" => {
            "Scheduling"
        }

        "image_describe" | "image_generate" | "audio_transcribe" | "tts_speak"
        | "speech_to_text" | "text_to_speech" => "Media",

        "docker_exec" | "docker_build" | "docker_run" => "Docker",

        "process_start" | "process_poll" | "process_write" | "process_kill" | "process_list" => {
            "Processes"
        }

        "capability_search"
        | "skill_search"
        | "skill_view"
        | "captain_docs"
        | "self_improvement_review"
        | "system_bug_report"
        | "system_bug_list"
        | "system_bug_update"
        | "tool_search"
        | "system_time"
        | "canvas_present" => "Meta",

        _ if name.starts_with("mcp_") => "MCP",
        _ if name.starts_with("skill_") => "Skills",
        _ => "Other",
    }
}
