use serde_json::{json, Value};
use std::collections::BTreeMap;

use super::project_runtime_actions::project_command;
use crate::{daemon_client, daemon_json, require_daemon, truncate_display, ui};

const DEFAULT_PROJECT_LIST_LIMIT: usize = 20;
const MAX_PROJECT_LIST_LIMIT: usize = 100;

pub(super) fn cmd_project_list(include_archived: bool, attention: bool, limit: usize, json: bool) {
    let base = require_daemon("project list");
    let client = daemon_client();
    let mut request = client.get(format!("{base}/api/projects"));
    if include_archived {
        request = request.query(&[("include_archived", "true")]);
    }
    let body = daemon_json(request.send());
    if let Some(error) = project_list_error(&body) {
        ui::error(&format!("Project list failed: {error}"));
        return;
    }
    let view = project_list_view(&body, include_archived, attention, limit);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&view).unwrap_or_default()
        );
    } else {
        print_project_list(&view);
    }
}

fn project_list_view(body: &Value, include_archived: bool, attention: bool, limit: usize) -> Value {
    let limit = limit.clamp(1, MAX_PROJECT_LIST_LIMIT);
    let rows = body["projects"].as_array().cloned().unwrap_or_default();
    let mut projects: Vec<Value> = rows.iter().map(project_list_item).collect();
    if attention {
        projects.retain(|project| project["needs_attention"].as_bool().unwrap_or(false));
    }
    let total = projects.len();
    projects.truncate(limit);
    json!({
        "include_archived": include_archived,
        "attention_only": attention,
        "count": projects.len(),
        "total": total,
        "limit": limit,
        "truncated": total > projects.len(),
        "projects": projects,
    })
}

fn project_list_item(project: &Value) -> Value {
    let runtime = &project["runtime"];
    let state = project_runtime_state(runtime);
    let phase = runtime["current_phase"]
        .as_str()
        .or_else(|| project["lifecycle_phase"].as_str())
        .unwrap_or("observe");
    let id = project["id"].as_str().unwrap_or("?");
    let slug = project["slug"].as_str().unwrap_or("");
    let command_id = if slug.trim().is_empty() { id } else { slug };
    json!({
        "id": id,
        "slug": slug,
        "name": project["name"]
            .as_str()
            .map(|name| truncate_display(name, 80))
            .unwrap_or_else(|| "(no name)".to_string()),
        "status": project["status"].as_str().unwrap_or("unknown"),
        "runtime_state": state,
        "runtime_status": runtime["status"].as_str().unwrap_or("ready"),
        "phase": phase,
        "progress": runtime["progress"].as_u64().unwrap_or(0),
        "workers": worker_counts(runtime),
        "goal_count": project["goal_count"].as_u64().unwrap_or(0),
        "active_goal_count": project["active_goal_count"].as_u64().unwrap_or(0),
        "updated_at": project["updated_at"].clone(),
        "needs_attention": project_needs_attention(&state),
        "next_action": project_command("status", command_id),
    })
}

fn project_runtime_state(runtime: &Value) -> String {
    if has_pending_question(runtime) {
        return "waiting_for_user".to_string();
    }
    if has_pending_tool_request(runtime) {
        return "tool_request_pending".to_string();
    }
    if runtime
        .get("resume_pending")
        .is_some_and(|value| !value.is_null())
    {
        return "resume_ready".to_string();
    }
    match runtime["status"].as_str().unwrap_or("ready") {
        "running" => "running",
        "paused" => "paused",
        "blocked" => "blocked",
        "failed" => "failed",
        "done" => "done",
        _ => "ready",
    }
    .to_string()
}

fn has_pending_question(runtime: &Value) -> bool {
    runtime["user_questions"]
        .as_array()
        .map(|questions| {
            questions.iter().any(|question| {
                let status = question["status"].as_str().unwrap_or("pending");
                let ask_id = question["ask_id"].as_str().unwrap_or("").trim();
                status.eq_ignore_ascii_case("pending") && !ask_id.is_empty()
            })
        })
        .unwrap_or(false)
}

fn has_pending_tool_request(runtime: &Value) -> bool {
    if runtime["worker_results"]
        .as_object()
        .map(|results| {
            results
                .values()
                .any(|result| tool_request_is_pending(&result["tool_request"]))
        })
        .unwrap_or(false)
    {
        return true;
    }
    runtime["workers"]
        .as_array()
        .map(|workers| {
            workers
                .iter()
                .any(|worker| tool_request_is_pending(&worker["tool_request"]))
        })
        .unwrap_or(false)
}

fn tool_request_is_pending(request: &Value) -> bool {
    if request.is_null() {
        return false;
    }
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

fn project_needs_attention(state: &str) -> bool {
    matches!(
        state,
        "waiting_for_user" | "tool_request_pending" | "resume_ready" | "blocked" | "failed"
    )
}

fn worker_counts(runtime: &Value) -> Value {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    if let Some(workers) = runtime["workers"].as_array() {
        for worker in workers {
            let status = worker["status"].as_str().unwrap_or("planned").to_string();
            *counts.entry(status).or_insert(0) += 1;
        }
    }
    json!({
        "total": counts.values().sum::<usize>(),
        "by_status": counts,
    })
}

fn print_project_list(view: &Value) {
    let projects = view["projects"].as_array().cloned().unwrap_or_default();
    if projects.is_empty() {
        ui::success("No projects found.");
        return;
    }
    ui::section("Projects");
    ui::blank();
    for project in projects {
        println!("{}", project_list_line(&project));
    }
    if view["truncated"].as_bool().unwrap_or(false) {
        let total = view["total"].as_u64().unwrap_or(0);
        let limit = view["limit"]
            .as_u64()
            .unwrap_or(DEFAULT_PROJECT_LIST_LIMIT as u64);
        ui::hint(&format!(
            "Showing newest {limit} of {total} matching project(s)."
        ));
    }
}

fn project_list_line(project: &Value) -> String {
    let slug = project["slug"].as_str().unwrap_or("");
    let id = project["id"].as_str().unwrap_or("?");
    let key = if slug.trim().is_empty() { id } else { slug };
    let name = project["name"].as_str().unwrap_or("(no name)");
    let state = project["runtime_state"].as_str().unwrap_or("unknown");
    let phase = project["phase"].as_str().unwrap_or("observe");
    let progress = project["progress"].as_u64().unwrap_or(0);
    let workers = project
        .pointer("/workers/total")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let marker = if project["needs_attention"].as_bool().unwrap_or(false) {
        "attention"
    } else {
        "ok"
    };
    format!(
        "    {key} -- {marker} -- {state}/{phase} -- {progress}% -- workers {workers} -- {name}"
    )
}

fn project_list_error(body: &Value) -> Option<String> {
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
    fn project_list_view_filters_attention_and_sanitizes_metadata() {
        let body = json!({
            "projects": [
                {
                    "id": "project-1",
                    "slug": "alpha",
                    "name": "Alpha",
                    "status": "active",
                    "updated_at": 2,
                    "workspace_path": "/private/project",
                    "metadata": {"secret": "do-not-print"},
                    "goal_count": 1,
                    "active_goal_count": 1,
                    "runtime": {
                        "status": "running",
                        "current_phase": "build",
                        "progress": 40,
                        "workers": [{"status": "blocked"}],
                        "user_questions": [{"ask_id": "ask-1", "status": "pending"}]
                    }
                },
                {
                    "id": "project-2",
                    "slug": "beta",
                    "name": "Beta",
                    "status": "active",
                    "runtime": {"status": "ready", "current_phase": "observe"}
                }
            ]
        });

        let view = project_list_view(&body, false, true, 20);

        assert_eq!(view["count"], 1);
        assert_eq!(view["projects"][0]["slug"], "alpha");
        assert_eq!(view["projects"][0]["runtime_state"], "waiting_for_user");
        assert!(view["projects"][0].get("metadata").is_none());
        assert!(view["projects"][0].get("workspace_path").is_none());
    }

    #[test]
    fn project_list_line_points_to_slug_without_private_paths() {
        let project = json!({
            "id": "project-1",
            "slug": "alpha",
            "name": "Alpha",
            "runtime_state": "resume_ready",
            "phase": "verify",
            "progress": 70,
            "workers": {"total": 3},
            "needs_attention": true,
            "workspace_path": "/private/project"
        });

        let line = project_list_line(&project);

        assert!(line.contains("alpha"));
        assert!(line.contains("attention"));
        assert!(!line.contains("/private/project"));
    }
}
