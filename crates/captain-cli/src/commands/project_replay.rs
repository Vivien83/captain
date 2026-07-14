use serde_json::{json, Value};

use super::project_replay_render::print_project_replay;
#[cfg(test)]
use super::project_replay_render::{replay_event_line, replay_question_line, replay_worker_line};
use super::project_runtime_actions::{project_answer_command, project_runtime_action_commands};
use super::project_worker_window::select_priority_recent_workers;
use crate::{daemon_client, daemon_json, require_daemon, truncate_display, ui};

const MAX_REPLAY_EVENTS: usize = 80;
const MAX_REPLAY_WORKERS: usize = 40;
const MAX_REPLAY_QUESTIONS: usize = 6;
const MAX_REPLAY_OPTIONS: usize = 6;

pub(super) fn cmd_project_replay(
    project_id: &str,
    event_limit: usize,
    worker_limit: usize,
    json: bool,
) {
    let base = require_daemon("project replay");
    let body = fetch_project_replay(&base, project_id, event_limit);
    if let Some(error) = project_replay_error(&body) {
        ui::error(&format!("Project replay failed: {error}"));
        return;
    }
    let replay = project_replay_view(&body, project_id, event_limit, worker_limit);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&replay).unwrap_or_default()
        );
    } else {
        print_project_replay(&replay);
    }
}

fn fetch_project_replay(base: &str, project_id: &str, event_limit: usize) -> Value {
    let client = daemon_client();
    daemon_json(
        client
            .get(project_replay_runtime_url(base, project_id, event_limit))
            .send(),
    )
}

fn project_replay_runtime_url(base: &str, project_id: &str, event_limit: usize) -> String {
    let event_limit = event_limit.clamp(1, MAX_REPLAY_EVENTS);
    format!("{base}/api/projects/{project_id}/runtime?events={event_limit}")
}

fn project_replay_view(
    body: &Value,
    command_project_id: &str,
    event_limit: usize,
    worker_limit: usize,
) -> Value {
    let event_limit = event_limit.clamp(1, MAX_REPLAY_EVENTS);
    let worker_limit = worker_limit.clamp(1, MAX_REPLAY_WORKERS);
    let events = project_replay_events(body);
    let event_total = events.len();
    let event_start = event_total.saturating_sub(event_limit);
    let replay_events: Vec<Value> = events
        .iter()
        .skip(event_start)
        .map(project_replay_event_view)
        .collect();
    let workers = body
        .pointer("/runtime/workers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let results = body
        .pointer("/runtime/worker_results")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let worker_refs = workers.iter().collect::<Vec<_>>();
    let replay_workers: Vec<Value> = select_priority_recent_workers(&worker_refs, worker_limit)
        .into_iter()
        .map(|worker| project_replay_worker_view(worker, &results))
        .collect();
    let pending_questions = project_replay_pending_questions(body, command_project_id);

    json!({
        "project_id": body.pointer("/operator_status/project_id")
            .or_else(|| body.pointer("/project/id"))
            .cloned()
            .unwrap_or(Value::Null),
        "project_slug": body.pointer("/operator_status/project_slug")
            .or_else(|| body.pointer("/project/slug"))
            .cloned()
            .unwrap_or(Value::Null),
        "state": body.pointer("/operator_status/state").cloned().unwrap_or(Value::Null),
        "phase": body.pointer("/operator_status/phase").cloned().unwrap_or(Value::Null),
        "progress": body.pointer("/operator_status/progress").cloned().unwrap_or(Value::Null),
        "session_id": body.pointer("/transcript/session_id")
            .or_else(|| body.get("session_id"))
            .cloned()
            .unwrap_or(Value::Null),
        "transcript": {
            "count": body.pointer("/transcript/count").cloned().unwrap_or(Value::Null),
            "stored_count": body.pointer("/transcript/stored_count").cloned().unwrap_or(Value::Null),
            "limit": body.pointer("/transcript/limit").cloned().unwrap_or(Value::Null),
            "truncated": body.pointer("/transcript/truncated").and_then(Value::as_bool).unwrap_or(false),
        },
        "worker_counts": body.pointer("/operator_status/workers")
            .cloned()
            .unwrap_or_else(|| json!({ "total": workers.len() })),
        "running_in_process": body.pointer("/operator_status/running_in_process").cloned().unwrap_or(Value::Null),
        "declared_active": body.pointer("/operator_status/declared_active").cloned().unwrap_or(Value::Null),
        "resume_pending": body.pointer("/operator_status/resume_pending").cloned().unwrap_or(Value::Null),
        "resume_pending_reason": body.pointer("/operator_status/resume_pending_reason").cloned().unwrap_or(Value::Null),
        "workers": {
            "count": replay_workers.len(),
            "total": workers.len(),
            "limit": worker_limit,
            "truncated": workers.len() > replay_workers.len(),
            "items": replay_workers,
        },
        "events": {
            "count": replay_events.len(),
            "total": event_total,
            "limit": event_limit,
            "truncated": event_total > replay_events.len()
                || body.pointer("/transcript/truncated").and_then(Value::as_bool).unwrap_or(false),
            "items": replay_events,
        },
        "pending_questions": pending_questions,
        "next_actions": project_replay_next_actions(body, command_project_id),
    })
}

fn project_replay_events(body: &Value) -> Vec<Value> {
    body.pointer("/transcript/events")
        .and_then(Value::as_array)
        .filter(|events| !events.is_empty())
        .or_else(|| body.pointer("/runtime/timeline").and_then(Value::as_array))
        .cloned()
        .unwrap_or_default()
}

fn project_replay_event_view(event: &Value) -> Value {
    json!({
        "id": event.get("id").cloned().unwrap_or(Value::Null),
        "ts": event.get("ts").cloned().unwrap_or(Value::Null),
        "kind": event.get("kind").cloned().unwrap_or(Value::Null),
        "phase": event.get("phase").cloned().unwrap_or(Value::Null),
        "status": event.get("status").cloned().unwrap_or(Value::Null),
        "title": bounded_str(event, "title", 120),
        "detail": bounded_str(event, "detail", 260),
    })
}

fn project_replay_worker_view(worker: &Value, results: &Value) -> Value {
    let phase = worker.get("phase").and_then(Value::as_str).unwrap_or("");
    let result = results.get(phase).unwrap_or(&Value::Null);
    json!({
        "id": worker.get("id").cloned().unwrap_or(Value::Null),
        "role": bounded_str(worker, "role", 80),
        "phase": worker.get("phase").cloned().unwrap_or(Value::Null),
        "status": worker.get("status").cloned().unwrap_or(Value::Null),
        "mode": worker.get("mode").cloned().unwrap_or(Value::Null),
        "tool_calls": worker.get("tool_calls").cloned().unwrap_or(Value::Null),
        "cost_usd": worker
            .get("cost_usd")
            .or_else(|| result.get("cost_usd"))
            .cloned()
            .unwrap_or(Value::Null),
        "tool_decisions": worker_tool_decisions(worker, result),
        "iterations": worker.get("iterations").cloned().unwrap_or(Value::Null),
        "cleanup_status": worker.get("cleanup_status").cloned().unwrap_or(Value::Null),
        "started_at": worker.get("started_at").cloned().unwrap_or(Value::Null),
        "completed_at": worker.get("completed_at").cloned().unwrap_or(Value::Null),
        "summary": worker_summary(worker, result),
        "tool_request": worker_tool_request(worker, result),
    })
}

fn worker_tool_decisions(worker: &Value, result: &Value) -> Vec<Value> {
    worker
        .get("tool_decisions")
        .or_else(|| result.get("tool_decisions"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .take(8)
                .map(worker_tool_decision_view)
                .collect()
        })
        .unwrap_or_default()
}

fn worker_tool_decision_view(decision: &Value) -> Value {
    json!({
        "tool": decision
            .get("tool")
            .or_else(|| decision.get("tool_name"))
            .and_then(Value::as_str)
            .map(|value| truncate_display(value, 80))
            .unwrap_or_default(),
        "reason": decision
            .get("reason")
            .and_then(Value::as_str)
            .map(|value| truncate_display(value, 160))
            .unwrap_or_default(),
        "status": decision
            .get("status")
            .and_then(Value::as_str)
            .map(|value| truncate_display(value, 24))
            .unwrap_or_else(|| "unknown".to_string()),
        "duration_ms": decision.get("duration_ms").cloned().unwrap_or(Value::Null),
    })
}

fn worker_summary(worker: &Value, result: &Value) -> Value {
    worker
        .get("summary")
        .or_else(|| result.get("summary"))
        .and_then(Value::as_str)
        .map(|value| json!(truncate_display(value, 260)))
        .unwrap_or(Value::Null)
}

fn worker_tool_request(worker: &Value, result: &Value) -> Value {
    let request = worker
        .get("tool_request")
        .or_else(|| result.get("tool_request"));
    let Some(request) = request.filter(|value| value.is_object()) else {
        return Value::Null;
    };
    json!({
        "status": request.get("status").cloned().unwrap_or(Value::Null),
        "tools": request_tool_names(request),
        "repeat_of_denied_tool_request": request.get("repeat_of_denied_tool_request")
            .cloned()
            .unwrap_or(Value::Null),
    })
}

fn request_tool_names(request: &Value) -> Vec<Value> {
    request
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(Value::as_str)
                .filter(|tool| !tool.trim().is_empty())
                .take(8)
                .map(|tool| json!(truncate_display(tool, 64)))
                .collect()
        })
        .unwrap_or_default()
}

fn project_replay_pending_questions(body: &Value, command_project_id: &str) -> Value {
    let rows = body
        .pointer("/runtime/user_questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let pending: Vec<&Value> = rows
        .iter()
        .filter(|question| question_pending(question))
        .collect();
    let items: Vec<Value> = pending
        .iter()
        .take(MAX_REPLAY_QUESTIONS)
        .map(|question| project_replay_question_view(question, command_project_id))
        .collect();
    json!({
        "count": items.len(),
        "total": pending.len(),
        "limit": MAX_REPLAY_QUESTIONS,
        "truncated": pending.len() > items.len(),
        "items": items,
    })
}

fn question_pending(question: &Value) -> bool {
    question
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("pending")
        .eq_ignore_ascii_case("pending")
}

fn project_replay_question_view(question: &Value, command_project_id: &str) -> Value {
    let ask_id = question.get("ask_id").and_then(Value::as_str).unwrap_or("");
    let next_action = if ask_id.is_empty() {
        String::new()
    } else {
        project_answer_command(command_project_id, ask_id)
    };
    json!({
        "ask_id": question.get("ask_id").cloned().unwrap_or(Value::Null),
        "phase": question.get("phase").cloned().unwrap_or(Value::Null),
        "worker_role": bounded_str(question, "worker_role", 80),
        "delivery": question.get("delivery").cloned().unwrap_or(Value::Null),
        "question": bounded_str(question, "question", 360),
        "options": question_options(question),
        "created_at": question.get("created_at").cloned().unwrap_or(Value::Null),
        "next_action": next_action,
    })
}

fn question_options(question: &Value) -> Vec<Value> {
    question
        .get("options")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(Value::as_str)
                .filter(|option| !option.trim().is_empty())
                .take(MAX_REPLAY_OPTIONS)
                .map(|option| json!(truncate_display(option, 120)))
                .collect()
        })
        .unwrap_or_default()
}

fn project_replay_next_actions(body: &Value, command_project_id: &str) -> Vec<Value> {
    project_runtime_action_commands(body, command_project_id)
        .into_iter()
        .map(|command| json!(command))
        .collect()
}

fn bounded_str(source: &Value, key: &str, max_chars: usize) -> Value {
    source
        .get(key)
        .and_then(Value::as_str)
        .map(|value| json!(truncate_display(value, max_chars)))
        .unwrap_or(Value::Null)
}

fn project_replay_error(body: &Value) -> Option<String> {
    if body["ok"].as_bool() == Some(false) || body.get("error").is_some() {
        let error = body["error"].as_str().unwrap_or("unknown error");
        return Some(truncate_display(error, 180));
    }
    None
}

#[cfg(test)]
#[path = "project_replay_tests.rs"]
mod tests;
