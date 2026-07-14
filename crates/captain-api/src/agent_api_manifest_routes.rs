//! Operator manifest for a per-agent API surface.

use crate::{
    agent_api_config_status::{agent_api_config_status, AgentApiConfigStatus},
    agent_api_routes::{agent_api_descriptor, AgentApiDescriptor},
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

/// GET /api/agents/:id/api/manifest - Return the external integration contract.
pub async fn agent_api_manifest(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    let entry = match state.kernel.registry.get(agent_id) {
        Some(entry) => entry,
        None => return error(StatusCode::NOT_FOUND, "Agent not found"),
    };

    let api = agent_api_descriptor(&agent_id);
    let config_status =
        agent_api_config_status(&state.kernel.config.home_dir, &agent_id, &api).await;
    (
        StatusCode::OK,
        Json(build_agent_api_manifest(
            &agent_id,
            &entry.name,
            &api,
            &config_status,
        )),
    )
        .into_response()
}

pub(crate) fn build_agent_api_manifest(
    agent_id: &AgentId,
    agent_name: &str,
    api: &AgentApiDescriptor,
    config_status: &AgentApiConfigStatus,
) -> serde_json::Value {
    serde_json::json!({
        "kind": "captain.agent_api.manifest",
        "version": 1,
        "agent": {
            "id": agent_id.to_string(),
            "name": agent_name,
            "channel_type": api.channel_type,
        },
        "readiness": config_status,
        "ingress": {
            "method": "POST",
            "url": api.ingress_url,
            "content_type": "application/json",
            "auth": {
                "scheme": "bearer",
                "header": "Authorization",
                "value_template": "Bearer ${TOKEN}",
                "token_env": api.token_env,
                "rotate_url": api.token_rotate_url,
            },
            "idempotency_key": api.idempotency_key,
            "idempotency_ttl_secs": api.idempotency_ttl_secs,
            "body_schema": ingress_body_schema(),
            "example_payload": {
                "request_id": "external-unique-id-123",
                "message": "Ask this agent to do one concrete task.",
                "sender_id": "external-service:user-or-job-id",
                "sender_name": "External Service",
                "metadata": {
                    "source": "external-service",
                },
            },
        },
        "egress": {
            "configure_url": api.egress.configure_url,
            "test_url": api.egress.test_url,
            "callback_url_env": api.egress.callback_url_env,
            "callback_secret_env": api.egress.callback_secret_env,
            "events": ["agent_api.completed", "agent_api.failed", "agent_api.test"],
            "headers": {
                "content-type": "application/json",
                "x-captain-agent-id": agent_id.to_string(),
                "x-captain-event": "${event}",
                "x-captain-signature": "sha256=${hex_hmac_sha256}",
            },
            "event_header": api.egress.event_header,
            "signature_header": api.egress.signature_header,
            "signature": {
                "algorithm": "hmac-sha256",
                "input": "raw JSON request body bytes",
                "format": "sha256=<hex digest>",
                "secret_env": api.egress.callback_secret_env,
            },
            "example_completed_payload": {
                "event": "agent_api.completed",
                "status": "completed",
                "agent_id": agent_id.to_string(),
                "request_id": "external-unique-id-123",
                "response": "Agent response text",
                "usage": {
                    "input_tokens": 0,
                    "output_tokens": 0,
                },
                "iterations": 0,
                "cost_usd": 0.0,
                "metadata": {
                    "source": "external-service",
                },
            },
        },
        "limits": {
            "max_body_bytes": api.max_body_bytes,
            "max_message_bytes": api.max_message_bytes,
            "rate_limit_per_minute": api.rate_limit_per_minute,
            "callback_max_payload_bytes": api.egress.max_payload_bytes,
            "callback_timeout_secs": api.egress.timeout_secs,
            "callback_max_attempts": api.egress.max_attempts,
        },
        "operations": {
            "status_url": api.manifest_url.trim_end_matches("/manifest"),
            "manifest_url": api.manifest_url,
            "events_url": api.audit_events_url,
            "queue_status_url": api.egress.queue_status_url,
            "retry_url_template": api.egress.retry_url_template,
        },
        "security": {
            "secrets_are_returned": "only on rotate/configure responses",
            "normal_status_leaks_secrets": false,
            "recommended_flow": [
                "rotate ingress token",
                "configure egress callback",
                "send egress test",
                "send ingress request with stable request_id",
            ],
        },
    })
}

fn ingress_body_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["message"],
        "properties": {
            "request_id": {
                "type": "string",
                "description": "Stable idempotency key for external retries.",
            },
            "message": {
                "type": "string",
                "description": "Task or message for the agent.",
            },
            "sender_id": {
                "type": "string",
                "description": "External service user/job id.",
            },
            "sender_name": {
                "type": "string",
            },
            "metadata": {
                "type": "object",
                "description": "Small correlation payload echoed to callbacks.",
            },
        },
    })
}

#[allow(clippy::result_large_err)]
fn parse_agent_id(id: &str) -> Result<AgentId, axum::response::Response> {
    id.parse()
        .map_err(|_| error(StatusCode::BAD_REQUEST, "Invalid agent ID"))
}

fn error(status: StatusCode, message: &str) -> axum::response::Response {
    (status, Json(serde_json::json!({"error": message}))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_agent_id() -> AgentId {
        "77777777-7777-7777-7777-777777777777".parse().unwrap()
    }

    #[tokio::test]
    async fn manifest_is_operator_safe_and_complete() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_id = sample_agent_id();
        let token = "token-value-token-value-token-value-77";
        let secret = "secret-value-secret-value-77";
        let callback_url = "https://example.com/hook?secret=value";
        std::env::set_var(
            crate::agent_api_routes::agent_api_token_env(&agent_id),
            token,
        );
        std::env::set_var(
            crate::agent_api_egress::agent_api_callback_url_env(&agent_id),
            callback_url,
        );
        std::env::set_var(
            crate::agent_api_egress::agent_api_callback_secret_env(&agent_id),
            secret,
        );

        let api = crate::agent_api_routes::agent_api_descriptor(&agent_id);
        let status = agent_api_config_status(tmp.path(), &agent_id, &api).await;
        let manifest = build_agent_api_manifest(&agent_id, "api-agent", &api, &status);
        let encoded = serde_json::to_string(&manifest).unwrap();

        assert_eq!(manifest["kind"], "captain.agent_api.manifest");
        assert_eq!(manifest["ingress"]["method"], "POST");
        assert_eq!(
            manifest["egress"]["events"],
            serde_json::json!(["agent_api.completed", "agent_api.failed", "agent_api.test"])
        );
        assert_eq!(
            manifest["operations"]["manifest_url"],
            format!("/api/agents/{agent_id}/api/manifest")
        );
        assert!(!encoded.contains(token));
        assert!(!encoded.contains(secret));
        assert!(!encoded.contains(callback_url));

        std::env::remove_var(crate::agent_api_routes::agent_api_token_env(&agent_id));
        std::env::remove_var(crate::agent_api_egress::agent_api_callback_url_env(
            &agent_id,
        ));
        std::env::remove_var(crate::agent_api_egress::agent_api_callback_secret_env(
            &agent_id,
        ));
    }
}
