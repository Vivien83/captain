//! Small helpers used by the per-agent API ingress route.

use crate::{
    agent_api_audit::record_egress_callback,
    agent_api_egress::AgentApiCallbackDelivery,
    agent_api_egress_queue::enqueue_agent_api_callback,
    agent_api_idempotency::{
        mark_agent_api_request_status, AgentApiIdempotencyEntry, AgentApiIdempotencyStatus,
    },
    state::AppState,
};
use axum::{http::StatusCode, response::IntoResponse, Json};
use captain_types::agent::AgentId;

pub(crate) async fn update_idempotency_status(
    state: &AppState,
    agent_id: &AgentId,
    request_id: Option<&str>,
    status: AgentApiIdempotencyStatus,
) {
    if let Err(err) =
        mark_agent_api_request_status(&state.kernel.config.home_dir, agent_id, request_id, status)
            .await
    {
        tracing::warn!(error = %err, "agent API idempotency update failed");
    }
}

pub(crate) fn duplicate_response(
    agent_id: &AgentId,
    entry: AgentApiIdempotencyEntry,
) -> axum::response::Response {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "duplicate",
            "agent_id": agent_id.to_string(),
            "request_id": entry.request_id,
            "original_status": entry.status.as_str(),
            "first_seen_at": entry.first_seen_at,
            "updated_at": entry.updated_at,
        })),
    )
        .into_response()
}

pub(crate) fn conflict_response(
    agent_id: &AgentId,
    entry: AgentApiIdempotencyEntry,
) -> axum::response::Response {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({
            "error": "request_id already used with a different request body",
            "agent_id": agent_id.to_string(),
            "request_id": entry.request_id,
            "original_status": entry.status.as_str(),
            "first_seen_at": entry.first_seen_at,
        })),
    )
        .into_response()
}

pub(crate) fn record_callback_audit(
    state: &AppState,
    agent_id: &AgentId,
    payload: &serde_json::Value,
    delivery: &AgentApiCallbackDelivery,
) {
    record_egress_callback(
        state.kernel.audit_log.as_ref(),
        agent_id,
        payload.get("request_id").and_then(|value| value.as_str()),
        payload
            .get("event")
            .and_then(|value| value.as_str())
            .unwrap_or("agent_api.callback"),
        &delivery.audit_outcome(),
    );
}

pub(crate) async fn queue_failed_callback(
    state: &AppState,
    agent_id: &AgentId,
    payload: &serde_json::Value,
    delivery: &mut AgentApiCallbackDelivery,
) {
    if !delivery.should_queue() {
        return;
    }
    match enqueue_agent_api_callback(
        &state.kernel.config.home_dir,
        agent_id,
        payload,
        delivery.error_message(),
    )
    .await
    {
        Ok(id) => delivery.mark_queued(id),
        Err(err) => delivery.mark_queue_error(err),
    }
}
