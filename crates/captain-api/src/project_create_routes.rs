use crate::project_metadata::project_metadata;
use crate::project_storage_error::safe_project_storage_error;
use crate::project_update_input::{
    normalize_project_create_goal, normalize_project_create_name, normalize_project_slug,
};
use crate::routes::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_memory::project;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct CreateProjectReq {
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub deadline: Option<i64>,
}

pub async fn create_project(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateProjectReq>,
) -> impl IntoResponse {
    let name = match normalize_project_create_name(req.name) {
        Ok(name) => name,
        Err(error) => return bad_request(error),
    };
    let slug = match normalize_project_slug(req.slug) {
        Ok(slug) => slug,
        Err(error) => return bad_request(error),
    };
    let goal = match normalize_project_create_goal(req.goal) {
        Ok(goal) => goal,
        Err(error) => return bad_request(error),
    };
    match state.kernel.memory.project_create(project::NewProject {
        name,
        slug,
        goal,
        deadline: req.deadline,
    }) {
        Ok(project) => {
            let project = state
                .kernel
                .memory
                .project_update(
                    &project.id,
                    project::ProjectPatch {
                        metadata: Some(project_metadata(None, "observe")),
                        ..Default::default()
                    },
                )
                .ok()
                .flatten()
                .unwrap_or(project);
            (
                StatusCode::CREATED,
                Json(crate::project_runtime_response::enrich_project(
                    &state, project,
                )),
            )
        }
        Err(error) => {
            let msg = error.to_string();
            if msg.to_lowercase().contains("unique") {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({ "error": "slug already exists" })),
                );
            }
            server_error(safe_project_storage_error(&msg))
        }
    }
}

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
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
#[path = "project_create_input_tests.rs"]
mod project_create_input_tests;
