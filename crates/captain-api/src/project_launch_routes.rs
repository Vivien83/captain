use crate::project_launch_error::safe_project_launch_error;
use crate::project_launch_flow::{prepare_project_launch_flow, ProjectLaunchFlowError};
use crate::project_launch_input::{normalize_project_launch_request, LaunchProjectReq};
use crate::project_launch_records::{publish_project_launch_created_event, ProjectLaunchRecords};
use crate::project_resume_view as resume_view;
use crate::project_storage_error::safe_project_storage_error;
use crate::routes::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_memory::project;
use std::sync::Arc;

pub async fn launch_project(
    State(state): State<Arc<AppState>>,
    Json(mut req): Json<LaunchProjectReq>,
) -> impl IntoResponse {
    let launch = match normalize_project_launch_request(&mut req) {
        Ok(launch) => launch,
        Err(error) => return bad_request(error),
    };
    let launch_flow = match prepare_project_launch_flow(&state, &req, &launch).await {
        Ok(launch_flow) => launch_flow,
        Err(ProjectLaunchFlowError::Workspace(error)) => {
            return bad_request(safe_project_launch_error(&error));
        }
        Err(ProjectLaunchFlowError::Conflict) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": "slug already exists" })),
            );
        }
        Err(ProjectLaunchFlowError::BadRequest(error)) => return bad_request(error),
        Err(ProjectLaunchFlowError::Storage(error)) => {
            return server_error(safe_project_storage_error(&error));
        }
    };

    publish_project_launch_created_event(
        &state,
        &launch_flow.project,
        &launch_flow.launch_state,
        &launch_flow.records,
        launch_flow.rules_file_created,
    )
    .await;

    (
        StatusCode::CREATED,
        Json(project_launch_response_body(
            &state,
            launch_flow.project,
            launch_flow.records,
            launch_flow.rules_file_created,
        )),
    )
}

fn project_launch_response_body(
    state: &AppState,
    project: project::Project,
    records: ProjectLaunchRecords,
    rules_file_created: bool,
) -> serde_json::Value {
    serde_json::json!({
        "project": crate::project_runtime_response::enrich_project(state, project),
        "tasks": resume_view::task_list_view(
            serde_json::to_value(records.tasks).unwrap_or(serde_json::Value::Null),
        ),
        "goals": resume_view::goal_list_view(
            serde_json::to_value(records.goals).unwrap_or(serde_json::Value::Null),
        ),
        "milestone": resume_view::milestone_item_view(
            serde_json::to_value(records.milestone).unwrap_or(serde_json::Value::Null),
        ),
        "checkpoint": resume_view::checkpoint_view(
            serde_json::to_value(records.checkpoint).unwrap_or(serde_json::Value::Null),
        ),
        "rules_file_created": rules_file_created,
    })
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
#[path = "project_launch_input_tests.rs"]
mod project_launch_input_tests;
