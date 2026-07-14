//! Agent runtime configuration route handlers.

use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_kernel::model_switch::ModelSwitchSessionStrategy;
use captain_runtime::mcp::extract_mcp_server;
use captain_types::agent::AgentId;
use std::collections::HashSet;
use std::sync::Arc;

type JsonResponse = (StatusCode, Json<serde_json::Value>);

#[derive(Debug, serde::Deserialize)]
pub struct ModelSwitchPlanRequest {
    pub model: String,
    pub provider: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ModelSwitchApplyRequest {
    pub model: String,
    pub provider: Option<String>,
    pub session_strategy: ModelSwitchSessionStrategy,
}

fn parse_agent_id(id: &str) -> Result<AgentId, JsonResponse> {
    id.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid agent ID"})),
        )
    })
}

fn json_string_vec(body: &serde_json::Value, key: &str) -> Vec<String> {
    body[key]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|value| value.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// POST /api/agents/{id}/model-switch/plan - safe provider/model switch preflight.
pub async fn model_switch_plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<ModelSwitchPlanRequest>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    if body.model.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing 'model' field"})),
        );
    }
    match state
        .kernel
        .plan_model_switch(agent_id, body.model.trim(), body.provider.as_deref())
    {
        Ok(plan) => (StatusCode::OK, Json(serde_json::json!(plan))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// POST /api/agents/{id}/model-switch/apply - transactional safe model/provider switch.
pub async fn model_switch_apply(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<ModelSwitchApplyRequest>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    if body.model.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing 'model' field"})),
        );
    }
    match state.kernel.apply_model_switch(
        agent_id,
        body.model.trim(),
        body.provider.as_deref(),
        body.session_strategy,
    ) {
        Ok(result) => (StatusCode::OK, Json(serde_json::json!(result))),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// PUT /api/agents/{id}/model - Switch an agent's model.
pub async fn set_model(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let model = match body["model"].as_str() {
        Some(m) if !m.is_empty() => m,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'model' field"})),
            )
        }
    };
    let explicit_provider = body["provider"].as_str();
    match state
        .kernel
        .set_agent_model(agent_id, model, explicit_provider)
    {
        Ok(()) => {
            // Read back normalized model/provider so frontend state matches the registry.
            let (resolved_model, resolved_provider) = state
                .kernel
                .registry
                .get(agent_id)
                .map(|entry| {
                    (
                        entry.manifest.model.model.clone(),
                        entry.manifest.model.provider.clone(),
                    )
                })
                .unwrap_or_else(|| (model.to_string(), String::new()));
            (
                StatusCode::OK,
                Json(
                    serde_json::json!({"status": "ok", "model": resolved_model, "provider": resolved_provider}),
                ),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// GET /api/agents/{id}/tools - Get an agent's tool allowlist/blocklist.
pub async fn get_agent_tools(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let entry = match state.kernel.registry.get(agent_id) {
        Some(entry) => entry,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            )
        }
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "tool_allowlist": entry.manifest.tool_allowlist,
            "tool_blocklist": entry.manifest.tool_blocklist,
        })),
    )
}

/// PUT /api/agents/{id}/tools - Update an agent's tool allowlist/blocklist.
pub async fn set_agent_tools(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let allowlist = body
        .get("tool_allowlist")
        .and_then(|value| value.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|value| value.as_str().map(String::from))
                .collect::<Vec<_>>()
        });
    let blocklist = body
        .get("tool_blocklist")
        .and_then(|value| value.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|value| value.as_str().map(String::from))
                .collect::<Vec<_>>()
        });

    if allowlist.is_none() && blocklist.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Provide 'tool_allowlist' and/or 'tool_blocklist'"})),
        );
    }

    match state
        .kernel
        .set_agent_tool_filters(agent_id, allowlist, blocklist)
    {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// GET /api/agents/{id}/skills - Get an agent's skill assignment info.
pub async fn get_agent_skills(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let entry = match state.kernel.registry.get(agent_id) {
        Some(entry) => entry,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            )
        }
    };
    let available = state
        .kernel
        .skill_registry
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .skill_names();
    let mode = if entry.manifest.skills.is_empty() {
        "all"
    } else {
        "allowlist"
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "assigned": entry.manifest.skills,
            "available": available,
            "mode": mode,
        })),
    )
}

/// PUT /api/agents/{id}/skills - Update an agent's skill allowlist.
pub async fn set_agent_skills(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let skills = json_string_vec(&body, "skills");
    match state.kernel.set_agent_skills(agent_id, skills.clone()) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "skills": skills})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// GET /api/agents/{id}/mcp_servers - Get an agent's MCP server assignment info.
pub async fn get_agent_mcp_servers(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let entry = match state.kernel.registry.get(agent_id) {
        Some(entry) => entry,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            )
        }
    };
    let mut available: Vec<String> = Vec::new();
    if let Ok(mcp_tools) = state.kernel.mcp_tools.lock() {
        let mut seen = HashSet::new();
        for tool in mcp_tools.iter() {
            if let Some(server) = extract_mcp_server(&tool.name) {
                if seen.insert(server.to_string()) {
                    available.push(server.to_string());
                }
            }
        }
    }
    let mode = if entry.manifest.mcp_servers.is_empty() {
        "all"
    } else {
        "allowlist"
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "assigned": entry.manifest.mcp_servers,
            "available": available,
            "mode": mode,
        })),
    )
}

/// PUT /api/agents/{id}/mcp_servers - Update an agent's MCP server allowlist.
pub async fn set_agent_mcp_servers(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let servers = json_string_vec(&body, "mcp_servers");
    match state
        .kernel
        .set_agent_mcp_servers(agent_id, servers.clone())
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "mcp_servers": servers})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}
