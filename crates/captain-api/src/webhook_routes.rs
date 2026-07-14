use crate::state::AppState;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use captain_runtime::kernel_handle::KernelHandle;
use captain_types::agent::AgentId;
use std::sync::Arc;

/// POST /hooks/wake - Inject a system event via webhook trigger.
pub async fn webhook_wake(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(body): Json<captain_types::webhook::WakePayload>,
) -> impl IntoResponse {
    let webhook_config = match &state.kernel.config.webhook_triggers {
        Some(config) if config.enabled => config,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Webhook triggers not enabled"})),
            );
        }
    };

    if !validate_webhook_token(&headers, &webhook_config.token_env) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Invalid or missing token"})),
        );
    }

    if let Err(e) = body.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        );
    }

    let event_payload = serde_json::json!({
        "source": "webhook",
        "mode": body.mode,
        "text": body.text,
    });
    if let Err(e) =
        KernelHandle::publish_event(state.kernel.as_ref(), "webhook.wake", event_payload).await
    {
        tracing::warn!("Webhook wake event publish failed: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Event publish failed: {e}")})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "accepted", "mode": body.mode})),
    )
}

/// POST /hooks/agent - Run an isolated agent turn via webhook.
pub async fn webhook_agent(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(body): Json<captain_types::webhook::AgentHookPayload>,
) -> impl IntoResponse {
    let webhook_config = match &state.kernel.config.webhook_triggers {
        Some(config) if config.enabled => config,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Webhook triggers not enabled"})),
            );
        }
    };

    if !validate_webhook_token(&headers, &webhook_config.token_env) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Invalid or missing token"})),
        );
    }

    if let Err(e) = body.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        );
    }

    let agent_id = match resolve_webhook_agent(&state, body.agent.as_deref()) {
        Some(agent_id) => agent_id,
        None => {
            let error = match &body.agent {
                Some(agent_ref) => format!("Agent not found: {agent_ref}"),
                None => "No agents available".to_string(),
            };
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": error})),
            );
        }
    };

    match state.kernel.send_message(agent_id, &body.message).await {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "completed",
                "agent_id": agent_id.to_string(),
                "response": result.response,
                "usage": {
                    "input_tokens": result.total_usage.input_tokens,
                    "output_tokens": result.total_usage.output_tokens,
                },
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Agent execution failed: {e}")})),
        ),
    }
}

fn resolve_webhook_agent(state: &AppState, agent_ref: Option<&str>) -> Option<AgentId> {
    match agent_ref {
        Some(agent_ref) => match agent_ref.parse() {
            Ok(agent_id) => Some(agent_id),
            Err(_) => state
                .kernel
                .registry
                .find_by_name(agent_ref)
                .map(|entry| entry.id),
        },
        None => {
            let agents = state.kernel.registry.list();
            agents
                .iter()
                .find(|entry| entry.name.eq_ignore_ascii_case("captain"))
                .or_else(|| agents.first())
                .map(|entry| entry.id)
        }
    }
}

fn validate_webhook_token(headers: &axum::http::HeaderMap, token_env: &str) -> bool {
    let expected = match std::env::var(token_env) {
        Ok(token) if token.len() >= 32 => token,
        _ => return false,
    };

    let provided = match headers.get("authorization") {
        Some(value) => match value.to_str() {
            Ok(header) => match header.strip_prefix("Bearer ") {
                Some(token) => token,
                None => return false,
            },
            Err(_) => return false,
        },
        None => return false,
    };

    use subtle::ConstantTimeEq;
    if provided.len() != expected.len() {
        return false;
    }
    provided.as_bytes().ct_eq(expected.as_bytes()).into()
}
