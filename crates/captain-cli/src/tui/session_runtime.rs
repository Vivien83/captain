use super::screens::chat::{ChatState, Role};
use captain_memory::session::Session;
use captain_types::message::{ContentBlock, MessageContent};

#[derive(Clone, Debug)]
pub struct LoadedSession {
    pub session_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub label: String,
    pub detail: serde_json::Value,
}

pub fn loaded_session_from_detail(
    detail: serde_json::Value,
    agent_name: String,
) -> Result<LoadedSession, String> {
    let session_id = detail
        .get("session_id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| uuid::Uuid::parse_str(value).is_ok())
        .ok_or_else(|| "Session response has no valid session ID".to_string())?
        .to_string();
    let agent_id = detail
        .get("agent_id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| uuid::Uuid::parse_str(value).is_ok())
        .ok_or_else(|| "Session response has no valid owner agent ID".to_string())?
        .to_string();
    let label = public_session_label(&detail).to_string();
    Ok(LoadedSession {
        session_id,
        agent_id,
        agent_name,
        label,
        detail,
    })
}

pub fn restore_public_session_messages(chat: &mut ChatState, detail: &serde_json::Value) -> usize {
    chat.messages.clear();
    chat.scroll_offset = 0;

    let mut restored = 0usize;
    if let Some(messages) = detail.get("messages").and_then(serde_json::Value::as_array) {
        for message in messages {
            let text = public_session_message_text(message);
            if text.trim().is_empty() {
                continue;
            }
            chat.push_message(public_session_message_role(message), text);
            restored += 1;
        }
    }
    restored
}

pub fn public_session_label(detail: &serde_json::Value) -> &str {
    detail
        .get("label")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("session persistée")
}

pub fn session_values(body: &serde_json::Value) -> Vec<serde_json::Value> {
    body.as_array()
        .or_else(|| body.get("sessions").and_then(serde_json::Value::as_array))
        .cloned()
        .unwrap_or_default()
}

/// Resolve a full UUID, a unique UUID prefix, or a title. Session lists are
/// newest-first, so duplicate exact titles deliberately select the latest.
pub fn resolve_session_selector(
    sessions: &[serde_json::Value],
    selector: &str,
) -> Result<String, String> {
    let selector = selector.trim();
    if selector.is_empty() {
        return Err("Usage: /resume <session-id ou titre>".to_string());
    }

    if let Some(id) = sessions
        .iter()
        .filter_map(session_id)
        .find(|id| *id == selector)
    {
        return Ok(id.to_string());
    }

    let prefixed = sessions
        .iter()
        .filter_map(session_id)
        .filter(|id| id.starts_with(selector))
        .collect::<Vec<_>>();
    match prefixed.as_slice() {
        [id] => return Ok((*id).to_string()),
        [_, _, ..] => return Err("Préfixe de session ambigu ; utilise plus de caractères.".into()),
        [] => {}
    }

    if let Some(id) = sessions.iter().find_map(|session| {
        let label = session.get("label")?.as_str()?;
        label
            .eq_ignore_ascii_case(selector)
            .then(|| session_id(session).map(str::to_string))
            .flatten()
    }) {
        return Ok(id);
    }

    let selector_lower = selector.to_lowercase();
    let title_matches = sessions
        .iter()
        .filter_map(|session| {
            let label = session.get("label")?.as_str()?;
            label
                .to_lowercase()
                .contains(&selector_lower)
                .then(|| session_id(session).map(str::to_string))
                .flatten()
        })
        .collect::<Vec<_>>();
    match title_matches.as_slice() {
        [id] => Ok(id.clone()),
        [_, _, ..] => Err("Titre de session ambigu ; précise le titre ou l'UUID.".into()),
        [] => Err(format!("Session introuvable : {selector}")),
    }
}

pub fn native_session_detail(session: &Session) -> serde_json::Value {
    serde_json::json!({
        "session_id": session.id.0.to_string(),
        "agent_id": session.agent_id.0.to_string(),
        "message_count": session.messages.len(),
        "context_window_tokens": session.context_window_tokens,
        "label": session.label,
        "messages": session.messages.iter().map(native_message_public).collect::<Vec<_>>(),
    })
}

fn session_id(value: &serde_json::Value) -> Option<&str> {
    value
        .get("session_id")
        .or_else(|| value.get("id"))
        .and_then(serde_json::Value::as_str)
}

fn public_session_message_role(message: &serde_json::Value) -> Role {
    match message
        .get("role")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "user" => Role::User,
        "assistant" | "agent" => Role::Agent,
        "tool" => Role::Tool,
        _ => Role::System,
    }
}

fn public_session_message_text(message: &serde_json::Value) -> String {
    let mut parts = Vec::new();
    if let Some(content) = message
        .get("content")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|content| !content.is_empty())
    {
        parts.push(content.to_string());
    }
    if let Some(images) = message.get("images").and_then(serde_json::Value::as_array) {
        for image in images {
            let media = image
                .get("media_type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("image");
            parts.push(format!("[Image: {media}]"));
        }
    }
    if let Some(tools) = message.get("tools").and_then(serde_json::Value::as_array) {
        for tool in tools {
            let name = tool
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("tool");
            let preview = tool
                .get("result")
                .or_else(|| tool.get("input"))
                .map(compact_json_preview)
                .unwrap_or_default();
            if preview.is_empty() {
                parts.push(format!("[Tool: {name}]"));
            } else {
                parts.push(format!("[Tool: {name}] {preview}"));
            }
        }
    }
    parts.join("\n\n")
}

fn compact_json_preview(value: &serde_json::Value) -> String {
    let raw = value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_default());
    let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    const MAX: usize = 240;
    if compact.chars().count() <= MAX {
        compact
    } else {
        compact.chars().take(MAX - 1).collect::<String>() + "…"
    }
}

fn native_message_public(message: &captain_types::message::Message) -> serde_json::Value {
    let mut parts = Vec::new();
    let mut tools = Vec::new();
    let mut images = Vec::new();
    match &message.content {
        MessageContent::Text(text) => parts.push(text.clone()),
        MessageContent::Blocks(blocks) => {
            for block in blocks {
                match block {
                    ContentBlock::Text { text, .. } => parts.push(text.clone()),
                    ContentBlock::Image { media_type, .. } => {
                        parts.push("[Image]".to_string());
                        images.push(serde_json::json!({"media_type": media_type}));
                    }
                    ContentBlock::ToolUse { name, input, .. } => {
                        tools.push(serde_json::json!({"name": name, "input": input}));
                    }
                    ContentBlock::ToolResult {
                        tool_name,
                        content,
                        is_error,
                        ..
                    } => {
                        let preview = content.chars().take(2_000).collect::<String>();
                        parts.push(format!(
                            "[Tool result{}] {}",
                            if *is_error { " error" } else { "" },
                            preview
                        ));
                        if !tool_name.is_empty() {
                            tools.push(serde_json::json!({
                                "name": tool_name,
                                "result": preview,
                                "is_error": is_error,
                            }));
                        }
                    }
                    ContentBlock::Thinking { .. } | ContentBlock::Unknown => {}
                }
            }
        }
    }
    let mut output = serde_json::json!({
        "role": format!("{:?}", message.role).to_lowercase(),
        "content": parts.join("\n"),
    });
    if !tools.is_empty() {
        output["tools"] = serde_json::Value::Array(tools);
    }
    if !images.is_empty() {
        output["images"] = serde_json::Value::Array(images);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_accepts_uuid_prefix_and_latest_exact_title() {
        let sessions = vec![
            serde_json::json!({"session_id": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "label": "Inventory health"}),
            serde_json::json!({"session_id": "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", "label": "Inventory health"}),
        ];
        assert_eq!(
            resolve_session_selector(&sessions, "aaaaaaaa").unwrap(),
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
        );
        assert_eq!(
            resolve_session_selector(&sessions, "Inventory health").unwrap(),
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
        );
    }

    #[test]
    fn selector_rejects_ambiguous_partial_title() {
        let sessions = vec![
            serde_json::json!({"session_id": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "label": "Inventory health"}),
            serde_json::json!({"session_id": "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", "label": "Inventory deploy"}),
        ];
        assert!(resolve_session_selector(&sessions, "Inventory").is_err());
    }
}
