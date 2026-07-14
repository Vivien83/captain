//! Agent and fleet lifecycle route handlers.

use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use captain_runtime::kernel_handle::KernelHandle;
use captain_types::agent::{
    effective_manifest_capabilities, AgentId, AgentState, OrchestrationMode,
};
use std::sync::Arc;

/// GET /api/fleets - List all fleet managers with children and autoscale config.
pub async fn list_fleets(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let fleets: Vec<serde_json::Value> = state
        .kernel
        .registry
        .list()
        .into_iter()
        .filter(|entry| entry.tags.iter().any(|tag| tag == "manager"))
        .map(|manager| {
            let workers: Vec<serde_json::Value> = manager
                .children
                .iter()
                .filter_map(|child_id| state.kernel.registry.get(*child_id))
                .map(|child| {
                    serde_json::json!({
                        "id": child.id.to_string(),
                        "name": child.name,
                        "state": format!("{:?}", child.state),
                        "last_active": child.last_active.to_rfc3339(),
                        "model": format!("{}:{}", child.manifest.model.provider, child.manifest.model.model),
                    })
                })
                .collect();

            let usage = state.kernel.scheduler.get_agent_usage(manager.id);
            let tokens = usage.map(|usage| usage.input_tokens + usage.output_tokens).unwrap_or(0);
            serde_json::json!({
                "id": manager.id.to_string(),
                "name": manager.name,
                "domain": manager.manifest.description,
                "state": format!("{:?}", manager.state),
                "model": format!("{}:{}", manager.manifest.model.provider, manager.manifest.model.model),
                "mission": manager.mission,
                "mission_set_at": manager.mission_set_at.map(|time| time.to_rfc3339()),
                "autoscale": manager.autoscale,
                "last_scale_event": manager.last_scale_event.map(|time| time.to_rfc3339()),
                "tokens_used_last_window": tokens,
                "last_active": manager.last_active.to_rfc3339(),
                "workers": workers,
            })
        })
        .collect();
    Json(fleets)
}

/// GET /api/fleets/{id}/metrics - Detailed metrics for a single fleet.
pub async fn fleet_metrics(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.fleet_metrics(&id) {
        Ok(metrics) => (StatusCode::OK, Json(metrics)).into_response(),
        Err(e) => error(StatusCode::NOT_FOUND, &e),
    }
}

/// GET /api/agents - List all agents.
pub async fn list_agents(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let catalog = state.kernel.model_catalog.read().ok();
    let default_model = state.kernel.effective_default_model();

    let mut agents: Vec<serde_json::Value> = state
        .kernel
        .registry
        .list()
        .into_iter()
        .map(|entry| {
            let provider = resolved_provider(&entry, default_model.provider.as_str());
            let model = resolved_model(&entry, default_model.model.as_str());
            let (tier, auth_status) = catalog
                .as_ref()
                .map(|catalog| {
                    let tier = catalog
                        .find_model(model)
                        .map(|model| format!("{:?}", model.tier).to_lowercase())
                        .unwrap_or_else(|| "unknown".to_string());
                    let auth = catalog
                        .get_provider(provider)
                        .map(|provider| format!("{:?}", provider.auth_status).to_lowercase())
                        .unwrap_or_else(|| "unknown".to_string());
                    (tier, auth)
                })
                .unwrap_or_else(|| ("unknown".to_string(), "unknown".to_string()));

            let ready = matches!(entry.state, AgentState::Running) && auth_status != "missing";
            serde_json::json!({
                "id": entry.id.to_string(),
                "name": entry.name,
                "state": format!("{:?}", entry.state),
                "mode": entry.mode,
                "created_at": entry.created_at.to_rfc3339(),
                "last_active": entry.last_active.to_rfc3339(),
                "model_provider": provider,
                "model_name": model,
                "model_tier": tier,
                "auth_status": auth_status,
                "ready": ready,
                "profile": entry.manifest.profile,
                "identity": {
                    "emoji": entry.identity.emoji,
                    "avatar_url": entry.identity.avatar_url,
                    "color": entry.identity.color,
                },
            })
        })
        .collect();

    agents.sort_by(|a, b| {
        let a_name = a["name"].as_str().unwrap_or("").to_ascii_lowercase();
        let b_name = b["name"].as_str().unwrap_or("").to_ascii_lowercase();
        let a_rank = if a_name == "captain" { 0 } else { 1 };
        let b_rank = if b_name == "captain" { 0 } else { 1 };
        a_rank.cmp(&b_rank).then_with(|| a_name.cmp(&b_name))
    });

    Json(agents)
}

/// GET /api/agents/:id - Get a single agent's detailed info.
pub async fn get_agent(
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
    let declared_capabilities = entry.manifest.capabilities.clone();
    let effective_capabilities = effective_manifest_capabilities(&entry.manifest);
    let resources = entry.manifest.resources.clone();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "id": entry.id.to_string(),
            "name": entry.name,
            "state": format!("{:?}", entry.state),
            "mode": entry.mode,
            "profile": entry.manifest.profile,
            "created_at": entry.created_at.to_rfc3339(),
            "session_id": entry.session_id.0.to_string(),
            "model": {
                "provider": entry.manifest.model.provider,
                "model": entry.manifest.model.model,
            },
            "capabilities": declared_capabilities,
            "capabilities_effective": effective_capabilities,
            "resources": resources,
            "description": entry.manifest.description,
            "tags": entry.manifest.tags,
            "identity": {
                "emoji": entry.identity.emoji,
                "avatar_url": entry.identity.avatar_url,
                "color": entry.identity.color,
            },
            "skills": entry.manifest.skills,
            "skills_mode": if entry.manifest.skills.is_empty() { "all" } else { "allowlist" },
            "mcp_servers": entry.manifest.mcp_servers,
            "mcp_servers_mode": if entry.manifest.mcp_servers.is_empty() { "all" } else { "allowlist" },
            "fallback_models": entry.manifest.fallback_models,
            "routing": entry.manifest.routing,
            "orchestration_mode": match entry.manifest.orchestration_mode {
                OrchestrationMode::Routing => "routing",
                OrchestrationMode::Delegation => "delegation",
                OrchestrationMode::Pinned => "pinned",
            },
        })),
    )
        .into_response()
}

/// DELETE /api/agents/:id - Kill an agent.
pub async fn kill_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    if let Some(entry) = state.kernel.registry.get(agent_id) {
        if entry.manifest.name.trim().eq_ignore_ascii_case("captain") {
            tracing::warn!(
                agent_id = %id,
                "Refused DELETE /api/agents/{id} - Captain is protected"
            );
            return error(
                StatusCode::FORBIDDEN,
                "Captain is the primary agent and cannot be killed via the API",
            );
        }
    }

    match state.kernel.kill_agent(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "killed", "agent_id": id})),
        )
            .into_response(),
        Err(e) => {
            tracing::warn!("kill_agent failed for {id}: {e}");
            error(
                StatusCode::NOT_FOUND,
                "Agent not found or already terminated",
            )
        }
    }
}

/// POST /api/agents/{id}/restart - Restart a crashed/stuck agent.
pub async fn restart_agent(
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

    let agent_name = entry.name.clone();
    let previous_state = format!("{:?}", entry.state);
    drop(entry);

    let was_running = state.kernel.stop_agent_run(agent_id).unwrap_or(false);
    let _ = state
        .kernel
        .registry
        .set_state(agent_id, AgentState::Running);

    tracing::info!(
        agent = %agent_name,
        previous_state = %previous_state,
        task_cancelled = was_running,
        "Agent restarted via API"
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "restarted",
            "agent": agent_name,
            "agent_id": id,
            "previous_state": previous_state,
            "task_cancelled": was_running,
        })),
    )
        .into_response()
}

fn resolved_provider<'a>(
    entry: &'a captain_types::agent::AgentEntry,
    default_provider: &'a str,
) -> &'a str {
    if entry.manifest.model.provider.is_empty() || entry.manifest.model.provider == "default" {
        default_provider
    } else {
        entry.manifest.model.provider.as_str()
    }
}

fn resolved_model<'a>(
    entry: &'a captain_types::agent::AgentEntry,
    default_model: &'a str,
) -> &'a str {
    if entry.manifest.model.model.is_empty() || entry.manifest.model.model == "default" {
        default_model
    } else {
        entry.manifest.model.model.as_str()
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

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::agent::{AgentManifest, ManifestCapabilities, ToolProfile};

    #[test]
    fn effective_capabilities_expand_profile_when_tools_are_implicit() {
        let manifest = AgentManifest {
            profile: Some(ToolProfile::Coding),
            capabilities: ManifestCapabilities {
                network: vec!["api.example.com:443".to_string()],
                memory_read: vec!["project.*".to_string()],
                ..Default::default()
            },
            ..Default::default()
        };

        let caps = effective_manifest_capabilities(&manifest);

        assert!(caps.tools.contains(&"shell_exec".to_string()));
        assert_eq!(caps.network, vec!["api.example.com:443"]);
        assert_eq!(caps.memory_read, vec!["project.*"]);
        assert!(caps.shell.contains(&"*".to_string()));
    }

    #[test]
    fn effective_capabilities_keep_explicit_tools_over_profile() {
        let manifest = AgentManifest {
            profile: Some(ToolProfile::Full),
            capabilities: ManifestCapabilities {
                tools: vec!["file_read".to_string()],
                ..Default::default()
            },
            ..Default::default()
        };

        let caps = effective_manifest_capabilities(&manifest);

        assert_eq!(caps.tools, vec!["file_read"]);
        assert!(caps.shell.is_empty());
        assert!(!caps.agent_spawn);
    }

    #[test]
    fn effective_capabilities_show_tool_allowlist_for_operator_views() {
        let manifest = AgentManifest {
            tool_allowlist: vec![
                "web_fetch".to_string(),
                "memory_recall".to_string(),
                "memory_save".to_string(),
            ],
            ..Default::default()
        };

        let caps = effective_manifest_capabilities(&manifest);

        assert_eq!(
            caps.tools,
            vec![
                "web_fetch".to_string(),
                "memory_recall".to_string(),
                "memory_save".to_string()
            ]
        );
        assert_eq!(caps.network, vec!["*"]);
        assert_eq!(caps.memory_read, vec!["self.*"]);
        assert_eq!(caps.memory_write, vec!["self.*"]);
    }
}
