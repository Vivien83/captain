use crate::project_lookup_input::{normalize_project_lookup_key, PROJECT_LOOKUP_NOT_FOUND_ERROR};
use crate::project_storage_error::safe_project_storage_error;
use crate::routes::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_memory::project;
use captain_types::agent::AgentId;
use captain_types::event::{Event, EventPayload, EventTarget};
use std::sync::Arc;

pub async fn delete_project(
    State(state): State<Arc<AppState>>,
    Path(id_or_slug): Path<String>,
) -> impl IntoResponse {
    let project = match resolve_project_for_request(&state, &id_or_slug) {
        Ok(project) => project,
        Err(response) => return response,
    };
    let removed_goals = match state
        .kernel
        .goal_store
        .remove_for_project(&project.id, &project.slug)
    {
        Ok(count) => count,
        Err(error) => return server_error(safe_project_storage_error(&error.to_string())),
    };

    match state.kernel.memory.project_delete(&project.id) {
        Ok(true) => {
            publish_project_deleted_event(&state, &project, removed_goals).await;
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "deleted",
                    "project_id": project.id,
                    "removed_goals": removed_goals,
                })),
            )
        }
        Ok(false) => not_found(PROJECT_LOOKUP_NOT_FOUND_ERROR),
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

async fn publish_project_deleted_event(
    state: &AppState,
    project: &project::Project,
    removed_goals: usize,
) {
    let event_payload = serde_json::json!({
        "event": "project.deleted",
        "project_id": project.id,
        "slug": project.slug,
        "name": project.name,
        "removed_goals": removed_goals,
    });
    if let Ok(bytes) = serde_json::to_vec(&event_payload) {
        state
            .kernel
            .publish_event(Event::new(
                AgentId::new(),
                EventTarget::Broadcast,
                EventPayload::Custom(bytes),
            ))
            .await;
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
#[path = "project_delete_routes_tests.rs"]
mod project_delete_routes_tests;
