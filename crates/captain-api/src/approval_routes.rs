//! Human approval route handlers.

use crate::state::AppState;
use axum::{
    body::Bytes,
    extract::{Extension, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use captain_kernel::hub_pairing_service::DeviceAccessIdentity;
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

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RejectApprovalRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

/// GET /api/approvals - List pending approval requests.
pub async fn list_approvals(
    State(state): State<Arc<AppState>>,
    client: Option<Extension<DeviceAccessIdentity>>,
) -> impl IntoResponse {
    let pending = state.kernel.approval_manager.list_pending();
    let total = pending.len();
    let rules = if client.is_none() {
        state.kernel.approval_manager.list_rules()
    } else {
        Vec::new()
    };
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

    Json(serde_json::json!({
        "approvals": approvals,
        "total": total,
        "rules": rules,
        "rules_total": rules.len()
    }))
}

/// POST /api/approvals - Create a manual approval request for external systems.
pub async fn create_approval(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateApprovalRequest>,
) -> Response {
    let policy = state.kernel.approval_manager.policy();
    let id = uuid::Uuid::new_v4();
    let action_summary = if req.action_summary.is_empty() {
        req.tool_name.clone()
    } else {
        req.action_summary
    };
    let approval_req = ApprovalRequest {
        id,
        agent_id: req.agent_id,
        tool_name: req.tool_name.clone(),
        description: if req.description.is_empty() {
            format!("Manual approval request for {}", req.tool_name)
        } else {
            req.description
        },
        action_digest: captain_types::approval::approval_action_digest(
            &req.tool_name,
            action_summary.as_bytes(),
        ),
        action_summary,
        risk_level: RiskLevel::High,
        requested_at: chrono::Utc::now(),
        timeout_secs: policy.timeout_secs,
    };
    if let Err(error) = approval_req.validate() {
        return json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": error}));
    }

    let kernel = Arc::clone(&state.kernel);
    tokio::spawn(async move {
        kernel.approval_manager.request_approval(approval_req).await;
    });

    json_response(
        StatusCode::CREATED,
        serde_json::json!({"id": id.to_string(), "status": "pending"}),
    )
}

/// POST /api/approvals/{id}/approve - Approve a pending request.
pub async fn approve_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    client: Option<Extension<DeviceAccessIdentity>>,
) -> Response {
    resolve_approval(
        state,
        id,
        ApprovalDecision::Approved,
        "approved",
        client.is_some(),
    )
}

/// POST /api/approvals/{id}/reject - Reject a pending request.
pub async fn reject_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    reject_with_scope(state, id, body, ApprovalDecision::Denied, "rejected")
}

/// POST /api/approvals/{id}/reject_session - Reject the exact action this session.
pub async fn reject_session_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    reject_with_scope(
        state,
        id,
        body,
        ApprovalDecision::DeniedSession,
        "rejected_session",
    )
}

/// POST /api/approvals/{id}/reject_always - Persist an exact-action deny rule.
pub async fn reject_always_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    reject_with_scope(
        state,
        id,
        body,
        ApprovalDecision::DeniedAlways,
        "rejected_always",
    )
}

/// POST /api/approvals/{id}/approve_session - Approve for the current daemon session.
pub async fn approve_session_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    client: Option<Extension<DeviceAccessIdentity>>,
) -> Response {
    resolve_approval(
        state,
        id,
        ApprovalDecision::ApprovedSession,
        "approved_session",
        client.is_some(),
    )
}

/// POST /api/approvals/{id}/approve_always - Persist an exact-action allow rule.
pub async fn approve_always_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    client: Option<Extension<DeviceAccessIdentity>>,
) -> Response {
    resolve_approval(
        state,
        id,
        ApprovalDecision::ApprovedAlways,
        "approved_always",
        client.is_some(),
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

/// DELETE /api/approvals/rules/{id} - Revoke a durable exact-action rule.
pub async fn revoke_approval_rule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let uuid = match parse_approval_id(&id) {
        Ok(uuid) => uuid,
        Err(response) => return response,
    };
    match state.kernel.approval_manager.revoke_rule(uuid, Some("api")) {
        Ok(Some(rule)) => json_response(
            StatusCode::OK,
            serde_json::json!({
                "id": rule.id,
                "status": "revoked",
                "tool_name": rule.tool_name,
                "effect": rule.effect
            }),
        ),
        Ok(None) => json_response(
            StatusCode::NOT_FOUND,
            serde_json::json!({"error": format!("No approval rule with id {uuid}")}),
        ),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({"error": error}),
        ),
    }
}

fn reject_with_scope(
    state: Arc<AppState>,
    id: String,
    body: Bytes,
    decision: ApprovalDecision,
    status: &'static str,
) -> Response {
    let request = match parse_reject_body(&body) {
        Ok(request) => request,
        Err(error) => {
            return json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": error}))
        }
    };
    if decision == ApprovalDecision::DeniedAlways
        && request
            .reason
            .as_deref()
            .is_none_or(|reason| reason.trim().is_empty())
    {
        return json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"error": "A durable deny rule requires a reason"}),
        );
    }
    resolve_approval_with_reason(
        state,
        id,
        decision,
        request.reason.as_deref(),
        status,
        false,
    )
}

fn parse_reject_body(body: &[u8]) -> Result<RejectApprovalRequest, String> {
    if body.is_empty() {
        return Ok(RejectApprovalRequest::default());
    }
    serde_json::from_slice(body).map_err(|error| format!("Invalid rejection body: {error}"))
}

fn resolve_approval(
    state: Arc<AppState>,
    id: String,
    decision: ApprovalDecision,
    status: &'static str,
    paired_client: bool,
) -> Response {
    resolve_approval_with_reason(state, id, decision, None, status, paired_client)
}

fn resolve_approval_with_reason(
    state: Arc<AppState>,
    id: String,
    decision: ApprovalDecision,
    reason: Option<&str>,
    status: &'static str,
    paired_client: bool,
) -> Response {
    let uuid = match parse_approval_id(&id) {
        Ok(uuid) => uuid,
        Err(response) => return response,
    };

    if paired_client && decision.is_approved() {
        let Some(request) = state
            .kernel
            .approval_manager
            .list_pending()
            .into_iter()
            .find(|request| request.id == uuid)
        else {
            return json_response(
                StatusCode::NOT_FOUND,
                serde_json::json!({"error": format!("No pending approval request with id {uuid}")}),
            );
        };
        if !captain_runtime::client_authority::paired_client_tool_name_is_allowed(
            &request.tool_name,
        ) {
            return json_response(
                StatusCode::FORBIDDEN,
                serde_json::json!({
                    "error": "Paired Client authority cannot approve this tool",
                    "code": "paired_client_authority_denied",
                    "tool_name": request.tool_name,
                }),
            );
        }
    }

    match state.kernel.approval_manager.resolve_with_reason(
        uuid,
        decision,
        reason,
        Some("api".to_string()),
    ) {
        Ok(response) => json_response(
            StatusCode::OK,
            serde_json::json!({
                "id": id,
                "status": status,
                "decision": response.decision,
                "decided_at": response.decided_at.to_rfc3339(),
                "reason": response.reason,
                "rule_id": response.rule_id
            }),
        ),
        Err(error) => json_response(
            approval_error_status(&error),
            serde_json::json!({"error": error}),
        ),
    }
}

fn approval_error_status(error: &str) -> StatusCode {
    if error.starts_with("No pending approval") || error.contains("already resolved") {
        StatusCode::NOT_FOUND
    } else if error.contains("expired") {
        StatusCode::GONE
    } else if error.contains("reason") || error.contains("decided_by") {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
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
    use axum::body::to_bytes;
    use captain_kernel::CaptainKernel;
    use captain_types::config::{DefaultModelConfig, KernelConfig};
    use std::time::Instant;

    fn test_state() -> (tempfile::TempDir, Arc<AppState>) {
        let tmp = tempfile::tempdir().unwrap();
        let config = KernelConfig {
            home_dir: tmp.path().join("home"),
            data_dir: tmp.path().join("data"),
            default_model: DefaultModelConfig {
                provider: "ollama".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OLLAMA_API_KEY".to_string(),
                base_url: None,
            },
            ..KernelConfig::default()
        };
        let kernel = Arc::new(CaptainKernel::boot_with_config(config).unwrap());
        kernel.set_self_handle();
        let state = Arc::new(AppState {
            kernel,
            started_at: Instant::now(),
            peer_registry: None,
            bridge_manager: tokio::sync::Mutex::new(None),
            channels_config: tokio::sync::RwLock::new(Default::default()),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            ask_user_channels: dashmap::DashMap::new(),
            provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
        });
        (tmp, state)
    }

    async fn queue_request(
        state: &Arc<AppState>,
        action: &str,
    ) -> (
        uuid::Uuid,
        tokio::task::JoinHandle<captain_types::approval::ApprovalOutcome>,
    ) {
        let request = ApprovalRequest {
            id: uuid::Uuid::new_v4(),
            agent_id: "captain".to_string(),
            tool_name: "shell_exec".to_string(),
            description: "approval route test".to_string(),
            action_summary: action.to_string(),
            action_digest: captain_types::approval::approval_action_digest(
                "shell_exec",
                action.as_bytes(),
            ),
            risk_level: RiskLevel::Critical,
            requested_at: chrono::Utc::now(),
            timeout_secs: 60,
        };
        let id = request.id;
        let kernel = Arc::clone(&state.kernel);
        let task =
            tokio::spawn(async move { kernel.approval_manager.request_approval(request).await });
        for _ in 0..50 {
            if state.kernel.approval_manager.pending_count() > 0 {
                return (id, task);
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        panic!("approval request was not queued");
    }

    async fn queue_request_for_tool(
        state: &Arc<AppState>,
        tool_name: &str,
        action: &str,
    ) -> (
        uuid::Uuid,
        tokio::task::JoinHandle<captain_types::approval::ApprovalOutcome>,
    ) {
        let request = ApprovalRequest {
            id: uuid::Uuid::new_v4(),
            agent_id: "captain".to_string(),
            tool_name: tool_name.to_string(),
            description: "paired Client approval test".to_string(),
            action_summary: action.to_string(),
            action_digest: captain_types::approval::approval_action_digest(
                tool_name,
                action.as_bytes(),
            ),
            risk_level: RiskLevel::High,
            requested_at: chrono::Utc::now(),
            timeout_secs: 60,
        };
        let id = request.id;
        let kernel = Arc::clone(&state.kernel);
        let task =
            tokio::spawn(async move { kernel.approval_manager.request_approval(request).await });
        for _ in 0..50 {
            if state
                .kernel
                .approval_manager
                .list_pending()
                .iter()
                .any(|request| request.id == id)
            {
                return (id, task);
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        panic!("approval request was not queued");
    }

    async fn json_body(response: Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

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

    #[test]
    fn reject_body_keeps_legacy_empty_body_and_rejects_unknown_fields() {
        assert!(parse_reject_body(b"").unwrap().reason.is_none());
        assert_eq!(
            parse_reject_body(br#"{"reason":"use staging"}"#)
                .unwrap()
                .reason
                .as_deref(),
            Some("use staging")
        );
        assert!(parse_reject_body(br#"{"scope":"global"}"#).is_err());
    }

    #[test]
    fn approval_errors_keep_client_and_server_failures_distinct() {
        assert_eq!(
            approval_error_status("No pending approval request with id x"),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            approval_error_status("approval reason too long"),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            approval_error_status("persist approval rules: disk full"),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn create_approval_rejects_unbounded_external_input_before_queueing() {
        let (_tmp, state) = test_state();
        let response = create_approval(
            State(Arc::clone(&state)),
            Json(CreateApprovalRequest {
                agent_id: "captain".to_string(),
                tool_name: "shell_exec".to_string(),
                description: String::new(),
                action_summary: "x".repeat(513),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(state.kernel.approval_manager.pending_count(), 0);
        state.kernel.shutdown();
    }

    #[tokio::test]
    async fn rejection_routes_preserve_scope_reason_and_pending_on_invalid_input() {
        let (_tmp, state) = test_state();
        let (id, task) = queue_request(&state, "deploy production").await;

        let invalid = reject_always_request(
            State(Arc::clone(&state)),
            Path(id.to_string()),
            Bytes::from_static(br#"{}"#),
        )
        .await;
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
        assert_eq!(state.kernel.approval_manager.pending_count(), 1);

        let response = reject_always_request(
            State(Arc::clone(&state)),
            Path(id.to_string()),
            Bytes::from_static(br#"{"reason":"use staging"}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert_eq!(body["decision"], "denied_always");
        assert_eq!(body["reason"], "use staging");
        assert!(body["rule_id"].is_string());
        let outcome = task.await.unwrap();
        assert_eq!(outcome.decision, ApprovalDecision::DeniedAlways);
        assert_eq!(outcome.reason.as_deref(), Some("use staging"));

        let (session_id, session_task) = queue_request(&state, "restart service").await;
        let response = reject_session_request(
            State(Arc::clone(&state)),
            Path(session_id.to_string()),
            Bytes::from_static(br#"{"reason":"wait for maintenance"}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            session_task.await.unwrap().decision,
            ApprovalDecision::DeniedSession
        );

        let (once_id, once_task) = queue_request(&state, "rotate logs").await;
        let response = reject_request(
            State(Arc::clone(&state)),
            Path(once_id.to_string()),
            Bytes::new(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(once_task.await.unwrap().decision, ApprovalDecision::Denied);
        state.kernel.shutdown();
    }

    #[tokio::test]
    async fn durable_allow_is_listed_then_revoked_by_exact_rule_id() {
        let (_tmp, state) = test_state();
        let (id, task) = queue_request(&state, "publish verified bundle").await;
        let response =
            approve_always_request(State(Arc::clone(&state)), Path(id.to_string()), None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let outcome = task.await.unwrap();
        assert_eq!(outcome.decision, ApprovalDecision::ApprovedAlways);
        let rule_id = outcome.rule_id.expect("durable rule id");

        let listed = list_approvals(State(Arc::clone(&state)), None)
            .await
            .into_response();
        let body = json_body(listed).await;
        assert_eq!(body["total"], 0);
        assert_eq!(body["rules_total"], 1);
        assert_eq!(body["rules"][0]["id"], rule_id.to_string());
        assert!(body["rules"][0].get("action_summary").is_none());

        let client_listed = list_approvals(
            State(Arc::clone(&state)),
            Some(Extension(DeviceAccessIdentity {
                device_id: "client-1".to_string(),
                role: captain_wire::DeviceRole::Client,
                grants_json: "{}".to_string(),
                protocol_version: captain_wire::HUB_NODE_PROTOCOL_VERSION,
            })),
        )
        .await
        .into_response();
        let client_body = json_body(client_listed).await;
        assert_eq!(client_body["rules_total"], 0);
        assert_eq!(client_body["rules"], serde_json::json!([]));

        let revoked =
            revoke_approval_rule(State(Arc::clone(&state)), Path(rule_id.to_string())).await;
        assert_eq!(revoked.status(), StatusCode::OK);
        assert!(state.kernel.approval_manager.list_rules().is_empty());
        state.kernel.shutdown();
    }

    #[tokio::test]
    async fn paired_client_cannot_approve_shell_but_can_approve_memory() {
        let (_tmp, state) = test_state();
        let identity = captain_kernel::hub_pairing_service::DeviceAccessIdentity {
            device_id: "client-1".to_string(),
            role: captain_wire::DeviceRole::Client,
            grants_json: "{}".to_string(),
            protocol_version: captain_wire::HUB_NODE_PROTOCOL_VERSION,
        };

        let (shell_id, shell_task) = queue_request_for_tool(&state, "shell_exec", "uname -a").await;
        let denied = approve_request(
            State(Arc::clone(&state)),
            Path(shell_id.to_string()),
            Some(Extension(identity.clone())),
        )
        .await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        assert!(state
            .kernel
            .approval_manager
            .list_pending()
            .iter()
            .any(|request| request.id == shell_id));

        state
            .kernel
            .approval_manager
            .resolve(
                shell_id,
                ApprovalDecision::Denied,
                Some("test cleanup".to_string()),
            )
            .unwrap();
        assert_eq!(shell_task.await.unwrap().decision, ApprovalDecision::Denied);

        let (memory_id, memory_task) =
            queue_request_for_tool(&state, "memory_save", "remember preference").await;
        let allowed = approve_request(
            State(Arc::clone(&state)),
            Path(memory_id.to_string()),
            Some(Extension(identity)),
        )
        .await;
        assert_eq!(allowed.status(), StatusCode::OK);
        assert_eq!(
            memory_task.await.unwrap().decision,
            ApprovalDecision::Approved
        );
        state.kernel.shutdown();
    }
}
