use crate::project_lifecycle::is_valid_lifecycle_phase;
use crate::project_runtime_ask_resume::{
    runtime_resume_pending_phase, runtime_resume_pending_reason,
};
use crate::project_runtime_orchestrator::{
    resume_runtime_orchestrator, runtime_resume_event_metadata,
};
use crate::project_runtime_resume::runtime_declares_active;
use crate::project_runtime_tool_status::prepare_denied_tool_request_retry;
use crate::routes::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

pub async fn resume_project_runtime(
    State(state): State<Arc<AppState>>,
    Path(id_or_slug): Path<String>,
) -> impl IntoResponse {
    let (response, spawn_key) = prepare_resume_project_runtime(&state, &id_or_slug).await;
    if let Some(project_key) = spawn_key {
        crate::project_runtime_runner::spawn_project_runtime_if_needed(state, project_key);
    }
    response
}

async fn prepare_resume_project_runtime(
    state: &Arc<AppState>,
    id_or_slug: &str,
) -> ((StatusCode, Json<serde_json::Value>), Option<String>) {
    let project_key = id_or_slug.to_string();
    let response = crate::project_runtime_route_support::mutate_project_runtime(
        state,
        id_or_slug,
        |runtime, _project| {
            let stale_resume = runtime_declares_active(runtime);
            let resume_pending_phase = runtime_resume_pending_phase(runtime)
                .filter(|phase| is_valid_lifecycle_phase(phase));
            let resume_pending_reason = runtime_resume_pending_reason(runtime);
            let phase = resume_pending_phase.clone().unwrap_or_else(|| {
                runtime
                    .get("current_phase")
                    .and_then(|value| value.as_str())
                    .filter(|phase| is_valid_lifecycle_phase(phase))
                    .unwrap_or("observe")
                    .to_string()
            });
            let resume = runtime_resume_event_metadata(resume_pending_reason.as_deref());
            let retry_after_denied_tool_request = resume_pending_phase.is_none()
                && prepare_denied_tool_request_retry(runtime, &phase);
            resume_runtime_orchestrator(
                runtime,
                &phase,
                if resume_pending_phase.is_some() {
                    resume.trigger
                } else {
                    "resume"
                },
                if resume_pending_phase.is_some() {
                    resume.kind
                } else {
                    "project.resumed"
                },
                if resume_pending_phase.is_some() {
                    resume.title
                } else {
                    "Run resumed"
                },
                if resume_pending_phase.is_some() {
                    resume.detail
                } else if stale_resume {
                    "Captain is resuming a persisted runtime after restart without resetting completed worker phases."
                } else if retry_after_denied_tool_request {
                    "Captain is relaunching the blocked phase after an operator denied a tool request; the worker receives that decision and must choose another path."
                } else {
                    "Captain can continue from the current lifecycle phase."
                },
                "user",
            );
        },
    )
    .await;
    let spawn_key = (response.0 == StatusCode::OK).then_some(project_key);
    (response, spawn_key)
}

#[cfg(test)]
#[path = "project_runtime_resume_routes_tests.rs"]
mod project_runtime_resume_routes_tests;
