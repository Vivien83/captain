use serde_json::{json, Value};

use super::project_runtime_actions::project_runtime_action_commands;
use crate::truncate_display;

pub(crate) fn project_runtime_response_view(body: &Value, fallback_project_id: &str) -> Value {
    if let Some(error) = project_runtime_error(body) {
        return json!({
            "ok": false,
            "error": error,
        });
    }

    let status = &body["operator_status"];
    let project_id = first_str(
        &[status.get("project_id"), body.pointer("/project/id")],
        fallback_project_id,
    );
    let project_slug = first_str(
        &[status.get("project_slug"), body.pointer("/project/slug")],
        &project_id,
    );
    let command_id = if project_slug.is_empty() {
        project_id.clone()
    } else {
        project_slug.clone()
    };

    json!({
        "ok": body.get("ok").cloned().unwrap_or(json!(true)),
        "project": {
            "id": project_id,
            "slug": project_slug,
            "name": bounded_field(body.pointer("/project/name"), 120),
            "status": bounded_field(body.pointer("/project/status"), 40),
            "updated_at": body.pointer("/project/updated_at")
                .cloned()
                .unwrap_or_else(|| status.get("updated_at").cloned().unwrap_or(Value::Null)),
        },
        "runtime": {
            "state": bounded_field(status.get("state"), 60),
            "status": bounded_field(status.get("status"), 60),
            "phase": bounded_field(status.get("phase"), 80),
            "progress": status.get("progress").cloned().unwrap_or(Value::Null),
            "summary": bounded_field(status.get("summary"), 220),
            "running_in_process": status.get("running_in_process").cloned().unwrap_or(Value::Null),
            "declared_active": status.get("declared_active").cloned().unwrap_or(Value::Null),
            "resume_pending": status.get("resume_pending").cloned().unwrap_or(Value::Null),
            "resume_pending_reason": bounded_field(status.get("resume_pending_reason"), 100),
            "workers": status.get("workers").cloned().unwrap_or(Value::Null),
        },
        "attention": {
            "pending_questions": status.get("pending_questions").cloned().unwrap_or(json!(0)),
            "first_pending_question": safe_question(&status["first_pending_question"]),
            "pending_tool_request": safe_tool_request(&status["pending_tool_request"]),
            "denied_tool_request": safe_tool_request(&status["denied_tool_request"]),
        },
        "last_event": safe_last_event(&status["last_event"]),
        "result": safe_result(body),
        "next_actions": project_runtime_action_commands(body, &command_id),
    })
}

fn project_runtime_error(body: &Value) -> Option<String> {
    if body["ok"].as_bool() == Some(false) || body.get("error").is_some() {
        return Some(
            body.get("error")
                .or_else(|| body.get("runtime_error"))
                .map(value_to_short_string)
                .unwrap_or_else(|| "unknown error".to_string()),
        );
    }
    None
}

fn safe_question(question: &Value) -> Value {
    if question.is_null() {
        return Value::Null;
    }
    json!({
        "ask_id": bounded_field(question.get("ask_id"), 120),
        "phase": bounded_field(question.get("phase"), 80),
        "role": bounded_field(question.get("role"), 80),
        "status": bounded_field(question.get("status"), 60),
        "question": bounded_field(question.get("question"), 260),
        "options": string_array(question.get("options"), 6, 120),
        "created_at": question.get("created_at").cloned().unwrap_or(Value::Null),
        "delivered_at": question.get("delivered_at").cloned().unwrap_or(Value::Null),
    })
}

fn safe_tool_request(request: &Value) -> Value {
    if request.is_null() {
        return Value::Null;
    }
    json!({
        "phase": bounded_field(request.get("phase"), 80),
        "status": bounded_field(request.get("status"), 80),
        "tools": string_array(request.get("tools"), 8, 80),
        "reason": bounded_field(request.get("reason"), 220),
        "decision_reason": bounded_field(request.get("decision_reason"), 220),
        "previous_decision_reason": bounded_field(request.get("previous_decision_reason"), 220),
        "repeat_of_denied_tool_request": request
            .get("repeat_of_denied_tool_request")
            .cloned()
            .unwrap_or(Value::Null),
        "repeated_denied_tools": string_array(request.get("repeated_denied_tools"), 8, 80),
    })
}

fn safe_last_event(event: &Value) -> Value {
    if event.is_null() {
        return Value::Null;
    }
    json!({
        "id": bounded_field(event.get("id"), 120),
        "kind": bounded_field(event.get("kind"), 120),
        "title": bounded_field(event.get("title"), 180),
        "phase": bounded_field(event.get("phase"), 80),
        "status": bounded_field(event.get("status"), 80),
        "ts": event.get("ts").cloned().unwrap_or(Value::Null),
    })
}

fn safe_result(body: &Value) -> Value {
    json!({
        "ask_id": bounded_field(body.get("ask_id"), 120),
        "delivered_to_active_worker": body
            .get("delivered_to_active_worker")
            .cloned()
            .unwrap_or(Value::Null),
        "runtime_resume_pending": body
            .get("runtime_resume_pending")
            .cloned()
            .unwrap_or(Value::Null),
        "phase": bounded_field(body.get("phase"), 80),
        "decision": bounded_field(body.get("decision"), 80),
        "tools": string_array(body.get("tools"), 8, 80),
        "warning": bounded_field(body.get("warning"), 220),
        "active_worker_error": bounded_field(body.get("active_worker_error"), 220),
    })
}

fn bounded_field(value: Option<&Value>, max_chars: usize) -> Value {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| json!(truncate_display(value, max_chars)))
        .unwrap_or(Value::Null)
}

fn string_array(value: Option<&Value>, max_items: usize, max_chars: usize) -> Value {
    let items: Vec<Value> = value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .take(max_items)
                .map(|value| json!(truncate_display(value, max_chars)))
                .collect()
        })
        .unwrap_or_default();
    Value::Array(items)
}

fn first_str(values: &[Option<&Value>], fallback: &str) -> String {
    values
        .iter()
        .filter_map(|value| value.and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn value_to_short_string(value: &Value) -> String {
    value
        .as_str()
        .map(|value| truncate_display(value, 180))
        .unwrap_or_else(|| truncate_display(&value.to_string(), 180))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_response_view_omits_raw_runtime_payloads() {
        let body = json!({
            "ok": true,
            "answer": "private answer do-not-print",
            "project": {
                "id": "project-1",
                "slug": "demo",
                "name": "Demo",
                "status": "active",
                "metadata": {
                    "workspace": "/Users/example/private",
                    "token": "secret"
                }
            },
            "runtime": {
                "workers": [{
                    "prompt": "raw prompt do-not-print",
                    "task": "raw task do-not-print"
                }],
                "timeline": [{
                    "data": {"secret": "timeline secret do-not-print"}
                }]
            },
            "transcript": {
                "events": [{"data": {"payload": "raw event do-not-print"}}]
            },
            "chat": {"agent_id": "agent-hidden"},
            "operator_status": {
                "project_id": "project-1",
                "project_slug": "demo",
                "state": "resume_ready",
                "status": "ready",
                "phase": "build",
                "progress": 40,
                "summary": "A user answer is stored; phase build is ready to resume.",
                "pending_questions": 0,
                "workers": {"total": 1},
                "last_event": {
                    "id": "event-1",
                    "kind": "worker.done",
                    "title": "Worker finished",
                    "data": {"raw": "hidden"}
                },
                "actions": [{"label": "resume_runtime"}]
            },
            "runtime_resume_pending": true
        });

        let view = project_runtime_response_view(&body, "fallback");
        let rendered = serde_json::to_string(&view).unwrap();

        assert_eq!(view["project"]["slug"], "demo");
        assert_eq!(view["next_actions"][0], "captain project resume demo");
        assert_eq!(view["result"]["runtime_resume_pending"], true);
        assert!(!rendered.contains("private answer"));
        assert!(!rendered.contains("raw prompt"));
        assert!(!rendered.contains("raw task"));
        assert!(!rendered.contains("timeline secret"));
        assert!(!rendered.contains("raw event"));
        assert!(!rendered.contains("agent-hidden"));
        assert!(!rendered.contains("metadata"));
        assert!(!rendered.contains("/Users/example/private"));
        assert!(!rendered.contains("token"));
    }

    #[test]
    fn runtime_response_view_bounds_actionable_details() {
        let body = json!({
            "operator_status": {
                "project_id": "project-1",
                "state": "tool_request_pending",
                "phase": "verify",
                "pending_questions": 1,
                "first_pending_question": {
                    "ask_id": "ask-1",
                    "question": "Which path?",
                    "options": ["safe", "fast"],
                    "answer": "stored answer do-not-print"
                },
                "pending_tool_request": {
                    "phase": "verify",
                    "tools": ["shell_exec"],
                    "reason": "Need one command",
                    "previous_denied_tool_request": {
                        "reason": "nested raw do-not-print"
                    }
                },
                "actions": [{
                    "label": "answer_question",
                    "body_hint": {"ask_id": "ask-1"}
                }]
            }
        });

        let view = project_runtime_response_view(&body, "fallback");
        let rendered = serde_json::to_string(&view).unwrap();

        assert_eq!(
            view["attention"]["first_pending_question"]["ask_id"],
            "ask-1"
        );
        assert_eq!(
            view["attention"]["pending_tool_request"]["tools"][0],
            "shell_exec"
        );
        assert_eq!(
            view["next_actions"][0],
            "captain project answer project-1 --ask-id ask-1 --answer \"...\""
        );
        assert!(!rendered.contains("stored answer"));
        assert!(!rendered.contains("previous_denied_tool_request"));
        assert!(!rendered.contains("nested raw"));
    }
}
