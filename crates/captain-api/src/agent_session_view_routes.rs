//! Agent session view route handlers.

use crate::state::AppState;
use crate::upload_routes::register_upload;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use captain_types::agent::AgentId;
use std::sync::Arc;

/// GET /api/agents/:id/session - Get agent session history.
pub async fn get_agent_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl axum::response::IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(agent_id) => agent_id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    let entry = match state.kernel.registry.get(agent_id) {
        Some(entry) => entry,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    };
    let effective_context_window = state
        .kernel
        .effective_context_window_for_agent(agent_id)
        .unwrap_or_default() as u64;

    match state.kernel.memory.get_session(entry.session_id) {
        Ok(Some(session)) => {
            let messages = build_session_messages(&session.messages);
            let estimated_context_tokens =
                captain_runtime::compactor::estimate_token_count(&session.messages, None, None);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "session_id": session.id.0.to_string(),
                    "agent_id": session.agent_id.0.to_string(),
                    "message_count": session.messages.len(),
                    "context_window_tokens": effective_context_window,
                    "estimated_context_tokens": estimated_context_tokens,
                    "label": session.label,
                    "messages": messages,
                })),
            )
        }
        Ok(None) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id": entry.session_id.0.to_string(),
                "agent_id": agent_id.to_string(),
                "message_count": 0,
                "context_window_tokens": effective_context_window,
                "estimated_context_tokens": 0,
                "messages": [],
            })),
        ),
        Err(e) => {
            tracing::warn!("Session load failed for agent {id}: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Session load failed"})),
            )
        }
    }
}

fn build_session_messages(messages: &[captain_types::message::Message]) -> Vec<serde_json::Value> {
    let mut built_messages = Vec::new();
    let mut tool_use_index = std::collections::HashMap::new();

    for message in messages {
        let (content, tools, images, tool_ids) = message_parts(message);
        if content.is_empty() && tools.is_empty() {
            continue;
        }

        let msg_idx = built_messages.len();
        for (tool_id, tool_idx) in tool_ids {
            tool_use_index.insert(tool_id, (msg_idx, tool_idx));
        }

        let mut value = serde_json::json!({
            "role": format!("{:?}", message.role),
            "content": content,
        });
        if !tools.is_empty() {
            value["tools"] = serde_json::Value::Array(tools);
        }
        if !images.is_empty() {
            value["images"] = serde_json::Value::Array(images);
        }
        built_messages.push(value);
    }

    attach_tool_results(messages, &tool_use_index, &mut built_messages);
    built_messages
}

#[allow(clippy::type_complexity)]
fn message_parts(
    message: &captain_types::message::Message,
) -> (
    String,
    Vec<serde_json::Value>,
    Vec<serde_json::Value>,
    Vec<(String, usize)>,
) {
    match &message.content {
        captain_types::message::MessageContent::Text(text) => {
            (text.clone(), Vec::new(), Vec::new(), Vec::new())
        }
        captain_types::message::MessageContent::Blocks(blocks) => blocks_parts(blocks),
    }
}

#[allow(clippy::type_complexity)]
fn blocks_parts(
    blocks: &[captain_types::message::ContentBlock],
) -> (
    String,
    Vec<serde_json::Value>,
    Vec<serde_json::Value>,
    Vec<(String, usize)>,
) {
    let mut texts = Vec::new();
    let mut tools = Vec::new();
    let mut images = Vec::new();
    let mut tool_ids = Vec::new();

    for block in blocks {
        match block {
            captain_types::message::ContentBlock::Text { text, .. } => texts.push(text.clone()),
            captain_types::message::ContentBlock::Image { media_type, data } => {
                texts.push("[Image]".to_string());
                if let Some(image) = persist_session_image(media_type, data) {
                    images.push(image);
                }
            }
            captain_types::message::ContentBlock::ToolUse {
                id, name, input, ..
            } => {
                let tool_idx = tools.len();
                tools.push(serde_json::json!({
                    "name": name,
                    "input": input,
                    "running": false,
                    "expanded": false,
                }));
                tool_ids.push((id.clone(), tool_idx));
            }
            captain_types::message::ContentBlock::ToolResult { .. } => {}
            _ => {}
        }
    }

    (texts.join("\n"), tools, images, tool_ids)
}

fn persist_session_image(media_type: &str, data: &str) -> Option<serde_json::Value> {
    use base64::Engine as _;

    let file_id = uuid::Uuid::new_v4().to_string();
    let upload_dir = std::env::temp_dir().join("captain_uploads");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .ok()?;
    std::fs::create_dir_all(&upload_dir).ok()?;
    std::fs::write(upload_dir.join(&file_id), bytes).ok()?;

    let filename = format!("image.{}", media_type.rsplit('/').next().unwrap_or("png"));
    register_upload(file_id.clone(), filename.clone(), media_type.to_string());
    Some(serde_json::json!({
        "file_id": file_id,
        "filename": filename,
    }))
}

fn attach_tool_results(
    messages: &[captain_types::message::Message],
    tool_use_index: &std::collections::HashMap<String, (usize, usize)>,
    built_messages: &mut [serde_json::Value],
) {
    for message in messages {
        if let captain_types::message::MessageContent::Blocks(blocks) = &message.content {
            for block in blocks {
                attach_tool_result_block(block, tool_use_index, built_messages);
            }
        }
    }
}

fn attach_tool_result_block(
    block: &captain_types::message::ContentBlock,
    tool_use_index: &std::collections::HashMap<String, (usize, usize)>,
    built_messages: &mut [serde_json::Value],
) {
    let captain_types::message::ContentBlock::ToolResult {
        tool_use_id,
        content,
        is_error,
        ..
    } = block
    else {
        return;
    };

    let Some(&(msg_idx, tool_idx)) = tool_use_index.get(tool_use_id) else {
        return;
    };
    let Some(message) = built_messages.get_mut(msg_idx) else {
        return;
    };
    let Some(tools) = message
        .get_mut("tools")
        .and_then(|value| value.as_array_mut())
    else {
        return;
    };
    let Some(tool) = tools.get_mut(tool_idx) else {
        return;
    };

    tool["result"] = serde_json::Value::String(content.chars().take(2000).collect());
    tool["is_error"] = serde_json::Value::Bool(*is_error);
}
