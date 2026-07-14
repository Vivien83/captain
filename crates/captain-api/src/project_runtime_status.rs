use crate::project_runtime_resume::runtime_declares_active;
use crate::project_runtime_status_safe::{
    safe_last_event, safe_pending_question, safe_resume_pending_reason, safe_runtime_phase,
    safe_runtime_status, safe_tool_request, safe_worker_status,
};
use crate::project_runtime_tool_status::{
    denied_tool_request, pending_tool_request, tool_request_tools_label,
};
use captain_memory::project;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(crate) fn project_runtime_operator_status(
    project: &project::Project,
    runtime: &Value,
    process_running: bool,
) -> Value {
    let status = runtime_string(runtime, "status", "ready").to_lowercase();
    let phase = runtime_string(runtime, "current_phase", "observe");
    let status_view = safe_runtime_status(&status);
    let phase_view = safe_runtime_phase(&phase);
    let pending = pending_questions(runtime);
    let pending_tool_request = pending_tool_request(runtime);
    let denied_tool_request = if status == "blocked" {
        denied_tool_request(runtime, &phase)
    } else {
        None
    };
    let resume_pending = runtime
        .get("resume_pending")
        .map(|value| !value.is_null())
        .unwrap_or(false);
    let raw_resume_pending_reason = runtime_resume_pending_reason(runtime);
    let resume_pending_reason = safe_resume_pending_reason(raw_resume_pending_reason.as_deref());
    let declared_active = runtime_declares_active(runtime);
    let stale_active = declared_active && !process_running;
    let state = operator_state(
        &status,
        process_running,
        stale_active,
        resume_pending,
        !pending.is_empty(),
        pending_tool_request.is_some(),
        denied_tool_request.is_some(),
    );
    let actions = operator_actions(
        project,
        state,
        pending.first(),
        pending_tool_request.as_ref(),
        denied_tool_request.as_ref(),
        resume_pending,
        resume_pending_reason,
        stale_active,
    );
    let pending_tool_request_view = safe_tool_request(pending_tool_request.as_ref());
    let denied_tool_request_view = safe_tool_request(denied_tool_request.as_ref());
    let summary_pending_tool_request = value_ref(&pending_tool_request_view);
    let summary_denied_tool_request = value_ref(&denied_tool_request_view);

    json!({
        "state": state,
        "summary": operator_summary(
            state,
            phase_view,
            pending.len(),
            summary_pending_tool_request,
            summary_denied_tool_request,
            stale_active,
            resume_pending,
            resume_pending_reason
        ),
        "project_id": project.id,
        "project_slug": project.slug,
        "updated_at": project.updated_at,
        "running_in_process": process_running,
        "declared_active": declared_active,
        "resume_pending": resume_pending,
        "resume_pending_reason": resume_pending_reason,
        "status": status_view,
        "phase": phase_view,
        "progress": runtime.get("progress").and_then(|v| v.as_u64()).unwrap_or(0),
        "pending_questions": pending.len(),
        "first_pending_question": safe_pending_question(pending.first()),
        "pending_tool_request": pending_tool_request_view,
        "denied_tool_request": denied_tool_request_view,
        "workers": worker_counts(runtime),
        "last_event": last_runtime_event(runtime),
        "actions": actions,
    })
}

pub(crate) fn project_runtime_needs_operator_attention(status: &Value) -> bool {
    matches!(
        status.get("state").and_then(|v| v.as_str()),
        Some(
            "waiting_for_user"
                | "tool_request_pending"
                | "tool_request_denied"
                | "resume_ready"
                | "stale_active"
                | "blocked"
                | "failed"
        )
    )
}

pub(crate) fn project_runtime_attention_priority(status: &Value) -> u8 {
    match status.get("state").and_then(|v| v.as_str()) {
        Some("waiting_for_user") => 0,
        Some("tool_request_pending") => 1,
        Some("resume_ready") => 2,
        Some("stale_active") => 3,
        Some("tool_request_denied") => 4,
        Some("failed") => 5,
        Some("blocked") => 6,
        _ => 9,
    }
}

pub(crate) fn limit_project_runtime_attention_items(items: &mut Vec<Value>) -> usize {
    items.sort_by(|a, b| {
        project_runtime_attention_priority(a)
            .cmp(&project_runtime_attention_priority(b))
            .then_with(|| {
                b["updated_at"]
                    .as_i64()
                    .unwrap_or(0)
                    .cmp(&a["updated_at"].as_i64().unwrap_or(0))
            })
    });
    let total = items.len();
    items.truncate(8);
    total
}

fn operator_state(
    status: &str,
    process_running: bool,
    stale_active: bool,
    resume_pending: bool,
    has_pending_question: bool,
    has_pending_tool_request: bool,
    has_denied_tool_request: bool,
) -> &'static str {
    if has_pending_question {
        return "waiting_for_user";
    }
    if has_pending_tool_request {
        return "tool_request_pending";
    }
    if resume_pending {
        return "resume_ready";
    }
    if process_running {
        return "running";
    }
    if stale_active {
        return "stale_active";
    }
    if has_denied_tool_request {
        return "tool_request_denied";
    }
    match status {
        "paused" => "paused",
        "blocked" => "blocked",
        "failed" => "failed",
        "done" => "done",
        "running" => "stale_active",
        _ => "ready",
    }
}

#[allow(clippy::too_many_arguments)]
fn operator_summary(
    state: &str,
    phase: &str,
    pending_questions: usize,
    pending_tool_request: Option<&Value>,
    denied_tool_request: Option<&Value>,
    stale_active: bool,
    resume_pending: bool,
    resume_pending_reason: Option<&str>,
) -> String {
    match state {
        "waiting_for_user" => {
            let plural = if pending_questions == 1 { "" } else { "s" };
            format!("{pending_questions} pending answer{plural} blocks phase {phase}.")
        }
        "tool_request_pending" => format!(
            "A project worker requested {} for phase {phase}.",
            tool_request_tools_label(pending_tool_request)
        ),
        "tool_request_denied" => denied_tool_request_summary(phase, denied_tool_request),
        "resume_ready" => resume_ready_summary(phase, resume_pending_reason),
        "running" => format!("The local orchestrator is actively running phase {phase}."),
        "stale_active" if stale_active => {
            format!("The runtime declares active work for phase {phase}, but no local orchestrator is running.")
        }
        "paused" => format!("The project run is paused at phase {phase}."),
        "blocked" => format!("The project run is blocked at phase {phase}."),
        "failed" => format!("The project run failed at phase {phase}."),
        "done" => "The project run is complete.".to_string(),
        _ if resume_pending => format!("Phase {phase} has resume state pending."),
        _ => format!("The project run is ready at phase {phase}."),
    }
}

fn resume_ready_summary(phase: &str, reason: Option<&str>) -> String {
    match reason {
        Some("tool_request_approved") => {
            format!("An approved tool request is stored; phase {phase} is ready to resume.")
        }
        Some("project_ask_answered") => {
            format!("A user answer is stored; phase {phase} is ready to resume.")
        }
        _ => format!("A resume marker is stored; phase {phase} is ready to resume."),
    }
}

fn denied_tool_request_summary(phase: &str, denied_tool_request: Option<&Value>) -> String {
    let tools = repeated_denied_tools_label(denied_tool_request);
    if tool_request_is_denied_repeat(denied_tool_request) {
        return format!(
            "A worker repeated a denied request for {tools} in phase {phase}; Captain kept it denied instead of asking again."
        );
    }
    format!(
        "The operator denied {tools} for phase {phase}; the project needs a different path or manual review."
    )
}

fn repeated_denied_tools_label(request: Option<&Value>) -> String {
    let Some(tools) = request
        .and_then(|request| request.get("repeated_denied_tools"))
        .and_then(|value| value.as_array())
    else {
        return tool_request_tools_label(request);
    };
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool.as_str())
        .filter(|tool| !tool.trim().is_empty())
        .take(4)
        .collect();
    if names.is_empty() {
        tool_request_tools_label(request)
    } else {
        names.join(", ")
    }
}

#[allow(clippy::too_many_arguments)]
fn operator_actions(
    project: &project::Project,
    state: &str,
    first_pending: Option<&Value>,
    pending_tool_request: Option<&Value>,
    denied_tool_request: Option<&Value>,
    resume_pending: bool,
    resume_pending_reason: Option<&str>,
    stale_active: bool,
) -> Value {
    let base = format!("/api/projects/{}/runtime", project.id);
    let mut actions = Vec::new();

    if let Some(question) = first_pending {
        let question_view = safe_pending_question(Some(question));
        actions.push(json!({
            "label": "answer_question",
            "method": "POST",
            "path": format!("{base}/answer"),
            "body_hint": {
                "ask_id": question_view.get("ask_id").cloned().unwrap_or(Value::Null),
                "answer": "..."
            },
            "reason": "A project worker is waiting for the user answer.",
        }));
    }

    if let Some(request) = pending_tool_request {
        let request_view = safe_tool_request(Some(request));
        let reason = request_view
            .get("reason")
            .and_then(|value| value.as_str())
            .unwrap_or("A worker requested additional tools before continuing.")
            .to_string();
        actions.push(json!({
            "label": "respond_tool_request",
            "method": "POST",
            "path": format!("{base}/tool-request"),
            "body_hint": {
                "phase": request_view.get("phase").cloned().unwrap_or(Value::Null),
                "tools": request_view.get("tools").cloned().unwrap_or_else(|| json!([])),
                "decision": "approve|deny",
                "reason": "...",
            },
            "reason": reason,
        }));
    }

    if resume_pending
        || stale_active
        || matches!(state, "paused" | "blocked" | "tool_request_denied")
    {
        actions.push(json!({
            "label": "resume_runtime",
            "method": "POST",
            "path": format!("{base}/resume"),
            "reason": resume_runtime_action_reason(resume_pending_reason, denied_tool_request),
        }));
    }

    if matches!(state, "ready" | "done" | "failed") {
        actions.push(json!({
            "label": "start_runtime",
            "method": "POST",
            "path": format!("{base}/start"),
            "reason": "Start a fresh autonomous project run.",
        }));
    }

    if state == "running" {
        actions.push(json!({
            "label": "pause_runtime",
            "method": "POST",
            "path": format!("{base}/pause"),
            "reason": "Pause autonomous execution without losing runtime context.",
        }));
        actions.push(json!({
            "label": "takeover_runtime",
            "method": "POST",
            "path": format!("{base}/takeover"),
            "reason": "Pause and switch the project back to manual control.",
        }));
    }

    Value::Array(actions)
}

fn resume_runtime_action_reason(
    reason: Option<&str>,
    denied_tool_request: Option<&Value>,
) -> &'static str {
    if tool_request_is_denied_repeat(denied_tool_request) {
        return "Review the repeated denied tool request; do not approve the same tools without new evidence.";
    }
    if denied_tool_request.is_some() {
        return "Continue only after reviewing the denied project tool request.";
    }
    match reason {
        Some("tool_request_approved") => "Continue after an approved project tool request.",
        Some("project_ask_answered") => "Continue after a persisted project answer.",
        _ => "Continue from persisted project runtime state.",
    }
}

fn tool_request_is_denied_repeat(request: Option<&Value>) -> bool {
    request
        .and_then(|request| request.get("repeat_of_denied_tool_request"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn runtime_resume_pending_reason(runtime: &Value) -> Option<String> {
    runtime
        .pointer("/resume_pending/reason")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .map(ToString::to_string)
}

fn runtime_string(runtime: &Value, key: &str, fallback: &str) -> String {
    runtime
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn value_ref(value: &Value) -> Option<&Value> {
    if value.is_null() {
        None
    } else {
        Some(value)
    }
}

fn pending_questions(runtime: &Value) -> Vec<Value> {
    runtime
        .get("user_questions")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    item.get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("pending")
                        .eq_ignore_ascii_case("pending")
                        && item
                            .get("ask_id")
                            .and_then(|v| v.as_str())
                            .map(|s| !s.trim().is_empty())
                            .unwrap_or(false)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn worker_counts(runtime: &Value) -> Value {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    if let Some(workers) = runtime.get("workers").and_then(|v| v.as_array()) {
        for worker in workers {
            let status = safe_worker_status(worker.get("status")).to_string();
            *counts.entry(status).or_insert(0) += 1;
        }
    }
    json!({
        "total": counts.values().sum::<usize>(),
        "by_status": counts,
    })
}

fn last_runtime_event(runtime: &Value) -> Value {
    safe_last_event(
        runtime
            .get("timeline")
            .and_then(|v| v.as_array())
            .and_then(|events| events.last()),
    )
}

#[cfg(test)]
#[path = "project_runtime_status_tests.rs"]
mod project_runtime_status_tests;
