//! Human approval route handlers.

use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use captain_types::approval::{ApprovalDecision, ApprovalRequest, RiskLevel};
use std::sync::Arc;

/// POST /api/approvals - Create a manual approval request.
#[derive(serde::Deserialize)]
pub struct CreateApprovalRequest {
    pub agent_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub action_summary: String,
}

/// GET /api/approvals - List pending approval requests.
pub async fn list_approvals(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let pending = state.kernel.approval_manager.list_pending();
    let total = pending.len();
    let registry_agents = state.kernel.registry.list();

    let approvals: Vec<serde_json::Value> = pending
        .into_iter()
        .map(|approval| {
            let agent_name = registry_agents
                .iter()
                .find(|agent| {
                    agent.id.to_string() == approval.agent_id || agent.name == approval.agent_id
                })
                .map(|agent| agent.name.as_str())
                .unwrap_or(&approval.agent_id);
            serde_json::json!({
                "id": approval.id,
                "agent_id": approval.agent_id,
                "agent_name": agent_name,
                "tool_name": approval.tool_name,
                "description": approval.description,
                "action_summary": approval.action_summary,
                "action": approval.action_summary,
                "risk_level": approval.risk_level,
                "requested_at": approval.requested_at,
                "created_at": approval.requested_at,
                "timeout_secs": approval.timeout_secs,
                "status": "pending"
            })
        })
        .collect();

    Json(serde_json::json!({"approvals": approvals, "total": total}))
}

/// POST /api/approvals - Create a manual approval request for external systems.
pub async fn create_approval(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateApprovalRequest>,
) -> impl IntoResponse {
    let policy = state.kernel.approval_manager.policy();
    let id = uuid::Uuid::new_v4();
    let approval_req = ApprovalRequest {
        id,
        agent_id: req.agent_id,
        tool_name: req.tool_name.clone(),
        description: if req.description.is_empty() {
            format!("Manual approval request for {}", req.tool_name)
        } else {
            req.description
        },
        action_summary: if req.action_summary.is_empty() {
            req.tool_name.clone()
        } else {
            req.action_summary
        },
        risk_level: RiskLevel::High,
        requested_at: chrono::Utc::now(),
        timeout_secs: policy.timeout_secs,
    };

    let kernel = Arc::clone(&state.kernel);
    tokio::spawn(async move {
        kernel.approval_manager.request_approval(approval_req).await;
    });

    (
        StatusCode::CREATED,
        Json(serde_json::json!({"id": id.to_string(), "status": "pending"})),
    )
}

/// POST /api/approvals/{id}/approve - Approve a pending request.
pub async fn approve_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    resolve_approval(state, id, ApprovalDecision::Approved, "approved")
}

/// POST /api/approvals/{id}/reject - Reject a pending request.
pub async fn reject_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    resolve_approval(state, id, ApprovalDecision::Denied, "rejected")
}

/// POST /api/approvals/{id}/approve_session - Approve for the current daemon session.
pub async fn approve_session_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    resolve_approval(
        state,
        id,
        ApprovalDecision::ApprovedSession,
        "approved_session",
    )
}

/// POST /api/approvals/{id}/approve_always - Approve permanently in policy.
pub async fn approve_always_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    resolve_approval(
        state,
        id,
        ApprovalDecision::ApprovedAlways,
        "approved_always",
    )
}

/// POST /api/approvals/clear_session - Drop cached session approvals.
pub async fn clear_session_approvals(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let before = state.kernel.approval_manager.session_cache_size();
    state.kernel.approval_manager.clear_session_cache();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "cleared",
            "removed": before
        })),
    )
}

fn resolve_approval(
    state: Arc<AppState>,
    id: String,
    decision: ApprovalDecision,
    status: &'static str,
) -> Response {
    let uuid = match parse_approval_id(&id) {
        Ok(uuid) => uuid,
        Err(response) => return response,
    };

    match state
        .kernel
        .approval_manager
        .resolve(uuid, decision, Some("api".to_string()))
    {
        Ok(response) => json_response(
            StatusCode::OK,
            serde_json::json!({
                "id": id,
                "status": status,
                "decided_at": response.decided_at.to_rfc3339()
            }),
        ),
        Err(error) => json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": error})),
    }
}

#[allow(clippy::result_large_err)]
fn parse_approval_id(id: &str) -> Result<uuid::Uuid, Response> {
    uuid::Uuid::parse_str(id).map_err(|_| {
        json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"error": "Invalid approval ID"}),
        )
    })
}

fn json_response(status: StatusCode, body: serde_json::Value) -> Response {
    (status, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_approval_id_accepts_uuid() {
        let id = uuid::Uuid::new_v4().to_string();

        assert!(parse_approval_id(&id).is_ok());
    }

    #[test]
    fn parse_approval_id_rejects_invalid_id() {
        let response = parse_approval_id("not-a-uuid").unwrap_err();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
