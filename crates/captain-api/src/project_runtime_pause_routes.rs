use crate::project_runtime_events::append_runtime_event;
use crate::project_runtime_orchestrator::deactivate_runtime_orchestrator;
use crate::routes::AppState;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use std::sync::Arc;

pub async fn pause_project_runtime(
    State(state): State<Arc<AppState>>,
    Path(id_or_slug): Path<String>,
) -> impl IntoResponse {
    crate::project_runtime_route_support::mutate_project_runtime(
        &state,
        &id_or_slug,
        |runtime, _project| {
            let now = chrono::Utc::now().to_rfc3339();
            runtime["status"] = serde_json::json!("paused");
            runtime["updated_at"] = serde_json::json!(now);
            runtime["control"] = serde_json::json!({
                "paused": true,
                "takeover": false,
            });
            let phase = runtime
                .get("current_phase")
                .and_then(|value| value.as_str())
                .unwrap_or("observe")
                .to_string();
            append_runtime_event(
                runtime,
                "project.paused",
                "Run paused",
                "Captain will keep the project context but should not continue autonomous execution until resumed.",
                "user",
                &phase,
                "paused",
                serde_json::json!({}),
            );
            deactivate_runtime_orchestrator(runtime, "paused");
        },
    )
    .await
}

#[cfg(test)]
#[path = "project_runtime_pause_routes_tests.rs"]
mod project_runtime_pause_routes_tests;
