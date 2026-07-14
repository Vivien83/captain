use crate::project_lookup_input::{normalize_project_lookup_key, PROJECT_LOOKUP_NOT_FOUND_ERROR};
use crate::project_runtime_mutation::update_project_runtime_state;
use crate::project_runtime_response::project_runtime_response;
use crate::project_storage_error::safe_project_storage_error;
use crate::routes::AppState;
use axum::http::StatusCode;
use axum::Json;
use captain_memory::project;
use std::sync::Arc;

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

fn resolve_project(
    state: &AppState,
    id_or_slug: &str,
) -> Result<Option<project::Project>, captain_types::error::CaptainError> {
    match state.kernel.memory.project_get(id_or_slug) {
        Ok(Some(project)) => Ok(Some(project)),
        Ok(None) => state.kernel.memory.project_find_by_slug(id_or_slug),
        Err(e) => Err(e),
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
        Err(e) => Err(server_error(safe_project_storage_error(&e.to_string()))),
    }
}

pub(crate) async fn mutate_project_runtime<F>(
    state: &Arc<AppState>,
    id_or_slug: &str,
    mutate: F,
) -> (StatusCode, Json<serde_json::Value>)
where
    F: FnOnce(&mut serde_json::Value, &project::Project) + Send,
{
    let project = match resolve_project_for_request(state, id_or_slug) {
        Ok(project) => project,
        Err(response) => return response,
    };
    match update_project_runtime_state(state, &project.id, mutate).await {
        Ok(updated) => (
            StatusCode::OK,
            Json(project_runtime_response(state, &updated)),
        ),
        Err(error) => server_error(safe_project_storage_error(&error)),
    }
}

#[cfg(test)]
#[path = "project_runtime_route_support_tests.rs"]
mod project_runtime_route_support_tests;
