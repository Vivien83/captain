use crate::project_lookup_input::PROJECT_LOOKUP_NOT_FOUND_ERROR;
use crate::project_milestone_input::{
    normalize_project_milestone_lookup_id, normalize_project_milestone_project_id,
    PROJECT_MILESTONE_NOT_FOUND_ERROR,
};
use crate::project_resume_view as resume_view;
use crate::project_storage_error::safe_project_storage_error;
use crate::project_update_input::{
    normalize_project_milestone_deliverables, normalize_project_milestone_name,
};
use crate::routes::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_memory::{milestone, project};
use serde::Deserialize;
use std::sync::Arc;

pub async fn list_milestones(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let id = match resolve_scoped_project_id(&state, id, normalize_project_milestone_project_id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match state.kernel.memory.milestone_list_for_project(&id) {
        Ok(rows) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "milestones": resume_view::milestone_list_view(
                    serde_json::to_value(rows).unwrap_or(serde_json::Value::Null),
                ),
            })),
        ),
        Err(error) => server_error(safe_project_storage_error(&error.to_string())),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateMilestoneReq {
    pub name: String,
    #[serde(default)]
    pub due_date: Option<i64>,
    #[serde(default)]
    pub deliverables: Vec<String>,
}

pub async fn create_milestone(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(req): Json<CreateMilestoneReq>,
) -> impl IntoResponse {
    let project_id =
        match resolve_scoped_project_id(&state, project_id, normalize_project_milestone_project_id)
        {
            Ok(project_id) => project_id,
            Err(response) => return response,
        };
    let name = match normalize_project_milestone_name(req.name) {
        Ok(name) => name,
        Err(error) => return bad_request(error),
    };
    let deliverables = match normalize_project_milestone_deliverables(req.deliverables) {
        Ok(deliverables) => deliverables,
        Err(error) => return bad_request(error),
    };
    match state
        .kernel
        .memory
        .milestone_create(milestone::NewMilestone {
            project_id,
            name,
            due_date: req.due_date,
            deliverables,
        }) {
        Ok(row) => (
            StatusCode::CREATED,
            Json(resume_view::milestone_item_view(
                serde_json::to_value(&row).unwrap_or(serde_json::Value::Null),
            )),
        ),
        Err(error) => server_error(safe_project_storage_error(&error.to_string())),
    }
}

pub async fn complete_milestone(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let id = match normalize_project_milestone_lookup_id(id) {
        Ok(id) => id,
        Err(error) => return bad_request(error),
    };
    match state.kernel.memory.milestone_complete(&id) {
        Ok(Some(row)) => (
            StatusCode::OK,
            Json(resume_view::milestone_item_view(
                serde_json::to_value(&row).unwrap_or(serde_json::Value::Null),
            )),
        ),
        Ok(None) => not_found(PROJECT_MILESTONE_NOT_FOUND_ERROR),
        Err(error) => server_error(safe_project_storage_error(&error.to_string())),
    }
}

pub async fn get_milestone_progress(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let id = match resolve_scoped_project_id(&state, id, normalize_project_milestone_project_id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let now = chrono::Utc::now().timestamp_millis();
    match state.kernel.memory.milestone_progress(&id, now) {
        Ok(progress) => (
            StatusCode::OK,
            Json(resume_view::milestone_progress_view(
                serde_json::to_value(progress).unwrap_or(serde_json::Value::Null),
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
#[path = "project_milestone_input_tests.rs"]
mod project_milestone_input_tests;
