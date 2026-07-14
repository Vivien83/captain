//! Agent creation route handlers.

use crate::state::AppState;
use crate::types::SpawnRequest;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use captain_types::{
    agent::AgentId,
    agent::AgentManifest,
    agent_api::{
        failed_egress_report, pending_egress_operator_action, pending_egress_report,
        ready_egress_report, ready_ingress_report, skipped_ingress_report,
        AgentApiEgressProvisionReport, AgentApiSpawnProvisionReport, AgentApiSpawnProvisionRequest,
    },
};
use std::path::Path;
use std::sync::Arc;

/// POST /api/agents - Spawn a new agent.
pub async fn spawn_agent(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SpawnRequest>,
) -> impl IntoResponse {
    let manifest_toml = match resolve_manifest_toml(&state, &req) {
        Ok(manifest_toml) => manifest_toml,
        Err(response) => return response,
    };

    const MAX_MANIFEST_SIZE: usize = 1024 * 1024;
    if manifest_toml.len() > MAX_MANIFEST_SIZE {
        return error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Manifest too large (max 1MB)",
        );
    }

    if let Some(ref signed_json) = req.signed_manifest {
        if let Err(response) = verify_manifest_signature(&state, signed_json, &manifest_toml) {
            return response;
        }
    }

    let manifest: AgentManifest = match toml::from_str(&manifest_toml) {
        Ok(manifest) => manifest,
        Err(e) => {
            tracing::warn!("Invalid manifest TOML: {e}");
            return error(
                StatusCode::BAD_REQUEST,
                &captain_types::agent::format_agent_manifest_parse_error(&e, &manifest_toml),
            );
        }
    };

    let name = manifest.name.clone();
    match state.kernel.spawn_agent(manifest) {
        Ok(id) => {
            if let Some(ref manager) = *state.bridge_manager.lock().await {
                manager.router().register_agent(name.clone(), id);
            }
            let agent_api_provisioning =
                provision_spawn_agent_api(&state.kernel.config.home_dir, &id, &req.agent_api);
            let (agent_api, agent_api_config_status) =
                agent_api_spawn_fields(&state.kernel.config.home_dir, &id).await;
            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "agent_id": id.to_string(),
                    "name": name,
                    "agent_api_provisioning": agent_api_provisioning,
                    "agent_api": agent_api,
                    "agent_api_config_status": agent_api_config_status,
                })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::warn!("Spawn failed: {e}");
            error(StatusCode::INTERNAL_SERVER_ERROR, "Agent spawn failed")
        }
    }
}

fn provision_spawn_agent_api(
    home_dir: &Path,
    agent_id: &AgentId,
    req: &AgentApiSpawnProvisionRequest,
) -> AgentApiSpawnProvisionReport {
    let mut actions = Vec::new();
    let ingress = if req.provision_ingress_token {
        match crate::agent_api_token_routes::rotate_token(home_dir, agent_id) {
            Ok(rotation) => ready_ingress_report(agent_id, rotation.token),
            Err(err) => {
                actions.push(format!(
                    "Rotate ingress token manually with {} after fixing: {err}",
                    captain_types::agent_api::agent_api_token_rotate_url(agent_id)
                ));
                skipped_ingress_report(agent_id)
            }
        }
    } else {
        actions.push(format!(
            "Rotate ingress token with {} before external callers use the agent.",
            captain_types::agent_api::agent_api_token_rotate_url(agent_id)
        ));
        skipped_ingress_report(agent_id)
    };

    let egress = provision_spawn_egress(home_dir, agent_id, req, &mut actions);
    AgentApiSpawnProvisionReport::new(agent_id, ingress, egress, actions)
}

fn provision_spawn_egress(
    home_dir: &Path,
    agent_id: &AgentId,
    req: &AgentApiSpawnProvisionRequest,
    actions: &mut Vec<String>,
) -> AgentApiEgressProvisionReport {
    let Some(callback_url) = req
        .egress_callback_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    else {
        actions.push(pending_egress_operator_action(agent_id));
        return pending_egress_report(agent_id);
    };

    let config_req = crate::agent_api_egress_config_routes::AgentApiCallbackConfigRequest {
        callback_url: callback_url.to_string(),
        callback_secret: req.egress_callback_secret.clone(),
        generate_secret: req.generate_callback_secret,
    };
    match crate::agent_api_egress_config_routes::configure_callback(home_dir, agent_id, config_req)
    {
        Ok(result) => ready_egress_report(agent_id, result.callback_secret),
        Err((_status, issue)) => {
            actions.push(format!("Fix egress callback configuration: {issue}"));
            failed_egress_report(agent_id, issue)
        }
    }
}

async fn agent_api_spawn_fields(
    home_dir: &Path,
    agent_id: &AgentId,
) -> (
    crate::agent_api_routes::AgentApiDescriptor,
    crate::agent_api_config_status::AgentApiConfigStatus,
) {
    let agent_api = crate::agent_api_routes::agent_api_descriptor(agent_id);
    let config_status =
        crate::agent_api_config_status::agent_api_config_status(home_dir, agent_id, &agent_api)
            .await;
    (agent_api, config_status)
}

#[allow(clippy::result_large_err)]
fn resolve_manifest_toml(
    state: &AppState,
    req: &SpawnRequest,
) -> Result<String, axum::response::Response> {
    if !req.manifest_toml.trim().is_empty() {
        return Ok(req.manifest_toml.clone());
    }

    let Some(template) = req.template.as_deref() else {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Either 'manifest_toml' or 'template' is required",
        ));
    };

    let safe_name = sanitize_template_name(template)?;
    let template_path = state
        .kernel
        .config
        .home_dir
        .join("agents")
        .join(&safe_name)
        .join("agent.toml");

    std::fs::read_to_string(&template_path).map_err(|_| {
        error(
            StatusCode::NOT_FOUND,
            &format!("Template '{safe_name}' not found"),
        )
    })
}

#[allow(clippy::result_large_err)]
fn sanitize_template_name(template: &str) -> Result<String, axum::response::Response> {
    let safe_name = template
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect::<String>();

    if safe_name.is_empty() || safe_name != template {
        return Err(error(StatusCode::BAD_REQUEST, "Invalid template name"));
    }
    Ok(safe_name)
}

#[allow(clippy::result_large_err)]
fn verify_manifest_signature(
    state: &AppState,
    signed_json: &str,
    manifest_toml: &str,
) -> Result<(), axum::response::Response> {
    match state.kernel.verify_signed_manifest(signed_json) {
        Ok(verified_toml) => {
            if verified_toml.trim() != manifest_toml.trim() {
                tracing::warn!("Signed manifest content does not match manifest_toml");
                return Err(error(
                    StatusCode::BAD_REQUEST,
                    "Signed manifest content does not match manifest_toml",
                ));
            }
            Ok(())
        }
        Err(e) => {
            tracing::warn!("Manifest signature verification failed: {e}");
            state.kernel.audit_log.record(
                "system",
                captain_runtime::audit::AuditAction::AuthAttempt,
                "manifest signature verification failed",
                format!("error: {e}"),
            );
            Err(error(
                StatusCode::FORBIDDEN,
                "Manifest signature verification failed",
            ))
        }
    }
}

fn error(status: StatusCode, message: &str) -> axum::response::Response {
    (status, Json(serde_json::json!({"error": message}))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_agent_id() -> AgentId {
        "44444444-4444-4444-4444-444444444444".parse().unwrap()
    }

    #[tokio::test]
    async fn spawn_agent_api_status_is_operator_safe() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_id: AgentId = "88888888-8888-8888-8888-888888888888".parse().unwrap();
        let token = "token-value-token-value-token-value-44";
        let secret = "secret-value-secret-value-44";
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

        let (agent_api, config_status) = agent_api_spawn_fields(tmp.path(), &agent_id).await;
        let payload = serde_json::json!({
            "agent_api": agent_api,
            "agent_api_config_status": config_status,
        });
        let encoded = serde_json::to_string(&payload).unwrap();

        assert_eq!(payload["agent_api_config_status"]["state"], "ready");
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

    #[tokio::test]
    async fn spawn_provisioning_rotates_ingress_and_marks_egress_pending() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_id = sample_agent_id();
        std::env::remove_var(crate::agent_api_routes::agent_api_token_env(&agent_id));
        std::env::remove_var(crate::agent_api_egress::agent_api_callback_url_env(
            &agent_id,
        ));
        std::env::remove_var(crate::agent_api_egress::agent_api_callback_secret_env(
            &agent_id,
        ));

        let report = provision_spawn_agent_api(
            tmp.path(),
            &agent_id,
            &AgentApiSpawnProvisionRequest::default(),
        );
        let encoded = serde_json::to_string(&report).unwrap();

        assert_eq!(report.protocol, "agent-as-service.v1");
        assert_eq!(report.status, "ingress_ready");
        assert_eq!(report.ingress.status, "ready");
        assert_eq!(report.egress.status, "pending_callback_url");
        assert!(report
            .ingress
            .token
            .as_deref()
            .unwrap_or("")
            .starts_with("cap_at_"));
        assert!(encoded.contains("/hooks/agents/"));
        assert!(report
            .operator_actions
            .iter()
            .any(|action| action.contains("cannot infer the external callback URL")));
        assert!(report
            .operator_actions
            .iter()
            .any(|action| action.contains("/api/egress/configure")));
        assert_eq!(
            std::env::var(crate::agent_api_routes::agent_api_token_env(&agent_id)).unwrap(),
            report.ingress.token.clone().unwrap()
        );

        std::env::remove_var(crate::agent_api_routes::agent_api_token_env(&agent_id));
    }

    #[tokio::test]
    async fn spawn_provisioning_can_configure_egress_callback() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_id: AgentId = "77777777-7777-7777-7777-777777777777".parse().unwrap();
        std::env::remove_var(crate::agent_api_routes::agent_api_token_env(&agent_id));
        std::env::remove_var(crate::agent_api_egress::agent_api_callback_url_env(
            &agent_id,
        ));
        std::env::remove_var(crate::agent_api_egress::agent_api_callback_secret_env(
            &agent_id,
        ));
        let req = AgentApiSpawnProvisionRequest {
            egress_callback_url: Some("https://example.com/captain-agent".to_string()),
            ..AgentApiSpawnProvisionRequest::default()
        };

        let report = provision_spawn_agent_api(tmp.path(), &agent_id, &req);

        assert_eq!(report.status, "ready");
        assert_eq!(report.egress.status, "ready");
        assert!(report
            .egress
            .callback_secret
            .as_deref()
            .unwrap_or("")
            .starts_with("cap_cb_"));
        assert_eq!(
            std::env::var(crate::agent_api_egress::agent_api_callback_url_env(
                &agent_id
            ))
            .unwrap(),
            "https://example.com/captain-agent"
        );

        std::env::remove_var(crate::agent_api_routes::agent_api_token_env(&agent_id));
        std::env::remove_var(crate::agent_api_egress::agent_api_callback_url_env(
            &agent_id,
        ));
        std::env::remove_var(crate::agent_api_egress::agent_api_callback_secret_env(
            &agent_id,
        ));
    }
}
