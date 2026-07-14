use serde_json::{json, Value};

use super::project_worker_window::select_priority_recent_workers;
use crate::{daemon_client, daemon_json, require_daemon, truncate_display, ui};

const DEFAULT_WORKER_LIMIT: usize = 20;
const MAX_WORKER_LIMIT: usize = 50;
const MAX_TOOL_NAMES: usize = 12;

pub(super) fn cmd_project_workers(project_id: &str, phase: Option<&str>, limit: usize, json: bool) {
    let base = require_daemon("project workers");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/projects/{project_id}/runtime"))
            .send(),
    );
    if let Some(error) = project_workers_error(&body) {
        ui::error(&format!("Project workers failed: {error}"));
        return;
    }
    let workers = project_workers_view(&body, phase, limit);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&workers).unwrap_or_default()
        );
    } else {
        print_project_workers(&workers);
    }
}

fn project_workers_view(body: &Value, phase: Option<&str>, limit: usize) -> Value {
    let limit = limit.clamp(1, MAX_WORKER_LIMIT);
    let phase_filter = phase.map(str::trim).filter(|value| !value.is_empty());
    let worker_rows = body
        .pointer("/runtime/workers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let results = body
        .pointer("/runtime/worker_results")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let filtered: Vec<&Value> = worker_rows
        .iter()
        .filter(|worker| worker_matches_phase(worker, phase_filter))
        .collect();
    let selected = select_priority_recent_workers(&filtered, limit);
    let workers: Vec<Value> = selected
        .into_iter()
        .map(|worker| project_worker_view(worker, &results))
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
        "state": body.pointer("/operator_status/state").cloned().unwrap_or(Value::Null),
        "phase": body.pointer("/operator_status/phase").cloned().unwrap_or(Value::Null),
        "progress": body.pointer("/operator_status/progress").cloned().unwrap_or(Value::Null),
        "worker_counts": body.pointer("/operator_status/workers")
            .cloned()
            .unwrap_or_else(|| json!({ "total": worker_rows.len() })),
        "phase_filter": phase_filter.unwrap_or(""),
        "count": workers.len(),
        "total": filtered.len(),
        "limit": limit,
        "truncated": filtered.len() > workers.len(),
        "workers": workers,
    })
}

fn worker_matches_phase(worker: &Value, phase: Option<&str>) -> bool {
    let Some(phase) = phase else {
        return true;
    };
    worker.get("phase").and_then(Value::as_str) == Some(phase)
}

fn project_worker_view(worker: &Value, results: &Value) -> Value {
    let phase = worker.get("phase").and_then(Value::as_str).unwrap_or("");
    let result = results.get(phase).unwrap_or(&Value::Null);
    let (tools, total_tools) = worker_tool_names(worker);
    json!({
        "id": worker.get("id").cloned().unwrap_or(Value::Null),
        "role": bounded_str(worker, "role", 80),
        "phase": worker.get("phase").cloned().unwrap_or(Value::Null),
        "status": worker.get("status").cloned().unwrap_or(Value::Null),
        "mode": worker.get("mode").cloned().unwrap_or(Value::Null),
        "agent_id": worker.get("agent_id").cloned().unwrap_or(Value::Null),
        "authorized_tools_count": total_tools,
        "authorized_tools": tools,
        "iterations": worker.get("iterations").cloned().unwrap_or(Value::Null),
        "tool_calls": worker.get("tool_calls").cloned().unwrap_or(Value::Null),
        "cost_usd": worker.get("cost_usd").cloned().unwrap_or(Value::Null),
        "started_at": worker.get("started_at").cloned().unwrap_or(Value::Null),
        "completed_at": worker.get("completed_at").cloned().unwrap_or(Value::Null),
        "stopped_at": worker.get("stopped_at").cloned().unwrap_or(Value::Null),
        "cleanup_status": worker.get("cleanup_status").cloned().unwrap_or(Value::Null),
        "recovered_from_stale_run": worker.get("recovered_from_stale_run").cloned().unwrap_or(Value::Null),
        "summary": worker_summary(worker, result),
        "tool_request": worker_tool_request(worker, result),
    })
}

fn bounded_str(source: &Value, key: &str, max_chars: usize) -> Value {
    source
        .get(key)
        .and_then(Value::as_str)
        .map(|value| json!(truncate_display(value, max_chars)))
        .unwrap_or(Value::Null)
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

fn worker_tool_names(worker: &Value) -> (Vec<Value>, usize) {
    let Some(tools) = worker.get("authorized_tools").and_then(Value::as_array) else {
        return (Vec::new(), 0);
    };
    let names: Vec<Value> = tools
        .iter()
        .filter_map(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .take(MAX_TOOL_NAMES)
        .map(|name| json!(truncate_display(name, 64)))
        .collect();
    (names, tools.len())
}

fn request_tool_names(request: &Value) -> Vec<Value> {
    request
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .take(MAX_TOOL_NAMES)
                .map(|name| json!(truncate_display(name, 64)))
                .collect()
        })
        .unwrap_or_default()
}

fn print_project_workers(workers: &Value) {
    let rows = workers["workers"].as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        ui::success("No project runtime workers.");
        return;
    }
    let project = workers["project_slug"]
        .as_str()
        .or_else(|| workers["project_id"].as_str())
        .unwrap_or("project");
    let state = workers["state"].as_str().unwrap_or("unknown");
    let phase = workers["phase"].as_str().unwrap_or("unknown");
    let progress = workers["progress"].as_u64().unwrap_or(0);
    ui::section("Project Workers");
    ui::blank();
    println!("    Project {project} -- {state}/{phase} -- progress {progress}%");
    for worker in rows {
        println!("{}", project_worker_line(&worker));
    }
    if workers["truncated"].as_bool().unwrap_or(false) {
        let total = workers["total"].as_u64().unwrap_or(0);
        let limit = workers["limit"]
            .as_u64()
            .unwrap_or(DEFAULT_WORKER_LIMIT as u64);
        ui::hint(&format!("Showing first {limit} of {total} worker(s)."));
    }
}

fn project_worker_line(worker: &Value) -> String {
    let phase = worker["phase"].as_str().unwrap_or("?");
    let status = worker["status"].as_str().unwrap_or("unknown");
    let role = worker["role"].as_str().unwrap_or("worker");
    let agent = worker["agent_id"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(|value| format!(" -- agent {}", truncate_display(value, 24)))
        .unwrap_or_default();
    let tools = worker["authorized_tools_count"].as_u64().unwrap_or(0);
    let tool_calls = worker["tool_calls"]
        .as_u64()
        .map(|count| format!(" -- tool_calls {count}"))
        .unwrap_or_default();
    let summary = worker["summary"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(|value| format!(" -- {value}"))
        .unwrap_or_default();
    let request = worker_tool_request_line(&worker["tool_request"]);
    format!(
        "    {phase} -- {status} -- {role}{agent} -- tools {tools}{tool_calls}{request}{summary}"
    )
}

fn worker_tool_request_line(request: &Value) -> String {
    let Some(tools) = request.get("tools").and_then(Value::as_array) else {
        return String::new();
    };
    if tools.is_empty() {
        return String::new();
    }
    let names = tools
        .iter()
        .filter_map(Value::as_str)
        .take(4)
        .collect::<Vec<_>>()
        .join(", ");
    format!(" -- needs {names}")
}

fn project_workers_error(body: &Value) -> Option<String> {
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
    fn workers_view_filters_limits_and_omits_raw_runtime_fields() {
        let body = json!({
            "project": {"id": "project-1", "slug": "demo"},
            "operator_status": {
                "project_id": "project-1",
                "project_slug": "demo",
                "state": "running",
                "phase": "build",
                "progress": 42,
                "workers": {"total": 2, "by_status": {"running": 1, "planned": 1}}
            },
            "runtime": {
                "workers": [
                    {
                        "id": "demo-build",
                        "role": "Builder",
                        "phase": "build",
                        "status": "running",
                        "mode": "write",
                        "agent_id": "agent-1234567890",
                        "task": "raw private task do-not-print",
                        "depends_on": ["secret-dependency"],
                        "prompt": "raw prompt do-not-print",
                        "authorized_tools": ["shell_exec", "file_read"],
                        "summary": "STATUS: running\nSUMMARY: editing focused files",
                        "tool_calls": 2,
                        "data": {"secret": "do-not-print"}
                    },
                    {
                        "id": "demo-verify",
                        "role": "Verifier",
                        "phase": "verify",
                        "status": "planned",
                        "task": "verify secret do-not-print"
                    }
                ],
                "worker_results": {
                    "build": {
                        "summary": "fallback raw do-not-print"
                    }
                }
            }
        });

        let view = project_workers_view(&body, Some("build"), 1);
        let rendered = serde_json::to_string(&view).unwrap();

        assert_eq!(view["count"], 1);
        assert_eq!(view["total"], 1);
        assert_eq!(view["workers"][0]["phase"], "build");
        assert_eq!(view["workers"][0]["authorized_tools_count"], 2);
        assert!(!rendered.contains("raw private task"));
        assert!(!rendered.contains("secret-dependency"));
        assert!(!rendered.contains("raw prompt"));
        assert!(!rendered.contains("\"data\""));
        assert!(!rendered.contains("fallback raw"));
    }

    #[test]
    fn worker_line_includes_request_without_raw_reason() {
        let worker = json!({
            "phase": "verify",
            "status": "blocked",
            "role": "Verifier",
            "agent_id": "agent-abcdef1234567890",
            "authorized_tools_count": 3,
            "tool_calls": 4,
            "summary": "STATUS: blocked",
            "tool_request": {
                "tools": ["shell_exec", "cargo"],
                "reason": "Private detail should stay JSON-only."
            }
        });

        let line = project_worker_line(&worker);
        let projected = worker_tool_request(&worker, &Value::Null);
        let rendered = serde_json::to_string(&projected).unwrap();

        assert!(line.contains("verify -- blocked -- Verifier"));
        assert!(line.contains("needs shell_exec, cargo"));
        assert!(!line.contains("Private detail"));
        assert!(!rendered.contains("Private detail"));
    }

    #[test]
    fn workers_view_limit_keeps_actionable_and_recent_tail() {
        let mut workers = vec![json!({
            "id": "worker-running-old",
            "role": "Builder",
            "phase": "build",
            "status": "running"
        })];
        workers.extend((0..12).map(|idx| {
            json!({
                "id": format!("worker-done-{idx}"),
                "role": "Worker",
                "phase": "verify",
                "status": "done"
            })
        }));
        let body = json!({
            "project": {"id": "project-1", "slug": "demo"},
            "runtime": {"workers": workers}
        });

        let view = project_workers_view(&body, None, 4);
        let ids = view["workers"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|worker| worker["id"].as_str())
            .collect::<Vec<_>>();

        assert_eq!(view["count"], 4);
        assert_eq!(view["total"], 13);
        assert_eq!(view["truncated"], true);
        assert!(ids.contains(&"worker-running-old"));
        assert!(!ids.contains(&"worker-done-4"));
        assert!(ids.contains(&"worker-done-11"));
    }
}
