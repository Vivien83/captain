//! Agent execution control route handlers.

use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_types::agent::AgentId;
use std::sync::Arc;

fn parse_agent_id(id: &str) -> Result<AgentId, (StatusCode, Json<serde_json::Value>)> {
    id.parse::<AgentId>().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid agent ID"})),
        )
    })
}

/// POST /api/agents/:id/interrupt - Abort the agent's active task.
pub async fn interrupt_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    let stream_cancelled = state.kernel.stop_agent_run(agent_id).unwrap_or(false);
    state.kernel.scheduler.abort_task(agent_id);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "interrupted",
            "agent_id": id,
            "stream_cancelled": stream_cancelled,
        })),
    )
}

/// POST /api/agents/{id}/stop - Cancel an agent's current LLM run.
pub async fn stop_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    match state.kernel.stop_agent_run(agent_id) {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Run cancelled"})),
        ),
        Ok(false) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "No active run"})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{error}")})),
        ),
    }
}
