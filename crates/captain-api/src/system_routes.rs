//! System operation route handlers.

use crate::shutdown_guard::{
    active_shutdown_work, record_shutdown_deferred, shutdown_deferred_body, shutdown_drain_state,
};
use crate::state::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use captain_runtime::kernel_handle::KernelHandle;
use captain_types::version::captain_version;
use std::path::PathBuf;
use std::sync::Arc;

pub async fn shutdown(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let work = active_shutdown_work(&state.kernel);
    if !work.is_empty() {
        let body = shutdown_deferred_body(work);
        shutdown_drain_state().mark_draining("api", work);
        record_shutdown_deferred(&state.kernel, "api", work);
        return (StatusCode::ACCEPTED, Json(body)).into_response();
    }

    tracing::info!("Shutdown requested via API");
    shutdown_drain_state().clear();
    state.kernel.audit_log.record(
        "system",
        captain_runtime::audit::AuditAction::ConfigChange,
        "shutdown requested via API",
        "ok",
    );
    state.kernel.shutdown();
    state.shutdown_notify.notify_one();
    Json(serde_json::json!({"status": "shutting_down"})).into_response()
}

pub async fn add_workspace_path(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> Response {
    let path = match workspace_path_from_body(&req) {
        Ok(path) => path,
        Err(response) => return response,
    };

    match state.kernel.add_workspace_path(&path) {
        Ok(()) => json_response(
            StatusCode::OK,
            serde_json::json!({"status": "ok", "path": path.display().to_string()}),
        ),
        Err(error) => json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": error})),
    }
}

pub async fn version() -> impl IntoResponse {
    Json(serde_json::json!({
        "name": "captain",
        "version": captain_version(),
        "build_date": option_env!("BUILD_DATE").unwrap_or("dev"),
        "git_sha": option_env!("GIT_SHA").unwrap_or("unknown"),
        "rust_version": option_env!("RUSTC_VERSION").unwrap_or("unknown"),
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    }))
}

#[allow(clippy::result_large_err)]
fn workspace_path_from_body(body: &serde_json::Value) -> Result<PathBuf, Response> {
    match body["path"].as_str() {
        Some(path) if !path.is_empty() => Ok(PathBuf::from(path)),
        _ => Err(json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"error": "Missing or empty 'path'"}),
        )),
    }
}

fn json_response(status: StatusCode, body: serde_json::Value) -> Response {
    (status, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_path_from_body_accepts_path() {
        let body = serde_json::json!({"path": "/tmp/captain-workspace"});

        assert_eq!(
            workspace_path_from_body(&body).unwrap(),
            PathBuf::from("/tmp/captain-workspace")
        );
    }

    #[test]
    fn workspace_path_from_body_rejects_missing_path() {
        let response = workspace_path_from_body(&serde_json::json!({})).unwrap_err();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn workspace_path_from_body_rejects_empty_path() {
        let response = workspace_path_from_body(&serde_json::json!({"path": ""})).unwrap_err();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
