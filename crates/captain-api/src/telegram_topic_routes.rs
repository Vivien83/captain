//! Telegram topic route handlers.

use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use captain_types::agent::AgentId;
use std::sync::Arc;

/// GET /api/channels/telegram/topics - List all topic to agent mappings.
pub async fn list_telegram_topics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mappings = state
        .kernel
        .registry
        .list()
        .into_iter()
        .filter_map(|entry| {
            state
                .kernel
                .get_telegram_topic(&entry.name)
                .map(|topic_id| {
                    serde_json::json!({
                        "topic_id": topic_id,
                        "agent_id": entry.id.to_string(),
                        "agent_name": entry.name,
                    })
                })
        })
        .collect::<Vec<_>>();

    Json(serde_json::json!({ "topics": mappings, "total": mappings.len() }))
}

/// POST /api/channels/telegram/topics - Set a topic to agent mapping.
pub async fn set_telegram_topic(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let topic_id = match body["topic_id"].as_str() {
        Some(topic_id) if !topic_id.is_empty() => topic_id,
        _ => return error(StatusCode::BAD_REQUEST, "Missing 'topic_id'"),
    };

    let agent_id = match resolve_agent_id(&state, &body) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    let key = format!("topic_agent:{topic_id}");
    let _ = state.kernel.memory.structured_set(
        AgentId(uuid::Uuid::nil()),
        &key,
        serde_json::Value::String(agent_id.to_string()),
    );

    if let Some(entry) = state.kernel.registry.get(agent_id) {
        state.kernel.set_telegram_topic(&entry.name, topic_id);
    }

    let name = state
        .kernel
        .registry
        .get(agent_id)
        .map(|entry| entry.name.clone())
        .unwrap_or_default();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "topic_id": topic_id,
            "agent_id": agent_id.to_string(),
            "agent_name": name,
        })),
    )
        .into_response()
}

/// DELETE /api/channels/telegram/topics/:thread_id - Remove a topic mapping.
pub async fn delete_telegram_topic(
    State(state): State<Arc<AppState>>,
    Path(thread_id): Path<String>,
) -> impl IntoResponse {
    let key = format!("topic_agent:{thread_id}");
    let _ = state.kernel.memory.structured_set(
        AgentId(uuid::Uuid::nil()),
        &key,
        serde_json::Value::Null,
    );

    for entry in state.kernel.registry.list() {
        if state.kernel.get_telegram_topic(&entry.name).as_deref() == Some(&thread_id) {
            let topic_key = format!("telegram_topic:{}", entry.name);
            let _ = state.kernel.memory.structured_set(
                AgentId(uuid::Uuid::nil()),
                &topic_key,
                serde_json::Value::Null,
            );
        }
    }

    Json(serde_json::json!({"status": "deleted", "topic_id": thread_id})).into_response()
}

#[allow(clippy::result_large_err)]
fn resolve_agent_id(
    state: &AppState,
    body: &serde_json::Value,
) -> Result<AgentId, axum::response::Response> {
    let agent_id = body["agent_id"].as_str().unwrap_or("");
    if !agent_id.is_empty() {
        return parse_known_agent_id(state, agent_id);
    }

    let agent_name = body["agent_name"].as_str().unwrap_or("");
    if !agent_name.is_empty() {
        return state
            .kernel
            .registry
            .find_by_name(agent_name)
            .map(|entry| entry.id)
            .ok_or_else(|| {
                error(
                    StatusCode::NOT_FOUND,
                    &format!("Agent '{agent_name}' not found"),
                )
            });
    }

    Err(error(
        StatusCode::BAD_REQUEST,
        "Missing 'agent_id' or 'agent_name'",
    ))
}

#[allow(clippy::result_large_err)]
fn parse_known_agent_id(
    state: &AppState,
    agent_id: &str,
) -> Result<AgentId, axum::response::Response> {
    let id = uuid::Uuid::parse_str(agent_id)
        .map(AgentId)
        .map_err(|_| error(StatusCode::BAD_REQUEST, "Invalid agent_id"))?;

    if state.kernel.registry.get(id).is_none() {
        return Err(error(StatusCode::NOT_FOUND, "Agent not found"));
    }
    Ok(id)
}

fn error(status: StatusCode, message: &str) -> axum::response::Response {
    (status, Json(serde_json::json!({"error": message}))).into_response()
}
