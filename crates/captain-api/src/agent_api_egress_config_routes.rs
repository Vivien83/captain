//! Per-agent API egress callback configuration.

use crate::{
    agent_api_config_status::agent_api_config_status,
    agent_api_egress::{
        agent_api_callback_secret_env, agent_api_callback_url_env, clip_for_callback,
        deliver_agent_api_callback, validate_agent_api_callback_url,
    },
    secret_env::write_secret_env,
    state::AppState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use captain_runtime::audit::AuditAction;
use captain_types::agent::AgentId;
use serde::{Deserialize, Serialize};
use std::{path::Path as FsPath, sync::Arc};

const MIN_CALLBACK_SECRET_LEN: usize = 16;

#[derive(Debug, Deserialize)]
pub struct AgentApiCallbackConfigRequest {
    pub callback_url: String,
    #[serde(default)]
    pub callback_secret: Option<String>,
    #[serde(default = "default_generate_secret")]
    pub generate_secret: bool,
}

#[derive(Debug, Serialize)]
pub struct AgentApiCallbackConfigResult {
    pub status: &'static str,
    pub agent_id: String,
    pub callback_url_env: String,
    pub callback_secret_env: String,
    pub generated_secret: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_secret: Option<String>,
    pub stored_in: &'static str,
    pub warning: &'static str,
}

#[derive(Debug, Default, Deserialize)]
pub struct AgentApiCallbackTestRequest {
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// POST /api/agents/:id/api/egress/configure - Configure outbound callbacks.
pub async fn configure_agent_api_egress(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AgentApiCallbackConfigRequest>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    if state.kernel.registry.get(agent_id).is_none() {
        return error(StatusCode::NOT_FOUND, "Agent not found");
    }

    let result = match configure_callback(&state.kernel.config.home_dir, &agent_id, req) {
        Ok(result) => result,
        Err((status, message)) => return error(status, &message),
    };
    state.kernel.audit_log.record(
        agent_id.to_string(),
        AuditAction::ConfigChange,
        "agent_api callback configured",
        format!(
            "url_env={} secret_env={} generated_secret={}",
            result.callback_url_env, result.callback_secret_env, result.generated_secret
        ),
    );

    let api = crate::agent_api_routes::agent_api_descriptor(&agent_id);
    let config_status =
        agent_api_config_status(&state.kernel.config.home_dir, &agent_id, &api).await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "callback": result,
            "api": api,
            "config_status": config_status,
        })),
    )
        .into_response()
}

/// POST /api/agents/:id/api/egress/test - Send a diagnostic callback without queueing.
pub async fn test_agent_api_egress(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AgentApiCallbackTestRequest>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    if state.kernel.registry.get(agent_id).is_none() {
        return error(StatusCode::NOT_FOUND, "Agent not found");
    }

    let payload = callback_test_payload(&agent_id, req);
    let delivery = deliver_agent_api_callback(&agent_id, &payload).await;
    let outcome = delivery.audit_outcome();
    let status = callback_test_status(delivery.delivered(), &outcome);
    state.kernel.audit_log.record(
        agent_id.to_string(),
        AuditAction::ConfigChange,
        "agent_api callback test",
        outcome.clone(),
    );

    (
        callback_test_http_status(status),
        Json(serde_json::json!({
            "status": status,
            "agent_id": agent_id.to_string(),
            "event": "agent_api.test",
            "queued": false,
            "outcome": outcome,
            "delivery": delivery,
        })),
    )
        .into_response()
}

pub(crate) fn configure_callback(
    home_dir: &FsPath,
    agent_id: &AgentId,
    req: AgentApiCallbackConfigRequest,
) -> Result<AgentApiCallbackConfigResult, (StatusCode, String)> {
    let callback_url = req.callback_url.trim().to_string();
    if callback_url.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "callback_url is required".to_string(),
        ));
    }
    if let Err(err) = validate_agent_api_callback_url(&callback_url) {
        return Err((StatusCode::BAD_REQUEST, format!("callback_url: {err}")));
    }

    let (secret, generated_secret) = match req.callback_secret {
        Some(secret) if !secret.trim().is_empty() => (secret.trim().to_string(), false),
        _ if req.generate_secret => (generate_callback_secret(), true),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "callback_secret is required when generate_secret is false".to_string(),
            ));
        }
    };
    if secret.len() < MIN_CALLBACK_SECRET_LEN {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("callback_secret must be at least {MIN_CALLBACK_SECRET_LEN} characters"),
        ));
    }

    let callback_url_env = agent_api_callback_url_env(agent_id);
    let callback_secret_env = agent_api_callback_secret_env(agent_id);
    let secrets_path = home_dir.join("secrets.env");
    write_secret_env(&secrets_path, &callback_url_env, &callback_url)
        .map_err(|err| server_err(format!("Failed to write callback URL: {err}")))?;
    write_secret_env(&secrets_path, &callback_secret_env, &secret)
        .map_err(|err| server_err(format!("Failed to write callback secret: {err}")))?;
    std::env::set_var(&callback_url_env, &callback_url);
    std::env::set_var(&callback_secret_env, &secret);

    Ok(AgentApiCallbackConfigResult {
        status: "configured",
        agent_id: agent_id.to_string(),
        callback_url_env,
        callback_secret_env,
        generated_secret,
        callback_secret: generated_secret.then_some(secret),
        stored_in: "secrets.env",
        warning: "Generated callback secrets are returned once. Normal status responses only expose env names and readiness.",
    })
}

fn callback_test_payload(
    agent_id: &AgentId,
    req: AgentApiCallbackTestRequest,
) -> serde_json::Value {
    serde_json::json!({
        "event": "agent_api.test",
        "status": "test",
        "agent_id": agent_id.to_string(),
        "request_id": req.request_id,
        "message": clip_for_callback(req.message.as_deref().unwrap_or("Captain agent API callback test")),
        "metadata": req.metadata,
    })
}

fn callback_test_status(delivered: bool, outcome: &str) -> &'static str {
    if delivered {
        "delivered"
    } else if outcome.starts_with("skipped:not_configured") {
        "not_configured"
    } else {
        "failed"
    }
}

fn callback_test_http_status(status: &str) -> StatusCode {
    match status {
        "delivered" => StatusCode::OK,
        "not_configured" => StatusCode::FAILED_DEPENDENCY,
        _ => StatusCode::BAD_GATEWAY,
    }
}

fn generate_callback_secret() -> String {
    captain_types::agent_api::generate_agent_api_callback_secret()
}

fn default_generate_secret() -> bool {
    true
}

fn server_err(message: String) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, message)
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
        "66666666-6666-6666-6666-666666666666".parse().unwrap()
    }

    fn agent_id(value: &str) -> AgentId {
        value.parse().unwrap()
    }

    #[test]
    fn configure_callback_generates_secret_once() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_id = agent_id("66666666-6666-6666-6666-666666666667");
        let req = AgentApiCallbackConfigRequest {
            callback_url: "https://example.com/callback".to_string(),
            callback_secret: None,
            generate_secret: true,
        };

        let result = configure_callback(tmp.path(), &agent_id, req).unwrap();
        let secret = result.callback_secret.clone().unwrap();
        let secrets = std::fs::read_to_string(tmp.path().join("secrets.env")).unwrap();

        assert_eq!(result.status, "configured");
        assert!(result.generated_secret);
        assert!(secret.len() >= 64);
        assert!(secrets.contains(&format!(
            "{}=https://example.com/callback",
            result.callback_url_env
        )));
        assert!(secrets.contains(&format!("{}={secret}", result.callback_secret_env)));
        assert_eq!(
            std::env::var(&result.callback_url_env).unwrap(),
            "https://example.com/callback"
        );
        assert_eq!(std::env::var(&result.callback_secret_env).unwrap(), secret);

        std::env::remove_var(result.callback_url_env);
        std::env::remove_var(result.callback_secret_env);
    }

    #[test]
    fn configure_callback_does_not_echo_provided_secret() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_id = sample_agent_id();
        let req = AgentApiCallbackConfigRequest {
            callback_url: "https://example.com/callback".to_string(),
            callback_secret: Some("provided-secret-value".to_string()),
            generate_secret: true,
        };

        let result = configure_callback(tmp.path(), &agent_id, req).unwrap();
        let encoded = serde_json::to_string(&result).unwrap();

        assert!(!result.generated_secret);
        assert!(result.callback_secret.is_none());
        assert!(!encoded.contains("provided-secret-value"));

        std::env::remove_var(result.callback_url_env);
        std::env::remove_var(result.callback_secret_env);
    }

    #[test]
    fn configure_callback_rejects_invalid_url() {
        let tmp = tempfile::tempdir().unwrap();
        let err = configure_callback(
            tmp.path(),
            &sample_agent_id(),
            AgentApiCallbackConfigRequest {
                callback_url: "http://localhost:9999/hook".to_string(),
                callback_secret: None,
                generate_secret: true,
            },
        )
        .unwrap_err();

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn callback_test_payload_is_bounded_and_identifiable() {
        let agent_id = sample_agent_id();
        let payload = callback_test_payload(
            &agent_id,
            AgentApiCallbackTestRequest {
                request_id: Some("test-1".to_string()),
                message: Some("x".repeat(70 * 1024)),
                metadata: Some(serde_json::json!({"source": "unit"})),
            },
        );

        assert_eq!(payload["event"], "agent_api.test");
        assert_eq!(payload["status"], "test");
        assert_eq!(payload["agent_id"], agent_id.to_string());
        let message = payload["message"].as_str().unwrap();
        assert_eq!(message.len(), 64 * 1024 + 3);
        assert!(message.ends_with("..."));
    }

    #[test]
    fn callback_test_status_maps_outcomes() {
        assert_eq!(
            callback_test_status(true, "delivered attempts=1"),
            "delivered"
        );
        assert_eq!(
            callback_test_status(false, "skipped:not_configured"),
            "not_configured"
        );
        assert_eq!(
            callback_test_status(false, "failed attempts=2 retryable=true"),
            "failed"
        );
        assert_eq!(
            callback_test_http_status("not_configured"),
            StatusCode::FAILED_DEPENDENCY
        );
        assert_eq!(callback_test_http_status("failed"), StatusCode::BAD_GATEWAY);
    }
}
