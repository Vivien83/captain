use serde_json::{json, Value};

use super::project_runtime_actions::project_command;
use crate::{daemon_client, daemon_json, require_daemon, truncate_display, ui};

const DEFAULT_CONTEXT_LIMIT: usize = 8;
const MAX_CONTEXT_LIMIT: usize = 50;

pub(super) fn cmd_project_context(project_id: &str, limit: usize, json: bool) {
    let base = require_daemon("project context");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/projects/{project_id}/resume"))
            .send(),
    );
    if let Some(error) = project_context_error(&body) {
        ui::error(&format!("Project context failed: {error}"));
        return;
    }

    let context = project_context_view(&body, limit);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&context).unwrap_or_default()
        );
    } else {
        print_project_context(&context);
    }
}

fn project_context_view(body: &Value, limit: usize) -> Value {
    let limit = limit.clamp(1, MAX_CONTEXT_LIMIT);
    let project = project_context_project_view(&body["project"]);
    let task_rows = body["tasks"].as_array().cloned().unwrap_or_default();
    let goal_rows = body["goals"].as_array().cloned().unwrap_or_default();
    let tasks: Vec<Value> = task_rows
        .iter()
        .take(limit)
        .map(project_task_view)
        .collect();
    let goals: Vec<Value> = goal_rows
        .iter()
        .take(limit)
        .map(project_goal_view)
        .collect();
    let command_id = project_command_id(&project);

    json!({
        "project": project,
        "latest_checkpoint": body.get("latest_checkpoint")
            .filter(|value| !value.is_null())
            .map(project_context_checkpoint_view)
            .unwrap_or(Value::Null),
        "tasks": {
            "count": tasks.len(),
            "total": task_rows.len(),
            "limit": limit,
            "truncated": task_rows.len() > tasks.len(),
            "items": tasks,
        },
        "goals": {
            "count": goals.len(),
            "total": goal_rows.len(),
            "limit": limit,
            "truncated": goal_rows.len() > goals.len(),
            "items": goals,
        },
        "milestone_progress": milestone_progress_view(&body["milestone_progress"]),
        "next_actions": project_context_next_actions(&command_id),
    })
}

fn project_context_project_view(project: &Value) -> Value {
    json!({
        "id": project.get("id").cloned().unwrap_or(Value::Null),
        "slug": project.get("slug").cloned().unwrap_or(Value::Null),
        "name": bounded_str(project, "name", 96),
        "status": project.get("status").cloned().unwrap_or(Value::Null),
        "lifecycle_phase": project.get("lifecycle_phase").cloned().unwrap_or(Value::Null),
        "updated_at": project.get("updated_at").cloned().unwrap_or(Value::Null),
    })
}

fn project_context_checkpoint_view(checkpoint: &Value) -> Value {
    json!({
        "id": checkpoint.get("id").cloned().unwrap_or(Value::Null),
        "created_at": checkpoint.get("created_at").cloned().unwrap_or(Value::Null),
        "session_id": checkpoint.get("session_id").cloned().unwrap_or(Value::Null),
        "summary": bounded_str(checkpoint, "summary", 240),
    })
}

fn project_task_view(task: &Value) -> Value {
    json!({
        "id": task.get("id").cloned().unwrap_or(Value::Null),
        "title": bounded_str(task, "title", 120),
        "status": task.get("status").cloned().unwrap_or(Value::Null),
        "priority": task.get("priority").cloned().unwrap_or(Value::Null),
        "deadline": task.get("deadline").cloned().unwrap_or(Value::Null),
        "completed_at": task.get("completed_at").cloned().unwrap_or(Value::Null),
    })
}

fn project_goal_view(goal: &Value) -> Value {
    json!({
        "id": goal.get("id").cloned().unwrap_or(Value::Null),
        "name": bounded_str(goal, "name", 120),
        "status": goal.get("status").cloned().unwrap_or(Value::Null),
        "consecutive_fails": goal.get("consecutive_fails").cloned().unwrap_or(Value::Null),
        "updated_at": goal.get("updated_at").cloned().unwrap_or(Value::Null),
    })
}

fn milestone_progress_view(progress: &Value) -> Value {
    if progress.is_null() {
        return Value::Null;
    }
    json!({
        "total": progress.get("total").cloned().unwrap_or(Value::Null),
        "completed": progress.get("completed").cloned().unwrap_or(Value::Null),
        "missed": progress.get("missed").cloned().unwrap_or(Value::Null),
        "pct": progress.get("pct").cloned().unwrap_or(Value::Null),
    })
}

fn print_project_context(context: &Value) {
    ui::section("Project Context");
    ui::blank();
    println!("{}", project_context_line(&context["project"]));
    println!(
        "{}",
        milestone_progress_line(&context["milestone_progress"])
    );
    println!("{}", checkpoint_context_line(&context["latest_checkpoint"]));
    print_context_rows("Tasks", &context["tasks"], project_task_line);
    print_context_rows("Goals", &context["goals"], project_goal_line);
    print_context_next_actions(&context["next_actions"]);
}

fn print_context_next_actions(actions: &Value) {
    let actions = actions.as_array().cloned().unwrap_or_default();
    if actions.is_empty() {
        return;
    }
    println!("    Next actions:");
    for action in actions {
        if let Some(action) = action.as_str().filter(|value| !value.is_empty()) {
            println!("      {action}");
        }
    }
}

fn print_context_rows(label: &str, group: &Value, render: fn(&Value) -> String) {
    let rows = group["items"].as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        println!("    {label}: none");
        return;
    }
    println!("    {label}:");
    for row in rows {
        println!("{}", render(&row));
    }
    if group["truncated"].as_bool().unwrap_or(false) {
        let total = group["total"].as_u64().unwrap_or(0);
        let limit = group["limit"]
            .as_u64()
            .unwrap_or(DEFAULT_CONTEXT_LIMIT as u64);
        ui::hint(&format!("Showing newest {limit} of {total} {label}."));
    }
}

fn project_context_line(project: &Value) -> String {
    let slug = project["slug"]
        .as_str()
        .or_else(|| project["id"].as_str())
        .unwrap_or("project");
    let status = project["status"].as_str().unwrap_or("unknown");
    let phase = project["lifecycle_phase"].as_str().unwrap_or("unknown");
    let name = project["name"].as_str().unwrap_or("(no name)");
    format!("    Project {slug} -- {status}/{phase} -- {name}")
}

fn milestone_progress_line(progress: &Value) -> String {
    if progress.is_null() {
        return "    Milestones: none".to_string();
    }
    let total = progress["total"].as_u64().unwrap_or(0);
    let completed = progress["completed"].as_u64().unwrap_or(0);
    let missed = progress["missed"].as_u64().unwrap_or(0);
    let pct = progress["pct"].as_f64().unwrap_or(0.0) * 100.0;
    format!("    Milestones: {completed}/{total} complete -- missed {missed} -- {pct:.0}%")
}

fn checkpoint_context_line(checkpoint: &Value) -> String {
    if checkpoint.is_null() {
        return "    Latest checkpoint: none".to_string();
    }
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
    format!("    Latest checkpoint: {created_at} -- {id_short}{session} -- {summary}")
}

fn project_task_line(task: &Value) -> String {
    let id = task["id"].as_str().unwrap_or("task");
    let id_short = id.chars().take(8).collect::<String>();
    let status = task["status"].as_str().unwrap_or("unknown");
    let title = task["title"].as_str().unwrap_or("Untitled task");
    format!("      {id_short} -- {status} -- {title}")
}

fn project_goal_line(goal: &Value) -> String {
    let id = goal["id"].as_str().unwrap_or("goal");
    let id_short = id.chars().take(8).collect::<String>();
    let status = goal["status"].as_str().unwrap_or("unknown");
    let name = goal["name"].as_str().unwrap_or("Untitled goal");
    format!("      {id_short} -- {status} -- {name}")
}

fn bounded_str(value: &Value, key: &str, max_chars: usize) -> Value {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(|text| json!(truncate_display(text, max_chars)))
        .unwrap_or(Value::Null)
}

fn project_command_id(project: &Value) -> String {
    project["slug"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| project["id"].as_str())
        .unwrap_or("project")
        .to_string()
}

fn project_context_next_actions(command_id: &str) -> Vec<String> {
    vec![
        project_command("replay", command_id),
        project_command("status", command_id),
        project_command("workers", command_id),
        project_command("questions", command_id),
        format!("{} --follow", project_command("timeline", command_id)),
        project_command("checkpoints", command_id),
    ]
}

fn project_context_error(body: &Value) -> Option<String> {
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
    fn context_view_limits_resume_context_and_omits_raw_fields() {
        let body = json!({
            "project": {
                "id": "project-1",
                "slug": "demo",
                "name": "Demo",
                "status": "active",
                "lifecycle_phase": "verify",
                "updated_at": 42,
                "metadata": {"workspace": "/Users/example/private"}
            },
            "latest_checkpoint": {
                "id": "checkpoint-1",
                "created_at": 40,
                "session_id": "session-1",
                "summary": "Ready to verify.",
                "state": {"token": "secret"}
            },
            "tasks": [
                {
                    "id": "task-1",
                    "title": "Verify behavior",
                    "description": "private path /Users/example/private",
                    "status": "doing",
                    "priority": 2
                },
                {"id": "task-2", "title": "Ship", "status": "todo"}
            ],
            "goals": [
                {
                    "id": "goal-1",
                    "name": "Keep checks green",
                    "status": "active",
                    "check_command": "cat /secret/token",
                    "recovery_command": "echo secret"
                },
                {"id": "goal-2", "name": "Deploy", "status": "paused"}
            ],
            "milestone_progress": {"total": 2, "completed": 1, "missed": 0, "pct": 0.5}
        });

        let view = project_context_view(&body, 1);
        let rendered = serde_json::to_string(&view).unwrap();

        assert_eq!(view["tasks"]["count"], 1);
        assert_eq!(view["tasks"]["total"], 2);
        assert_eq!(view["goals"]["count"], 1);
        assert_eq!(view["latest_checkpoint"]["id"], "checkpoint-1");
        assert_eq!(view["next_actions"][0], "captain project replay demo");
        assert_eq!(
            view["next_actions"][4],
            "captain project timeline demo --follow"
        );
        assert!(view["tasks"]["truncated"].as_bool().unwrap());
        assert!(!rendered.contains("metadata"));
        assert!(!rendered.contains("workspace"));
        assert!(!rendered.contains("description"));
        assert!(!rendered.contains("check_command"));
        assert!(!rendered.contains("recovery_command"));
        assert!(!rendered.contains("state"));
        assert!(!rendered.contains("secret"));
    }

    #[test]
    fn context_lines_show_resume_signals() {
        let project = json!({
            "id": "project-1",
            "slug": "demo",
            "name": "Demo",
            "status": "active",
            "lifecycle_phase": "verify"
        });
        let checkpoint = json!({
            "id": "abcdef123456",
            "created_at": 42,
            "session_id": "session-123",
            "summary": "Ready to resume."
        });

        assert!(project_context_line(&project).contains("demo -- active/verify"));
        assert!(checkpoint_context_line(&checkpoint).contains("abcdef12"));
        assert!(milestone_progress_line(&json!({
            "total": 4,
            "completed": 3,
            "missed": 1,
            "pct": 0.75
        }))
        .contains("3/4 complete"));
    }
}
