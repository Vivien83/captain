use crate::project_runtime_view_window::{
    is_actionable_worker, is_pending_question, select_priority_recent_items,
};
use captain_memory::project;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const ID_LIMIT: usize = 120;
const TEXT_LIMIT: usize = 500;
const DETAIL_LIMIT: usize = 900;
const TOOL_LIMIT: usize = 80;
const LIST_LIMIT: usize = 12;
const RUNTIME_TIMELINE_VIEW_LIMIT: usize = 100;
const RUNTIME_QUESTION_VIEW_LIMIT: usize = 20;
const RUNTIME_WORKER_VIEW_LIMIT: usize = 50;
const PROJECT_PHASES: &[&str] = &[
    "observe", "think", "plan", "build", "execute", "verify", "learn",
];

pub(crate) fn project_runtime_project_view(project: &project::Project) -> Value {
    json!({
        "id": project.id,
        "name": project.name,
        "slug": project.slug,
        "goal": bounded_text(&project.goal, TEXT_LIMIT),
        "status": project.status,
        "deadline": project.deadline,
        "created_at": project.created_at,
        "updated_at": project.updated_at,
    })
}

pub(crate) fn project_runtime_view(runtime: &Value) -> Value {
    json!({
        "protocol": safe_scalar(runtime.get("protocol"), ID_LIMIT),
        "generation": safe_scalar(runtime.get("generation"), ID_LIMIT),
        "status": safe_runtime_status_value(runtime.get("status")),
        "current_phase": safe_phase_value(runtime.get("current_phase")),
        "progress": runtime.get("progress").and_then(Value::as_u64).unwrap_or(0).min(100),
        "control": safe_control(runtime.get("control")),
        "orchestrator": safe_orchestrator(runtime.get("orchestrator")),
        "manager_agent": safe_manager_agent(runtime.get("manager_agent")),
        "parallelism": safe_parallelism(runtime.get("parallelism")),
        "resume_pending": safe_resume_pending(runtime.get("resume_pending")),
        "user_questions": safe_questions(runtime.get("user_questions")),
        "workers": safe_workers(runtime.get("workers")),
        "worker_results": safe_worker_results(runtime.get("worker_results")),
        "timeline": safe_runtime_event_array(runtime.get("timeline")),
    })
}

pub(crate) fn project_lifecycle_view(lifecycle: &Value) -> Value {
    json!({
        "protocol": safe_scalar_or(lifecycle.get("protocol"), ID_LIMIT, "captain.project_lifecycle.v1"),
        "required": safe_scalar(lifecycle.get("required"), ID_LIMIT),
        "current_phase": safe_phase_value(lifecycle.get("current_phase")),
        "phases": safe_phase_list(lifecycle.get("phases")),
    })
}

pub(crate) fn project_source_view(source: &Value) -> Value {
    json!({
        "type": safe_scalar_or(source.get("type"), ID_LIMIT, "legacy"),
        "full_name": safe_scalar(source.get("full_name"), TEXT_LIMIT),
        "repository": safe_scalar(source.get("repository"), TEXT_LIMIT),
        "branch": safe_scalar(source.get("branch"), ID_LIMIT),
        "repo_id": safe_scalar(source.get("repo_id").or_else(|| source.get("id")), ID_LIMIT),
    })
}

pub(crate) fn project_workspace_view(workspace: &Value) -> Value {
    json!({
        "authorized": safe_scalar(workspace.get("authorized"), ID_LIMIT),
        "authorization_error": safe_scalar(workspace.get("authorization_error"), DETAIL_LIMIT),
        "platform": safe_scalar(workspace.get("platform"), ID_LIMIT),
    })
}

pub(crate) fn safe_runtime_events(events: Vec<Value>) -> Vec<Value> {
    events.iter().map(safe_runtime_event).collect()
}

fn safe_runtime_event_array(value: Option<&Value>) -> Value {
    let Some(events) = value.and_then(Value::as_array) else {
        return Value::Array(Vec::new());
    };
    let start = events.len().saturating_sub(RUNTIME_TIMELINE_VIEW_LIMIT);
    Value::Array(events[start..].iter().map(safe_runtime_event).collect())
}

fn safe_runtime_event(event: &Value) -> Value {
    json!({
        "id": safe_scalar(event.get("id"), ID_LIMIT),
        "kind": safe_scalar(event.get("kind"), ID_LIMIT),
        "title": safe_scalar(event.get("title"), TEXT_LIMIT),
        "detail": safe_scalar(event.get("detail"), DETAIL_LIMIT),
        "actor": safe_scalar(event.get("actor"), ID_LIMIT),
        "phase": safe_phase_value(event.get("phase")),
        "status": safe_worker_status_value(event.get("status")),
        "ts": safe_scalar(event.get("ts"), ID_LIMIT),
    })
}

fn safe_questions(value: Option<&Value>) -> Value {
    let Some(questions) = value.and_then(Value::as_array) else {
        return Value::Array(Vec::new());
    };
    Value::Array(
        select_priority_recent_items(questions, RUNTIME_QUESTION_VIEW_LIMIT, is_pending_question)
            .into_iter()
            .map(safe_question)
            .collect(),
    )
}

fn safe_question(question: &Value) -> Value {
    json!({
        "ask_id": safe_scalar(question.get("ask_id"), ID_LIMIT),
        "phase": safe_phase_value(question.get("phase")),
        "worker_role": safe_scalar(
            question.get("worker_role").or_else(|| question.get("role")),
            ID_LIMIT
        ),
        "status": safe_scalar(question.get("status"), ID_LIMIT),
        "delivery": safe_scalar(question.get("delivery"), ID_LIMIT),
        "question": safe_scalar(question.get("question"), TEXT_LIMIT),
        "options": safe_string_list(question.get("options"), LIST_LIMIT, TEXT_LIMIT),
        "created_at": safe_scalar(question.get("created_at"), ID_LIMIT),
        "updated_at": safe_scalar(question.get("updated_at"), ID_LIMIT),
        "answered_at": safe_scalar(question.get("answered_at"), ID_LIMIT),
        "closed_at": safe_scalar(question.get("closed_at"), ID_LIMIT),
    })
}

fn safe_workers(value: Option<&Value>) -> Value {
    let Some(workers) = value.and_then(Value::as_array) else {
        return Value::Array(Vec::new());
    };
    Value::Array(
        select_priority_recent_items(workers, RUNTIME_WORKER_VIEW_LIMIT, is_actionable_worker)
            .into_iter()
            .map(safe_worker)
            .collect(),
    )
}

fn safe_worker(worker: &Value) -> Value {
    json!({
        "id": safe_scalar(worker.get("id"), ID_LIMIT),
        "role": safe_scalar(worker.get("role"), ID_LIMIT),
        "phase": safe_phase_value(worker.get("phase")),
        "status": safe_worker_status_value(worker.get("status")),
        "mode": safe_scalar(worker.get("mode"), ID_LIMIT),
        "agent_id": safe_scalar(worker.get("agent_id"), ID_LIMIT),
        "authorized_tools": safe_string_list(worker.get("authorized_tools"), LIST_LIMIT, TOOL_LIMIT),
        "iterations": safe_scalar(worker.get("iterations"), ID_LIMIT),
        "tool_calls": safe_scalar(worker.get("tool_calls"), ID_LIMIT),
        "cost_usd": safe_scalar(worker.get("cost_usd"), ID_LIMIT),
        "started_at": safe_scalar(worker.get("started_at"), ID_LIMIT),
        "completed_at": safe_scalar(worker.get("completed_at"), ID_LIMIT),
        "stopped_at": safe_scalar(worker.get("stopped_at"), ID_LIMIT),
        "cleanup_status": safe_worker_status_value(worker.get("cleanup_status")),
        "recovered_from_stale_run": safe_scalar(worker.get("recovered_from_stale_run"), ID_LIMIT),
        "summary": safe_scalar(worker.get("summary"), DETAIL_LIMIT),
        "tool_request": safe_tool_request(
            worker.get("phase").and_then(Value::as_str).unwrap_or("unknown"),
            worker.get("tool_request")
        ),
    })
}

fn safe_worker_results(value: Option<&Value>) -> Value {
    let Some(results) = value.and_then(Value::as_object) else {
        return json!({});
    };
    let mut safe = BTreeMap::new();
    for phase in PROJECT_PHASES {
        if let Some(result) = results.get(*phase) {
            safe.insert((*phase).to_string(), safe_worker_result(phase, result));
        }
    }
    json!(safe)
}

fn safe_worker_result(phase: &str, result: &Value) -> Value {
    json!({
        "status": safe_worker_status_value(result.get("status")),
        "blocked": safe_scalar(result.get("blocked"), ID_LIMIT),
        "summary": safe_scalar(result.get("summary"), DETAIL_LIMIT),
        "error": safe_scalar(result.get("error"), DETAIL_LIMIT),
        "retry_after_denied_tool_request": safe_scalar(
            result.get("retry_after_denied_tool_request"),
            ID_LIMIT
        ),
        "tool_request": safe_tool_request(phase, result.get("tool_request")),
    })
}

fn safe_tool_request(phase: &str, request: Option<&Value>) -> Value {
    let Some(request) = request.filter(|value| value.is_object()) else {
        return Value::Null;
    };
    json!({
        "phase": safe_runtime_phase(phase),
        "tools": safe_string_list(request.get("tools"), LIST_LIMIT, TOOL_LIMIT),
        "reason": safe_scalar(request.get("reason"), TEXT_LIMIT),
        "status": safe_scalar(request.get("status"), ID_LIMIT),
        "decision_reason": safe_scalar(request.get("decision_reason"), TEXT_LIMIT),
        "decided_at": safe_scalar(request.get("decided_at"), ID_LIMIT),
        "decided_by": safe_scalar(request.get("decided_by"), ID_LIMIT),
        "repeat_of_denied_tool_request": safe_scalar(
            request.get("repeat_of_denied_tool_request"),
            ID_LIMIT
        ),
        "repeated_denied_tools": safe_optional_string_list(
            request.get("repeated_denied_tools"),
            LIST_LIMIT,
            TOOL_LIMIT
        ),
        "previous_decision_reason": safe_scalar(
            request.get("previous_decision_reason"),
            TEXT_LIMIT
        ),
    })
}

fn safe_control(value: Option<&Value>) -> Value {
    let Some(control) = value.filter(|value| value.is_object()) else {
        return json!({});
    };
    json!({
        "paused": safe_scalar(control.get("paused"), ID_LIMIT),
        "takeover": safe_scalar(control.get("takeover"), ID_LIMIT),
    })
}

fn safe_orchestrator(value: Option<&Value>) -> Value {
    let Some(orchestrator) = value.filter(|value| value.is_object()) else {
        return json!({});
    };
    json!({
        "active": safe_scalar(orchestrator.get("active"), ID_LIMIT),
    })
}

fn safe_manager_agent(value: Option<&Value>) -> Value {
    let Some(agent) = value.filter(|value| value.is_object()) else {
        return json!({});
    };
    json!({
        "name": safe_scalar(agent.get("name"), ID_LIMIT),
        "model": safe_scalar(agent.get("model"), ID_LIMIT),
    })
}

fn safe_parallelism(value: Option<&Value>) -> Value {
    let Some(parallelism) = value.filter(|value| value.is_object()) else {
        return json!({});
    };
    json!({
        "running": safe_scalar(parallelism.get("running"), ID_LIMIT),
        "max_parallel_agents": safe_scalar(parallelism.get("max_parallel_agents"), ID_LIMIT),
    })
}

fn safe_resume_pending(value: Option<&Value>) -> Value {
    let Some(pending) = value.filter(|value| value.is_object()) else {
        return Value::Null;
    };
    json!({
        "reason": safe_resume_pending_reason(pending.get("reason")),
        "phase": safe_phase_value(pending.get("phase")),
    })
}

fn safe_runtime_status_value(value: Option<&Value>) -> Value {
    Value::String(safe_runtime_status(value.and_then(Value::as_str).unwrap_or("ready")).to_string())
}

fn safe_worker_status_value(value: Option<&Value>) -> Value {
    Value::String(safe_worker_status(value).to_string())
}

fn safe_phase_value(value: Option<&Value>) -> Value {
    Value::String(
        safe_runtime_phase(value.and_then(Value::as_str).unwrap_or("unknown")).to_string(),
    )
}

fn safe_resume_pending_reason(value: Option<&Value>) -> Value {
    match value.and_then(Value::as_str) {
        Some("tool_request_approved") => json!("tool_request_approved"),
        Some("project_ask_answered") => json!("project_ask_answered"),
        _ => Value::Null,
    }
}

fn safe_scalar(value: Option<&Value>, limit: usize) -> Value {
    match value {
        Some(Value::String(text)) => {
            let text = bounded_text(text, limit);
            if text.is_empty() {
                Value::Null
            } else {
                Value::String(text)
            }
        }
        Some(Value::Number(number)) => Value::Number(number.clone()),
        Some(Value::Bool(flag)) => Value::Bool(*flag),
        _ => Value::Null,
    }
}

fn safe_string_list(value: Option<&Value>, limit: usize, item_limit: usize) -> Value {
    let Some(items) = value.and_then(Value::as_array) else {
        return Value::Array(Vec::new());
    };
    Value::Array(
        items
            .iter()
            .filter_map(Value::as_str)
            .map(|item| bounded_text(item, item_limit))
            .filter(|item| !item.is_empty())
            .take(limit)
            .map(Value::String)
            .collect(),
    )
}

fn safe_optional_string_list(value: Option<&Value>, limit: usize, item_limit: usize) -> Value {
    if value.and_then(Value::as_array).is_none() {
        return Value::Null;
    }
    safe_string_list(value, limit, item_limit)
}

fn safe_phase_list(value: Option<&Value>) -> Value {
    let phases = value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|phase| {
                    let phase = safe_runtime_phase(phase);
                    (phase != "unknown").then_some(Value::String(phase.to_string()))
                })
                .take(LIST_LIMIT)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if phases.is_empty() {
        return Value::Array(
            PROJECT_PHASES
                .iter()
                .map(|phase| Value::String((*phase).to_string()))
                .collect(),
        );
    }
    Value::Array(phases)
}

fn safe_scalar_or(value: Option<&Value>, limit: usize, fallback: &str) -> Value {
    let safe = safe_scalar(value, limit);
    if safe.is_null() {
        Value::String(fallback.to_string())
    } else {
        safe
    }
}

fn safe_runtime_status(status: &str) -> &'static str {
    match status {
        "paused" => "paused",
        "blocked" => "blocked",
        "failed" => "failed",
        "done" => "done",
        "running" => "running",
        "ready" => "ready",
        _ => "ready",
    }
}

fn safe_runtime_phase(phase: &str) -> &'static str {
    match phase {
        "observe" => "observe",
        "think" => "think",
        "plan" => "plan",
        "build" => "build",
        "execute" => "execute",
        "verify" => "verify",
        "learn" => "learn",
        _ => "unknown",
    }
}

fn safe_worker_status(status: Option<&Value>) -> &'static str {
    match status.and_then(Value::as_str).unwrap_or("planned") {
        "planned" => "planned",
        "ready" => "ready",
        "running" => "running",
        "done" => "done",
        "blocked" => "blocked",
        "failed" => "failed",
        "paused" => "paused",
        "cancelled" => "cancelled",
        "skipped" => "skipped",
        "waiting" => "waiting",
        "cleaning" => "cleaning",
        "cleaned" => "cleaned",
        _ => "other",
    }
}

fn bounded_text(value: &str, limit: usize) -> String {
    let cleaned = value
        .trim()
        .chars()
        .filter(|ch| !ch.is_control() || matches!(*ch, '\n' | '\t'))
        .collect::<String>();
    if cleaned.chars().count() <= limit {
        return cleaned;
    }
    let keep = limit.saturating_sub(3);
    let mut truncated = cleaned.chars().take(keep).collect::<String>();
    truncated.push_str("...");
    truncated
}

#[cfg(test)]
#[path = "project_runtime_view_tests.rs"]
mod project_runtime_view_tests;
