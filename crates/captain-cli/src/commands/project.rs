use serde_json::{json, Value};

use super::project_checkpoints::cmd_project_checkpoints;
use super::project_context::cmd_project_context;
use super::project_goals::cmd_project_goal;
use super::project_list::cmd_project_list;
use super::project_milestones::cmd_project_milestone;
use super::project_questions::cmd_project_questions;
use super::project_replay::cmd_project_replay;
use super::project_runtime_actions::{project_command, project_runtime_action_commands};
use super::project_runtime_view::project_runtime_response_view;
use super::project_tasks::cmd_project_task;
use super::project_timeline::cmd_project_timeline;
use super::project_workers::cmd_project_workers;
use crate::{
    daemon_client, daemon_json, require_daemon, truncate_display, ui, ProjectCommands,
    ProjectToolDecisionArg,
};

pub(crate) fn cmd_project(command: ProjectCommands) {
    match command {
        ProjectCommands::List {
            include_archived,
            attention,
            limit,
            json,
        } => cmd_project_list(include_archived, attention, limit, json),
        ProjectCommands::Status { project_id, json } => cmd_project_status(&project_id, json),
        ProjectCommands::Workers {
            project_id,
            phase,
            limit,
            json,
        } => cmd_project_workers(&project_id, phase.as_deref(), limit, json),
        ProjectCommands::Questions {
            project_id,
            phase,
            all,
            limit,
            json,
        } => cmd_project_questions(&project_id, phase.as_deref(), all, limit, json),
        ProjectCommands::Replay {
            project_id,
            events,
            workers,
            json,
        } => cmd_project_replay(&project_id, events, workers, json),
        ProjectCommands::Context {
            project_id,
            limit,
            json,
        } => cmd_project_context(&project_id, limit, json),
        ProjectCommands::Task { command } => cmd_project_task(command),
        ProjectCommands::Milestone { command } => cmd_project_milestone(command),
        ProjectCommands::Goal { command } => cmd_project_goal(command),
        ProjectCommands::Timeline {
            project_id,
            limit,
            follow,
            json,
        } => cmd_project_timeline(&project_id, limit, follow, json),
        ProjectCommands::Checkpoints {
            project_id,
            limit,
            json,
        } => cmd_project_checkpoints(&project_id, limit, json),
        ProjectCommands::Archive { project_id, json } => {
            cmd_project_lifecycle(&project_id, "archived", json)
        }
        ProjectCommands::Unarchive { project_id, json } => {
            cmd_project_lifecycle(&project_id, "active", json)
        }
        ProjectCommands::Start { project_id, json } => {
            cmd_project_runtime_control(&project_id, "start", json)
        }
        ProjectCommands::Resume { project_id, json } => {
            cmd_project_runtime_control(&project_id, "resume", json)
        }
        ProjectCommands::Pause { project_id, json } => {
            cmd_project_runtime_control(&project_id, "pause", json)
        }
        ProjectCommands::Takeover { project_id, json } => {
            cmd_project_runtime_control(&project_id, "takeover", json)
        }
        ProjectCommands::Answer {
            project_id,
            ask_id,
            answer,
            json,
        } => cmd_project_answer(&project_id, &ask_id, &answer, json),
        ProjectCommands::ToolRequest {
            project_id,
            decision,
            phase,
            tools,
            reason,
            json,
        } => cmd_project_tool_request(
            &project_id,
            decision,
            phase.as_deref(),
            &tools,
            reason.as_deref(),
            json,
        ),
    }
}

fn cmd_project_status(project_id: &str, json: bool) {
    let base = require_daemon("project status");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/projects/{project_id}/runtime"))
            .send(),
    );
    print_project_response(&body, json, "Project runtime status loaded.");
}

fn cmd_project_lifecycle(project_id: &str, status: &str, json: bool) {
    let base = require_daemon("project lifecycle");
    let client = daemon_client();
    let body = daemon_json(
        client
            .patch(format!("{base}/api/projects/{project_id}"))
            .json(&json!({ "status": status }))
            .send(),
    );
    let message = match status {
        "archived" => "Project archived.",
        "active" => "Project reactivated.",
        _ => "Project lifecycle updated.",
    };
    print_project_lifecycle_response(&body, json, message);
}

fn cmd_project_runtime_control(project_id: &str, action: &str, json: bool) {
    let base = require_daemon("project runtime");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/projects/{project_id}/runtime/{action}"))
            .send(),
    );
    print_project_response(
        &body,
        json,
        &format!("Project runtime {action} request accepted."),
    );
}

fn cmd_project_answer(project_id: &str, ask_id: &str, answer: &str, json: bool) {
    let base = require_daemon("project answer");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/projects/{project_id}/runtime/answer"))
            .json(&json!({
                "ask_id": ask_id,
                "answer": answer,
            }))
            .send(),
    );
    let message = if body["delivered_to_active_worker"]
        .as_bool()
        .unwrap_or(false)
    {
        "Project answer delivered to active worker."
    } else if body["runtime_resume_pending"].as_bool().unwrap_or(false) {
        "Project answer recorded; runtime is ready to resume."
    } else {
        "Project answer recorded."
    };
    print_project_response(&body, json, message);
}

fn cmd_project_tool_request(
    project_id: &str,
    decision: ProjectToolDecisionArg,
    phase: Option<&str>,
    tools: &[String],
    reason: Option<&str>,
    json: bool,
) {
    let base = require_daemon("project tool-request");
    let client = daemon_client();
    let payload = project_tool_request_body(decision, phase, tools, reason);
    let body = daemon_json(
        client
            .post(format!(
                "{base}/api/projects/{project_id}/runtime/tool-request"
            ))
            .json(&payload)
            .send(),
    );
    let message = match decision {
        ProjectToolDecisionArg::Approve => "Project tool request approved.",
        ProjectToolDecisionArg::Deny => "Project tool request denied.",
    };
    print_project_response(&body, json, message);
}

fn project_tool_request_body(
    decision: ProjectToolDecisionArg,
    phase: Option<&str>,
    tools: &[String],
    reason: Option<&str>,
) -> Value {
    let mut body = json!({ "decision": decision.as_api_str() });
    if let Some(phase) = clean_arg(phase) {
        body["phase"] = json!(phase);
    }
    let tools: Vec<String> = tools
        .iter()
        .filter_map(|tool| clean_arg(Some(tool)).map(ToString::to_string))
        .collect();
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    if let Some(reason) = clean_arg(reason) {
        body["reason"] = json!(reason);
    }
    body
}

fn print_project_response(body: &Value, json: bool, success: &str) {
    if json {
        let view = project_runtime_response_view(body, "<project_id>");
        println!(
            "{}",
            serde_json::to_string_pretty(&view).unwrap_or_default()
        );
        return;
    }
    if let Some(error) = project_error(body) {
        ui::error(&format!("Project runtime action failed: {error}"));
        return;
    }
    ui::success(success);
    println!("{}", project_runtime_summary(body));
    for action in project_runtime_action_commands(body, "<project_id>") {
        ui::hint(&format!("Next: {action}"));
    }
}

fn print_project_lifecycle_response(body: &Value, json: bool, success: &str) {
    if let Some(error) = project_error(body) {
        ui::error(&format!("Project lifecycle action failed: {error}"));
        return;
    }

    let view = project_lifecycle_view(body);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&view).unwrap_or_default()
        );
        return;
    }

    ui::success(success);
    println!("{}", project_lifecycle_summary(&view));
    if let Some(action) = view["next_action"]
        .as_str()
        .filter(|value| !value.is_empty())
    {
        ui::hint(&format!("Next: {action}"));
    }
}

fn project_error(body: &Value) -> Option<String> {
    if body["ok"].as_bool() == Some(false) || body.get("error").is_some() {
        let error = body["error"]
            .as_str()
            .or_else(|| body["runtime_error"].as_str())
            .unwrap_or("unknown error");
        return Some(truncate_display(error, 180));
    }
    None
}

fn project_lifecycle_view(body: &Value) -> Value {
    let id = body["id"].as_str().unwrap_or("project");
    let slug = body["slug"].as_str().unwrap_or(id);
    let status = body["status"].as_str().unwrap_or("unknown");
    let command_id = if slug.is_empty() { id } else { slug };
    let next_action = match status {
        "archived" => project_command("unarchive", command_id),
        "active" | "planning" | "paused" | "done" => project_command("status", command_id),
        _ => String::new(),
    };

    json!({
        "id": id,
        "slug": slug,
        "name": body["name"].as_str().unwrap_or(""),
        "status": status,
        "updated_at": body.get("updated_at").cloned().unwrap_or(Value::Null),
        "next_action": next_action,
    })
}

fn project_lifecycle_summary(project: &Value) -> String {
    let slug = project["slug"].as_str().unwrap_or("project");
    let status = project["status"].as_str().unwrap_or("unknown");
    let updated_at = project["updated_at"]
        .as_i64()
        .map(|value| format!(" -- updated_at {value}"))
        .unwrap_or_default();
    format!("Project {slug} -- {status}{updated_at}.")
}

fn project_runtime_summary(body: &Value) -> String {
    let status = &body["operator_status"];
    let project = status["project_slug"]
        .as_str()
        .or_else(|| body.pointer("/project/slug").and_then(Value::as_str))
        .or_else(|| status["project_id"].as_str())
        .or_else(|| body.pointer("/project/id").and_then(Value::as_str))
        .unwrap_or("project");
    let state = status["state"].as_str().unwrap_or("unknown");
    let phase = status["phase"].as_str().unwrap_or("observe");
    let progress = status["progress"]
        .as_u64()
        .or_else(|| body.pointer("/runtime/progress").and_then(Value::as_u64))
        .unwrap_or(0);
    let workers = status.pointer("/workers/total").and_then(Value::as_u64);
    let summary = status["summary"]
        .as_str()
        .unwrap_or("Project runtime updated.");

    match workers {
        Some(total) if total > 0 => format!(
            "Project {project} -- {state} -- {phase} -- progress {progress}% -- workers {total}. {summary}"
        ),
        _ => format!("Project {project} -- {state} -- {phase} -- progress {progress}%. {summary}"),
    }
}

fn clean_arg(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_request_body_omits_empty_optional_fields() {
        let body = project_tool_request_body(
            ProjectToolDecisionArg::Approve,
            Some(" build "),
            &[" shell_exec ".into(), "".into()],
            Some(" "),
        );

        assert_eq!(body["decision"], "approve");
        assert_eq!(body["phase"], "build");
        assert_eq!(body["tools"][0], "shell_exec");
        assert!(body.get("reason").is_none());
    }

    #[test]
    fn runtime_summary_is_operator_safe() {
        let body = json!({
            "answer": "private answer",
            "operator_status": {
                "project_id": "project-1",
                "project_slug": "demo",
                "state": "resume_ready",
                "phase": "build",
                "progress": 42,
                "workers": {"total": 2},
                "summary": "A user answer is stored; phase build is ready to resume."
            }
        });

        let summary = project_runtime_summary(&body);

        assert!(summary.contains("demo"));
        assert!(summary.contains("progress 42%"));
        assert!(!summary.contains("private answer"));
    }

    #[test]
    fn lifecycle_view_is_operator_safe() {
        let body = json!({
            "id": "project-1",
            "slug": "demo",
            "name": "Demo",
            "status": "archived",
            "updated_at": 42,
            "metadata": {
                "workspace": "/Users/example/private",
                "token": "secret"
            }
        });

        let view = project_lifecycle_view(&body);
        let rendered = serde_json::to_string(&view).unwrap();

        assert_eq!(view["next_action"], "captain project unarchive demo");
        assert!(!rendered.contains("private"));
        assert!(!rendered.contains("secret"));
        assert!(!rendered.contains("metadata"));
    }
}
