use crate::project_storage_error::safe_project_storage_error;
use crate::routes::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::collections::HashMap;
use std::sync::Arc;

pub async fn list_projects(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let include_archived = params
        .get("include_archived")
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    match state.kernel.memory.project_list(include_archived) {
        Ok(rows) => {
            let projects = rows
                .into_iter()
                .map(|project| crate::project_runtime_response::enrich_project(&state, project))
                .collect::<Vec<_>>();
            (
                StatusCode::OK,
                Json(serde_json::json!({ "projects": projects })),
            )
        }
        Err(error) => server_error(safe_project_storage_error(&error.to_string())),
    }
}

fn server_error(msg: impl Into<String>) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": msg.into() })),
    )
}

#[cfg(test)]
#[path = "project_list_routes_tests.rs"]
mod project_list_routes_tests;
