//! Agent identity, config, and clone route handlers.

use crate::agent_file_routes::KNOWN_IDENTITY_FILES;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use captain_types::agent::{AgentId, AgentIdentity};
use std::sync::Arc;

#[derive(serde::Deserialize)]
pub struct UpdateIdentityRequest {
    pub emoji: Option<String>,
    pub avatar_url: Option<String>,
    pub color: Option<String>,
    #[serde(default)]
    pub archetype: Option<String>,
    #[serde(default)]
    pub vibe: Option<String>,
    #[serde(default)]
    pub greeting_style: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct PatchAgentConfigRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub emoji: Option<String>,
    pub avatar_url: Option<String>,
    pub color: Option<String>,
    pub archetype: Option<String>,
    pub vibe: Option<String>,
    pub greeting_style: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
    pub fallback_models: Option<Vec<captain_types::agent::FallbackModel>>,
}

#[derive(serde::Deserialize)]
pub struct CloneAgentRequest {
    pub new_name: String,
}

/// PATCH /api/agents/{id}/identity - Update an agent's visual identity.
pub async fn update_agent_identity(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateIdentityRequest>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    if let Err(response) = validate_visual_identity(req.color.as_deref(), req.avatar_url.as_deref())
    {
        return response;
    }

    let identity = AgentIdentity {
        emoji: req.emoji,
        avatar_url: req.avatar_url,
        color: req.color,
        archetype: req.archetype,
        vibe: req.vibe,
        greeting_style: req.greeting_style,
    };

    match state.kernel.registry.update_identity(agent_id, identity) {
        Ok(()) => {
            if let Some(entry) = state.kernel.registry.get(agent_id) {
                let _ = state.kernel.memory.save_agent(&entry);
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "agent_id": id})),
            )
                .into_response()
        }
        Err(_) => error(StatusCode::NOT_FOUND, "Agent not found"),
    }
}

/// PATCH /api/agents/{id}/config - Hot-update agent config.
pub async fn patch_agent_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<PatchAgentConfigRequest>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    if let Err(response) = validate_config_request(&req) {
        return response;
    }
    if let Err(response) = apply_text_config(&state, agent_id, &req) {
        return response;
    }
    if let Err(response) = apply_identity_config(&state, agent_id, req_identity(&req)) {
        return response;
    }
    if let Err(response) = apply_model_config(&state, agent_id, &req) {
        return response;
    }
    if let Some(fallbacks) = req.fallback_models {
        if state
            .kernel
            .registry
            .update_fallback_models(agent_id, fallbacks)
            .is_err()
        {
            return error(StatusCode::NOT_FOUND, "Agent not found");
        }
    }

    if let Some(entry) = state.kernel.registry.get(agent_id) {
        if let Err(e) = state.kernel.memory.save_agent(&entry) {
            tracing::warn!("Failed to persist agent config update: {e}");
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "ok", "agent_id": id})),
    )
        .into_response()
}

/// POST /api/agents/{id}/clone - Clone an agent with its workspace files.
pub async fn clone_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CloneAgentRequest>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    if req.new_name.len() > 256 {
        return error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Name exceeds max length (256 chars)",
        );
    }
    if req.new_name.trim().is_empty() {
        return error(StatusCode::BAD_REQUEST, "new_name cannot be empty");
    }

    let source = match state.kernel.registry.get(agent_id) {
        Some(entry) => entry,
        None => return error(StatusCode::NOT_FOUND, "Agent not found"),
    };

    let mut cloned_manifest = source.manifest.clone();
    cloned_manifest.name = req.new_name.clone();
    cloned_manifest.workspace = None;

    let new_id = match state.kernel.spawn_agent(cloned_manifest) {
        Ok(new_id) => new_id,
        Err(e) => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Clone spawn failed: {e}"),
            );
        }
    };

    copy_workspace_identity_files(&state, &source, new_id);
    let _ = state
        .kernel
        .registry
        .update_identity(new_id, source.identity.clone());

    if let Some(ref manager) = *state.bridge_manager.lock().await {
        manager
            .router()
            .register_agent(req.new_name.clone(), new_id);
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "agent_id": new_id.to_string(),
            "name": req.new_name,
        })),
    )
        .into_response()
}

#[allow(clippy::result_large_err)]
fn validate_config_request(req: &PatchAgentConfigRequest) -> Result<(), axum::response::Response> {
    if req.name.as_ref().is_some_and(|value| value.len() > 256) {
        return Err(error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Name exceeds max length (256 chars)",
        ));
    }
    if req
        .description
        .as_ref()
        .is_some_and(|value| value.len() > 4096)
    {
        return Err(error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Description exceeds max length (4096 chars)",
        ));
    }
    if req
        .system_prompt
        .as_ref()
        .is_some_and(|value| value.len() > 65_536)
    {
        return Err(error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "System prompt exceeds max length (65536 chars)",
        ));
    }
    validate_visual_identity(req.color.as_deref(), req.avatar_url.as_deref())
}

#[allow(clippy::result_large_err)]
fn validate_visual_identity(
    color: Option<&str>,
    avatar_url: Option<&str>,
) -> Result<(), axum::response::Response> {
    if color.is_some_and(|value| !value.is_empty() && !value.starts_with('#')) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Color must be a hex code starting with '#'",
        ));
    }
    if avatar_url.is_some_and(|value| {
        !value.is_empty()
            && !value.starts_with("http://")
            && !value.starts_with("https://")
            && !value.starts_with("data:")
    }) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Avatar URL must be http/https or data URI",
        ));
    }
    Ok(())
}

#[allow(clippy::result_large_err)]
fn apply_text_config(
    state: &AppState,
    agent_id: AgentId,
    req: &PatchAgentConfigRequest,
) -> Result<(), axum::response::Response> {
    if let Some(name) = req.name.as_ref().filter(|value| !value.is_empty()) {
        state
            .kernel
            .registry
            .update_name(agent_id, name.clone())
            .map_err(|e| error(StatusCode::CONFLICT, &format!("{e}")))?;
    }
    if let Some(description) = &req.description {
        state
            .kernel
            .registry
            .update_description(agent_id, description.clone())
            .map_err(|_| error(StatusCode::NOT_FOUND, "Agent not found"))?;
    }
    if let Some(prompt) = &req.system_prompt {
        state
            .kernel
            .registry
            .update_system_prompt(agent_id, prompt.clone())
            .map_err(|_| error(StatusCode::NOT_FOUND, "Agent not found"))?;
    }
    Ok(())
}

fn req_identity(req: &PatchAgentConfigRequest) -> Option<AgentIdentity> {
    if req.emoji.is_none()
        && req.avatar_url.is_none()
        && req.color.is_none()
        && req.archetype.is_none()
        && req.vibe.is_none()
        && req.greeting_style.is_none()
    {
        return None;
    }
    Some(AgentIdentity {
        emoji: req.emoji.clone(),
        avatar_url: req.avatar_url.clone(),
        color: req.color.clone(),
        archetype: req.archetype.clone(),
        vibe: req.vibe.clone(),
        greeting_style: req.greeting_style.clone(),
    })
}

#[allow(clippy::result_large_err)]
fn apply_identity_config(
    state: &AppState,
    agent_id: AgentId,
    identity: Option<AgentIdentity>,
) -> Result<(), axum::response::Response> {
    let Some(identity) = identity else {
        return Ok(());
    };
    let current = state
        .kernel
        .registry
        .get(agent_id)
        .map(|entry| entry.identity)
        .unwrap_or_default();
    let merged = AgentIdentity {
        emoji: identity.emoji.or(current.emoji),
        avatar_url: identity.avatar_url.or(current.avatar_url),
        color: identity.color.or(current.color),
        archetype: identity.archetype.or(current.archetype),
        vibe: identity.vibe.or(current.vibe),
        greeting_style: identity.greeting_style.or(current.greeting_style),
    };
    state
        .kernel
        .registry
        .update_identity(agent_id, merged)
        .map_err(|_| error(StatusCode::NOT_FOUND, "Agent not found"))
}

#[allow(clippy::result_large_err)]
fn apply_model_config(
    state: &AppState,
    agent_id: AgentId,
    req: &PatchAgentConfigRequest,
) -> Result<(), axum::response::Response> {
    let Some(model) = req.model.as_ref().filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    match req.provider.as_ref().filter(|value| !value.is_empty()) {
        Some(provider) => state
            .kernel
            .registry
            .update_model_and_provider(agent_id, model.clone(), provider.clone())
            .map_err(|_| error(StatusCode::NOT_FOUND, "Agent not found")),
        None => state
            .kernel
            .set_agent_model(agent_id, model, None)
            .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{e}"))),
    }
}

fn copy_workspace_identity_files(
    state: &AppState,
    source: &captain_types::agent::AgentEntry,
    new_id: AgentId,
) {
    let new_entry = state.kernel.registry.get(new_id);
    if let (Some(src_ws), Some(new_entry)) = (&source.manifest.workspace, new_entry) {
        if let Some(dst_ws) = &new_entry.manifest.workspace {
            if let (Ok(src_can), Ok(dst_can)) = (src_ws.canonicalize(), dst_ws.canonicalize()) {
                for &filename in KNOWN_IDENTITY_FILES {
                    let src_file = src_can.join(filename);
                    let dst_file = dst_can.join(filename);
                    if src_file.exists() {
                        let _ = std::fs::copy(&src_file, &dst_file);
                    }
                }
            }
        }
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
