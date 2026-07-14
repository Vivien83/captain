use crate::project_lookup_input::{normalize_project_lookup_key, PROJECT_LOOKUP_NOT_FOUND_ERROR};
use crate::project_storage_error::safe_project_storage_error;
use crate::routes::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_memory::project;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize, Default)]
pub struct ProjectRuntimeQuery {
    #[serde(default)]
    pub events: Option<usize>,
}

pub async fn project_runtime(
    State(state): State<Arc<AppState>>,
    Path(id_or_slug): Path<String>,
    Query(params): Query<ProjectRuntimeQuery>,
) -> impl IntoResponse {
    let project = match resolve_project_for_request(&state, &id_or_slug) {
        Ok(project) => project,
        Err(response) => return response,
    };
    (
        StatusCode::OK,
        Json(
            crate::project_runtime_response::project_runtime_response_with_limit(
                &state,
                &project,
                crate::project_runtime_response::project_runtime_transcript_limit(params.events),
            ),
        ),
    )
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
#[path = "project_runtime_routes_tests.rs"]
mod project_runtime_routes_tests;
