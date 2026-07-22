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
use captain_types::workflow_learning::ProposalCardAction;
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

fn memory_writes_metrics(journal: &memory_writer::JournalHealth) -> serde_json::Value {
    serde_json::json!({
        "total": journal.total,
        "synced": journal.synced,
        "pending": journal.pending,
        "error": journal.error,
        "retracted": journal.retracted,
        "oldest_unsynced_at": journal.oldest_unsynced_at,
        "next_retry_at": journal.next_retry_at,
        "max_sync_attempts": journal.max_sync_attempts,
        "last_sync_error": journal.last_sync_error,
        "continuity": "local_journal_available",
        "recovery": if journal.error > 0 || journal.pending > 0 { "automatic_retry_active" } else { "in_sync" },
    })
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
    let journal = match memory_writer::journal_health(&guard) {
        Ok(health) => health,
        Err(error) => return server_error(format!("memory journal health failed: {error}")),
    };
    let review_pending = learning_review::list_pending(&guard, 10_000)
        .map(|v| v.len() as i64)
        .unwrap_or(0);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "memory_writes": memory_writes_metrics(&journal),
            "review_queue_pending": review_pending,
            "learning_mode": format!("{:?}", state.kernel.config.learning.mode).to_lowercase(),
            "learning_enabled": state.kernel.config.learning.enabled,
        })),
    )
}

/// GET /api/learning/workflows — shared durable Skill/CapSpec/Automation view.
pub async fn list_workflows(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let limit = parse_limit(&params, 100, 500);
    match state.kernel.workflow_learning_list(limit) {
        Ok(list) => (
            StatusCode::OK,
            Json(serde_json::to_value(list).unwrap_or_else(|error| {
                serde_json::json!({ "error": format!("workflow projection encoding failed: {error}") })
            })),
        ),
        Err(error) => server_error(error),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDecisionBody {
    pub decision_version: u64,
    pub action: ProposalCardAction,
    #[serde(default)]
    pub surface: Option<String>,
}

/// POST /api/learning/workflows/{token}/decide — exact CAS operator action.
pub async fn decide_workflow(
    State(state): State<Arc<AppState>>,
    Path(operator_token): Path<String>,
    Json(body): Json<WorkflowDecisionBody>,
) -> impl IntoResponse {
    let actor = match workflow_surface_actor(body.surface.as_deref()) {
        Ok(actor) => actor,
        Err(error) => return bad_request(error),
    };
    match state.kernel.workflow_learning_resolve_surface_action(
        &operator_token,
        body.decision_version,
        body.action,
        actor,
    ) {
        Ok(resolution) => (
            StatusCode::OK,
            Json(serde_json::to_value(resolution).unwrap_or_else(|error| {
                serde_json::json!({ "error": format!("workflow decision encoding failed: {error}") })
            })),
        ),
        Err(error) => {
            let normalized = error.to_ascii_lowercase();
            if normalized.contains("unknown or expired") || normalized.contains("not found") {
                (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": error })),
                )
            } else if normalized.contains("stale")
                || normalized.contains("unavailable while")
                || normalized.contains("conflict")
            {
                (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({ "error": error })),
                )
            } else {
                bad_request(error)
            }
        }
    }
}

fn workflow_surface_actor(surface: Option<&str>) -> Result<&'static str, String> {
    match surface
        .unwrap_or("api")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "api" => Ok("api:authenticated"),
        "web" => Ok("web:authenticated"),
        "desktop" => Ok("desktop:authenticated"),
        "tui" => Ok("tui:authenticated"),
        _ => Err("surface must be api, web, desktop, or tui".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_metrics_expose_durable_recovery_state() {
        let payload = memory_writes_metrics(&memory_writer::JournalHealth {
            total: 12,
            synced: 9,
            pending: 2,
            error: 1,
            retracted: 3,
            oldest_unsynced_at: Some(1_000),
            next_retry_at: Some(2_000),
            max_sync_attempts: 7,
            last_sync_error: Some("backend offline".into()),
        });
        assert_eq!(payload["continuity"], "local_journal_available");
        assert_eq!(payload["recovery"], "automatic_retry_active");
        assert_eq!(payload["retracted"], 3);
        assert_eq!(payload["max_sync_attempts"], 7);
        assert_eq!(payload["last_sync_error"], "backend offline");
    }

    #[test]
    fn memory_metrics_report_in_sync_only_with_empty_backlog() {
        let payload = memory_writes_metrics(&memory_writer::JournalHealth {
            total: 4,
            synced: 4,
            ..Default::default()
        });
        assert_eq!(payload["recovery"], "in_sync");
    }

    #[test]
    fn workflow_decisions_only_accept_known_authenticated_surfaces() {
        assert_eq!(
            workflow_surface_actor(Some("web")).unwrap(),
            "web:authenticated"
        );
        assert_eq!(
            workflow_surface_actor(Some("tui")).unwrap(),
            "tui:authenticated"
        );
        assert!(workflow_surface_actor(Some("telegram")).is_err());
    }

    #[test]
    fn web_learning_consumes_the_durable_workflow_contract() {
        let api_source = include_str!("../static/js/app/api.js");
        let learning_source = include_str!("../static/js/app/views/Learning.js");
        assert!(api_source.contains("/api/learning/workflows"));
        assert!(learning_source.contains("workflowLearning"));
        assert!(!learning_source.contains("/api/skills/proposals"));
    }
}
