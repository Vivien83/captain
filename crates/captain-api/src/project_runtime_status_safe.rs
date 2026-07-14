use serde_json::{json, Value};

const ID_LIMIT: usize = 120;
const TEXT_LIMIT: usize = 500;
const OPTION_LIMIT: usize = 160;
const TOOL_LIMIT: usize = 80;
const LIST_LIMIT: usize = 8;

pub(crate) fn safe_runtime_status(status: &str) -> &'static str {
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

pub(crate) fn safe_runtime_phase(phase: &str) -> &'static str {
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

pub(crate) fn safe_resume_pending_reason(reason: Option<&str>) -> Option<&'static str> {
    match reason {
        Some("tool_request_approved") => Some("tool_request_approved"),
        Some("project_ask_answered") => Some("project_ask_answered"),
        _ => None,
    }
}

pub(crate) fn safe_worker_status(status: Option<&Value>) -> &'static str {
    match status.and_then(|value| value.as_str()).unwrap_or("planned") {
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

pub(crate) fn safe_pending_question(question: Option<&Value>) -> Value {
    let Some(question) = question else {
        return Value::Null;
    };

    json!({
        "ask_id": safe_scalar(question.get("ask_id"), ID_LIMIT),
        "phase": safe_phase_value(question.get("phase")),
        "role": safe_scalar(
            question.get("role").or_else(|| question.get("worker_role")),
            ID_LIMIT
        ),
        "status": safe_scalar(question.get("status"), ID_LIMIT),
        "delivery": safe_scalar(question.get("delivery"), ID_LIMIT),
        "question": safe_scalar(question.get("question"), TEXT_LIMIT),
        "options": safe_string_list(question.get("options"), LIST_LIMIT, OPTION_LIMIT),
        "created_at": safe_scalar(question.get("created_at"), ID_LIMIT),
        "updated_at": safe_scalar(question.get("updated_at"), ID_LIMIT),
    })
}

pub(crate) fn safe_tool_request(request: Option<&Value>) -> Value {
    let Some(request) = request else {
        return Value::Null;
    };

    json!({
        "phase": safe_phase_value(request.get("phase")),
        "worker_role": safe_scalar(request.get("worker_role"), ID_LIMIT),
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
        "source": safe_scalar(request.get("source"), ID_LIMIT),
    })
}

pub(crate) fn safe_last_event(event: Option<&Value>) -> Value {
    let Some(event) = event else {
        return Value::Null;
    };

    json!({
        "id": safe_scalar(event.get("id"), ID_LIMIT),
        "kind": safe_scalar(event.get("kind"), ID_LIMIT),
        "title": safe_scalar(event.get("title"), TEXT_LIMIT),
        "phase": safe_phase_value(event.get("phase")),
        "status": Value::String(safe_worker_status(event.get("status")).to_string()),
        "ts": safe_scalar(event.get("ts"), ID_LIMIT),
    })
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

fn safe_phase_value(value: Option<&Value>) -> Value {
    Value::String(
        safe_runtime_phase(value.and_then(|value| value.as_str()).unwrap_or("unknown")).to_string(),
    )
}

fn safe_string_list(value: Option<&Value>, limit: usize, item_limit: usize) -> Value {
    let Some(items) = value.and_then(|value| value.as_array()) else {
        return Value::Array(Vec::new());
    };

    Value::Array(
        items
            .iter()
            .filter_map(|item| item.as_str())
            .map(|item| bounded_text(item, item_limit))
            .filter(|item| !item.is_empty())
            .take(limit)
            .map(Value::String)
            .collect(),
    )
}

fn safe_optional_string_list(value: Option<&Value>, limit: usize, item_limit: usize) -> Value {
    if value.and_then(|value| value.as_array()).is_none() {
        return Value::Null;
    }
    safe_string_list(value, limit, item_limit)
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
