use serde_json::{json, Value};

use crate::{
    cli_args_project::ProjectMilestoneCommands, daemon_client, daemon_json, require_daemon,
    truncate_display, ui,
};

const DEFAULT_MILESTONE_LIMIT: usize = 20;
const MAX_MILESTONE_LIMIT: usize = 100;

pub(super) fn cmd_project_milestone(command: ProjectMilestoneCommands) {
    match command {
        ProjectMilestoneCommands::List {
            project_id,
            limit,
            json,
        } => cmd_project_milestone_list(&project_id, limit, json),
        ProjectMilestoneCommands::Create {
            project_id,
            name,
            due_date,
            deliverables,
            json,
        } => cmd_project_milestone_create(&project_id, &name, due_date, &deliverables, json),
        ProjectMilestoneCommands::Complete { milestone_id, json } => {
            cmd_project_milestone_complete(&milestone_id, json)
        }
        ProjectMilestoneCommands::Progress { project_id, json } => {
            cmd_project_milestone_progress(&project_id, json)
        }
    }
}

fn cmd_project_milestone_list(project_id: &str, limit: usize, json: bool) {
    let base = require_daemon("project milestone list");
    let client = daemon_client();
    let resolved_id = match resolve_project_id(&client, &base, project_id) {
        Ok(id) => id,
        Err(error) => {
            ui::error(&format!("Project milestone list failed: {error}"));
            return;
        }
    };
    let body = daemon_json(
        client
            .get(format!("{base}/api/projects/{resolved_id}/milestones"))
            .send(),
    );
    if let Some(error) = project_milestone_error(&body) {
        ui::error(&format!("Project milestone list failed: {error}"));
        return;
    }
    let view = project_milestone_list_view(&body, limit);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&view).unwrap_or_default()
        );
    } else {
        print_project_milestone_list(&view);
    }
}

fn cmd_project_milestone_create(
    project_id: &str,
    name: &str,
    due_date: Option<i64>,
    deliverables: &[String],
    json: bool,
) {
    let base = require_daemon("project milestone create");
    let client = daemon_client();
    let resolved_id = match resolve_project_id(&client, &base, project_id) {
        Ok(id) => id,
        Err(error) => {
            ui::error(&format!("Project milestone create failed: {error}"));
            return;
        }
    };
    let body = daemon_json(
        client
            .post(format!("{base}/api/projects/{resolved_id}/milestones"))
            .json(&project_milestone_create_body(name, due_date, deliverables))
            .send(),
    );
    print_project_milestone_response(&body, json, "Project milestone created.");
}

fn cmd_project_milestone_complete(milestone_id: &str, json: bool) {
    let base = require_daemon("project milestone complete");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/milestones/{milestone_id}/complete"))
            .send(),
    );
    print_project_milestone_response(&body, json, "Project milestone completed.");
}

fn cmd_project_milestone_progress(project_id: &str, json: bool) {
    let base = require_daemon("project milestone progress");
    let client = daemon_client();
    let resolved_id = match resolve_project_id(&client, &base, project_id) {
        Ok(id) => id,
        Err(error) => {
            ui::error(&format!("Project milestone progress failed: {error}"));
            return;
        }
    };
    let body = daemon_json(
        client
            .get(format!(
                "{base}/api/projects/{resolved_id}/milestones/progress"
            ))
            .send(),
    );
    if let Some(error) = project_milestone_error(&body) {
        ui::error(&format!("Project milestone progress failed: {error}"));
        return;
    }
    let view = milestone_progress_view(&body);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&view).unwrap_or_default()
        );
    } else {
        ui::section("Project Milestone Progress");
        ui::blank();
        println!("{}", milestone_progress_line(&view));
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
    if let Some(error) = project_milestone_error(&body) {
        return Err(error);
    }
    body.pointer("/project/id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| "project id missing from resume context".to_string())
}

fn project_milestone_list_view(body: &Value, limit: usize) -> Value {
    let limit = limit.clamp(1, MAX_MILESTONE_LIMIT);
    let rows = body["milestones"].as_array().cloned().unwrap_or_default();
    let mut milestones: Vec<Value> = rows.iter().map(project_milestone_view).collect();
    let total = milestones.len();
    milestones.truncate(limit);
    json!({
        "count": milestones.len(),
        "total": total,
        "limit": limit,
        "truncated": total > milestones.len(),
        "milestones": milestones,
    })
}

fn project_milestone_view(milestone: &Value) -> Value {
    json!({
        "id": milestone.get("id").cloned().unwrap_or(Value::Null),
        "project_id": milestone.get("project_id").cloned().unwrap_or(Value::Null),
        "name": bounded_text(milestone, "name", 120),
        "status": milestone.get("status").cloned().unwrap_or(Value::Null),
        "due_date": milestone.get("due_date").cloned().unwrap_or(Value::Null),
        "completed_at": milestone.get("completed_at").cloned().unwrap_or(Value::Null),
        "updated_at": milestone.get("updated_at").cloned().unwrap_or(Value::Null),
        "deliverable_count": milestone.get("deliverables")
            .and_then(Value::as_array)
            .map(|items| json!(items.len()))
            .unwrap_or(Value::Null),
    })
}

fn project_milestone_create_body(
    name: &str,
    due_date: Option<i64>,
    deliverables: &[String],
) -> Value {
    let mut body = json!({"name": name.trim()});
    if let Some(due_date) = due_date {
        body["due_date"] = json!(due_date);
    }
    let deliverables: Vec<String> = deliverables
        .iter()
        .filter_map(|item| clean_arg(Some(item)).map(ToString::to_string))
        .collect();
    if !deliverables.is_empty() {
        body["deliverables"] = json!(deliverables);
    }
    body
}

fn milestone_progress_view(progress: &Value) -> Value {
    json!({
        "total": progress.get("total").cloned().unwrap_or(Value::Null),
        "completed": progress.get("completed").cloned().unwrap_or(Value::Null),
        "missed": progress.get("missed").cloned().unwrap_or(Value::Null),
        "pct": progress.get("pct").cloned().unwrap_or(Value::Null),
    })
}

fn print_project_milestone_list(view: &Value) {
    let milestones = view["milestones"].as_array().cloned().unwrap_or_default();
    if milestones.is_empty() {
        ui::success("No project milestones found.");
        return;
    }
    ui::section("Project Milestones");
    ui::blank();
    for milestone in milestones {
        println!("{}", project_milestone_line(&milestone));
    }
    if view["truncated"].as_bool().unwrap_or(false) {
        let total = view["total"].as_u64().unwrap_or(0);
        let limit = view["limit"]
            .as_u64()
            .unwrap_or(DEFAULT_MILESTONE_LIMIT as u64);
        ui::hint(&format!("Showing first {limit} of {total} milestone(s)."));
    }
}

fn print_project_milestone_response(body: &Value, json: bool, success: &str) {
    if let Some(error) = project_milestone_error(body) {
        ui::error(&format!("Project milestone action failed: {error}"));
        return;
    }
    let view = project_milestone_view(body);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&view).unwrap_or_default()
        );
        return;
    }
    ui::success(success);
    println!("{}", project_milestone_line(&view));
}

fn project_milestone_line(milestone: &Value) -> String {
    let id = milestone["id"].as_str().unwrap_or("milestone");
    let id_short = id.chars().take(8).collect::<String>();
    let status = milestone["status"].as_str().unwrap_or("unknown");
    let name = milestone["name"].as_str().unwrap_or("Untitled milestone");
    let due = milestone["due_date"]
        .as_i64()
        .map(|value| format!(" -- due {value}"))
        .unwrap_or_default();
    format!("    {id_short} -- {status}{due} -- {name}")
}

fn milestone_progress_line(progress: &Value) -> String {
    let total = progress["total"].as_u64().unwrap_or(0);
    let completed = progress["completed"].as_u64().unwrap_or(0);
    let missed = progress["missed"].as_u64().unwrap_or(0);
    let pct = progress["pct"].as_f64().unwrap_or(0.0) * 100.0;
    format!("    {completed}/{total} complete -- missed {missed} -- {pct:.0}%")
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

fn project_milestone_error(body: &Value) -> Option<String> {
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
    fn milestone_list_view_limits_and_omits_deliverables() {
        let body = json!({
            "milestones": [
                {
                    "id": "milestone-1",
                    "project_id": "project-1",
                    "name": "Verify",
                    "status": "in_progress",
                    "due_date": 42,
                    "deliverables": ["secret path /Users/example/private"]
                },
                {"id": "milestone-2", "name": "Ship", "status": "upcoming"}
            ]
        });

        let view = project_milestone_list_view(&body, 1);
        let rendered = serde_json::to_string(&view).unwrap();

        assert_eq!(view["count"], 1);
        assert_eq!(view["total"], 2);
        assert_eq!(view["milestones"][0]["deliverable_count"], 1);
        assert!(view["truncated"].as_bool().unwrap());
        assert!(!rendered.contains("secret path"));
        assert!(!rendered.contains("deliverables"));
    }

    #[test]
    fn milestone_create_body_omits_empty_deliverables() {
        let body = project_milestone_create_body(
            "  Verify release  ",
            Some(42),
            &[" docs ".into(), " ".into()],
        );

        assert_eq!(body["name"], "Verify release");
        assert_eq!(body["due_date"], 42);
        assert_eq!(body["deliverables"][0], "docs");
        assert_eq!(body["deliverables"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn milestone_lines_are_compact() {
        let milestone = json!({
            "id": "abcdef123456",
            "name": "Verify runtime",
            "status": "in_progress",
            "due_date": 42
        });
        let progress = json!({"total": 4, "completed": 3, "missed": 1, "pct": 0.75});

        assert!(project_milestone_line(&milestone).contains("abcdef12"));
        assert!(project_milestone_line(&milestone).contains("due 42"));
        assert!(milestone_progress_line(&progress).contains("3/4 complete"));
    }
}
