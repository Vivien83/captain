use crate::project_runtime_start::apply_project_runtime_start;
use crate::routes::AppState;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_kernel::hub_pairing_service::DeviceAccessIdentity;
use std::sync::Arc;

pub async fn start_project_runtime(
    State(state): State<Arc<AppState>>,
    Path(id_or_slug): Path<String>,
    client: Option<Extension<DeviceAccessIdentity>>,
) -> impl IntoResponse {
    let authority_origin = client
        .as_ref()
        .map(|_| captain_runtime::client_authority::paired_client_origin("project"));
    let (response, spawn_key) =
        prepare_start_project_runtime(&state, &id_or_slug, authority_origin.as_deref()).await;
    if let Some(project_key) = spawn_key {
        crate::project_runtime_runner::spawn_project_runtime_if_needed(state, project_key);
    }
    response
}

async fn prepare_start_project_runtime(
    state: &Arc<AppState>,
    id_or_slug: &str,
    authority_origin: Option<&str>,
) -> ((StatusCode, Json<serde_json::Value>), Option<String>) {
    let project_key = id_or_slug.to_string();
    let response = crate::project_runtime_route_support::mutate_project_runtime(
        state,
        id_or_slug,
        |runtime, project| {
            apply_project_runtime_start(
                runtime,
                project,
                crate::project_runtime_runner::project_runtime_is_running(&project.id),
                authority_origin,
            );
        },
    )
    .await;
    let spawn_key = (response.0 == StatusCode::OK).then_some(project_key);
    (response, spawn_key)
}

#[cfg(test)]
#[path = "project_runtime_start_routes_tests.rs"]
mod project_runtime_start_routes_tests;
