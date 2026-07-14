use serde_json::{json, Value};
use std::{collections::BTreeSet, thread, time::Duration};

use crate::{daemon_client, daemon_json, require_daemon, truncate_display, ui};

const DEFAULT_TIMELINE_LIMIT: usize = 12;
const MAX_TIMELINE_LIMIT: usize = 50;
const TIMELINE_FOLLOW_INTERVAL_SECS: u64 = 2;

pub(super) fn cmd_project_timeline(project_id: &str, limit: usize, follow: bool, json: bool) {
    if follow && json {
        ui::error("Project timeline --follow does not support --json; omit --json for live text.");
        return;
    }
    let base = require_daemon("project timeline");
    let body = fetch_project_timeline(&base, project_id, limit);
    if let Some(error) = project_timeline_error(&body) {
        ui::error(&format!("Project timeline failed: {error}"));
        return;
    }
    if follow {
        follow_project_timeline(&base, project_id, &body, limit);
        return;
    }
    let timeline = project_timeline_view(&body, limit);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&timeline).unwrap_or_default()
        );
    } else {
        print_project_timeline(&timeline);
    }
}

fn fetch_project_timeline(base: &str, project_id: &str, limit: usize) -> Value {
    let client = daemon_client();
    daemon_json(
        client
            .get(project_timeline_runtime_url(base, project_id, limit))
            .send(),
    )
}

fn project_timeline_runtime_url(base: &str, project_id: &str, limit: usize) -> String {
    let limit = limit.clamp(1, MAX_TIMELINE_LIMIT);
    format!("{base}/api/projects/{project_id}/runtime?events={limit}")
}

fn follow_project_timeline(base: &str, project_id: &str, body: &Value, limit: usize) {
    let timeline = project_timeline_view(body, limit);
    let mut seen = BTreeSet::new();
    remember_timeline_events(&project_timeline_events(body), &mut seen);
    print_project_timeline(&timeline);
    ui::hint("Following project timeline; press Ctrl+C to stop.");

    loop {
        thread::sleep(Duration::from_secs(TIMELINE_FOLLOW_INTERVAL_SECS));
        let body = fetch_project_timeline(base, project_id, limit);
        if let Some(error) = project_timeline_error(&body) {
            ui::error(&format!("Project timeline follow failed: {error}"));
            return;
        }
        for event in project_timeline_new_events(&body, &mut seen) {
            println!("{}", project_timeline_line(&event));
        }
    }
}

fn project_timeline_view(body: &Value, limit: usize) -> Value {
    let limit = limit.clamp(1, MAX_TIMELINE_LIMIT);
    let events = project_timeline_events(body);
    let total = events.len();
    let start = total.saturating_sub(limit);
    let visible: Vec<Value> = events
        .iter()
        .skip(start)
        .map(project_timeline_event_view)
        .collect();
    json!({
        "project_id": body.pointer("/operator_status/project_id")
            .or_else(|| body.pointer("/project/id"))
            .cloned()
            .unwrap_or(Value::Null),
        "project_slug": body.pointer("/operator_status/project_slug")
            .or_else(|| body.pointer("/project/slug"))
            .cloned()
            .unwrap_or(Value::Null),
        "session_id": body.pointer("/transcript/session_id")
            .or_else(|| body.get("session_id"))
            .cloned()
            .unwrap_or(Value::Null),
        "count": visible.len(),
        "total": total,
        "limit": limit,
        "truncated": total > visible.len()
            || body.pointer("/transcript/truncated").and_then(Value::as_bool).unwrap_or(false),
        "events": visible,
    })
}

fn project_timeline_events(body: &Value) -> Vec<Value> {
    body.pointer("/transcript/events")
        .and_then(Value::as_array)
        .filter(|events| !events.is_empty())
        .or_else(|| body.pointer("/runtime/timeline").and_then(Value::as_array))
        .cloned()
        .unwrap_or_default()
}

fn project_timeline_event_view(event: &Value) -> Value {
    json!({
        "id": event.get("id").cloned().unwrap_or(Value::Null),
        "ts": event.get("ts").cloned().unwrap_or(Value::Null),
        "kind": event.get("kind").cloned().unwrap_or(Value::Null),
        "title": bounded_event_field(event, "title", 96),
        "detail": bounded_event_field(event, "detail", 180),
        "actor": event.get("actor").cloned().unwrap_or(Value::Null),
        "phase": event.get("phase").cloned().unwrap_or(Value::Null),
        "status": event.get("status").cloned().unwrap_or(Value::Null),
    })
}

fn remember_timeline_events(events: &[Value], seen: &mut BTreeSet<String>) {
    for event in events {
        seen.insert(project_timeline_event_key(event));
    }
}

fn project_timeline_new_events(body: &Value, seen: &mut BTreeSet<String>) -> Vec<Value> {
    project_timeline_events(body)
        .into_iter()
        .filter_map(|event| {
            let key = project_timeline_event_key(&event);
            if seen.insert(key) {
                Some(project_timeline_event_view(&event))
            } else {
                None
            }
        })
        .collect()
}

fn project_timeline_event_key(event: &Value) -> String {
    event
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            format!(
                "{}|{}|{}|{}|{}",
                event.get("ts").and_then(Value::as_str).unwrap_or(""),
                event.get("kind").and_then(Value::as_str).unwrap_or(""),
                event.get("phase").and_then(Value::as_str).unwrap_or(""),
                event.get("status").and_then(Value::as_str).unwrap_or(""),
                event.get("title").and_then(Value::as_str).unwrap_or("")
            )
        })
}

fn bounded_event_field(event: &Value, key: &str, max_chars: usize) -> Value {
    event
        .get(key)
        .and_then(Value::as_str)
        .map(|value| json!(truncate_display(value, max_chars)))
        .unwrap_or(Value::Null)
}

fn print_project_timeline(timeline: &Value) {
    let events = timeline["events"].as_array().cloned().unwrap_or_default();
    if events.is_empty() {
        ui::success("No project runtime timeline events.");
        return;
    }
    ui::section("Project Timeline");
    ui::blank();
    for event in events {
        println!("{}", project_timeline_line(&event));
    }
    if timeline["truncated"].as_bool().unwrap_or(false) {
        let total = timeline["total"].as_u64().unwrap_or(0);
        let limit = timeline["limit"]
            .as_u64()
            .unwrap_or(DEFAULT_TIMELINE_LIMIT as u64);
        ui::hint(&format!(
            "Showing newest {limit} of {total} timeline event(s)."
        ));
    }
}

fn project_timeline_line(event: &Value) -> String {
    let ts = event["ts"].as_str().unwrap_or("?");
    let phase = event["phase"].as_str().unwrap_or("?");
    let status = event["status"].as_str().unwrap_or("?");
    let kind = event["kind"].as_str().unwrap_or("event");
    let title = event["title"].as_str().unwrap_or("Untitled event");
    let detail = event["detail"].as_str().unwrap_or("");
    if detail.is_empty() {
        format!("    {ts} -- {phase}/{status} -- {kind} -- {title}")
    } else {
        format!("    {ts} -- {phase}/{status} -- {kind} -- {title}: {detail}")
    }
}

fn project_timeline_error(body: &Value) -> Option<String> {
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
    fn timeline_view_prefers_transcript_and_limits_newest_events() {
        let body = json!({
            "project": {"id": "project-1", "slug": "demo"},
            "runtime": {
                "timeline": [{
                    "id": "runtime-old",
                    "ts": "2026-05-23T10:00:00Z",
                    "kind": "runtime.only",
                    "title": "Runtime only",
                    "detail": "old",
                    "phase": "observe",
                    "status": "ready"
                }]
            },
            "transcript": {
                "session_id": "session-1",
                "events": [
                    {"id": "event-1", "ts": "2026-05-23T10:01:00Z", "kind": "one", "title": "One", "detail": "first", "phase": "observe", "status": "ready"},
                    {"id": "event-2", "ts": "2026-05-23T10:02:00Z", "kind": "two", "title": "Two", "detail": "second", "phase": "build", "status": "running"}
                ]
            }
        });

        let timeline = project_timeline_view(&body, 1);

        assert_eq!(timeline["count"], 1);
        assert_eq!(timeline["total"], 2);
        assert_eq!(timeline["events"][0]["id"], "event-2");
        assert_eq!(timeline["events"][0]["kind"], "two");
    }

    #[test]
    fn timeline_event_view_does_not_expose_raw_data() {
        let event = json!({
            "id": "event-1",
            "ts": "2026-05-23T10:00:00Z",
            "kind": "worker.done",
            "title": "Worker done",
            "detail": "Phase completed",
            "actor": "captain",
            "phase": "verify",
            "status": "done",
            "data": {"secret": "do-not-print"}
        });

        let view = project_timeline_event_view(&event);
        let line = project_timeline_line(&view);

        assert!(view.get("data").is_none());
        assert!(!line.contains("do-not-print"));
        assert!(line.contains("worker.done"));
    }

    #[test]
    fn timeline_follow_filters_seen_events_and_sanitizes_new_events() {
        let mut seen = BTreeSet::new();
        let first = json!({
            "transcript": {
                "events": [
                    {"id": "event-1", "ts": "2026-05-23T10:01:00Z", "kind": "one", "title": "One", "phase": "observe", "status": "done"}
                ]
            }
        });
        remember_timeline_events(&project_timeline_events(&first), &mut seen);

        let second = json!({
            "transcript": {
                "events": [
                    {"id": "event-1", "ts": "2026-05-23T10:01:00Z", "kind": "one", "title": "One", "phase": "observe", "status": "done"},
                    {"id": "event-2", "ts": "2026-05-23T10:02:00Z", "kind": "two", "title": "Two", "detail": "safe", "phase": "build", "status": "running", "data": {"secret": "do-not-print"}}
                ]
            }
        });

        let new_events = project_timeline_new_events(&second, &mut seen);
        let rendered = serde_json::to_string(&new_events).unwrap();

        assert_eq!(new_events.len(), 1);
        assert_eq!(new_events[0]["id"], "event-2");
        assert!(!rendered.contains("do-not-print"));
        assert!(!rendered.contains("\"data\""));
    }

    #[test]
    fn timeline_runtime_url_sends_clamped_event_limit() {
        assert_eq!(
            project_timeline_runtime_url("http://127.0.0.1:50051", "demo", 0),
            "http://127.0.0.1:50051/api/projects/demo/runtime?events=1"
        );
        assert_eq!(
            project_timeline_runtime_url("http://127.0.0.1:50051", "demo", 999),
            "http://127.0.0.1:50051/api/projects/demo/runtime?events=50"
        );
    }
}
