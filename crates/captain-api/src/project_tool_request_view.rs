use crate::project_runtime_view as runtime_view;
use axum::{http::StatusCode, Json};
use captain_memory::project;
use serde_json::Value;

const TOOL_LIMIT: usize = 80;

pub(crate) fn project_tool_request_success_view(
    project: &project::Project,
    operator_status: Value,
    phase: &str,
    decision: &str,
    tools: &[String],
    runtime_resume_pending: bool,
) -> Value {
    serde_json::json!({
        "ok": true,
        "project": runtime_view::project_runtime_project_view(project),
        "operator_status": project_tool_request_operator_status_view(operator_status),
        "phase": safe_project_tool_request_phase(phase),
        "decision": decision,
        "tools": safe_project_tool_request_tools(tools),
        "runtime_resume_pending": runtime_resume_pending,
    })
}

pub(crate) fn safe_project_tool_request_phase(phase: &str) -> String {
    if matches!(
        phase,
        "observe" | "think" | "plan" | "build" | "execute" | "verify" | "learn"
    ) {
        phase.to_string()
    } else {
        "unknown".to_string()
    }
}

pub(crate) fn safe_project_tool_request_tools(tools: &[String]) -> Vec<String> {
    tools
        .iter()
        .map(|tool| safe_project_tool_request_tool(tool))
        .filter(|tool| !tool.is_empty())
        .take(12)
        .collect()
}

pub(crate) fn safe_project_tool_request_tool(tool: &str) -> String {
    tool.trim()
        .trim_matches('`')
        .chars()
        .filter(|ch| !ch.is_control())
        .take(TOOL_LIMIT)
        .collect()
}

pub(crate) fn safe_project_tool_request_storage_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("not found") {
        return "Project could not be found".to_string();
    }
    "Project tool request state could not be saved; verify project storage availability".to_string()
}

pub(crate) fn safe_project_tool_request_runtime_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("no pending tool request") {
        return "No pending project tool request found".to_string();
    }
    "Project tool request state could not be updated".to_string()
}

pub(crate) fn project_tool_request_json_error(
    status: StatusCode,
    code: impl Into<String>,
    message: impl Into<String>,
) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(serde_json::json!({
            "ok": false,
            "error": code.into(),
            "message": message.into(),
        })),
    )
}

fn project_tool_request_operator_status_view(mut status: Value) -> Value {
    strip_tool_request_free_text(status.get_mut("pending_tool_request"));
    strip_tool_request_free_text(status.get_mut("denied_tool_request"));
    status
}

fn strip_tool_request_free_text(value: Option<&mut Value>) {
    let Some(Value::Object(request)) = value else {
        return;
    };
    for key in [
        "reason",
        "decision_reason",
        "previous_decision_reason",
        "source",
    ] {
        request.remove(key);
    }
}

#[cfg(test)]
#[path = "project_tool_request_view_tests.rs"]
mod project_tool_request_view_tests;
