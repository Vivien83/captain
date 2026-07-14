use crate::project_lookup_input::PROJECT_LOOKUP_NOT_FOUND_ERROR;
use crate::project_resume_view as resume_view;
use crate::project_storage_error::safe_project_storage_error;
use crate::project_task_input::{
    normalize_project_task_lookup_id, normalize_project_task_parent_id,
    normalize_project_task_project_id, normalize_project_task_update_parent_id,
    PROJECT_TASK_NOT_FOUND_ERROR,
};
use crate::project_update_input::{
    normalize_project_task_description, normalize_project_task_title,
    normalize_project_task_update_description, normalize_project_task_update_status,
    normalize_project_task_update_title,
};
use crate::routes::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_memory::{project, project_task};
use serde::Deserialize;
use std::sync::Arc;

pub async fn list_project_tasks(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let id = match resolve_scoped_project_id(&state, id, normalize_project_task_project_id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match state.kernel.memory.task_list_for_project(&id) {
        Ok(rows) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "tasks": resume_view::task_list_view(
                    serde_json::to_value(rows).unwrap_or(serde_json::Value::Null),
                ),
            })),
        ),
        Err(error) => server_error(safe_project_storage_error(&error.to_string())),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateTaskReq {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub deadline: Option<i64>,
}

pub async fn create_project_task(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(req): Json<CreateTaskReq>,
) -> impl IntoResponse {
    let project_id =
        match resolve_scoped_project_id(&state, project_id, normalize_project_task_project_id) {
            Ok(project_id) => project_id,
            Err(response) => return response,
        };
    let title = match normalize_project_task_title(req.title) {
        Ok(title) => title,
        Err(error) => return bad_request(error),
    };
    let description = match normalize_project_task_description(req.description) {
        Ok(description) => description,
        Err(error) => return bad_request(error),
    };
    let parent_id = match normalize_project_task_parent_id(req.parent_id) {
        Ok(parent_id) => parent_id,
        Err(error) => return bad_request(error),
    };
    match state
        .kernel
        .memory
        .task_create(project_task::NewProjectTask {
            project_id,
            parent_id,
            title,
            description,
            priority: req.priority.unwrap_or(0),
            deadline: req.deadline,
            assignee_agent_id: None,
        }) {
        Ok(task) => (
            StatusCode::CREATED,
            Json(resume_view::task_item_view(
                serde_json::to_value(&task).unwrap_or(serde_json::Value::Null),
            )),
        ),
        Err(error) => server_error(safe_project_storage_error(&error.to_string())),
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateTaskReq {
    pub status: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub priority: Option<i32>,
    #[serde(default, deserialize_with = "deserialize_nullable")]
    pub parent_id: Option<Option<String>>,
}

pub async fn update_project_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTaskReq>,
) -> impl IntoResponse {
    let id = match normalize_project_task_lookup_id(id) {
        Ok(id) => id,
        Err(error) => return bad_request(error),
    };
    let status = match normalize_project_task_update_status(req.status) {
        Ok(status) => status,
        Err(error) => return bad_request(error),
    };
    let title = match normalize_project_task_update_title(req.title) {
        Ok(title) => title,
        Err(error) => return bad_request(error),
    };
    let description = match normalize_project_task_update_description(req.description) {
        Ok(description) => description,
        Err(error) => return bad_request(error),
    };
    let parent_id = match normalize_project_task_update_parent_id(req.parent_id) {
        Ok(parent_id) => parent_id,
        Err(error) => return bad_request(error),
    };
    let patch = project_task::TaskPatch {
        title,
        description,
        status,
        parent_id,
        priority: req.priority,
        ..Default::default()
    };
    match state.kernel.memory.task_update(&id, patch) {
        Ok(Some(task)) => (
            StatusCode::OK,
            Json(resume_view::task_item_view(
                serde_json::to_value(&task).unwrap_or(serde_json::Value::Null),
            )),
        ),
        Ok(None) => not_found(PROJECT_TASK_NOT_FOUND_ERROR),
        Err(error) => server_error(safe_project_storage_error(&error.to_string())),
    }
}

pub async fn delete_project_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let id = match normalize_project_task_lookup_id(id) {
        Ok(id) => id,
        Err(error) => return bad_request(error),
    };
    match state.kernel.memory.task_delete(&id) {
        Ok(true) => (StatusCode::NO_CONTENT, Json(serde_json::Value::Null)),
        Ok(false) => not_found(PROJECT_TASK_NOT_FOUND_ERROR),
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

/// serde helper: allow `"parent_id": null` to mean "clear the parent"
/// while `absent` means "don't touch it".
fn deserialize_nullable<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: serde_json::Value = serde::Deserialize::deserialize(deserializer)?;
    Ok(Some(match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(id) => Some(id),
        other => {
            return Err(serde::de::Error::custom(format!(
                "expected string or null, got {other}"
            )))
        }
    }))
}

#[cfg(test)]
#[path = "project_task_input_tests.rs"]
mod project_task_input_tests;

#[cfg(test)]
#[path = "project_task_routes_tests.rs"]
mod project_task_routes_tests;
