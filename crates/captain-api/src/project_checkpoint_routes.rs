use crate::project_checkpoint_input::{
    normalize_project_checkpoint_limit, normalize_project_checkpoint_project_id,
};
use crate::project_lookup_input::PROJECT_LOOKUP_NOT_FOUND_ERROR;
use crate::project_resume_view as resume_view;
use crate::project_storage_error::safe_project_storage_error;
use crate::project_update_input::{
    normalize_project_checkpoint_session_id, normalize_project_checkpoint_summary,
    rejects_project_checkpoint_state, PROJECT_CHECKPOINT_STATE_ERROR,
};
use crate::routes::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_memory::{project, project_checkpoint};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

pub async fn list_checkpoints(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let id = match resolve_scoped_project_id(&state, id, normalize_project_checkpoint_project_id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let limit = match normalize_project_checkpoint_limit(params.get("limit")) {
        Ok(limit) => limit,
        Err(error) => return bad_request(error),
    };
    match state.kernel.memory.checkpoint_history(&id, limit) {
        Ok(rows) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "checkpoints": resume_view::checkpoint_list_view(
                    serde_json::to_value(rows).unwrap_or(serde_json::Value::Null),
                ),
            })),
        ),
        Err(error) => server_error(safe_project_storage_error(&error.to_string())),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateCheckpointReq {
    pub summary: String,
    #[serde(default)]
    pub state: serde_json::Value,
    #[serde(default)]
    pub session_id: Option<String>,
}

pub async fn create_checkpoint(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(req): Json<CreateCheckpointReq>,
) -> impl IntoResponse {
    let project_id = match resolve_scoped_project_id(
        &state,
        project_id,
        normalize_project_checkpoint_project_id,
    ) {
        Ok(project_id) => project_id,
        Err(response) => return response,
    };
    let summary = match normalize_project_checkpoint_summary(req.summary) {
        Ok(summary) => summary,
        Err(error) => return bad_request(error),
    };
    let session_id = match normalize_project_checkpoint_session_id(req.session_id) {
        Ok(session_id) => session_id,
        Err(error) => return bad_request(error),
    };
    if rejects_project_checkpoint_state(&req.state) {
        return bad_request(PROJECT_CHECKPOINT_STATE_ERROR);
    }
    match state
        .kernel
        .memory
        .checkpoint_append(project_checkpoint::NewCheckpoint {
            project_id,
            session_id,
            summary,
            state: serde_json::Value::Object(Default::default()),
        }) {
        Ok(checkpoint) => (
            StatusCode::CREATED,
            Json(resume_view::checkpoint_view(
                serde_json::to_value(&checkpoint).unwrap_or(serde_json::Value::Null),
            )),
        ),
        Err(error) => server_error(safe_project_storage_error(&error.to_string())),
    }
}

fn resolve_scoped_project_id<N>(
    state: &AppState,
    id_or_slug: String,
    normalize: N,
) -> Result<String, (StatusCode, Json<serde_json::Value>)>
where
    N: FnOnce(String) -> Result<String, &'static str>,
{
    let id_or_slug = normalize(id_or_slug).map_err(bad_request)?;
    match resolve_project(state, &id_or_slug) {
        Ok(Some(project)) => Ok(project.id),
        Ok(None) => Err(not_found(PROJECT_LOOKUP_NOT_FOUND_ERROR)),
        Err(error) => Err(server_error(safe_project_storage_error(&error))),
    }
}

fn resolve_project(state: &AppState, id_or_slug: &str) -> Result<Option<project::Project>, String> {
    state
        .kernel
        .memory
        .project_get(id_or_slug)
        .map_err(|error| error.to_string())
        .and_then(|found| {
            if found.is_some() {
                Ok(found)
            } else {
                state
                    .kernel
                    .memory
                    .project_find_by_slug(id_or_slug)
                    .map_err(|error| error.to_string())
            }
        })
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
#[path = "project_checkpoint_input_tests.rs"]
mod project_checkpoint_input_tests;
