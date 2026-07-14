//! Provider OAuth route handlers.

use crate::secret_env::write_secret_env;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use dashmap::DashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

struct CopilotFlowState {
    device_code: String,
    interval: u64,
    expires_at: Instant,
}

static COPILOT_FLOWS: LazyLock<DashMap<String, CopilotFlowState>> = LazyLock::new(DashMap::new);

/// POST /api/providers/github-copilot/oauth/start
pub async fn copilot_oauth_start() -> impl IntoResponse {
    COPILOT_FLOWS.retain(|_, flow| flow.expires_at > Instant::now());

    match captain_runtime::copilot_oauth::start_device_flow().await {
        Ok(response) => {
            let poll_id = uuid::Uuid::new_v4().to_string();

            COPILOT_FLOWS.insert(
                poll_id.clone(),
                CopilotFlowState {
                    device_code: response.device_code,
                    interval: response.interval,
                    expires_at: Instant::now() + Duration::from_secs(response.expires_in),
                },
            );

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "user_code": response.user_code,
                    "verification_uri": response.verification_uri,
                    "poll_id": poll_id,
                    "expires_in": response.expires_in,
                    "interval": response.interval,
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        ),
    }
}

/// GET /api/providers/github-copilot/oauth/poll/{poll_id}
pub async fn copilot_oauth_poll(
    State(state): State<Arc<AppState>>,
    Path(poll_id): Path<String>,
) -> impl IntoResponse {
    let flow = match COPILOT_FLOWS.get(&poll_id) {
        Some(flow) => flow,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"status": "not_found", "error": "Unknown poll_id"})),
            );
        }
    };

    if flow.expires_at <= Instant::now() {
        drop(flow);
        COPILOT_FLOWS.remove(&poll_id);
        return (
            StatusCode::OK,
            Json(serde_json::json!({"status": "expired"})),
        );
    }

    let device_code = flow.device_code.clone();
    drop(flow);

    match captain_runtime::copilot_oauth::poll_device_flow(&device_code).await {
        captain_runtime::copilot_oauth::DeviceFlowStatus::Pending => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "pending"})),
        ),
        captain_runtime::copilot_oauth::DeviceFlowStatus::Complete { access_token } => {
            state.kernel.store_credential("GITHUB_TOKEN", &access_token);

            let secrets_path = state.kernel.config.home_dir.join("secrets.env");
            if let Err(e) = write_secret_env(&secrets_path, "GITHUB_TOKEN", &access_token) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(
                        serde_json::json!({"status": "error", "error": format!("Failed to save token: {e}")}),
                    ),
                );
            }

            std::env::set_var("GITHUB_TOKEN", access_token.as_str());
            state
                .kernel
                .model_catalog
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .detect_auth();
            COPILOT_FLOWS.remove(&poll_id);

            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "complete"})),
            )
        }
        captain_runtime::copilot_oauth::DeviceFlowStatus::SlowDown { new_interval } => {
            if let Some(mut flow) = COPILOT_FLOWS.get_mut(&poll_id) {
                flow.interval = new_interval;
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "pending", "interval": new_interval})),
            )
        }
        captain_runtime::copilot_oauth::DeviceFlowStatus::Expired => {
            COPILOT_FLOWS.remove(&poll_id);
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "expired"})),
            )
        }
        captain_runtime::copilot_oauth::DeviceFlowStatus::AccessDenied => {
            COPILOT_FLOWS.remove(&poll_id);
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "denied"})),
            )
        }
        captain_runtime::copilot_oauth::DeviceFlowStatus::Error(e) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "error", "error": e})),
        ),
    }
}
