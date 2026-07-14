use crate::project_environment_view as environment_view;
use crate::project_workspace::github_token;
use crate::routes::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

pub async fn projects_environment(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(environment_view::projects_environment_view(
            github_token(&state).is_some(),
        )),
    )
}

#[cfg(test)]
#[path = "project_environment_routes_tests.rs"]
mod project_environment_routes_tests;
