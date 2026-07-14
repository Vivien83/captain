//! Tool grouping for Captain — reduces 60+ tools to ~15 meta-tools.
//!
//! Each meta-tool uses action-based dispatch: `tool({"action": "xxx", ...params})`.
//! The `resolve_grouped_tool` function maps a grouped call back to the original
//! flat tool name + input, so the existing `execute_tool` path is unchanged.

use captain_types::tool::ToolDefinition;

struct ToolGroupRow {
    name: &'static str,
    description: &'static str,
    actions: &'static [&'static str],
    detail: &'static str,
}

/// Mapping from (group_name, action) → original_tool_name.
/// Actions not listed here fall through as `{group}_{action}` (e.g. `file_read`).
const GROUP_MAPPINGS: &[(&str, &[(&str, &str)])] = &[
    (
        "file",
        &[
            ("read", "file_read"),
            ("write", "file_write"),
            ("list", "file_list"),
            ("patch", "apply_patch"),
        ],
    ),
    (
        "web",
        &[
            ("search", "web_search"),
            ("fetch", "web_fetch"),
            ("download", "web_download"),
        ],
    ),
    (
        "document",
        &[
            ("create", "document_create"),
            ("extract", "document_extract"),
            ("pipeline", "document_pipeline"),
        ],
    ),
    (
        "exec",
        &[
            ("command", "shell_exec"),
            ("browser", "browser_navigate"),
            ("screenshot", "browser_screenshot"),
            ("click", "browser_click"),
            ("type", "browser_type"),
            ("keys", "browser_keys"),
            ("select", "browser_select"),
            ("hover", "browser_hover"),
            ("read_page", "browser_read_page"),
            ("scroll", "browser_scroll"),
            ("wait", "browser_wait"),
            ("run_js", "browser_run_js"),
            ("back", "browser_back"),
            ("status", "browser_status"),
            ("network_log", "browser_network_log"),
            ("close", "browser_close"),
        ],
    ),
    (
        "agent",
        &[
            ("list", "agent_list"),
            ("send", "agent_send"),
            ("spawn", "agent_spawn"),
            ("kill", "agent_kill"),
            ("find", "agent_find"),
        ],
    ),
    (
        "memory",
        &[("store", "memory_store"), ("recall", "memory_recall")],
    ),
    (
        "session",
        &[
            ("list", "task_list"),
            ("post", "task_post"),
            ("claim", "task_claim"),
            ("complete", "task_complete"),
            ("publish", "event_publish"),
        ],
    ),
    (
        "cron",
        &[
            ("list", "cron_list"),
            ("create", "cron_create"),
            ("update", "cron_update"),
            ("delete", "cron_cancel"),
        ],
    ),
    (
        "schedule",
        &[
            ("list", "schedule_list"),
            ("create", "schedule_create"),
            ("delete", "schedule_delete"),
        ],
    ),
    (
        "file_trigger",
        &[
            ("register", "file_trigger_register"),
            ("list", "file_trigger_list"),
            ("set_enabled", "file_trigger_set_enabled"),
            ("remove", "file_trigger_remove"),
        ],
    ),
    (
        "todo",
        &[
            ("create", "todo_create"),
            ("list", "todo_list"),
            ("complete", "todo_complete"),
            ("reopen", "todo_reopen"),
            ("delete", "todo_delete"),
        ],
    ),
    (
        "knowledge",
        &[
            ("add_entity", "knowledge_add_entity"),
            ("add_relation", "knowledge_add_relation"),
            ("query", "knowledge_query"),
        ],
    ),
    (
        "media",
        &[
            ("analyze", "image_analyze"),
            ("describe", "media_describe"),
            ("transcribe", "media_transcribe"),
            ("generate", "image_generate"),
        ],
    ),
    ("channel", &[("send", "channel_send")]),
    (
        "hand",
        &[
            ("list", "hand_list"),
            ("activate", "hand_activate"),
            ("status", "hand_status"),
            ("deactivate", "hand_deactivate"),
        ],
    ),
    ("a2a", &[("discover", "a2a_discover"), ("send", "a2a_send")]),
    (
        "audio",
        &[("tts", "text_to_speech"), ("stt", "speech_to_text")],
    ),
    (
        "skill",
        &[("check", "skill_check"), ("execute", "skill_execute")],
    ),
    ("location", &[("get", "location_get")]),
];

/// Reverse lookup: given a flat tool name (e.g. "shell_exec"), find its group ("exec").
pub fn tool_to_group(tool_name: &str) -> Option<&'static str> {
    for &(group, mappings) in GROUP_MAPPINGS {
        for &(_, orig) in mappings {
            if orig == tool_name {
                return Some(group);
            }
        }
        // Also check {group}_{anything} convention
        if tool_name.starts_with(group) && tool_name.as_bytes().get(group.len()) == Some(&b'_') {
            return Some(group);
        }
    }
    None
}

/// Check if a tool name is a grouped meta-tool and resolve it to the
/// original flat tool name + rewritten input (action field stripped).
///
/// Returns `Some((original_name, rewritten_input))` if it's a grouped tool,
/// `None` if it should be handled by the normal tool runner path.
pub fn resolve_grouped_tool(
    tool_name: &str,
    input: &serde_json::Value,
) -> Option<(String, serde_json::Value)> {
    let action = input.get("action")?.as_str()?;

    for &(group, mappings) in GROUP_MAPPINGS {
        if tool_name != group {
            continue;
        }
        // Find the mapped original tool name
        let original = mappings
            .iter()
            .find(|&&(a, _)| a == action)
            .map(|&(_, orig)| orig.to_string())
            // Fallback: try {group}_{action} convention
            .unwrap_or_else(|| format!("{group}_{action}"));

        // Strip the "action" field from input, pass the rest through
        let mut rewritten = input.clone();
        if let Some(obj) = rewritten.as_object_mut() {
            obj.remove("action");
        }

        return Some((original, rewritten));
    }

    None
}

/// Return grouped tool definitions for Captain.
///
/// Each definition is a meta-tool with an `action` enum listing available actions,
/// plus passthrough properties for any additional arguments.
pub fn grouped_tool_definitions() -> Vec<ToolDefinition> {
    all_grouped_tool_definitions()
        .into_iter()
        .filter(|tool| crate::surface_gates::source_is_discoverable_by_default(&tool.name))
        .collect()
}

fn all_grouped_tool_definitions() -> Vec<ToolDefinition> {
    TOOL_GROUP_ROWS
        .iter()
        .map(|row| make_group(row.name, row.description, row.actions, row.detail))
        .collect()
}

const TOOL_GROUP_ROWS: &[ToolGroupRow] = &[
    ToolGroupRow {
        name: "file",
        description: "File operations",
        actions: &["read", "write", "list", "patch"],
        detail: "Read, write, list files or apply patches. Pass file path as 'path', content as 'content'.",
    },
    ToolGroupRow {
        name: "web",
        description: "Web search, fetch, and download",
        actions: &["search", "fetch", "download"],
        detail: "Search the web, fetch a readable URL, or download a source file. Pass 'query' for search, 'url' for fetch/download.",
    },
    ToolGroupRow {
        name: "document",
        description: "Document creation and extraction",
        actions: &["create", "extract", "pipeline"],
        detail: "Create a document, extract text from a downloaded source, or create+send a document pipeline.",
    },
    ToolGroupRow {
        name: "exec",
        description: "Shell execution and browser automation",
        actions: &[
            "command",
            "browser",
            "screenshot",
            "click",
            "type",
            "keys",
            "select",
            "hover",
            "read_page",
            "scroll",
            "wait",
            "run_js",
            "back",
            "status",
            "network_log",
            "close",
        ],
        detail: "Run shell commands or control a browser. Pass 'command' for shell, 'url' for browser, 'limit' for network_log.",
    },
    ToolGroupRow {
        name: "agent",
        description: "Agent management",
        actions: &["list", "send", "spawn", "kill", "find"],
        detail: "List, message, spawn, kill, or find agents.",
    },
    ToolGroupRow {
        name: "memory",
        description: "Shared memory",
        actions: &["store", "recall"],
        detail: "Store or recall key-value pairs in shared memory.",
    },
    ToolGroupRow {
        name: "session",
        description: "Tasks and events",
        actions: &["list", "post", "claim", "complete", "publish"],
        detail: "Manage tasks and publish events for inter-agent collaboration.",
    },
    ToolGroupRow {
        name: "cron",
        description: "Scheduled jobs",
        actions: &["list", "create", "delete"],
        detail: "Create, list, or delete cron jobs. Include 'workflow' array for direct execution.",
    },
    ToolGroupRow {
        name: "schedule",
        description: "One-time schedules",
        actions: &["list", "create", "delete"],
        detail: "Create, list, or delete one-time scheduled tasks.",
    },
    ToolGroupRow {
        name: "knowledge",
        description: "Knowledge graph",
        actions: &["add_entity", "add_relation", "query"],
        detail: "Add entities/relations or query the knowledge graph.",
    },
    ToolGroupRow {
        name: "media",
        description: "Image and media processing",
        actions: &["analyze", "describe", "transcribe", "generate"],
        detail: "Analyze images, describe media, transcribe audio, or generate images.",
    },
    ToolGroupRow {
        name: "channel",
        description: "Channel messaging",
        actions: &["send"],
        detail: "Send a message to a specific channel/user.",
    },
    ToolGroupRow {
        name: "hand",
        description: "Hardware hands (IoT)",
        actions: &["list", "activate", "status", "deactivate"],
        detail: "Control hardware hands/IoT devices.",
    },
    ToolGroupRow {
        name: "a2a",
        description: "Agent-to-Agent protocol",
        actions: &["discover", "send"],
        detail: "Discover or send tasks to external A2A agents.",
    },
    ToolGroupRow {
        name: "audio",
        description: "Text-to-speech and speech-to-text",
        actions: &["tts", "stt"],
        detail: "Convert text to speech or speech to text.",
    },
    ToolGroupRow {
        name: "skill",
        description: "Skill execution",
        actions: &["execute"],
        detail: "Execute a capability from a skill .md file. Pass 'skill' name and 'capability' name.",
    },
    ToolGroupRow {
        name: "location",
        description: "Geolocation",
        actions: &["get"],
        detail: "Get current device location.",
    },
];

fn make_group(name: &str, description: &str, actions: &[&str], detail: &str) -> ToolDefinition {
    let actions_json: Vec<serde_json::Value> = actions
        .iter()
        .map(|a| serde_json::Value::String(a.to_string()))
        .collect();

    ToolDefinition {
        name: name.to_string(),
        description: format!("{description}. {detail}"),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": actions_json,
                    "description": "The action to perform"
                }
            },
            "required": ["action"],
            "additionalProperties": true
        }),
    }
}

/// Names of all grouped meta-tools.
pub fn grouped_tool_names() -> Vec<&'static str> {
    GROUP_MAPPINGS
        .iter()
        .map(|&(name, _)| name)
        .filter(|name| crate::surface_gates::source_is_discoverable_by_default(name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_file_read() {
        let input = serde_json::json!({"action": "read", "path": "/tmp/test.txt"});
        let (name, rewritten) = resolve_grouped_tool("file", &input).unwrap();
        assert_eq!(name, "file_read");
        assert_eq!(
            rewritten.get("path").unwrap().as_str().unwrap(),
            "/tmp/test.txt"
        );
        assert!(rewritten.get("action").is_none());
    }

    #[test]
    fn test_resolve_unknown_action_falls_back() {
        let input = serde_json::json!({"action": "compress", "path": "/tmp"});
        let (name, _) = resolve_grouped_tool("file", &input).unwrap();
        assert_eq!(name, "file_compress"); // fallback convention
    }

    #[test]
    fn test_resolve_non_grouped_tool_returns_none() {
        let input = serde_json::json!({"path": "/tmp/test.txt"});
        assert!(resolve_grouped_tool("file_read", &input).is_none());
    }

    #[test]
    fn test_resolve_no_action_returns_none() {
        let input = serde_json::json!({"path": "/tmp/test.txt"});
        assert!(resolve_grouped_tool("file", &input).is_none());
    }

    #[test]
    fn test_grouped_definitions_count() {
        let defs = grouped_tool_definitions();
        const MAX_GROUPED_TOOLS: usize = 15;
        assert!(
            defs.len() <= MAX_GROUPED_TOOLS,
            "Should have at most {MAX_GROUPED_TOOLS} grouped tools, got {}",
            defs.len()
        );
    }

    #[test]
    fn test_all_grouped_definitions_keep_public_order() {
        let names: Vec<_> = all_grouped_tool_definitions()
            .into_iter()
            .map(|tool| tool.name)
            .collect();

        assert_eq!(
            names,
            vec![
                "file",
                "web",
                "document",
                "exec",
                "agent",
                "memory",
                "session",
                "cron",
                "schedule",
                "knowledge",
                "media",
                "channel",
                "hand",
                "a2a",
                "audio",
                "skill",
                "location",
            ]
        );
    }

    #[test]
    fn test_all_groups_have_action_enum() {
        for def in grouped_tool_definitions() {
            let action_prop = def
                .input_schema
                .get("properties")
                .unwrap()
                .get("action")
                .unwrap();
            assert!(
                action_prop.get("enum").is_some(),
                "Tool '{}' missing action enum",
                def.name
            );
        }
    }

    #[test]
    fn test_grouped_definitions_hide_frozen_surfaces() {
        let names: Vec<String> = grouped_tool_definitions()
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        assert!(!names.contains(&"hand".to_string()));
        assert!(!names.contains(&"a2a".to_string()));
        assert!(names.contains(&"file".to_string()));
        assert!(names.contains(&"skill".to_string()));
    }

    #[test]
    fn test_grouped_names_hide_frozen_surfaces() {
        let names = grouped_tool_names();
        assert!(!names.contains(&"hand"));
        assert!(!names.contains(&"a2a"));
        assert!(names.contains(&"file"));
    }
}
