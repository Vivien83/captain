//! Shared tool name mappings between OpenClaw and Captain.
//!
//! These mappings are used by both the migration engine and the skill system
//! to normalize OpenClaw tool names into Captain equivalents.

/// Map an OpenClaw tool name to its Captain equivalent.
///
/// Returns `None` if the name has no known mapping (may already be
/// an Captain tool name — check with [`is_known_captain_tool`]).
pub fn map_tool_name(openclaw_name: &str) -> Option<&'static str> {
    match openclaw_name {
        // Claude-style tool names (capitalized)
        "Read" | "read" | "read_file" => Some("file_read"),
        "Write" | "write" | "write_file" => Some("file_write"),
        "Edit" | "edit" => Some("file_write"),
        "Glob" | "glob" | "list_files" => Some("file_list"),
        "Grep" | "grep" => Some("file_list"),
        "Bash" | "bash" | "exec" | "execute_command" => Some("shell_exec"),
        "WebSearch" | "web_search" => Some("web_search"),
        "WebFetch" | "fetch_url" | "web_fetch" => Some("web_fetch"),
        "WebDownload" | "download_url" | "web_download" => Some("web_download"),
        "DocumentExtract" | "extract_document" | "document_extract" => Some("document_extract"),
        "browser_batch" => Some("browser_batch"),
        "browser_navigate" => Some("browser_navigate"),
        "browser_click" => Some("browser_click"),
        "browser_type" => Some("browser_type"),
        "browser_keys" | "browser_press" | "browser_key" => Some("browser_keys"),
        "browser_select" => Some("browser_select"),
        "browser_hover" => Some("browser_hover"),
        "browser_screenshot" => Some("browser_screenshot"),
        "browser_read_page" => Some("browser_read_page"),
        "browser_scroll" => Some("browser_scroll"),
        "browser_wait" => Some("browser_wait"),
        "browser_run_js" | "browser_evaluate" => Some("browser_run_js"),
        "browser_back" => Some("browser_back"),
        "browser_status" => Some("browser_status"),
        "browser_network_log" => Some("browser_network_log"),
        "browser_observe" => Some("browser_observe"),
        "browser_diagnostics" => Some("browser_diagnostics"),
        "browser_close" => Some("browser_close"),
        "memory_search" | "memory_recall" => Some("memory_recall"),
        "memory_save" => Some("memory_save"),
        "memory_store" => Some("memory_store"),
        "sessions_send" | "agent_message" => Some("agent_send"),
        "sessions_list" | "agents_list" | "agent_list" => Some("agent_list"),
        "sessions_spawn" => Some("agent_send"),

        // LLM-hallucinated aliases (fs-* style names)
        "fs-read" | "fs_read" | "fsRead" | "readFile" => Some("file_read"),
        "fs-write" | "fs_write" | "fsWrite" | "writeFile" => Some("file_write"),
        "fs-list" | "fs_list" | "fsList" | "listFiles" | "list_dir" | "ls" => Some("file_list"),
        "fs-exec" | "run" | "run_command" | "runCommand" | "execute" | "shell" => {
            Some("shell_exec")
        }

        _ => None,
    }
}

/// Normalize a tool name to its canonical Captain form.
///
/// If the name is already a known Captain tool, returns it as-is.
/// Otherwise, tries to map it through [`map_tool_name`].
/// Returns the original name if no mapping is found.
pub fn normalize_tool_name(name: &str) -> &str {
    if is_known_captain_tool(name) {
        return name;
    }
    map_tool_name(name).unwrap_or(name)
}

/// Check if a tool name is a known Captain built-in tool.
pub fn is_known_captain_tool(name: &str) -> bool {
    matches!(
        name,
        "file_read"
            | "file_write"
            | "file_list"
            | "shell_exec"
            | "web_search"
            | "web_fetch"
            | "web_download"
            | "web_research_batch"
            | "document_create"
            | "document_extract"
            | "document_pipeline"
            | "browser_batch"
            | "browser_navigate"
            | "browser_click"
            | "browser_type"
            | "browser_keys"
            | "browser_select"
            | "browser_hover"
            | "browser_screenshot"
            | "browser_read_page"
            | "browser_close"
            | "browser_scroll"
            | "browser_wait"
            | "browser_run_js"
            | "browser_back"
            | "browser_status"
            | "browser_network_log"
            | "browser_observe"
            | "browser_diagnostics"
            | "memory_save"
            | "memory_recall"
            | "memory_store"
            | "agent_send"
            | "agent_list"
            | "agent_spawn"
            | "agent_kill"
            | "agent_find"
            | "task_post"
            | "task_claim"
            | "task_complete"
            | "task_list"
            | "event_publish"
            | "schedule_create"
            | "schedule_list"
            | "schedule_delete"
            | "image_analyze"
            | "location_get"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_tool_name_all_mappings() {
        // Claude-style capitalized
        assert_eq!(map_tool_name("Read"), Some("file_read"));
        assert_eq!(map_tool_name("Write"), Some("file_write"));
        assert_eq!(map_tool_name("Edit"), Some("file_write"));
        assert_eq!(map_tool_name("Glob"), Some("file_list"));
        assert_eq!(map_tool_name("Grep"), Some("file_list"));
        assert_eq!(map_tool_name("Bash"), Some("shell_exec"));
        assert_eq!(map_tool_name("WebSearch"), Some("web_search"));
        assert_eq!(map_tool_name("WebFetch"), Some("web_fetch"));
        assert_eq!(map_tool_name("WebDownload"), Some("web_download"));
        assert_eq!(map_tool_name("DocumentExtract"), Some("document_extract"));

        // Lowercase variants
        assert_eq!(map_tool_name("read"), Some("file_read"));
        assert_eq!(map_tool_name("write"), Some("file_write"));
        assert_eq!(map_tool_name("edit"), Some("file_write"));
        assert_eq!(map_tool_name("glob"), Some("file_list"));
        assert_eq!(map_tool_name("grep"), Some("file_list"));
        assert_eq!(map_tool_name("bash"), Some("shell_exec"));
        assert_eq!(map_tool_name("exec"), Some("shell_exec"));
        assert_eq!(map_tool_name("execute_command"), Some("shell_exec"));

        // Other aliases
        assert_eq!(map_tool_name("read_file"), Some("file_read"));
        assert_eq!(map_tool_name("write_file"), Some("file_write"));
        assert_eq!(map_tool_name("list_files"), Some("file_list"));
        assert_eq!(map_tool_name("fetch_url"), Some("web_fetch"));
        assert_eq!(map_tool_name("web_fetch"), Some("web_fetch"));
        assert_eq!(map_tool_name("web_search"), Some("web_search"));
        assert_eq!(map_tool_name("download_url"), Some("web_download"));
        assert_eq!(map_tool_name("extract_document"), Some("document_extract"));
        assert_eq!(map_tool_name("browser_batch"), Some("browser_batch"));
        assert_eq!(map_tool_name("browser_navigate"), Some("browser_navigate"));
        assert_eq!(map_tool_name("browser_press"), Some("browser_keys"));
        assert_eq!(map_tool_name("browser_select"), Some("browser_select"));
        assert_eq!(map_tool_name("browser_hover"), Some("browser_hover"));
        assert_eq!(map_tool_name("browser_observe"), Some("browser_observe"));
        assert_eq!(
            map_tool_name("browser_diagnostics"),
            Some("browser_diagnostics")
        );
        assert_eq!(map_tool_name("browser_status"), Some("browser_status"));
        assert_eq!(
            map_tool_name("browser_network_log"),
            Some("browser_network_log")
        );
        assert_eq!(map_tool_name("browser_evaluate"), Some("browser_run_js"));
        assert_eq!(map_tool_name("memory_search"), Some("memory_recall"));
        assert_eq!(map_tool_name("memory_recall"), Some("memory_recall"));
        assert_eq!(map_tool_name("memory_save"), Some("memory_save"));
        assert_eq!(map_tool_name("memory_store"), Some("memory_store"));
        assert_eq!(map_tool_name("sessions_send"), Some("agent_send"));
        assert_eq!(map_tool_name("agent_message"), Some("agent_send"));
        assert_eq!(map_tool_name("sessions_list"), Some("agent_list"));
        assert_eq!(map_tool_name("agents_list"), Some("agent_list"));
        assert_eq!(map_tool_name("agent_list"), Some("agent_list"));
        assert_eq!(map_tool_name("sessions_spawn"), Some("agent_send"));

        // LLM-hallucinated fs-* aliases
        assert_eq!(map_tool_name("fs-read"), Some("file_read"));
        assert_eq!(map_tool_name("fs_read"), Some("file_read"));
        assert_eq!(map_tool_name("fsRead"), Some("file_read"));
        assert_eq!(map_tool_name("readFile"), Some("file_read"));
        assert_eq!(map_tool_name("fs-write"), Some("file_write"));
        assert_eq!(map_tool_name("fs_write"), Some("file_write"));
        assert_eq!(map_tool_name("fsWrite"), Some("file_write"));
        assert_eq!(map_tool_name("writeFile"), Some("file_write"));
        assert_eq!(map_tool_name("fs-list"), Some("file_list"));
        assert_eq!(map_tool_name("fs_list"), Some("file_list"));
        assert_eq!(map_tool_name("fsList"), Some("file_list"));
        assert_eq!(map_tool_name("listFiles"), Some("file_list"));
        assert_eq!(map_tool_name("list_dir"), Some("file_list"));
        assert_eq!(map_tool_name("ls"), Some("file_list"));
        assert_eq!(map_tool_name("fs-exec"), Some("shell_exec"));
        assert_eq!(map_tool_name("run"), Some("shell_exec"));
        assert_eq!(map_tool_name("run_command"), Some("shell_exec"));
        assert_eq!(map_tool_name("runCommand"), Some("shell_exec"));
        assert_eq!(map_tool_name("execute"), Some("shell_exec"));
        assert_eq!(map_tool_name("shell"), Some("shell_exec"));

        // Unknown
        assert_eq!(map_tool_name("unknown_tool"), None);
        assert_eq!(map_tool_name(""), None);
    }

    #[test]
    fn test_normalize_tool_name() {
        // Known Captain tools pass through unchanged
        assert_eq!(normalize_tool_name("file_read"), "file_read");
        assert_eq!(normalize_tool_name("file_write"), "file_write");
        assert_eq!(normalize_tool_name("shell_exec"), "shell_exec");
        assert_eq!(normalize_tool_name("web_search"), "web_search");
        assert_eq!(normalize_tool_name("memory_save"), "memory_save");

        // Aliases get normalized to canonical names
        assert_eq!(normalize_tool_name("fs-read"), "file_read");
        assert_eq!(normalize_tool_name("fs-write"), "file_write");
        assert_eq!(normalize_tool_name("fs-list"), "file_list");
        assert_eq!(normalize_tool_name("fs-exec"), "shell_exec");
        assert_eq!(normalize_tool_name("Read"), "file_read");
        assert_eq!(normalize_tool_name("Bash"), "shell_exec");

        // Unknown names pass through unchanged
        assert_eq!(normalize_tool_name("my_custom_tool"), "my_custom_tool");
        assert_eq!(normalize_tool_name("mcp_server_tool"), "mcp_server_tool");
    }

    #[test]
    fn test_is_known_captain_tool() {
        // Representative built-in tools.
        let known = [
            "file_read",
            "file_write",
            "file_list",
            "shell_exec",
            "web_search",
            "web_fetch",
            "web_download",
            "web_research_batch",
            "document_create",
            "document_extract",
            "document_pipeline",
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
            "memory_save",
            "memory_recall",
            "memory_store",
            "agent_send",
            "agent_list",
            "agent_spawn",
            "agent_kill",
            "agent_find",
            "task_post",
            "task_claim",
            "task_complete",
            "task_list",
            "event_publish",
            "schedule_create",
            "schedule_list",
            "schedule_delete",
            "image_analyze",
            "location_get",
        ];
        for tool in &known {
            assert!(is_known_captain_tool(tool), "Expected {tool} to be known");
        }

        // Unknown
        assert!(!is_known_captain_tool("unknown"));
        assert!(!is_known_captain_tool("Read"));
        assert!(!is_known_captain_tool("Bash"));
    }
}
