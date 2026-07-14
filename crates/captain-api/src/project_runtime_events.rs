use chrono::Utc;
use std::collections::HashSet;

const PROJECT_RUNTIME_TIMELINE_LIMIT: usize = 120;

pub(crate) fn runtime_timeline_event_ids(runtime: &serde_json::Value) -> HashSet<String> {
    runtime
        .get("timeline")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|event| event.get("id").and_then(|v| v.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn new_runtime_timeline_events(
    before_event_ids: &HashSet<String>,
    runtime: &serde_json::Value,
) -> Vec<serde_json::Value> {
    runtime
        .get("timeline")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter(|event| {
                    event
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(|id| !before_event_ids.contains(id))
                        .unwrap_or(true)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn mark_runtime_event_seen(
    seen: &mut HashSet<String>,
    event: &serde_json::Value,
) -> bool {
    if let Some(id) = event.get("id").and_then(|v| v.as_str()) {
        return seen.insert(id.to_string());
    }
    true
}

pub(crate) fn merged_runtime_transcript_events(
    stored_events: Vec<serde_json::Value>,
    runtime: &serde_json::Value,
    limit: usize,
) -> (Vec<serde_json::Value>, usize) {
    let mut seen = HashSet::new();
    let mut events = Vec::new();
    for event in stored_events {
        if !event.is_object() {
            continue;
        }
        if mark_runtime_event_seen(&mut seen, &event) {
            events.push(event);
        }
    }
    if let Some(timeline) = runtime.get("timeline").and_then(|value| value.as_array()) {
        for event in timeline {
            if mark_runtime_event_seen(&mut seen, event) {
                events.push(event.clone());
            }
        }
    }
    events.sort_by(|a, b| {
        let ats = a.get("ts").and_then(|value| value.as_str()).unwrap_or("");
        let bts = b.get("ts").and_then(|value| value.as_str()).unwrap_or("");
        ats.cmp(bts).then_with(|| {
            let aid = a.get("id").and_then(|value| value.as_str()).unwrap_or("");
            let bid = b.get("id").and_then(|value| value.as_str()).unwrap_or("");
            aid.cmp(bid)
        })
    });
    let merged_count = events.len();
    if merged_count > limit {
        events.drain(0..(merged_count - limit));
    }
    (events, merged_count)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn append_runtime_event(
    runtime: &mut serde_json::Value,
    kind: &str,
    title: &str,
    detail: &str,
    actor: &str,
    phase: &str,
    status: &str,
    data: serde_json::Value,
) {
    if !runtime
        .get("timeline")
        .map(|v| v.is_array())
        .unwrap_or(false)
    {
        runtime["timeline"] = serde_json::json!([]);
    }
    let event = runtime_event(kind, title, detail, actor, phase, status, data);
    if let Some(items) = runtime.get_mut("timeline").and_then(|v| v.as_array_mut()) {
        items.push(event);
        if items.len() > PROJECT_RUNTIME_TIMELINE_LIMIT {
            let drain = items.len() - PROJECT_RUNTIME_TIMELINE_LIMIT;
            items.drain(0..drain);
        }
    }
}

pub(crate) fn runtime_event(
    kind: &str,
    title: &str,
    detail: &str,
    actor: &str,
    phase: &str,
    status: &str,
    data: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "ts": Utc::now().to_rfc3339(),
        "kind": kind,
        "title": title,
        "detail": detail,
        "actor": actor,
        "phase": phase,
        "status": status,
        "data": data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_transcript_detects_only_new_timeline_events() {
        let mut runtime = serde_json::json!({
            "timeline": [
                {"id": "old-1", "ts": "2026-05-15T10:00:00Z", "title": "old"}
            ]
        });
        let before = runtime_timeline_event_ids(&runtime);
        runtime["timeline"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "id": "new-1",
                "ts": "2026-05-15T10:01:00Z",
                "title": "new"
            }));

        let new_events = new_runtime_timeline_events(&before, &runtime);
        assert_eq!(new_events.len(), 1);
        assert_eq!(new_events[0]["id"], "new-1");
    }

    #[test]
    fn mark_runtime_event_seen_deduplicates_by_event_id() {
        let mut seen = HashSet::new();
        let event = serde_json::json!({"id": "evt-1", "title": "one"});
        assert!(mark_runtime_event_seen(&mut seen, &event));
        assert!(!mark_runtime_event_seen(&mut seen, &event));
    }

    #[test]
    fn mark_runtime_event_seen_keeps_idless_events() {
        let mut seen = HashSet::new();
        let event = serde_json::json!({"title": "without id"});
        assert!(mark_runtime_event_seen(&mut seen, &event));
        assert!(mark_runtime_event_seen(&mut seen, &event));
    }

    #[test]
    fn merged_runtime_transcript_events_sorts_deduplicates_and_caps_tail() {
        let stored_events = vec![
            serde_json::json!({"id": "stored-2", "ts": "2026-05-15T10:02:00Z"}),
            serde_json::json!({"id": "shared", "ts": "2026-05-15T10:01:00Z"}),
            serde_json::json!("ignored"),
        ];
        let runtime = serde_json::json!({
            "timeline": [
                {"id": "stored-1", "ts": "2026-05-15T10:00:00Z"},
                {"id": "shared", "ts": "2026-05-15T10:01:30Z"},
                {"id": "stored-3", "ts": "2026-05-15T10:03:00Z"}
            ]
        });

        let (events, merged_count) = merged_runtime_transcript_events(stored_events, &runtime, 3);

        assert_eq!(merged_count, 4);
        assert_eq!(
            events
                .iter()
                .map(|event| event["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["shared", "stored-2", "stored-3"]
        );
        assert_eq!(events[0]["ts"], "2026-05-15T10:01:00Z");
    }

    #[test]
    fn merged_runtime_transcript_events_orders_same_timestamp_by_id() {
        let stored_events = vec![
            serde_json::json!({"id": "b", "ts": "2026-05-15T10:00:00Z"}),
            serde_json::json!({"id": "a", "ts": "2026-05-15T10:00:00Z"}),
        ];

        let (events, merged_count) =
            merged_runtime_transcript_events(stored_events, &serde_json::json!({}), 10);

        assert_eq!(merged_count, 2);
        assert_eq!(events[0]["id"], "a");
        assert_eq!(events[1]["id"], "b");
    }

    #[test]
    fn append_runtime_event_initializes_and_caps_timeline() {
        let mut runtime = serde_json::json!({});
        for idx in 0..125 {
            append_runtime_event(
                &mut runtime,
                "worker.note",
                &format!("event {idx}"),
                "detail",
                "captain",
                "build",
                "running",
                serde_json::json!({"idx": idx}),
            );
        }
        let events = runtime["timeline"].as_array().unwrap();
        assert_eq!(events.len(), PROJECT_RUNTIME_TIMELINE_LIMIT);
        assert_eq!(events[0]["data"]["idx"], 5);
        assert_eq!(events.last().unwrap()["data"]["idx"], 124);
    }
}
