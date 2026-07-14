pub(super) fn session_detail_text(detail: &serde_json::Value) -> String {
    detail["messages"]
        .as_array()
        .map(|messages| {
            messages
                .iter()
                .map(message_plain_text)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn message_plain_text(message: &serde_json::Value) -> String {
    let mut out = String::new();
    if let Some(content) = message["content"].as_str() {
        out.push_str(content);
    } else {
        out.push_str(&serde_json::to_string(&message["content"]).unwrap_or_default());
    }
    if let Some(tools) = message["tools"].as_array() {
        for tool in tools {
            let result = tool["result"].as_str().unwrap_or("");
            if !result.trim().is_empty() && out.contains(result.trim()) {
                continue;
            }
            if !out.trim().is_empty() {
                out.push('\n');
            }
            out.push_str("[tool] ");
            out.push_str(tool["name"].as_str().unwrap_or("?"));
            if !result.is_empty() {
                out.push_str(": ");
                out.push_str(result);
            }
        }
    }
    out
}

pub(super) fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

pub(super) fn match_snippet(haystack: &str, needle: &str, max_chars: usize) -> String {
    let lower_haystack = haystack.to_lowercase();
    let lower_needle = needle.to_lowercase();
    let idx_chars = lower_haystack
        .find(&lower_needle)
        .map(|idx| lower_haystack[..idx].chars().count())
        .unwrap_or(0);
    let start = idx_chars.saturating_sub(max_chars / 3);
    haystack
        .chars()
        .skip(start)
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_string()
}

pub(super) fn format_session_markdown(detail: &serde_json::Value) -> String {
    let mut out = String::new();
    out.push_str("# Captain Session\n\n");
    out.push_str(&format!(
        "- Session: `{}`\n",
        detail["session_id"].as_str().unwrap_or("?")
    ));
    out.push_str(&format!(
        "- Agent: `{}`\n",
        detail["agent_id"].as_str().unwrap_or("?")
    ));
    out.push_str(&format!(
        "- Messages: {}\n",
        detail["message_count"].as_u64().unwrap_or(0)
    ));
    out.push_str(&format!(
        "- Context tokens: {}\n",
        detail["context_window_tokens"].as_u64().unwrap_or(0)
    ));
    if let Some(label) = detail["label"].as_str().filter(|s| !s.is_empty()) {
        out.push_str(&format!("- Label: {label}\n"));
    }
    out.push_str("\n## Messages\n\n");
    if let Some(messages) = detail["messages"].as_array() {
        for (idx, message) in messages.iter().enumerate() {
            out.push_str(&format!(
                "### {}. {}\n\n",
                idx + 1,
                message["role"].as_str().unwrap_or("?")
            ));
            let text = message_plain_text(message);
            out.push_str(text.trim());
            out.push_str("\n\n");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn contains_ci_matches_case_insensitively() {
        assert!(contains_ci("Captain Session History", "session"));
        assert!(contains_ci("Captain Session History", "CAPTAIN"));
        assert!(!contains_ci("Captain Session History", "memory"));
    }

    #[test]
    fn match_snippet_returns_trimmed_context() {
        let snippet = match_snippet("alpha beta gamma delta epsilon", "gamma", 12);

        assert!(snippet.contains("gamma"));
        assert!(snippet.len() <= 12);
        assert_eq!(snippet.trim(), snippet);
    }

    #[test]
    fn session_markdown_includes_messages_and_tool_results() {
        let detail = json!({
            "session_id": "session-1",
            "agent_id": "captain",
            "message_count": 1,
            "context_window_tokens": 42,
            "label": "debug",
            "messages": [{
                "role": "assistant",
                "content": "Done",
                "tools": [{
                    "name": "shell",
                    "result": "cargo check ok"
                }]
            }]
        });

        let rendered = format_session_markdown(&detail);

        assert!(rendered.contains("# Captain Session"));
        assert!(rendered.contains("- Session: `session-1`"));
        assert!(rendered.contains("### 1. assistant"));
        assert!(rendered.contains("[tool] shell: cargo check ok"));
    }
}
