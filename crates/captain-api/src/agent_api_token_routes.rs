//! Per-agent API token operations.

use crate::{agent_api_config_status::agent_api_config_status, secret_env::write_secret_env};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use captain_runtime::audit::AuditAction;
use captain_types::agent::AgentId;
use serde::Serialize;
use std::{path::Path as FsPath, sync::Arc};

use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct AgentApiTokenRotation {
    pub status: &'static str,
    pub agent_id: String,
    pub token_env: String,
    pub token: String,
    pub stored_in: &'static str,
    pub warning: &'static str,
}

/// POST /api/agents/:id/api/token/rotate - Generate and store a new ingress bearer token.
pub async fn rotate_agent_api_token(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    if state.kernel.registry.get(agent_id).is_none() {
        return error(StatusCode::NOT_FOUND, "Agent not found");
    }

    let rotation = match rotate_token(&state.kernel.config.home_dir, &agent_id) {
        Ok(rotation) => rotation,
        Err(err) => return error(StatusCode::INTERNAL_SERVER_ERROR, &err),
    };
    state.kernel.audit_log.record(
        agent_id.to_string(),
        AuditAction::ConfigChange,
        "agent_api token rotated",
        format!("token_env={} stored_in=secrets.env", rotation.token_env),
    );

    let api = crate::agent_api_routes::agent_api_descriptor(&agent_id);
    let config_status =
        agent_api_config_status(&state.kernel.config.home_dir, &agent_id, &api).await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "rotation": rotation,
            "api": api,
            "config_status": config_status,
        })),
    )
        .into_response()
}

pub(crate) fn rotate_token(
    home_dir: &FsPath,
    agent_id: &AgentId,
) -> Result<AgentApiTokenRotation, String> {
    let token_env = crate::agent_api_routes::agent_api_token_env(agent_id);
    let token = generate_agent_api_token();
    write_secret_env(&home_dir.join("secrets.env"), &token_env, &token)
        .map_err(|err| format!("Failed to write secrets.env: {err}"))?;
    std::env::set_var(&token_env, &token);
    Ok(AgentApiTokenRotation {
        status: "rotated",
        agent_id: agent_id.to_string(),
        token_env,
        token,
        stored_in: "secrets.env",
        warning: "Token is returned once. Store it in the external service and use Authorization: Bearer <token>.",
    })
}

fn generate_agent_api_token() -> String {
    captain_types::agent_api::generate_agent_api_token()
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
        "55555555-5555-5555-5555-555555555555".parse().unwrap()
    }

    #[test]
    fn generated_token_is_long_and_non_deterministic() {
        let first = generate_agent_api_token();
        let second = generate_agent_api_token();

        assert!(first.len() >= 64);
        assert!(first.starts_with("cap_at_"));
        assert_ne!(first, second);
    }

    #[test]
    fn rotate_token_writes_secret_env_and_process_env() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_id = sample_agent_id();
        let token_env = crate::agent_api_routes::agent_api_token_env(&agent_id);
        std::env::remove_var(&token_env);

        let rotation = rotate_token(tmp.path(), &agent_id).unwrap();
        let secrets = std::fs::read_to_string(tmp.path().join("secrets.env")).unwrap();

        assert_eq!(rotation.status, "rotated");
        assert_eq!(rotation.token_env, token_env);
        assert!(rotation.token.len() >= 64);
        assert!(secrets.contains(&format!("{token_env}={}", rotation.token)));
        assert_eq!(std::env::var(&token_env).unwrap(), rotation.token);

        std::env::remove_var(&token_env);
    }
}
