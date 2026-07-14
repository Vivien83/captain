use super::status_project_attention_safe::{
    safe_last_event, safe_pending_question, safe_resume_pending_reason, safe_runtime_phase,
    safe_runtime_status, safe_tool_request, safe_worker_status,
};
use std::collections::BTreeMap;

pub(super) fn project_attention_from_metadata(
    project: &captain_memory::project::Project,
) -> Option<serde_json::Value> {
    let runtime = project
        .metadata
        .get("runtime")
        .filter(|value| value.is_object())?;
    let pending_questions = runtime_pending_questions(runtime);
    let first_pending_question = runtime_first_pending_question(runtime);
    let pending_tool_request = runtime_pending_tool_request(runtime);
    let raw_status = runtime["status"]
        .as_str()
        .unwrap_or("ready")
        .to_ascii_lowercase();
    let status = safe_runtime_status(&raw_status);
    let raw_phase = runtime["current_phase"].as_str().unwrap_or("observe");
    let phase = safe_runtime_phase(raw_phase);
    let denied_tool_request = if raw_status == "blocked" {
        runtime_denied_tool_request(runtime, raw_phase)
    } else {
        None
    };
    let resume_pending = runtime
        .get("resume_pending")
        .map(|value| !value.is_null())
        .unwrap_or(false);
    let raw_resume_pending_reason = runtime_resume_pending_reason(runtime);
    let resume_pending_reason = safe_resume_pending_reason(raw_resume_pending_reason.as_deref());
    let declared_active = raw_status == "running"
        || runtime
            .pointer("/orchestrator/active")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
    let state = if pending_questions > 0 {
        "waiting_for_user"
    } else if pending_tool_request.is_some() {
        "tool_request_pending"
    } else if resume_pending {
        "resume_ready"
    } else if declared_active {
        "stale_active"
    } else if denied_tool_request.is_some() {
        "tool_request_denied"
    } else if raw_status == "blocked" {
        "blocked"
    } else if raw_status == "failed" {
        "failed"
    } else {
        return None;
    };
    let actions = project_attention_actions(
        &project.id,
        state,
        first_pending_question.as_ref(),
        pending_tool_request.as_ref(),
        denied_tool_request.as_ref(),
        resume_pending,
        resume_pending_reason,
    );
    Some(serde_json::json!({
        "project_id": project.id,
        "project_slug": project.slug,
        "state": state,
        "status": status,
        "phase": phase,
        "summary": project_attention_summary(
            state,
            phase,
            pending_questions,
            pending_tool_request.as_ref(),
            denied_tool_request.as_ref(),
            resume_pending_reason
        ),
        "pending_questions": pending_questions,
        "first_pending_question": first_pending_question.unwrap_or(serde_json::Value::Null),
        "pending_tool_request": pending_tool_request.unwrap_or(serde_json::Value::Null),
        "denied_tool_request": denied_tool_request.unwrap_or(serde_json::Value::Null),
        "resume_pending_reason": resume_pending_reason,
        "progress": runtime.get("progress").and_then(|value| value.as_u64()).unwrap_or(0),
        "workers": worker_counts(runtime),
        "last_event": runtime_last_event(runtime),
        "actions": actions,
        "updated_at": project.updated_at,
    }))
}

fn runtime_pending_tool_request(runtime: &serde_json::Value) -> Option<serde_json::Value> {
    if let Some(results) = runtime["worker_results"].as_object() {
        for (phase, result) in results {
            if let Some(request) = result.get("tool_request") {
                if tool_request_is_pending(request) {
                    return Some(simple_tool_request(phase, request));
                }
            }
        }
    }
    for worker in runtime["workers"].as_array().into_iter().flatten() {
        if let Some(request) = worker.get("tool_request") {
            if tool_request_is_pending(request) {
                let phase = worker["phase"].as_str().unwrap_or("unknown");
                return Some(simple_tool_request(phase, request));
            }
        }
    }
    None
}

fn runtime_denied_tool_request(
    runtime: &serde_json::Value,
    current_phase: &str,
) -> Option<serde_json::Value> {
    runtime_tool_request_with_status(runtime, Some(current_phase), "denied")
        .or_else(|| runtime_tool_request_with_status(runtime, None, "denied"))
}

fn runtime_tool_request_with_status(
    runtime: &serde_json::Value,
    phase_filter: Option<&str>,
    wanted: &str,
) -> Option<serde_json::Value> {
    if let Some(results) = runtime["worker_results"].as_object() {
        for (phase, result) in results {
            if phase_filter.map(|filter| filter != phase).unwrap_or(false) {
                continue;
            }
            if let Some(request) = result.get("tool_request") {
                if tool_request_has_status(request, wanted) {
                    return Some(simple_tool_request(phase, request));
                }
            }
        }
    }
    for worker in runtime["workers"].as_array().into_iter().flatten() {
        let phase = worker["phase"].as_str().unwrap_or("unknown");
        if phase_filter.map(|filter| filter != phase).unwrap_or(false) {
            continue;
        }
        if let Some(request) = worker.get("tool_request") {
            if tool_request_has_status(request, wanted) {
                return Some(simple_tool_request(phase, request));
            }
        }
    }
    None
}

fn tool_request_is_pending(request: &serde_json::Value) -> bool {
    request["status"]
        .as_str()
        .map(|status| {
            matches!(
                status.to_ascii_lowercase().as_str(),
                "pending" | "pending_captain_decision" | "pending_operator" | "open"
            )
        })
        .unwrap_or(true)
}

fn tool_request_has_status(request: &serde_json::Value, wanted: &str) -> bool {
    request["status"]
        .as_str()
        .map(|status| status.eq_ignore_ascii_case(wanted))
        .unwrap_or(false)
}

fn simple_tool_request(phase: &str, request: &serde_json::Value) -> serde_json::Value {
    safe_tool_request(phase, request)
}

fn runtime_pending_questions(runtime: &serde_json::Value) -> usize {
    runtime["user_questions"]
        .as_array()
        .map(|questions| {
            questions
                .iter()
                .filter(|question| question_is_pending_with_id(question))
                .count()
        })
        .unwrap_or(0)
}

fn runtime_first_pending_question(runtime: &serde_json::Value) -> Option<serde_json::Value> {
    runtime["user_questions"]
        .as_array()?
        .iter()
        .find(|question| question_is_pending_with_id(question))
        .map(|question| safe_pending_question(Some(question)))
}

fn question_is_pending_with_id(question: &serde_json::Value) -> bool {
    question["status"]
        .as_str()
        .unwrap_or("pending")
        .eq_ignore_ascii_case("pending")
        && question["ask_id"]
            .as_str()
            .map(|ask_id| !ask_id.trim().is_empty())
            .unwrap_or(false)
}

fn runtime_resume_pending_reason(runtime: &serde_json::Value) -> Option<String> {
    runtime
        .pointer("/resume_pending/reason")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .map(ToString::to_string)
}

fn project_attention_summary(
    state: &str,
    phase: &str,
    pending_questions: usize,
    pending_tool_request: Option<&serde_json::Value>,
    denied_tool_request: Option<&serde_json::Value>,
    resume_pending_reason: Option<&str>,
) -> String {
    match state {
        "waiting_for_user" => format!("{pending_questions} pending answer(s) block {phase}."),
        "tool_request_pending" => {
            let tools = pending_tool_request
                .and_then(|request| request["tools"].as_array())
                .map(|tools| {
                    tools
                        .iter()
                        .filter_map(|tool| tool.as_str())
                        .take(4)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|tools| !tools.is_empty())
                .unwrap_or_else(|| "additional tools".to_string());
            format!("Worker requests {tools} before {phase} can continue.")
        }
        "tool_request_denied" => {
            let tools = repeated_denied_tools_label(denied_tool_request);
            if tool_request_is_denied_repeat(denied_tool_request) {
                format!("Worker repeated denied {tools}; {phase} still needs another path.")
            } else {
                format!("Operator denied {tools}; {phase} needs another path.")
            }
        }
        "resume_ready" => resume_ready_summary(phase, resume_pending_reason),
        "stale_active" => format!("Runtime declares active {phase}, but daemon is not running."),
        "blocked" => format!("Runtime is blocked at {phase}."),
        "failed" => format!("Runtime failed at {phase}."),
        _ => format!("Runtime needs attention at {phase}."),
    }
}

fn tool_request_tools_label(request: Option<&serde_json::Value>) -> String {
    request
        .and_then(|request| request["tools"].as_array())
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool.as_str())
                .take(4)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|tools| !tools.is_empty())
        .unwrap_or_else(|| "additional tools".to_string())
}

fn repeated_denied_tools_label(request: Option<&serde_json::Value>) -> String {
    request
        .and_then(|request| request["repeated_denied_tools"].as_array())
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool.as_str())
                .take(4)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|tools| !tools.is_empty())
        .unwrap_or_else(|| tool_request_tools_label(request))
}

fn tool_request_is_denied_repeat(request: Option<&serde_json::Value>) -> bool {
    request
        .and_then(|request| request["repeat_of_denied_tool_request"].as_bool())
        .unwrap_or(false)
}

fn resume_ready_summary(phase: &str, reason: Option<&str>) -> String {
    match reason {
        Some("tool_request_approved") => {
            format!("An approved tool request is ready to resume {phase}.")
        }
        Some("project_ask_answered") => format!("A stored answer is ready to resume {phase}."),
        _ => format!("A stored resume marker is ready to resume {phase}."),
    }
}

fn runtime_last_event(runtime: &serde_json::Value) -> serde_json::Value {
    safe_last_event(
        runtime
            .get("timeline")
            .and_then(|value| value.as_array())
            .and_then(|events| events.last()),
    )
}

fn worker_counts(runtime: &serde_json::Value) -> serde_json::Value {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    if let Some(workers) = runtime.get("workers").and_then(|value| value.as_array()) {
        for worker in workers {
            let status = safe_worker_status(worker.get("status")).to_string();
            *counts.entry(status).or_insert(0) += 1;
        }
    }
    serde_json::json!({
        "total": counts.values().sum::<usize>(),
        "by_status": counts,
    })
}

fn project_attention_actions(
    project_id: &str,
    state: &str,
    first_pending_question: Option<&serde_json::Value>,
    pending_tool_request: Option<&serde_json::Value>,
    denied_tool_request: Option<&serde_json::Value>,
    resume_pending: bool,
    resume_pending_reason: Option<&str>,
) -> serde_json::Value {
    let base = format!("/api/projects/{project_id}/runtime");
    let mut actions = Vec::new();

    if let Some(question) = first_pending_question {
        actions.push(serde_json::json!({
            "label": "answer_question",
            "method": "POST",
            "path": format!("{base}/answer"),
            "body_hint": {
                "ask_id": question.get("ask_id").cloned().unwrap_or(serde_json::Value::Null),
                "answer": "...",
            },
            "reason": "A project worker is waiting for the user answer.",
        }));
    }

    if let Some(request) = pending_tool_request {
        actions.push(serde_json::json!({
            "label": "respond_tool_request",
            "method": "POST",
            "path": format!("{base}/tool-request"),
            "body_hint": {
                "phase": request.get("phase").cloned().unwrap_or(serde_json::Value::Null),
                "tools": request.get("tools").cloned().unwrap_or(serde_json::Value::Null),
                "decision": "approve|deny",
                "reason": "...",
            },
            "reason": request["reason"]
                .as_str()
                .unwrap_or("A worker requested additional tools before continuing."),
        }));
    }

    if resume_pending || matches!(state, "stale_active" | "blocked" | "tool_request_denied") {
        actions.push(serde_json::json!({
            "label": "resume_runtime",
            "method": "POST",
            "path": format!("{base}/resume"),
            "reason": resume_action_reason(resume_pending_reason, denied_tool_request),
        }));
    } else if state == "failed" {
        actions.push(serde_json::json!({
            "label": "start_runtime",
            "method": "POST",
            "path": format!("{base}/start"),
            "reason": "Start a fresh autonomous project run.",
        }));
    }

    serde_json::Value::Array(actions)
}

fn resume_action_reason(
    reason: Option<&str>,
    denied_tool_request: Option<&serde_json::Value>,
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
