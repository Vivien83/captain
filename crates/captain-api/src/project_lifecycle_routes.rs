use crate::project_lifecycle::set_lifecycle_phase;
use crate::project_lookup_input::{normalize_project_lookup_key, PROJECT_LOOKUP_NOT_FOUND_ERROR};
use crate::project_storage_error::safe_project_storage_error;
use crate::project_update_input::normalize_project_lifecycle_phase;
use crate::routes::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_memory::project;
use captain_types::agent::AgentId;
use captain_types::event::{Event, EventPayload, EventTarget};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct SetLifecyclePhaseReq {
    pub phase: String,
}

pub async fn set_project_lifecycle_phase(
    State(state): State<Arc<AppState>>,
    Path(id_or_slug): Path<String>,
    Json(req): Json<SetLifecyclePhaseReq>,
) -> impl IntoResponse {
    let project = match resolve_project_for_request(&state, &id_or_slug) {
        Ok(project) => project,
        Err(response) => return response,
    };
    let phase = match normalize_project_lifecycle_phase(req.phase) {
        Ok(phase) => phase,
        Err(error) => return bad_request(error),
    };

    let metadata = set_lifecycle_phase(project.metadata.clone(), &phase);
    match state.kernel.memory.project_update(
        &project.id,
        project::ProjectPatch {
            metadata: Some(metadata),
            ..Default::default()
        },
    ) {
        Ok(Some(project)) => {
            publish_project_event(
                &state,
                serde_json::json!({
                    "event": "project.lifecycle.updated",
                    "project_id": project.id,
                    "slug": project.slug,
                    "phase": phase,
                }),
            )
            .await;
            (
                StatusCode::OK,
                Json(crate::project_runtime_response::enrich_project(
                    &state, project,
                )),
            )
        }
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

async fn publish_project_event(state: &AppState, payload: serde_json::Value) {
    if let Ok(bytes) = serde_json::to_vec(&payload) {
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

#[cfg(test)]
#[path = "project_lifecycle_routes_tests.rs"]
mod project_lifecycle_routes_tests;
