//! Authenticated operator surface for observable tool executions.
//!
//! This module deliberately projects a narrow metadata type instead of
//! serializing the runtime snapshot. Raw input, result previews and managed
//! output paths are never part of this API contract.

use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::Response,
};
use captain_kernel::ToolRunOperatorSurface;
use captain_runtime::{
    tool_run_operator::{
        operator_tail, OperatorToolRun, OPERATOR_MAX_TAIL_LINES as MAX_TAIL_LINES,
    },
    tool_runs::{global_registry, parse_status_filter, ToolRunCancelError, MAX_RUNS},
};
#[cfg(test)]
use captain_runtime::{
    tool_run_operator::{
        OPERATOR_MAX_TAIL_BYTES as MAX_TAIL_BYTES, OPERATOR_WITHHELD_TAIL as WITHHELD_TAIL,
    },
    tool_run_output::ToolRunOutputPage,
    tool_runs::ToolRunStatus,
};
use serde::Deserialize;
use std::sync::Arc;

const DEFAULT_LIST_LIMIT: usize = 50;
const DEFAULT_TAIL_LINES: usize = 80;

#[derive(Debug, Deserialize)]
pub struct ToolRunListQuery {
    status: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct ToolRunTailQuery {
    max_lines: Option<usize>,
}

/// GET /api/tool-runs - List bounded, selective run metadata.
pub async fn list_tool_runs(Query(query): Query<ToolRunListQuery>) -> Response<Body> {
    let limit = match bounded_value(query.limit, DEFAULT_LIST_LIMIT, MAX_RUNS, "limit") {
        Ok(limit) => limit,
        Err(response) => return *response,
    };
    let status = match parse_status_filter(query.status.as_deref()) {
        Ok(status) => status,
        Err(_) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_status",
                "status must be running, completed, failed, cancelled, or interrupted",
            )
        }
    };
    let items = global_registry()
        .list(status, limit)
        .into_iter()
        .map(OperatorToolRun::from)
        .collect::<Vec<_>>();
    json_response(
        StatusCode::OK,
        serde_json::json!({"count": items.len(), "items": items}),
    )
}

/// GET /api/tool-runs/{run_id} - Inspect selective metadata for one run.
pub async fn inspect_tool_run(Path(run_id): Path<String>) -> Response<Body> {
    if !valid_run_id(&run_id) {
        return invalid_run_id();
    }
    match global_registry().snapshot(&run_id) {
        Some(snapshot) => json_response(
            StatusCode::OK,
            serde_json::json!({"run": OperatorToolRun::from(snapshot)}),
        ),
        None => tool_run_not_found(),
    }
}

/// GET /api/tool-runs/{run_id}/tail - Read a bounded, fail-closed output tail.
pub async fn tail_tool_run(
    Path(run_id): Path<String>,
    Query(query): Query<ToolRunTailQuery>,
) -> Response<Body> {
    if !valid_run_id(&run_id) {
        return invalid_run_id();
    }
    let max_lines = match bounded_value(
        query.max_lines,
        DEFAULT_TAIL_LINES,
        MAX_TAIL_LINES,
        "max_lines",
    ) {
        Ok(max_lines) => max_lines,
        Err(response) => return *response,
    };
    let registry = global_registry();
    let Some(snapshot) = registry.snapshot(&run_id) else {
        return tool_run_not_found();
    };
    match registry.tail_output(&run_id, max_lines) {
        Ok(page) => json_response(
            StatusCode::OK,
            serde_json::json!({"tail": operator_tail(&run_id, snapshot.status, page)}),
        ),
        Err(error) => {
            tracing::warn!(run_id, error = %error, "operator tool run tail unavailable");
            api_error(
                StatusCode::CONFLICT,
                "tool_run_output_unavailable",
                "retained tool output is unavailable or failed integrity verification",
            )
        }
    }
}

/// POST /api/tool-runs/{run_id}/cancel - Abort one active cancellable run.
pub async fn cancel_tool_run(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> Response<Body> {
    if !valid_run_id(&run_id) {
        return invalid_run_id();
    }
    match state
        .kernel
        .operator_cancel_tool_run(ToolRunOperatorSurface::Api, &run_id)
    {
        Ok(snapshot) => json_response(
            StatusCode::OK,
            serde_json::json!({
                "status": "cancelled",
                "run": OperatorToolRun::from(snapshot),
            }),
        ),
        Err(ToolRunCancelError::NotFound) => tool_run_not_found(),
        Err(ToolRunCancelError::NotActive { .. }) => api_error(
            StatusCode::CONFLICT,
            "tool_run_not_active",
            "only an active tool run can be cancelled",
        ),
        Err(ToolRunCancelError::NotCancellable) => api_error(
            StatusCode::CONFLICT,
            "tool_run_not_cancellable",
            "this active tool run has no cancellable task handle",
        ),
    }
}

fn bounded_value(
    value: Option<usize>,
    default: usize,
    maximum: usize,
    field: &str,
) -> Result<usize, Box<Response<Body>>> {
    let value = value.unwrap_or(default);
    if (1..=maximum).contains(&value) {
        Ok(value)
    } else {
        Err(Box::new(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_limit",
            &format!("{field} must be between 1 and {maximum}"),
        )))
    }
}

fn valid_run_id(value: &str) -> bool {
    let Some(suffix) = value.strip_prefix("toolrun-") else {
        return false;
    };
    !suffix.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn invalid_run_id() -> Response<Body> {
    api_error(
        StatusCode::BAD_REQUEST,
        "invalid_tool_run_id",
        "tool run id must use Captain's bounded toolrun-* format",
    )
}

fn tool_run_not_found() -> Response<Body> {
    api_error(
        StatusCode::NOT_FOUND,
        "tool_run_not_found",
        "tool run not found",
    )
}

fn json_response(status: StatusCode, value: serde_json::Value) -> Response<Body> {
    let data = serde_json::to_vec(&value)
        .unwrap_or_else(|_| b"{\"error\":\"serialization_failed\"}".to_vec());
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "private, no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(Body::from(data))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn api_error(status: StatusCode, code: &str, message: &str) -> Response<Body> {
    json_response(
        status,
        serde_json::json!({"error": code, "message": message}),
    )
}

#[cfg(test)]
#[path = "tool_run_routes_tests.rs"]
mod tests;
