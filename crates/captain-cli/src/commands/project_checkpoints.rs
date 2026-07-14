use serde_json::{json, Value};

use crate::{daemon_client, daemon_json, require_daemon, truncate_display, ui};

const DEFAULT_CHECKPOINT_LIMIT: usize = 8;
const MAX_CHECKPOINT_LIMIT: usize = 50;

pub(super) fn cmd_project_checkpoints(project_id: &str, limit: usize, json: bool) {
    let base = require_daemon("project checkpoints");
    let client = daemon_client();
    let runtime = daemon_json(
        client
            .get(format!("{base}/api/projects/{project_id}/runtime"))
            .send(),
    );
    if let Some(error) = project_checkpoints_error(&runtime) {
        ui::error(&format!("Project checkpoints failed: {error}"));
        return;
    }

    let resolved_id = runtime
        .pointer("/project/id")
        .and_then(Value::as_str)
        .or_else(|| {
            runtime
                .pointer("/operator_status/project_id")
                .and_then(Value::as_str)
        })
        .unwrap_or(project_id);
    let body = daemon_json(
        client
            .get(format!("{base}/api/projects/{resolved_id}/checkpoints"))
            .query(&[("limit", limit.clamp(1, MAX_CHECKPOINT_LIMIT).to_string())])
            .send(),
    );
    if let Some(error) = project_checkpoints_error(&body) {
        ui::error(&format!("Project checkpoints failed: {error}"));
        return;
    }

    let checkpoints = project_checkpoints_view(&body, &runtime, limit);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&checkpoints).unwrap_or_default()
        );
    } else {
        print_project_checkpoints(&checkpoints);
    }
}

fn project_checkpoints_view(body: &Value, runtime: &Value, limit: usize) -> Value {
    let limit = limit.clamp(1, MAX_CHECKPOINT_LIMIT);
    let rows = body["checkpoints"].as_array().cloned().unwrap_or_default();
    let visible: Vec<Value> = rows
        .iter()
        .take(limit)
        .map(project_checkpoint_view)
        .collect();
    json!({
        "project_id": runtime.pointer("/project/id")
            .or_else(|| runtime.pointer("/operator_status/project_id"))
            .cloned()
            .unwrap_or(Value::Null),
        "project_slug": runtime.pointer("/project/slug")
            .or_else(|| runtime.pointer("/operator_status/project_slug"))
            .cloned()
            .unwrap_or(Value::Null),
        "count": visible.len(),
        "total": rows.len(),
        "limit": limit,
        "truncated": rows.len() > visible.len(),
        "checkpoints": visible,
    })
}

fn project_checkpoint_view(checkpoint: &Value) -> Value {
    json!({
        "id": checkpoint.get("id").cloned().unwrap_or(Value::Null),
        "created_at": checkpoint.get("created_at").cloned().unwrap_or(Value::Null),
        "session_id": checkpoint.get("session_id").cloned().unwrap_or(Value::Null),
        "summary": checkpoint.get("summary")
            .and_then(Value::as_str)
            .map(|summary| json!(truncate_display(summary, 240)))
            .unwrap_or(Value::Null),
    })
}

fn print_project_checkpoints(view: &Value) {
    let checkpoints = view["checkpoints"].as_array().cloned().unwrap_or_default();
    if checkpoints.is_empty() {
        ui::success("No project checkpoints.");
        return;
    }
    ui::section("Project Checkpoints");
    ui::blank();
    for checkpoint in checkpoints {
        println!("{}", project_checkpoint_line(&checkpoint));
    }
    if view["truncated"].as_bool().unwrap_or(false) {
        let total = view["total"].as_u64().unwrap_or(0);
        let limit = view["limit"]
            .as_u64()
            .unwrap_or(DEFAULT_CHECKPOINT_LIMIT as u64);
        ui::hint(&format!("Showing newest {limit} of {total} checkpoint(s)."));
    }
}

fn project_checkpoint_line(checkpoint: &Value) -> String {
    let created_at = checkpoint["created_at"]
        .as_i64()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "?".to_string());
    let id = checkpoint["id"].as_str().unwrap_or("checkpoint");
    let id_short = id.chars().take(8).collect::<String>();
    let session = checkpoint["session_id"]
        .as_str()
        .map(|value| format!(" -- session {}", truncate_display(value, 32)))
        .unwrap_or_default();
    let summary = checkpoint["summary"].as_str().unwrap_or("No summary");
    format!("    {created_at} -- {id_short}{session} -- {summary}")
}

fn project_checkpoints_error(body: &Value) -> Option<String> {
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
    fn checkpoints_view_limits_and_omits_raw_state() {
        let runtime = json!({
            "project": {"id": "project-1", "slug": "demo"}
        });
        let body = json!({
            "checkpoints": [
                {
                    "id": "checkpoint-1",
                    "created_at": 42,
                    "session_id": "session-1",
                    "summary": "First checkpoint",
                    "state": {"secret": "do-not-print"}
                },
                {
                    "id": "checkpoint-2",
                    "created_at": 41,
                    "summary": "Second checkpoint",
                    "state": {"workspace": "/Users/example/private"}
                }
            ]
        });

        let view = project_checkpoints_view(&body, &runtime, 1);
        let rendered = serde_json::to_string(&view).unwrap();

        assert_eq!(view["count"], 1);
        assert_eq!(view["total"], 2);
        assert_eq!(view["checkpoints"][0]["id"], "checkpoint-1");
        assert!(view["truncated"].as_bool().unwrap());
        assert!(!rendered.contains("do-not-print"));
        assert!(!rendered.contains("/Users/example/private"));
        assert!(!rendered.contains("state"));
    }

    #[test]
    fn checkpoint_line_includes_short_id_session_and_summary() {
        let checkpoint = json!({
            "id": "abcdef123456",
            "created_at": 42,
            "session_id": "session-123",
            "summary": "Ready to resume verification."
        });

        let line = project_checkpoint_line(&checkpoint);

        assert!(line.contains("abcdef12"));
        assert!(line.contains("session session-123"));
        assert!(line.contains("Ready to resume verification."));
    }
}
