use crate::project_lookup_input::{normalize_project_lookup_key, PROJECT_LOOKUP_NOT_FOUND_ERROR};
use crate::project_storage_error::safe_project_storage_error;
use crate::routes::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

pub async fn archive_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let id = match normalize_project_lookup_key(&id) {
        Ok(id) => id,
        Err(error) => return bad_request(error),
    };
    match state.kernel.memory.project_archive(&id) {
        Ok(Some(project)) => (
            StatusCode::OK,
            Json(crate::project_runtime_response::enrich_project(
                &state, project,
            )),
        ),
        Ok(None) => not_found(PROJECT_LOOKUP_NOT_FOUND_ERROR),
        Err(error) => server_error(safe_project_storage_error(&error.to_string())),
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
#[path = "project_archive_routes_tests.rs"]
mod project_archive_routes_tests;
