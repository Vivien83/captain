//! Operator routes for managed background processes.

use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use captain_runtime::audit::AuditAction;
use std::sync::Arc;

/// DELETE /api/processes/{process_id} - Stop a managed background process.
pub async fn kill_process(
    State(state): State<Arc<AppState>>,
    Path(process_id): Path<String>,
) -> impl IntoResponse {
    if !valid_process_id(&process_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid process ID"})),
        );
    }

    match state.kernel.process_manager.kill(&process_id).await {
        Ok(()) => {
            state.kernel.audit_log.record(
                "system",
                AuditAction::ConfigChange,
                "managed background process stopped by operator",
                format!("process_id={process_id}"),
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "killed",
                    "process_id": process_id,
                    "operator_actions": ["Run captain status to verify shutdown drain can continue."]
                })),
            )
        }
        Err(error) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": error, "process_id": process_id})),
        ),
    }
}

fn valid_process_id(process_id: &str) -> bool {
    let Some(suffix) = process_id.strip_prefix("proc_") else {
        return false;
    };
    !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_id_validation_accepts_managed_ids_only() {
        assert!(valid_process_id("proc_42"));
        assert!(!valid_process_id("42"));
        assert!(!valid_process_id("proc_"));
        assert!(!valid_process_id("proc_42/private"));
    }
}
