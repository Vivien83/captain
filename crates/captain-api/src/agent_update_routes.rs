//! Agent update route handlers.

use crate::state::AppState;
use crate::types::AgentUpdateRequest;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use captain_types::agent::{AgentId, AgentManifest};
use std::sync::Arc;

const REMOVED_ROUTING_ERROR: &str = "Automatic per-turn model routing was removed. Configure the agent model explicitly or create a specialist sub-agent.";

pub async fn update_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AgentUpdateRequest>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    if state.kernel.registry.get(agent_id).is_none() {
        return error(StatusCode::NOT_FOUND, "Agent not found");
    }
    let manifest_value: toml::Value = match toml::from_str(&req.manifest_toml) {
        Ok(value) => value,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid manifest: {e}")})),
            )
                .into_response();
        }
    };
    if manifest_uses_removed_routing(&manifest_value) {
        return error(StatusCode::BAD_REQUEST, REMOVED_ROUTING_ERROR);
    }

    let _manifest: AgentManifest = match manifest_value.try_into() {
        Ok(manifest) => manifest,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid manifest: {e}")})),
            )
                .into_response();
        }
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "acknowledged",
            "agent_id": id,
            "note": "Full manifest update requires agent restart. Use DELETE + POST to apply.",
        })),
    )
        .into_response()
}

/// PATCH /api/agents/{id} - Partial update of agent fields.
pub async fn patch_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    if state.kernel.registry.get(agent_id).is_none() {
        return error(StatusCode::NOT_FOUND, "Agent not found");
    }
    if body.get("routing").is_some()
        || matches!(
            body.get("orchestration_mode")
                .and_then(|value| value.as_str()),
            Some("routing" | "pinned")
        )
    {
        return error(StatusCode::BAD_REQUEST, REMOVED_ROUTING_ERROR);
    }

    if let Some(name) = body.get("name").and_then(|value| value.as_str()) {
        if let Err(e) = state
            .kernel
            .registry
            .update_name(agent_id, name.to_string())
        {
            return bad_request(e);
        }
    }
    if let Some(description) = body.get("description").and_then(|value| value.as_str()) {
        if let Err(e) = state
            .kernel
            .registry
            .update_description(agent_id, description.to_string())
        {
            return bad_request(e);
        }
    }
    if let Some(model) = body.get("model").and_then(|value| value.as_str()) {
        let provider = body.get("provider").and_then(|value| value.as_str());
        if let Err(e) = state.kernel.set_agent_model(agent_id, model, provider) {
            return bad_request(e);
        }
    }
    if let Some(prompt) = body.get("system_prompt").and_then(|value| value.as_str()) {
        if let Err(e) = state
            .kernel
            .registry
            .update_system_prompt(agent_id, prompt.to_string())
        {
            return bad_request(e);
        }
    }
    if let Some(mode) = body
        .get("orchestration_mode")
        .and_then(|value| value.as_str())
    {
        if let Err(response) = update_orchestration_mode(&state, agent_id, mode) {
            return response;
        }
    }
    if let Some(entry) = state.kernel.registry.get(agent_id) {
        let _ = state.kernel.memory.save_agent(&entry);
        (
            StatusCode::OK,
            Json(
                serde_json::json!({"status": "ok", "agent_id": entry.id.to_string(), "name": entry.name}),
            ),
        )
            .into_response()
    } else {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Agent vanished during update",
        )
    }
}

#[allow(clippy::result_large_err)]
fn update_orchestration_mode(
    state: &AppState,
    agent_id: AgentId,
    mode: &str,
) -> Result<(), axum::response::Response> {
    let mode = match mode {
        "direct" => captain_types::agent::OrchestrationMode::Direct,
        "delegation" => captain_types::agent::OrchestrationMode::Delegation,
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("Invalid orchestration_mode '{other}'. Expected: direct or delegation")
                })),
            )
                .into_response());
        }
    };
    state
        .kernel
        .registry
        .update_orchestration_mode(agent_id, mode)
        .map_err(bad_request)
}

fn manifest_uses_removed_routing(manifest: &toml::Value) -> bool {
    manifest.get("routing").is_some()
        || matches!(
            manifest
                .get("orchestration_mode")
                .and_then(|value| value.as_str()),
            Some("routing" | "pinned")
        )
}

#[allow(clippy::result_large_err)]
fn parse_agent_id(id: &str) -> Result<AgentId, axum::response::Response> {
    id.parse()
        .map_err(|_| error(StatusCode::BAD_REQUEST, "Invalid agent ID"))
}

fn bad_request(err: impl std::fmt::Display) -> axum::response::Response {
    error(StatusCode::BAD_REQUEST, &format!("{err}"))
}

fn error(status: StatusCode, message: &str) -> axum::response::Response {
    (status, Json(serde_json::json!({"error": message}))).into_response()
}
