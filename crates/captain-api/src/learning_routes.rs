//! REST surface for the v3.12 LearningEngine API.
//!
//! Four endpoints:
//! - GET /api/learning/committed — recent memory_writes (source LIKE 'learning.%')
//! - GET /api/learning/review — pending approval queue
//! - POST /api/learning/review/{id}/decide — approve or deny a pending item
//! - GET /api/learning/metrics — aggregate counts for the sidebar

use crate::routes::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_memory::{learning_review, memory_writer};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

fn server_error(msg: String) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": msg })),
    )
}

fn bad_request(msg: String) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": msg })),
    )
}

fn parse_limit(params: &HashMap<String, String>, default: usize, cap: usize) -> usize {
    params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
        .min(cap)
}

pub async fn list_committed(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let limit = parse_limit(&params, 50, 500);
    let conn = state.kernel.memory.usage_conn();
    let rows = {
        let guard = match conn.lock() {
            Ok(g) => g,
            Err(e) => return server_error(format!("sqlite poisoned: {e}")),
        };
        match memory_writer::list_recent(&guard, Some("learning."), limit) {
            Ok(r) => r,
            Err(e) => return server_error(e.to_string()),
        }
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({ "committed": rows })),
    )
}

pub async fn list_review(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let limit = parse_limit(&params, 50, 500);
    let conn = state.kernel.memory.usage_conn();
    let rows = {
        let guard = match conn.lock() {
            Ok(g) => g,
            Err(e) => return server_error(format!("sqlite poisoned: {e}")),
        };
        match learning_review::list_pending(&guard, limit) {
            Ok(r) => r,
            Err(e) => return server_error(e.to_string()),
        }
    };
    (StatusCode::OK, Json(serde_json::json!({ "pending": rows })))
}

#[derive(Deserialize)]
pub struct DecideBody {
    pub approve: bool,
    #[serde(default)]
    pub decided_by: Option<String>,
}

pub async fn decide_review(
    State(state): State<Arc<AppState>>,
    Path(review_id): Path<String>,
    Json(body): Json<DecideBody>,
) -> impl IntoResponse {
    use captain_runtime::kernel_handle::KernelHandle;
    let kh: Arc<dyn KernelHandle> = Arc::clone(&state.kernel) as Arc<dyn KernelHandle>;
    match kh
        .learning_review_decide(&review_id, body.approve, body.decided_by.as_deref())
        .await
    {
        Ok(v) => (StatusCode::OK, Json(v)),
        Err(e) => {
            if e.to_lowercase().contains("not found") {
                (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": e })),
                )
            } else if e.to_lowercase().contains("already decided") {
                (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({ "error": e })),
                )
            } else {
                bad_request(e)
            }
        }
    }
}

pub async fn metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let conn = state.kernel.memory.usage_conn();
    let guard = match conn.lock() {
        Ok(g) => g,
        Err(e) => return server_error(format!("sqlite poisoned: {e}")),
    };
    let total_synced =
        memory_writer::count_by_status(&guard, memory_writer::SyncStatus::Synced).unwrap_or(0);
    let total_pending =
        memory_writer::count_by_status(&guard, memory_writer::SyncStatus::Pending).unwrap_or(0);
    let total_error =
        memory_writer::count_by_status(&guard, memory_writer::SyncStatus::Error).unwrap_or(0);
    let review_pending = learning_review::list_pending(&guard, 10_000)
        .map(|v| v.len() as i64)
        .unwrap_or(0);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "memory_writes": {
                "synced": total_synced,
                "pending": total_pending,
                "error": total_error,
            },
            "review_queue_pending": review_pending,
            "learning_mode": format!("{:?}", state.kernel.config.learning.mode).to_lowercase(),
            "learning_enabled": state.kernel.config.learning.enabled,
        })),
    )
}
