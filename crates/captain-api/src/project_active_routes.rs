use crate::project_active_input::{
    normalize_active_project_agent_id, normalize_active_project_slug,
    ACTIVE_PROJECT_NOT_FOUND_ERROR,
};
use crate::project_storage_error::safe_project_storage_error;
use crate::routes::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

pub async fn get_active_project(
    State(_state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match normalize_active_project_agent_id(&agent_id) {
        Ok(agent_id) => agent_id,
        Err(error) => return bad_request(error),
    };
    let slug = captain_runtime::active_project::global().and_then(|reg| reg.get(&agent_id));
    (
        StatusCode::OK,
        Json(serde_json::json!({ "agent_id": agent_id, "slug": slug })),
    )
}

#[derive(Debug, Deserialize)]
pub struct SetActiveProjectReq {
    pub slug: String,
}

pub async fn set_active_project(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Json(req): Json<SetActiveProjectReq>,
) -> impl IntoResponse {
    let agent_id = match normalize_active_project_agent_id(&agent_id) {
        Ok(agent_id) => agent_id,
        Err(error) => return bad_request(error),
    };
    let slug = match normalize_active_project_slug(req.slug) {
        Ok(slug) => slug,
        Err(error) => return bad_request(error),
    };
    match state.kernel.memory.project_find_by_slug(&slug) {
        Ok(None) => return not_found(ACTIVE_PROJECT_NOT_FOUND_ERROR),
        Err(error) => return server_error(safe_project_storage_error(&error.to_string())),
        Ok(Some(_)) => {}
    }
    let Some(registry) = captain_runtime::active_project::global() else {
        return server_error(
            "Active project selection is unavailable; restart Captain".to_string(),
        );
    };
    registry.set(agent_id.clone(), slug.clone());
    (
        StatusCode::OK,
        Json(serde_json::json!({ "agent_id": agent_id, "slug": slug })),
    )
}

pub async fn clear_active_project(
    State(_state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match normalize_active_project_agent_id(&agent_id) {
        Ok(agent_id) => agent_id,
        Err(error) => return bad_request(error),
    };
    let removed = captain_runtime::active_project::global()
        .map(|registry| registry.clear(&agent_id))
        .unwrap_or(false);
    (
        StatusCode::OK,
        Json(serde_json::json!({ "agent_id": agent_id, "cleared": removed })),
    )
}

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": msg.into() })),
    )
}

fn not_found(msg: impl Into<String>) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": msg.into() })),
    )
}

fn server_error(msg: impl Into<String>) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": msg.into() })),
    )
}

#[cfg(test)]
#[path = "project_active_input_tests.rs"]
mod project_active_input_tests;
