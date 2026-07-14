use crate::project_lookup_input::{normalize_project_lookup_key, PROJECT_LOOKUP_NOT_FOUND_ERROR};
use crate::project_storage_error::safe_project_storage_error;
use crate::project_update_input::{
    normalize_project_update_goal, normalize_project_update_name, normalize_project_update_status,
    rejects_project_metadata_patch, PROJECT_METADATA_PATCH_ERROR,
};
use crate::routes::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_memory::project;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct UpdateProjectReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub deadline: Option<i64>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

pub async fn update_project(
    State(state): State<Arc<AppState>>,
    Path(id_or_slug): Path<String>,
    Json(req): Json<UpdateProjectReq>,
) -> impl IntoResponse {
    let project = match resolve_project_for_request(&state, &id_or_slug) {
        Ok(project) => project,
        Err(response) => return response,
    };

    let name = match normalize_project_update_name(req.name) {
        Ok(name) => name,
        Err(error) => return bad_request(error),
    };
    let goal = match normalize_project_update_goal(req.goal) {
        Ok(goal) => goal,
        Err(error) => return bad_request(error),
    };
    let status = match normalize_project_update_status(req.status) {
        Ok(status) => status,
        Err(error) => return bad_request(error),
    };
    if rejects_project_metadata_patch(&req.metadata) {
        return bad_request(PROJECT_METADATA_PATCH_ERROR);
    }
    if name.is_none() && goal.is_none() && status.is_none() && req.deadline.is_none() {
        return bad_request("at least one project field is required");
    }

    match state.kernel.memory.project_update(
        &project.id,
        project::ProjectPatch {
            name,
            goal,
            status,
            deadline: req.deadline.map(Some),
            metadata: None,
        },
    ) {
        Ok(Some(project)) => (
            StatusCode::OK,
            Json(crate::project_runtime_response::enrich_project(
                &state, project,
            )),
        ),
        Ok(None) => not_found(PROJECT_LOOKUP_NOT_FOUND_ERROR),
        Err(error) => server_error(safe_project_storage_error(&error.to_string())),
    }
}

fn resolve_project_for_request(
    state: &AppState,
    id_or_slug: &str,
) -> Result<project::Project, (StatusCode, Json<serde_json::Value>)> {
    let id_or_slug = normalize_project_lookup_key(id_or_slug).map_err(bad_request)?;
    match resolve_project(state, &id_or_slug) {
        Ok(Some(project)) => Ok(project),
        Ok(None) => Err(not_found(PROJECT_LOOKUP_NOT_FOUND_ERROR)),
        Err(error) => Err(server_error(safe_project_storage_error(&error.to_string()))),
    }
}

fn resolve_project(
    state: &AppState,
    id_or_slug: &str,
) -> Result<Option<project::Project>, captain_types::error::CaptainError> {
    match state.kernel.memory.project_get(id_or_slug) {
        Ok(Some(project)) => Ok(Some(project)),
        Ok(None) => state.kernel.memory.project_find_by_slug(id_or_slug),
        Err(error) => Err(error),
    }
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
#[path = "project_update_routes_tests.rs"]
mod project_update_routes_tests;
