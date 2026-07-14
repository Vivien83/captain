/// Format a one-line tool trace for streaming display (v3.8e).
///
/// Produces `<emoji> <tool_name>: <preview 50 chars>` so channels can show
/// concise real-time tool activity.
pub fn format_tool_trace(tool_name: &str, input: &serde_json::Value) -> String {
    let emoji = tool_emoji(tool_name);
    let preview = tool_input_preview(tool_name, input);
    if preview.is_empty() {
        format!("{emoji} {tool_name}")
    } else {
        format!("{emoji} {tool_name}: {preview}")
    }
}

pub fn tool_emoji(tool_name: &str) -> &'static str {
    match tool_name {
        "shell_exec" | "shell_background" => "💻",
        "file_read" => "📖",
        "file_write" | "apply_patch" => "✍️",
        "file_list" | "file_search" => "📂",
        "file_delete" | "file_move" | "file_copy" => "📁",
        "memory_store" | "memory_recall" | "memory_delete" | "memory_list" => "🧠",
        "knowledge_query" | "knowledge_add_entity" | "knowledge_add_relation" => "🔍",
        "web_search" | "web_fetch" | "web_research_batch" | "web_download" => "🌐",
        "document_create" | "document_extract" | "document_pipeline" => "📄",
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
        | "browser_network_log" => "🧭",
        "screenshot" => "📸",
        "agent_send" | "agent_spawn" | "agent_list" | "agent_kill" | "agent_delegate"
        | "agent_status" | "agent_watch" | "agent_correct" | "agent_find" => "🤖",
        "execute_code" | "python_runtime" => "▸",
        "skill_execute" | "scaffold_skill" => "📚",
        "cron_create" | "cron_list" | "cron_update" | "cron_cancel" | "schedule_create"
        | "schedule_list" | "schedule_delete" | "reminder_set" => "⏰",
        "process_start" | "process_poll" | "process_write" | "process_kill" | "process_list"
        | "docker_exec" | "docker_build" | "docker_run" => "⚙️",
        "channel_send" => "📱",
        "image_generate" | "image_analyze" | "media_describe" | "media_transcribe"
        | "audio_transcribe" | "tts_speak" | "text_to_speech" | "speech_to_text" => "🎨",
        "secret_read" | "secret_write" => "🔐",
        "config_read" | "config_write" | "self_configure" => "🛠️",
        "hand_list" | "hand_activate" | "hand_status" | "hand_deactivate" | "scaffold_hand" => "✋",
        "ask_user" => "❓",
        _ if tool_name.starts_with("mcp_") => "🔌",
        _ => "🔧",
    }
}

pub fn tool_input_preview(tool_name: &str, input: &serde_json::Value) -> String {
    let raw = match tool_name {
        "shell_exec" | "shell_background" => input["command"].as_str().unwrap_or("").to_string(),
        "file_read" | "file_write" | "file_delete" | "file_list" => {
            input["path"].as_str().unwrap_or("").to_string()
        }
        "web_fetch" | "web_search" | "web_research_batch" | "web_download" | "browser_navigate" => {
            input["url"]
                .as_str()
                .or_else(|| input["query"].as_str())
                .or_else(|| input["path"].as_str())
                .unwrap_or("")
                .to_string()
        }
        "document_create" | "document_extract" | "document_pipeline" => input["path"]
            .as_str()
            .or_else(|| input["title"].as_str())
            .or_else(|| input["document"]["path"].as_str())
            .or_else(|| input["document"]["title"].as_str())
            .unwrap_or("")
            .to_string(),
        "memory_store" => input["key"].as_str().unwrap_or("").to_string(),
        "memory_recall" | "knowledge_query" => input["query"].as_str().unwrap_or("").to_string(),
        "agent_send" | "agent_spawn" => input["agent"]
            .as_str()
            .or_else(|| input["name"].as_str())
            .unwrap_or("")
            .to_string(),
        "channel_send" => format!(
            "{} → {}",
            input["channel"].as_str().unwrap_or("?"),
            input["recipient"].as_str().unwrap_or("default"),
        ),
        "execute_code" | "python_runtime" => input["code"].as_str().unwrap_or("").to_string(),
        "screenshot" => input["save_path"].as_str().unwrap_or("(auto)").to_string(),
        "cron_create" | "schedule_create" => input["schedule"]
            .as_str()
            .or_else(|| input["cron"].as_str())
            .map(ToString::to_string)
            .unwrap_or_else(|| input["schedule"].to_string()),
        "cron_update" => {
            let id = input["job_id"].as_str().unwrap_or("?");
            let changed = [
                "name", "schedule", "action", "delivery", "enabled", "one_shot",
            ]
            .into_iter()
            .filter(|field| input.get(*field).is_some())
            .collect::<Vec<_>>()
            .join(",");
            format!("{id} {changed}")
        }
        "skill_execute" => format!(
            "{}/{}",
            input["skill"].as_str().unwrap_or("?"),
            input["capability"].as_str().unwrap_or("?"),
        ),
        _ => input
            .as_object()
            .and_then(|o| o.values().find_map(|v| v.as_str()).map(|s| s.to_string()))
            .unwrap_or_default(),
    };
    let trimmed = raw.trim().replace(['\n', '\r'], " ");
    if trimmed.chars().count() <= 50 {
        trimmed
    } else {
        let truncated: String = trimmed.chars().take(47).collect();
        format!("{truncated}...")
    }
}

/// Generate a human-like narration for tool calls when the LLM didn't produce text.
/// Kept for channels that cannot render tool UI.
#[allow(dead_code)]
fn build_tool_narration(tool_calls: &[captain_types::tool::ToolCall]) -> String {
    let parts: Vec<String> = tool_calls
        .iter()
        .map(|tc| match tc.name.as_str() {
            "shell_exec" | "bash" => {
                let cmd = tc.input["command"].as_str().unwrap_or("...");
                let preview: String = cmd.chars().take(50).collect();
                format!("⚡ `{}`", preview)
            }
            "file_read" | "read_file" => {
                let path = tc.input["path"]
                    .as_str()
                    .or(tc.input["file"].as_str())
                    .unwrap_or("fichier");
                format!("📖 Lecture de {}", path)
            }
            "file_write" | "write_file" => {
                let path = tc.input["path"].as_str().unwrap_or("fichier");
                format!("✏️ Écriture dans {}", path)
            }
            "web_search" | "search" => {
                let q = tc.input["query"].as_str().unwrap_or("...");
                let preview: String = q.chars().take(40).collect();
                format!("🔍 Recherche : \"{}\"", preview)
            }
            "web_fetch" | "fetch_url" => {
                let url = tc.input["url"].as_str().unwrap_or("url");
                format!("🌐 {}", url)
            }
            "web_download" => {
                let url = tc.input["url"].as_str().unwrap_or("url");
                format!("⬇️ Source : {}", url)
            }
            "document_extract" => {
                let path = tc.input["path"].as_str().unwrap_or("document");
                format!("📄 Extraction : {}", path)
            }
            "memory_store" | "remember" => "💾 Mémorisation...".to_string(),
            "memory_recall" | "recall" => "🧠 Je vérifie ma mémoire...".to_string(),
            "agent_send" => {
                let to = tc.input["to"]
                    .as_str()
                    .or(tc.input["agent_id"].as_str())
                    .unwrap_or("agent");
                format!("📨 Message à {}", to)
            }
            "agent_spawn" => "🚀 Lancement d'un sous-agent...".to_string(),
            "cron_create" => "⏰ Création d'une tâche planifiée...".to_string(),
            "cron_update" => "⏰ Mise à jour d'une tâche planifiée...".to_string(),
            "channel_send" => {
                let ch = tc.input["channel"].as_str().unwrap_or("canal");
                format!("📤 Envoi sur {}", ch)
            }
            "skill_execute" => {
                let skill = tc.input["skill"].as_str().unwrap_or("skill");
                format!("🎯 Exécution de {}", skill)
            }
            "image_analyze" | "media_describe" => "👁️ Analyse d'image...".to_string(),
            "browser_navigate" => {
                let url = tc.input["url"].as_str().unwrap_or("page");
                format!("🌐 Navigation vers {}", url)
            }
            name => format!("⚙️ {}...", name.replace('_', " ")),
        })
        .collect();

    match parts.len() {
        0 => "Je m'en occupe...".to_string(),
        1 => parts[0].clone(),
        _ => parts.join("\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::tool::ToolCall;

    #[test]
    fn format_tool_trace_common_tools() {
        let shell = format_tool_trace(
            "shell_exec",
            &serde_json::json!({"command": "ps aux | grep mlx_lm"}),
        );
        assert!(shell.starts_with("💻 shell_exec:"));
        assert!(shell.contains("ps aux"));

        let fread = format_tool_trace("file_read", &serde_json::json!({"path": "Cargo.toml"}));
        assert_eq!(fread, "📖 file_read: Cargo.toml");

        let py = format_tool_trace(
            "execute_code",
            &serde_json::json!({"code": "import requests\nprint(requests.get('x').text)"}),
        );
        assert!(py.starts_with("▸ execute_code:"));

        let screenshot = format_tool_trace(
            "screenshot",
            &serde_json::json!({"save_path": "/tmp/shot.png"}),
        );
        assert_eq!(screenshot, "📸 screenshot: /tmp/shot.png");

        let unknown_mcp = format_tool_trace(
            "mcp_mempalace_search",
            &serde_json::json!({"query": "deployment"}),
        );
        assert!(unknown_mcp.starts_with("🔌 mcp_mempalace_search"));
    }

    #[test]
    fn format_tool_trace_truncates_long_inputs() {
        let long_cmd = "echo ".to_string() + &"x".repeat(200);
        let trace = format_tool_trace("shell_exec", &serde_json::json!({"command": long_cmd}));
        assert!(trace.ends_with("..."));
        assert!(trace.chars().count() <= 80);
    }

    #[test]
    fn tool_input_preview_truncates_long_inputs_on_char_boundaries() {
        let long_cmd = "echo ".to_string() + &"é".repeat(200);
        let preview = tool_input_preview("shell_exec", &serde_json::json!({"command": long_cmd}));

        assert!(preview.ends_with("..."));
        assert_eq!(preview.chars().count(), 50);
    }

    #[test]
    fn build_tool_narration_joins_multiple_tool_lines() {
        let calls = vec![
            ToolCall {
                id: "1".to_string(),
                name: "file_read".to_string(),
                input: serde_json::json!({"path": "Cargo.toml"}),
            },
            ToolCall {
                id: "2".to_string(),
                name: "web_search".to_string(),
                input: serde_json::json!({"query": "captain"}),
            },
        ];

        let narration = build_tool_narration(&calls);

        assert!(narration.contains("Lecture de Cargo.toml"));
        assert!(narration.contains("Recherche"));
        assert!(narration.contains('\n'));
    }
}
