use serde_json::{json, Value};

use crate::{
    cli_args_project::ProjectGoalCommands, daemon_client, daemon_json, require_daemon,
    truncate_display, ui,
};

const DEFAULT_GOAL_LIMIT: usize = 20;
const MAX_GOAL_LIMIT: usize = 100;

pub(super) fn cmd_project_goal(command: ProjectGoalCommands) {
    match command {
        ProjectGoalCommands::List {
            project_id,
            limit,
            json,
        } => cmd_project_goal_list(&project_id, limit, json),
        ProjectGoalCommands::Create {
            project_id,
            id,
            name,
            description,
            check_command,
            recovery_command,
            interval_secs,
            escalation_threshold,
            max_llm_calls_per_hour,
            json,
        } => cmd_project_goal_create(
            &project_id,
            ProjectGoalWrite {
                id: id.as_deref(),
                name: name.as_deref(),
                description: description.as_deref(),
                check_command: Some(&check_command),
                recovery_command: recovery_command.as_deref(),
                interval_secs,
                escalation_threshold,
                max_llm_calls_per_hour,
            },
            json,
        ),
        ProjectGoalCommands::Update {
            project_id,
            goal_id,
            name,
            description,
            check_command,
            recovery_command,
            interval_secs,
            escalation_threshold,
            max_llm_calls_per_hour,
            json,
        } => cmd_project_goal_update(
            &project_id,
            &goal_id,
            ProjectGoalWrite {
                id: None,
                name: name.as_deref(),
                description: description.as_deref(),
                check_command: check_command.as_deref(),
                recovery_command: recovery_command.as_deref(),
                interval_secs,
                escalation_threshold,
                max_llm_calls_per_hour,
            },
            json,
        ),
        ProjectGoalCommands::Pause {
            project_id,
            goal_id,
            json,
        } => cmd_project_goal_status(&project_id, &goal_id, "pause", json),
        ProjectGoalCommands::Resume {
            project_id,
            goal_id,
            json,
        } => cmd_project_goal_status(&project_id, &goal_id, "resume", json),
        ProjectGoalCommands::Delete {
            project_id,
            goal_id,
            yes,
            json,
        } => cmd_project_goal_delete(&project_id, &goal_id, yes, json),
    }
}

struct ProjectGoalWrite<'a> {
    id: Option<&'a str>,
    name: Option<&'a str>,
    description: Option<&'a str>,
    check_command: Option<&'a str>,
    recovery_command: Option<&'a str>,
    interval_secs: Option<u64>,
    escalation_threshold: Option<u32>,
    max_llm_calls_per_hour: Option<u32>,
}

fn cmd_project_goal_list(project_id: &str, limit: usize, json: bool) {
    let base = require_daemon("project goal list");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/projects/{project_id}/goals"))
            .send(),
    );
    if let Some(error) = project_goal_error(&body) {
        ui::error(&format!("Project goal list failed: {error}"));
        return;
    }
    let view = project_goal_list_view(&body, limit);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&view).unwrap_or_default()
        );
    } else {
        print_project_goal_list(&view);
    }
}

fn cmd_project_goal_create(project_id: &str, write: ProjectGoalWrite<'_>, json: bool) {
    let base = require_daemon("project goal create");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/projects/{project_id}/goals"))
            .json(&project_goal_write_body(write, true))
            .send(),
    );
    print_project_goal_response(&body, json, "Project goal created.");
}

fn cmd_project_goal_update(
    project_id: &str,
    goal_id: &str,
    write: ProjectGoalWrite<'_>,
    json: bool,
) {
    let body = project_goal_write_body(write, false);
    if body.as_object().map(|obj| obj.is_empty()).unwrap_or(true) {
        ui::error("Project goal update failed: provide at least one field to update.");
        return;
    }
    let base = require_daemon("project goal update");
    let client = daemon_client();
    let body = daemon_json(
        client
            .patch(format!("{base}/api/projects/{project_id}/goals/{goal_id}"))
            .json(&body)
            .send(),
    );
    print_project_goal_response(&body, json, "Project goal updated.");
}

fn cmd_project_goal_status(project_id: &str, goal_id: &str, action: &str, json: bool) {
    let base = require_daemon("project goal status");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!(
                "{base}/api/projects/{project_id}/goals/{goal_id}/{action}"
            ))
            .send(),
    );
    let message = match action {
        "pause" => "Project goal paused.",
        "resume" => "Project goal resumed.",
        _ => "Project goal updated.",
    };
    print_project_goal_response(&body, json, message);
}

fn cmd_project_goal_delete(project_id: &str, goal_id: &str, yes: bool, json: bool) {
    if !yes {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "ok": false,
                    "error": "refusing to delete project goal without --yes",
                    "goal_id": goal_id,
                }))
                .unwrap_or_default()
            );
        } else {
            ui::error("Project goal delete failed: rerun with --yes to confirm deletion.");
        }
        return;
    }
    let base = require_daemon("project goal delete");
    let client = daemon_client();
    let body = daemon_json(
        client
            .delete(format!("{base}/api/projects/{project_id}/goals/{goal_id}"))
            .send(),
    );
    if let Some(error) = project_goal_error(&body) {
        ui::error(&format!("Project goal delete failed: {error}"));
        return;
    }
    let view = json!({"status": "deleted", "goal_id": goal_id});
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&view).unwrap_or_default()
        );
    } else {
        ui::success("Project goal deleted.");
        println!("Goal {goal_id} deleted.");
    }
}

fn project_goal_list_view(body: &Value, limit: usize) -> Value {
    let limit = limit.clamp(1, MAX_GOAL_LIMIT);
    let rows = body["goals"].as_array().cloned().unwrap_or_default();
    let mut goals: Vec<Value> = rows.iter().map(project_goal_view).collect();
    let total = goals.len();
    goals.truncate(limit);
    json!({
        "project_id": body.pointer("/project/id").cloned().unwrap_or(Value::Null),
        "project_slug": body.pointer("/project/slug").cloned().unwrap_or(Value::Null),
        "count": goals.len(),
        "total": total,
        "limit": limit,
        "truncated": total > goals.len(),
        "goals": goals,
    })
}

fn project_goal_view(goal: &Value) -> Value {
    json!({
        "id": goal.get("id").cloned().unwrap_or(Value::Null),
        "project_id": goal.get("project_id").cloned().unwrap_or(Value::Null),
        "project_slug": goal.get("project_slug").cloned().unwrap_or(Value::Null),
        "name": bounded_text(goal, "name", 120),
        "status": goal.get("status").cloned().unwrap_or(Value::Null),
        "interval_secs": goal.get("interval_secs").cloned().unwrap_or(Value::Null),
        "escalation_threshold": goal.get("escalation_threshold").cloned().unwrap_or(Value::Null),
        "max_llm_calls_per_hour": goal.get("max_llm_calls_per_hour").cloned().unwrap_or(Value::Null),
        "consecutive_fails": goal.get("consecutive_fails").cloned().unwrap_or(Value::Null),
        "last_check_ts": goal.get("last_check_ts").cloned().unwrap_or(Value::Null),
        "escalated_at": goal.get("escalated_at").cloned().unwrap_or(Value::Null),
        "updated_at": goal.get("updated_at").cloned().unwrap_or(Value::Null),
    })
}

fn project_goal_write_body(write: ProjectGoalWrite<'_>, include_id: bool) -> Value {
    let mut body = json!({});
    if include_id {
        if let Some(id) = clean_arg(write.id) {
            body["id"] = json!(id);
        }
    }
    if let Some(name) = clean_arg(write.name) {
        body["name"] = json!(name);
    }
    if let Some(description) = clean_arg(write.description) {
        body["description"] = json!(description);
    }
    if let Some(check_command) = clean_arg(write.check_command) {
        body["check_command"] = json!(check_command);
    }
    if let Some(recovery_command) = write.recovery_command {
        body["recovery_command"] = json!(recovery_command.trim());
    }
    if let Some(interval_secs) = write.interval_secs {
        body["interval_secs"] = json!(interval_secs);
    }
    if let Some(escalation_threshold) = write.escalation_threshold {
        body["escalation_threshold"] = json!(escalation_threshold);
    }
    if let Some(max_llm_calls_per_hour) = write.max_llm_calls_per_hour {
        body["max_llm_calls_per_hour"] = json!(max_llm_calls_per_hour);
    }
    body
}

fn print_project_goal_list(view: &Value) {
    let goals = view["goals"].as_array().cloned().unwrap_or_default();
    if goals.is_empty() {
        ui::success("No project goals found.");
        return;
    }
    ui::section("Project Goals");
    ui::blank();
    for goal in goals {
        println!("{}", project_goal_line(&goal));
    }
    if view["truncated"].as_bool().unwrap_or(false) {
        let total = view["total"].as_u64().unwrap_or(0);
        let limit = view["limit"].as_u64().unwrap_or(DEFAULT_GOAL_LIMIT as u64);
        ui::hint(&format!("Showing first {limit} of {total} goal(s)."));
    }
}

fn print_project_goal_response(body: &Value, json: bool, success: &str) {
    if let Some(error) = project_goal_error(body) {
        ui::error(&format!("Project goal action failed: {error}"));
        return;
    }
    let view = project_goal_view(body);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&view).unwrap_or_default()
        );
        return;
    }
    ui::success(success);
    println!("{}", project_goal_line(&view));
}

fn project_goal_line(goal: &Value) -> String {
    let id = goal["id"].as_str().unwrap_or("goal");
    let id_short = id.chars().take(8).collect::<String>();
    let status = goal["status"].as_str().unwrap_or("unknown");
    let name = goal["name"].as_str().unwrap_or("Untitled goal");
    let interval = goal["interval_secs"]
        .as_u64()
        .map(|value| format!(" -- every {value}s"))
        .unwrap_or_default();
    let fails = goal["consecutive_fails"]
        .as_u64()
        .map(|value| format!(" -- fails {value}"))
        .unwrap_or_default();
    format!("    {id_short} -- {status}{interval}{fails} -- {name}")
}

fn bounded_text(value: &Value, key: &str, max_chars: usize) -> Value {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(|text| json!(truncate_display(text, max_chars)))
        .unwrap_or(Value::Null)
}

fn clean_arg(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn project_goal_error(body: &Value) -> Option<String> {
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
    fn goal_list_view_limits_and_omits_commands() {
        let body = json!({
            "project": {"id": "project-1", "slug": "demo"},
            "goals": [
                {
                    "id": "goal-1",
                    "project_id": "project-1",
                    "name": "Keep healthy",
                    "description": "private context",
                    "status": "active",
                    "interval_secs": 60,
                    "check_command": "cat /secret/token",
                    "recovery_command": "echo secret",
                    "consecutive_fails": 2
                },
                {"id": "goal-2", "name": "Deploy", "status": "paused"}
            ]
        });

        let view = project_goal_list_view(&body, 1);
        let rendered = serde_json::to_string(&view).unwrap();

        assert_eq!(view["count"], 1);
        assert_eq!(view["total"], 2);
        assert_eq!(view["goals"][0]["id"], "goal-1");
        assert!(view["truncated"].as_bool().unwrap());
        assert!(!rendered.contains("check_command"));
        assert!(!rendered.contains("recovery_command"));
        assert!(!rendered.contains("/secret/token"));
        assert!(!rendered.contains("private context"));
    }

    #[test]
    fn goal_write_body_trims_and_allows_recovery_clear() {
        let body = project_goal_write_body(
            ProjectGoalWrite {
                id: Some(" goal-1 "),
                name: Some(" Keep healthy "),
                description: Some(" "),
                check_command: Some(" cargo test "),
                recovery_command: Some(" "),
                interval_secs: Some(60),
                escalation_threshold: Some(3),
                max_llm_calls_per_hour: Some(5),
            },
            true,
        );

        assert_eq!(body["id"], "goal-1");
        assert_eq!(body["name"], "Keep healthy");
        assert_eq!(body["check_command"], "cargo test");
        assert_eq!(body["recovery_command"], "");
        assert_eq!(body["interval_secs"], 60);
        assert!(body.get("description").is_none());
    }

    #[test]
    fn goal_line_is_compact() {
        let goal = json!({
            "id": "abcdef123456",
            "name": "Keep tests green",
            "status": "active",
            "interval_secs": 300,
            "consecutive_fails": 1
        });

        let line = project_goal_line(&goal);

        assert!(line.contains("abcdef12"));
        assert!(line.contains("active"));
        assert!(line.contains("every 300s"));
        assert!(line.contains("fails 1"));
    }
}
