//! Per-agent API egress operation routes.

use crate::{
    agent_api_egress_queue::{retry_agent_api_callback_now, AgentApiEgressRetryError},
    state::AppState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use captain_types::agent::AgentId;
use std::sync::Arc;

/// POST /api/agents/:id/api/egress/:queue_id/retry - Retry one queued callback now.
pub async fn agent_api_egress_retry(
    State(state): State<Arc<AppState>>,
    Path((id, queue_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    if state.kernel.registry.get(agent_id).is_none() {
        return error(StatusCode::NOT_FOUND, "Agent not found");
    }

    match retry_agent_api_callback_now(
        &state.kernel.config.home_dir,
        state.kernel.audit_log.as_ref(),
        &agent_id,
        &queue_id,
    )
    .await
    {
        Ok(result) => (StatusCode::OK, Json(serde_json::json!(result))).into_response(),
        Err(AgentApiEgressRetryError::NotFound) => {
            error(StatusCode::NOT_FOUND, "Queued callback not found")
        }
        Err(AgentApiEgressRetryError::Store(err)) => error(StatusCode::INTERNAL_SERVER_ERROR, &err),
    }
}

#[allow(clippy::result_large_err)]
fn parse_agent_id(id: &str) -> Result<AgentId, axum::response::Response> {
    id.parse()
        .map_err(|_| error(StatusCode::BAD_REQUEST, "Invalid agent ID"))
}

fn error(status: StatusCode, message: &str) -> axum::response::Response {
    (status, Json(serde_json::json!({"error": message}))).into_response()
}
