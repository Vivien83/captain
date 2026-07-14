use crate::project_lookup_input::{normalize_project_lookup_key, PROJECT_LOOKUP_NOT_FOUND_ERROR};
use crate::project_resume_view as resume_view;
use crate::project_storage_error::safe_project_storage_error;
use crate::routes::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_memory::project;
use std::sync::Arc;

pub async fn resume_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let project = match resolve_project_for_request(&state, &id) {
        Ok(project) => project,
        Err(response) => return response,
    };
    let checkpoint = state
        .kernel
        .memory
        .checkpoint_latest(&project.id)
        .unwrap_or(None);
    let tasks = state
        .kernel
        .memory
        .task_list_for_project(&project.id)
        .unwrap_or_default();
    let now = chrono::Utc::now().timestamp_millis();
    let progress = state
        .kernel
        .memory
        .milestone_progress(&project.id, now)
        .ok();
    let goals = state
        .kernel
        .goal_store
        .list_for_project(&project.id, &project.slug);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "project": crate::project_runtime_response::enrich_project(&state, project),
            "latest_checkpoint": resume_view::checkpoint_view(
                serde_json::to_value(checkpoint).unwrap_or(serde_json::Value::Null),
            ),
            "tasks": resume_view::task_list_view(
                serde_json::to_value(tasks).unwrap_or(serde_json::Value::Null),
            ),
            "goals": resume_view::goal_list_view(
                serde_json::to_value(goals).unwrap_or(serde_json::Value::Null),
            ),
            "milestone_progress": resume_view::milestone_progress_view(
                serde_json::to_value(progress).unwrap_or(serde_json::Value::Null),
            ),
        })),
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
#[path = "project_resume_routes_tests.rs"]
mod project_resume_routes_tests;
