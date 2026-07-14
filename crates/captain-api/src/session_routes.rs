//! Session route handlers.

use crate::state::AppState;
use crate::types::SessionRestoreRequest;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_types::agent::{AgentId, SessionId, SessionLabel};
use captain_types::message::{ContentBlock, Message, MessageContent};
use std::collections::HashMap;
use std::sync::Arc;

type JsonResponse = (StatusCode, Json<serde_json::Value>);

fn default_activate_session() -> bool {
    true
}

#[derive(Debug, serde::Deserialize)]
pub struct CreateAgentSessionRequest {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default = "default_activate_session")]
    pub activate: bool,
}

fn parse_agent_id(id: &str) -> Result<AgentId, JsonResponse> {
    id.parse::<AgentId>().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid agent ID"})),
        )
    })
}

fn parse_session_id(id: &str) -> Result<SessionId, JsonResponse> {
    id.parse::<uuid::Uuid>().map(SessionId).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid session ID"})),
        )
    })
}

fn session_messages_public(messages: &[Message]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|message| {
            let mut parts = Vec::new();
            let mut tools = Vec::new();
            let mut images = Vec::new();
            match &message.content {
                MessageContent::Text(text) => {
                    parts.push(text.clone());
                }
                MessageContent::Blocks(blocks) => {
                    for block in blocks {
                        match block {
                            ContentBlock::Text { text, .. } => parts.push(text.clone()),
                            ContentBlock::Image { media_type, .. } => {
                                parts.push("[Image]".to_string());
                                images.push(serde_json::json!({ "media_type": media_type }));
                            }
                            ContentBlock::ToolUse { name, input, .. } => {
                                tools.push(serde_json::json!({
                                    "name": name,
                                    "input": input,
                                }));
                            }
                            ContentBlock::ToolResult {
                                tool_name,
                                content,
                                is_error,
                                ..
                            } => {
                                let preview: String = content.chars().take(2000).collect();
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
        })
        .collect()
}

/// GET /api/sessions - List all sessions with metadata.
pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    match state.kernel.memory.list_sessions() {
        Ok(mut sessions) => {
            let agents = state
                .kernel
                .registry
                .list()
                .into_iter()
                .map(|entry| {
                    (
                        entry.id.to_string(),
                        (entry.name, entry.session_id.to_string()),
                    )
                })
                .collect::<HashMap<_, _>>();
            enrich_session_rows(&mut sessions, &agents);
            if let Some(agent) = params
                .get("agent")
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            {
                let resolved_agent = uuid::Uuid::parse_str(agent)
                    .map(|id| AgentId(id).to_string())
                    .ok()
                    .or_else(|| {
                        state
                            .kernel
                            .registry
                            .find_by_name(agent)
                            .map(|entry| entry.id.to_string())
                    });
                sessions.retain(|session| {
                    let session_agent = session
                        .get("agent_id")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    resolved_agent
                        .as_deref()
                        .map(|id| session_agent == id)
                        .unwrap_or_else(|| session_agent == agent)
                });
            }
            Json(serde_json::json!({"sessions": sessions}))
        }
        Err(_) => Json(serde_json::json!({"sessions": []})),
    }
}

fn enrich_session_rows(
    sessions: &mut [serde_json::Value],
    agents: &HashMap<String, (String, String)>,
) {
    for session in sessions {
        let Some(object) = session.as_object_mut() else {
            continue;
        };
        let agent_id = object
            .get("agent_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let session_id = object
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        if let Some((agent_name, active_session_id)) = agents.get(&agent_id) {
            object.insert(
                "agent_name".to_string(),
                serde_json::Value::String(agent_name.clone()),
            );
            object.insert(
                "active".to_string(),
                serde_json::Value::Bool(active_session_id == &session_id),
            );
        } else {
            object.insert("active".to_string(), serde_json::Value::Bool(false));
        }
    }
}

/// GET /api/sessions/:id - Load a persisted session without switching agents.
pub async fn get_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let session_id = match parse_session_id(&id) {
        Ok(session_id) => session_id,
        Err(response) => return response,
    };

    match state.kernel.memory.get_session(session_id) {
        Ok(Some(session)) => {
            let message_count = session.messages.len();
            let messages = session_messages_public(&session.messages);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "session_id": session.id.0.to_string(),
                    "agent_id": session.agent_id.0.to_string(),
                    "message_count": message_count,
                    "context_window_tokens": session.context_window_tokens,
                    "label": session.label,
                    "messages": messages,
                })),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Session not found"})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": error.to_string()})),
        ),
    }
}

/// GET /api/sessions/:id/events - Timeline replay window for a session.
pub async fn list_session_events(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let from_ts = params
        .get("from")
        .and_then(|value| value.parse::<i64>().ok());
    let to_ts = params.get("to").and_then(|value| value.parse::<i64>().ok());
    let limit = params
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .or(Some(1000));

    let query = captain_memory::event_log::RangeQuery {
        session_id: session_id.clone(),
        from_ts,
        to_ts,
        limit,
    };

    match state.kernel.memory.read_session_events(&query) {
        Ok(events) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id": session_id,
                "events": events,
                "count": events.len(),
            })),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": error.to_string()})),
        ),
    }
}

/// DELETE /api/sessions/:id - Delete a session.
pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let session_id = match parse_session_id(&id) {
        Ok(session_id) => session_id,
        Err(response) => return response,
    };

    match state.kernel.memory.delete_session(session_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "deleted", "session_id": id})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": error.to_string()})),
        ),
    }
}

/// PUT /api/sessions/:id/label - Set a session label.
pub async fn set_session_label(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let session_id = match parse_session_id(&id) {
        Ok(session_id) => session_id,
        Err(response) => return response,
    };
    let label = req.get("label").and_then(|value| value.as_str());
    if let Some(label_value) = label {
        if let Err(error) = SessionLabel::new(label_value) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": error.to_string()})),
            );
        }
    }

    match state.kernel.memory.set_session_label(session_id, label) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "updated",
                "session_id": id,
                "label": label,
            })),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": error.to_string()})),
        ),
    }
}

/// GET /api/sessions/by-label/:label - Find session by label scoped to agent.
pub async fn find_session_by_label(
    State(state): State<Arc<AppState>>,
    Path((agent_id_str, label)): Path<(String, String)>,
) -> impl IntoResponse {
    let agent_id = match agent_id_str.parse::<uuid::Uuid>() {
        Ok(id) => AgentId(id),
        Err(_) => match state.kernel.registry.find_by_name(&agent_id_str) {
            Some(entry) => entry.id,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "Agent not found"})),
                );
            }
        },
    };

    match state.kernel.memory.find_session_by_label(agent_id, &label) {
        Ok(Some(session)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id": session.id.0.to_string(),
                "agent_id": session.agent_id.0.to_string(),
                "label": session.label,
                "message_count": session.messages.len(),
            })),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "No session found with that label"})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": error.to_string()})),
        ),
    }
}

/// GET /api/agents/{id}/sessions - List all sessions for an agent.
pub async fn list_agent_sessions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };
    match state.kernel.list_agent_sessions(agent_id) {
        Ok(sessions) => (
            StatusCode::OK,
            Json(serde_json::json!({"sessions": sessions})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{error}")})),
        ),
    }
}

/// POST /api/agents/{id}/sessions - Create a new session for an agent.
pub async fn create_agent_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateAgentSessionRequest>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };
    if let Some(label) = req.label.as_deref() {
        if let Err(error) = SessionLabel::new(label) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": error.to_string()})),
            );
        }
    }
    let result = if req.activate {
        state
            .kernel
            .create_agent_session(agent_id, req.label.as_deref())
    } else {
        state
            .kernel
            .create_agent_session_detached(agent_id, req.label.as_deref())
    };
    match result {
        Ok(session) => (StatusCode::OK, Json(session)),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{error}")})),
        ),
    }
}

/// POST /api/agents/{id}/sessions/{session_id}/switch - Switch sessions.
pub async fn switch_agent_session(
    State(state): State<Arc<AppState>>,
    Path((id, session_id_str)): Path<(String, String)>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };
    let session_id = match parse_session_id(&session_id_str) {
        Ok(session_id) => session_id,
        Err(response) => return response,
    };
    match state.kernel.switch_agent_session(agent_id, session_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Session switched"})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{error}")})),
        ),
    }
}

/// POST /api/agents/{id}/session/restore - Restore supplied message history.
pub async fn restore_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<SessionRestoreRequest>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };
    match state.kernel.restore_agent_session(agent_id, body.messages) {
        Ok(session_id) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "session_id": session_id.0.to_string(),
            })),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{error}")})),
        ),
    }
}

/// POST /api/agents/{id}/session/reset - Reset an agent's session.
pub async fn reset_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };
    match state.kernel.reset_session(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Session reset"})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{error}")})),
        ),
    }
}

/// DELETE /api/agents/{id}/history - Clear all conversation history.
pub async fn clear_agent_history(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };
    if state.kernel.registry.get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Agent not found"})),
        );
    }
    match state.kernel.clear_agent_history(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "All history cleared"})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{error}")})),
        ),
    }
}

/// POST /api/agents/{id}/session/compact - Trigger LLM session compaction.
pub async fn compact_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };
    match state.kernel.compact_agent_session(agent_id).await {
        Ok(message) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": message})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{error}")})),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_session_rows_include_owner_and_active_state() {
        let mut sessions = vec![
            serde_json::json!({"session_id": "session-active", "agent_id": "agent-a"}),
            serde_json::json!({"session_id": "session-old", "agent_id": "agent-a"}),
            serde_json::json!({"session_id": "session-orphan", "agent_id": "agent-missing"}),
        ];
        let agents = HashMap::from([(
            "agent-a".to_string(),
            ("captain".to_string(), "session-active".to_string()),
        )]);

        enrich_session_rows(&mut sessions, &agents);

        assert_eq!(sessions[0]["agent_name"], "captain");
        assert_eq!(sessions[0]["active"], true);
        assert_eq!(sessions[1]["active"], false);
        assert_eq!(sessions[2]["active"], false);
    }
}
