use serde_json::{json, Value};

use crate::{
    daemon_client, daemon_json, require_daemon, truncate_display, ui, ProjectTaskCommands,
    ProjectTaskStatusArg,
};

const DEFAULT_TASK_LIMIT: usize = 20;
const MAX_TASK_LIMIT: usize = 100;

pub(super) fn cmd_project_task(command: ProjectTaskCommands) {
    match command {
        ProjectTaskCommands::List {
            project_id,
            status,
            limit,
            json,
        } => cmd_project_task_list(&project_id, status, limit, json),
        ProjectTaskCommands::Create {
            project_id,
            title,
            description,
            parent_id,
            priority,
            deadline,
            json,
        } => cmd_project_task_create(
            &project_id,
            &title,
            description.as_deref(),
            parent_id.as_deref(),
            priority,
            deadline,
            json,
        ),
        ProjectTaskCommands::Update {
            task_id,
            status,
            title,
            description,
            parent_id,
            clear_parent,
            priority,
            json,
        } => cmd_project_task_update(
            &task_id,
            status,
            title.as_deref(),
            description.as_deref(),
            parent_id.as_deref(),
            clear_parent,
            priority,
            json,
        ),
        ProjectTaskCommands::Delete { task_id, yes, json } => {
            cmd_project_task_delete(&task_id, yes, json)
        }
    }
}

fn cmd_project_task_list(
    project_id: &str,
    status: Option<ProjectTaskStatusArg>,
    limit: usize,
    json: bool,
) {
    let base = require_daemon("project task list");
    let client = daemon_client();
    let resolved_id = match resolve_project_id(&client, &base, project_id) {
        Ok(id) => id,
        Err(error) => {
            ui::error(&format!("Project task list failed: {error}"));
            return;
        }
    };
    let body = daemon_json(
        client
            .get(format!("{base}/api/projects/{resolved_id}/tasks"))
            .send(),
    );
    if let Some(error) = project_task_error(&body) {
        ui::error(&format!("Project task list failed: {error}"));
        return;
    }
    let view = project_task_list_view(&body, status, limit);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&view).unwrap_or_default()
        );
    } else {
        print_project_task_list(&view);
    }
}

fn cmd_project_task_create(
    project_id: &str,
    title: &str,
    description: Option<&str>,
    parent_id: Option<&str>,
    priority: Option<i32>,
    deadline: Option<i64>,
    json: bool,
) {
    let base = require_daemon("project task create");
    let client = daemon_client();
    let resolved_id = match resolve_project_id(&client, &base, project_id) {
        Ok(id) => id,
        Err(error) => {
            ui::error(&format!("Project task create failed: {error}"));
            return;
        }
    };
    let body = daemon_json(
        client
            .post(format!("{base}/api/projects/{resolved_id}/tasks"))
            .json(&project_task_create_body(
                title,
                description,
                parent_id,
                priority,
                deadline,
            ))
            .send(),
    );
    print_project_task_response(&body, json, "Project task created.");
}

#[allow(clippy::too_many_arguments)]
fn cmd_project_task_update(
    task_id: &str,
    status: Option<ProjectTaskStatusArg>,
    title: Option<&str>,
    description: Option<&str>,
    parent_id: Option<&str>,
    clear_parent: bool,
    priority: Option<i32>,
    json: bool,
) {
    let base = require_daemon("project task update");
    let client = daemon_client();
    let body = project_task_update_body(
        status,
        title,
        description,
        parent_id,
        clear_parent,
        priority,
    );
    if body.as_object().map(|obj| obj.is_empty()).unwrap_or(true) {
        ui::error("Project task update failed: provide at least one field to update.");
        return;
    }
    let body = daemon_json(
        client
            .patch(format!("{base}/api/project-tasks/{task_id}"))
            .json(&body)
            .send(),
    );
    print_project_task_response(&body, json, "Project task updated.");
}

fn cmd_project_task_delete(task_id: &str, yes: bool, json: bool) {
    if !yes {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "ok": false,
                    "error": "refusing to delete project task without --yes",
                    "task_id": task_id,
                }))
                .unwrap_or_default()
            );
        } else {
            ui::error("Project task delete failed: rerun with --yes to confirm deletion.");
        }
        return;
    }
    let base = require_daemon("project task delete");
    let client = daemon_client();
    let body = daemon_json(
        client
            .delete(format!("{base}/api/project-tasks/{task_id}"))
            .send(),
    );
    if let Some(error) = project_task_error(&body) {
        ui::error(&format!("Project task delete failed: {error}"));
        return;
    }
    let view = json!({"status": "deleted", "task_id": task_id});
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&view).unwrap_or_default()
        );
    } else {
        ui::success("Project task deleted.");
        println!("Task {task_id} deleted.");
    }
}

fn resolve_project_id(
    client: &reqwest::blocking::Client,
    base: &str,
    project_id: &str,
) -> Result<String, String> {
    let body = daemon_json(
        client
            .get(format!("{base}/api/projects/{project_id}/resume"))
            .send(),
    );
    if let Some(error) = project_task_error(&body) {
        return Err(error);
    }
    body.pointer("/project/id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| "project id missing from resume context".to_string())
}

fn project_task_list_view(
    body: &Value,
    status: Option<ProjectTaskStatusArg>,
    limit: usize,
) -> Value {
    let limit = limit.clamp(1, MAX_TASK_LIMIT);
    let status = status.map(ProjectTaskStatusArg::as_api_str);
    let rows = body["tasks"].as_array().cloned().unwrap_or_default();
    let mut tasks: Vec<Value> = rows
        .iter()
        .filter(|task| {
            status
                .map(|expected| task["status"].as_str() == Some(expected))
                .unwrap_or(true)
        })
        .map(project_task_view)
        .collect();
    let total = tasks.len();
    tasks.truncate(limit);
    json!({
        "status_filter": status,
        "count": tasks.len(),
        "total": total,
        "limit": limit,
        "truncated": total > tasks.len(),
        "tasks": tasks,
    })
}

fn project_task_view(task: &Value) -> Value {
    json!({
        "id": task.get("id").cloned().unwrap_or(Value::Null),
        "project_id": task.get("project_id").cloned().unwrap_or(Value::Null),
        "parent_id": task.get("parent_id").cloned().unwrap_or(Value::Null),
        "title": bounded_task_text(task, "title", 120),
        "status": task.get("status").cloned().unwrap_or(Value::Null),
        "priority": task.get("priority").cloned().unwrap_or(Value::Null),
        "deadline": task.get("deadline").cloned().unwrap_or(Value::Null),
        "completed_at": task.get("completed_at").cloned().unwrap_or(Value::Null),
        "updated_at": task.get("updated_at").cloned().unwrap_or(Value::Null),
    })
}

fn project_task_create_body(
    title: &str,
    description: Option<&str>,
    parent_id: Option<&str>,
    priority: Option<i32>,
    deadline: Option<i64>,
) -> Value {
    let mut body = json!({"title": title.trim()});
    if let Some(description) = clean_arg(description) {
        body["description"] = json!(description);
    }
    if let Some(parent_id) = clean_arg(parent_id) {
        body["parent_id"] = json!(parent_id);
    }
    if let Some(priority) = priority {
        body["priority"] = json!(priority);
    }
    if let Some(deadline) = deadline {
        body["deadline"] = json!(deadline);
    }
    body
}

fn project_task_update_body(
    status: Option<ProjectTaskStatusArg>,
    title: Option<&str>,
    description: Option<&str>,
    parent_id: Option<&str>,
    clear_parent: bool,
    priority: Option<i32>,
) -> Value {
    let mut body = json!({});
    if let Some(status) = status {
        body["status"] = json!(status.as_api_str());
    }
    if let Some(title) = clean_arg(title) {
        body["title"] = json!(title);
    }
    if let Some(description) = clean_arg(description) {
        body["description"] = json!(description);
    }
    if clear_parent {
        body["parent_id"] = Value::Null;
    } else if let Some(parent_id) = clean_arg(parent_id) {
        body["parent_id"] = json!(parent_id);
    }
    if let Some(priority) = priority {
        body["priority"] = json!(priority);
    }
    body
}

fn print_project_task_list(view: &Value) {
    let tasks = view["tasks"].as_array().cloned().unwrap_or_default();
    if tasks.is_empty() {
        ui::success("No project tasks found.");
        return;
    }
    ui::section("Project Tasks");
    ui::blank();
    for task in tasks {
        println!("{}", project_task_line(&task));
    }
    if view["truncated"].as_bool().unwrap_or(false) {
        let total = view["total"].as_u64().unwrap_or(0);
        let limit = view["limit"].as_u64().unwrap_or(DEFAULT_TASK_LIMIT as u64);
        ui::hint(&format!(
            "Showing first {limit} of {total} matching task(s)."
        ));
    }
}

fn print_project_task_response(body: &Value, json: bool, success: &str) {
    if let Some(error) = project_task_error(body) {
        ui::error(&format!("Project task action failed: {error}"));
        return;
    }
    let view = project_task_view(body);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&view).unwrap_or_default()
        );
        return;
    }
    ui::success(success);
    println!("{}", project_task_line(&view));
}

fn project_task_line(task: &Value) -> String {
    let id = task["id"].as_str().unwrap_or("task");
    let id_short = id.chars().take(8).collect::<String>();
    let status = task["status"].as_str().unwrap_or("unknown");
    let title = task["title"].as_str().unwrap_or("Untitled task");
    let priority = task["priority"]
        .as_i64()
        .map(|value| format!(" -- p{value}"))
        .unwrap_or_default();
    format!("    {id_short} -- {status}{priority} -- {title}")
}

fn bounded_task_text(value: &Value, key: &str, max_chars: usize) -> Value {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(|text| json!(truncate_display(text, max_chars)))
        .unwrap_or(Value::Null)
}

fn clean_arg(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn project_task_error(body: &Value) -> Option<String> {
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
    fn task_list_view_filters_limits_and_omits_raw_fields() {
        let body = json!({
            "tasks": [
                {
                    "id": "task-1",
                    "project_id": "project-1",
                    "title": "Build",
                    "description": "secret details",
                    "assignee_agent_id": "agent-secret",
                    "status": "doing",
                    "priority": 2
                },
                {"id": "task-2", "title": "Verify", "status": "done"}
            ]
        });

        let view = project_task_list_view(&body, Some(ProjectTaskStatusArg::Doing), 1);
        let rendered = serde_json::to_string(&view).unwrap();

        assert_eq!(view["count"], 1);
        assert_eq!(view["total"], 1);
        assert_eq!(view["tasks"][0]["id"], "task-1");
        assert!(!rendered.contains("secret details"));
        assert!(!rendered.contains("assignee_agent_id"));
        assert!(!rendered.contains("description"));
    }

    #[test]
    fn task_update_body_handles_parent_clear_and_omits_empty_fields() {
        let body = project_task_update_body(
            Some(ProjectTaskStatusArg::Review),
            Some("  Review it  "),
            Some(""),
            Some("parent-1"),
            true,
            Some(4),
        );

        assert_eq!(body["status"], "review");
        assert_eq!(body["title"], "Review it");
        assert_eq!(body["parent_id"], Value::Null);
        assert_eq!(body["priority"], 4);
        assert!(body.get("description").is_none());
    }

    #[test]
    fn task_line_is_compact() {
        let task = json!({
            "id": "abcdef123456",
            "title": "Verify runtime resume",
            "status": "doing",
            "priority": 3
        });

        let line = project_task_line(&task);

        assert!(line.contains("abcdef12"));
        assert!(line.contains("doing"));
        assert!(line.contains("p3"));
        assert!(line.contains("Verify runtime resume"));
    }
}
