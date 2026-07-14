use serde_json::{json, Value};

use crate::{daemon_client, daemon_json, require_daemon, truncate_display, ui};

const MAX_SESSION_REPLAY_EVENTS: usize = 200;

pub(crate) fn cmd_replay(session_id: &str, event_limit: usize, json: bool) {
    let base = require_daemon("replay");
    let body = fetch_session_replay(&base, session_id, event_limit);
    if let Some(error) = replay_error(&body) {
        ui::error(&format!("Session replay failed: {error}"));
        return;
    }
    let replay = session_replay_view(&body, session_id, event_limit);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&replay).unwrap_or_default()
        );
    } else {
        print_session_replay(&replay);
    }
}

fn fetch_session_replay(base: &str, session_id: &str, event_limit: usize) -> Value {
    let client = daemon_client();
    daemon_json(
        client
            .get(session_replay_url(base, session_id, event_limit))
            .send(),
    )
}

fn session_replay_url(base: &str, session_id: &str, event_limit: usize) -> String {
    let event_limit = event_limit.clamp(1, MAX_SESSION_REPLAY_EVENTS);
    format!("{base}/api/sessions/{session_id}/events?limit={event_limit}")
}

fn session_replay_view(body: &Value, command_session_id: &str, event_limit: usize) -> Value {
    let event_limit = event_limit.clamp(1, MAX_SESSION_REPLAY_EVENTS);
    let events = body
        .get("events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let event_total = events.len();
    let event_start = event_total.saturating_sub(event_limit);
    let replay_events = events
        .iter()
        .skip(event_start)
        .map(session_replay_event_view)
        .collect::<Vec<_>>();

    json!({
        "session_id": body
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or(command_session_id),
        "count": replay_events.len(),
        "total": event_total,
        "limit": event_limit,
        "truncated": event_total > replay_events.len(),
        "events": replay_events,
    })
}

fn session_replay_event_view(event: &Value) -> Value {
    let event_type = event
        .get("event_type")
        .or_else(|| event.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("event");
    let payload = event.get("payload").unwrap_or(&Value::Null);
    json!({
        "id": event.get("id").cloned().unwrap_or(Value::Null),
        "ts": event.get("ts").cloned().unwrap_or(Value::Null),
        "event_type": truncate_display(event_type, 80),
        "action": replay_action(event_type, payload),
        "reason": replay_reason(event_type, payload),
        "status": replay_status(event_type, payload),
        "cost_usd": payload.get("cost_usd").cloned().unwrap_or(Value::Null),
    })
}

fn replay_action(event_type: &str, payload: &Value) -> String {
    let tool = payload
        .get("name")
        .or_else(|| payload.get("tool"))
        .or_else(|| payload.get("tool_name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if !tool.is_empty() {
        return match event_type {
            "tool_use_start" => format!("tool {tool} started"),
            "tool_use_end" => format!("tool {tool} selected"),
            "tool_execution_result" => format!("tool {tool} completed"),
            _ => format!("tool {tool}"),
        };
    }

    payload
        .get("title")
        .or_else(|| payload.get("phase"))
        .or_else(|| payload.get("content"))
        .and_then(Value::as_str)
        .map(|value| truncate_display(value, 120))
        .unwrap_or_else(|| truncate_display(event_type, 120))
}

fn replay_reason(event_type: &str, payload: &Value) -> String {
    explicit_reason(payload)
        .or_else(|| payload.get("input").and_then(explicit_reason))
        .unwrap_or_else(|| fallback_reason(event_type, payload).to_string())
}

fn explicit_reason(value: &Value) -> Option<String> {
    ["reason", "why", "purpose", "rationale", "decision_reason"]
        .iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| truncate_display(value, 160))
}

fn fallback_reason(event_type: &str, payload: &Value) -> &'static str {
    let tool = payload
        .get("name")
        .or_else(|| payload.get("tool"))
        .or_else(|| payload.get("tool_name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    match tool {
        "shell_exec" | "shell_exec_critical" => "Run a shell command needed for the task.",
        "apply_patch" | "file_write" | "file_edit" | "file_read" | "file_list" => {
            "Inspect or update workspace files for the task."
        }
        "web_search" | "web_fetch" | "web_research" => {
            "Use web context needed to answer or verify the task."
        }
        "ask_user" => "Ask the user for missing context before continuing.",
        _ => match event_type {
            "tool_execution_result" => "Record the result of the selected tool.",
            "phase_change" => "Record progress through the agent turn.",
            "intermediate_message" => "Record an operator-facing progress note.",
            _ => "Replay a meaningful runtime event.",
        },
    }
}

fn replay_status(event_type: &str, payload: &Value) -> String {
    if let Some(is_error) = payload.get("is_error").and_then(Value::as_bool) {
        return if is_error { "error" } else { "ok" }.to_string();
    }
    payload
        .get("status")
        .and_then(Value::as_str)
        .map(|value| truncate_display(value, 40))
        .unwrap_or_else(|| match event_type {
            "tool_use_start" | "tool_use_end" => "selected".to_string(),
            _ => "recorded".to_string(),
        })
}

fn print_session_replay(replay: &Value) {
    ui::section("Session Replay");
    ui::blank();
    let session_id = replay["session_id"].as_str().unwrap_or("session");
    let count = replay["count"].as_u64().unwrap_or(0);
    let total = replay["total"].as_u64().unwrap_or(count);
    println!("    Session {session_id} -- events {count}/{total}");
    for event in replay["events"].as_array().cloned().unwrap_or_default() {
        println!("{}", session_replay_line(&event));
    }
    if replay["truncated"].as_bool().unwrap_or(false) {
        let limit = replay["limit"].as_u64().unwrap_or(80);
        ui::hint(&format!("Showing newest {limit} of {total} event(s)."));
    }
}

fn session_replay_line(event: &Value) -> String {
    let ts = event["ts"]
        .as_i64()
        .map(|value| value.to_string())
        .or_else(|| event["ts"].as_str().map(str::to_string))
        .unwrap_or_else(|| "?".to_string());
    let event_type = event["event_type"].as_str().unwrap_or("event");
    let action = event["action"].as_str().unwrap_or("runtime event");
    let reason = event["reason"].as_str().unwrap_or("no reason recorded");
    let status = event["status"].as_str().unwrap_or("recorded");
    let cost = event["cost_usd"]
        .as_f64()
        .map(|cost| format!(" -- cost ${cost:.4}"))
        .unwrap_or_default();
    format!("      {ts} -- {event_type} -- {action} -- {status} -- reason: {reason}{cost}")
}

fn replay_error(body: &Value) -> Option<String> {
    if body["ok"].as_bool() == Some(false) || body.get("error").is_some() {
        let error = body["error"].as_str().unwrap_or("unknown error");
        return Some(truncate_display(error, 180));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_replay_url_clamps_event_limit() {
        assert_eq!(
            session_replay_url("http://127.0.0.1:50051", "sess-1", 0),
            "http://127.0.0.1:50051/api/sessions/sess-1/events?limit=1"
        );
        assert_eq!(
            session_replay_url("http://127.0.0.1:50051", "sess-1", 999),
            "http://127.0.0.1:50051/api/sessions/sess-1/events?limit=200"
        );
    }

    #[test]
    fn session_replay_view_sanitizes_tool_decisions() {
        let body = json!({
            "session_id": "sess-1",
            "events": [{
                "id": 1,
                "session_id": "sess-1",
                "ts": 1000,
                "event_type": "tool_use_end",
                "payload": {
                    "tool_use_id": "tool-1",
                    "name": "shell_exec",
                    "input": {
                        "cmd": "secret do-not-print",
                        "reason": "Verify the build before commit."
                    }
                }
            }, {
                "id": 2,
                "ts": 1010,
                "event_type": "tool_execution_result",
                "payload": {
                    "name": "shell_exec",
                    "result_preview": "private output do-not-print",
                    "is_error": false,
                    "cost_usd": 0.12
                }
            }]
        });

        let replay = session_replay_view(&body, "sess-1", 80);
        let rendered = serde_json::to_string(&replay).unwrap();

        assert_eq!(replay["events"][0]["action"], "tool shell_exec selected");
        assert_eq!(
            replay["events"][0]["reason"],
            "Verify the build before commit."
        );
        assert_eq!(replay["events"][1]["status"], "ok");
        assert_eq!(replay["events"][1]["cost_usd"], 0.12);
        assert!(!rendered.contains("secret do-not-print"));
        assert!(!rendered.contains("private output"));
        assert!(!rendered.contains("payload"));
        assert!(!rendered.contains("input"));
    }

    #[test]
    fn session_replay_line_includes_reason_and_cost() {
        let event = json!({
            "ts": 1000,
            "event_type": "tool_execution_result",
            "action": "tool shell_exec completed",
            "status": "ok",
            "reason": "Record the result.",
            "cost_usd": 0.25
        });

        assert_eq!(
            session_replay_line(&event),
            "      1000 -- tool_execution_result -- tool shell_exec completed -- ok -- reason: Record the result. -- cost $0.2500"
        );
    }
}
