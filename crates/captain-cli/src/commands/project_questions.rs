use serde_json::{json, Value};

use super::project_runtime_actions::project_answer_command;
use crate::{daemon_client, daemon_json, require_daemon, truncate_display, ui};

const DEFAULT_QUESTION_LIMIT: usize = 20;
const MAX_QUESTION_LIMIT: usize = 50;
const MAX_OPTIONS: usize = 6;

pub(super) fn cmd_project_questions(
    project_id: &str,
    phase: Option<&str>,
    all: bool,
    limit: usize,
    json: bool,
) {
    let base = require_daemon("project questions");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/projects/{project_id}/runtime"))
            .send(),
    );
    if let Some(error) = project_questions_error(&body) {
        ui::error(&format!("Project questions failed: {error}"));
        return;
    }
    let questions = project_questions_view(&body, project_id, phase, all, limit);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&questions).unwrap_or_default()
        );
    } else {
        print_project_questions(&questions);
    }
}

fn project_questions_view(
    body: &Value,
    command_project_id: &str,
    phase: Option<&str>,
    all: bool,
    limit: usize,
) -> Value {
    let limit = limit.clamp(1, MAX_QUESTION_LIMIT);
    let phase_filter = phase.map(str::trim).filter(|value| !value.is_empty());
    let rows = body
        .pointer("/runtime/user_questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let filtered: Vec<&Value> = rows
        .iter()
        .filter(|question| question_matches(question, phase_filter, all))
        .collect();
    let questions: Vec<Value> = filtered
        .iter()
        .take(limit)
        .map(|question| project_question_view(question, command_project_id))
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
        "phase_filter": phase_filter.unwrap_or(""),
        "include_all": all,
        "count": questions.len(),
        "total": filtered.len(),
        "limit": limit,
        "truncated": filtered.len() > questions.len(),
        "questions": questions,
    })
}

fn question_matches(question: &Value, phase: Option<&str>, all: bool) -> bool {
    if let Some(phase) = phase {
        if question.get("phase").and_then(Value::as_str) != Some(phase) {
            return false;
        }
    }
    all || question_status(question).eq_ignore_ascii_case("pending")
}

fn project_question_view(question: &Value, command_project_id: &str) -> Value {
    let ask_id = question.get("ask_id").and_then(Value::as_str).unwrap_or("");
    let status = question_status(question);
    let next_action = if status.eq_ignore_ascii_case("pending") && !ask_id.is_empty() {
        project_answer_command(command_project_id, ask_id)
    } else {
        String::new()
    };
    json!({
        "ask_id": bounded_str(question, "ask_id", 120),
        "phase": bounded_str(question, "phase", 80),
        "worker_role": bounded_str(question, "worker_role", 80),
        "status": status,
        "delivery": bounded_str(question, "delivery", 120),
        "question": bounded_str(question, "question", 360),
        "options": question_options(question),
        "created_at": question.get("created_at").cloned().unwrap_or(Value::Null),
        "updated_at": question.get("updated_at").cloned().unwrap_or(Value::Null),
        "answered_at": question.get("answered_at").cloned().unwrap_or(Value::Null),
        "closed_at": question.get("closed_at").cloned().unwrap_or(Value::Null),
        "next_action": next_action,
    })
}

fn question_status(question: &Value) -> String {
    question
        .get("status")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("pending")
        .to_string()
}

fn bounded_str(source: &Value, key: &str, max_chars: usize) -> Value {
    source
        .get(key)
        .and_then(Value::as_str)
        .map(|value| json!(truncate_display(value, max_chars)))
        .unwrap_or(Value::Null)
}

fn question_options(question: &Value) -> Vec<Value> {
    question
        .get("options")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(Value::as_str)
                .filter(|option| !option.trim().is_empty())
                .take(MAX_OPTIONS)
                .map(|option| json!(truncate_display(option, 120)))
                .collect()
        })
        .unwrap_or_default()
}

fn print_project_questions(questions: &Value) {
    let rows = questions["questions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if rows.is_empty() {
        ui::success("No matching project questions.");
        return;
    }
    let project = questions["project_slug"]
        .as_str()
        .or_else(|| questions["project_id"].as_str())
        .unwrap_or("project");
    let state = questions["state"].as_str().unwrap_or("unknown");
    let phase = questions["phase"].as_str().unwrap_or("unknown");
    ui::section("Project Questions");
    ui::blank();
    println!("    Project {project} -- {state}/{phase}");
    for question in rows {
        println!("{}", project_question_line(&question));
        for option in question_options_lines(&question) {
            println!("{option}");
        }
        if let Some(next) = question["next_action"]
            .as_str()
            .filter(|value| !value.is_empty())
        {
            println!("      next: {next}");
        }
    }
    if questions["truncated"].as_bool().unwrap_or(false) {
        let total = questions["total"].as_u64().unwrap_or(0);
        let limit = questions["limit"]
            .as_u64()
            .unwrap_or(DEFAULT_QUESTION_LIMIT as u64);
        ui::hint(&format!("Showing first {limit} of {total} question(s)."));
    }
}

fn project_question_line(question: &Value) -> String {
    let ask_id = question["ask_id"].as_str().unwrap_or("ask");
    let id_short = ask_id.chars().take(8).collect::<String>();
    let phase = question["phase"].as_str().unwrap_or("?");
    let status = question["status"].as_str().unwrap_or("pending");
    let role = question["worker_role"].as_str().unwrap_or("worker");
    let text = question["question"]
        .as_str()
        .unwrap_or("(no question text)");
    format!("    {id_short} -- {phase}/{status} -- {role}: {text}")
}

fn question_options_lines(question: &Value) -> Vec<String> {
    question["options"]
        .as_array()
        .map(|options| {
            options
                .iter()
                .enumerate()
                .filter_map(|(idx, option)| {
                    option
                        .as_str()
                        .map(|text| format!("      [{}] {text}", idx + 1))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn project_questions_error(body: &Value) -> Option<String> {
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
    fn questions_view_defaults_to_pending_and_omits_answers() {
        let body = json!({
            "project": {"id": "project-1", "slug": "demo"},
            "operator_status": {"project_id": "project-1", "project_slug": "demo", "state": "waiting_for_user", "phase": "build"},
            "runtime": {
                "user_questions": [
                    {
                        "ask_id": "ask-pending",
                        "run_id": "run-secret",
                        "phase": "build",
                        "worker_id": "worker-build-secret",
                        "agent_id": "agent-secret",
                        "worker_role": "Builder",
                        "question": "Which path should I take?",
                        "options": ["Simple", "Complex"],
                        "status": "pending",
                        "delivery": "waiting_for_user"
                    },
                    {
                        "ask_id": "ask-answered",
                        "phase": "verify",
                        "worker_role": "Verifier",
                        "question": "Already answered?",
                        "answer": "private answer do-not-print",
                        "status": "answered"
                    }
                ]
            }
        });

        let view = project_questions_view(&body, "demo", None, false, 20);
        let rendered = serde_json::to_string(&view).unwrap();

        assert_eq!(view["count"], 1);
        assert_eq!(view["questions"][0]["ask_id"], "ask-pending");
        assert_eq!(
            view["questions"][0]["next_action"],
            "captain project answer demo --ask-id ask-pending --answer \"...\""
        );
        assert!(!rendered.contains("private answer"));
        assert!(!rendered.contains("agent-secret"));
        assert!(!rendered.contains("run-secret"));
        assert!(!rendered.contains("worker-build-secret"));
        assert!(!rendered.contains("\"agent_id\""));
        assert!(!rendered.contains("\"run_id\""));
        assert!(!rendered.contains("\"worker_id\""));
    }

    #[test]
    fn questions_view_can_include_all_without_echoing_answer() {
        let body = json!({
            "runtime": {
                "user_questions": [{
                    "ask_id": "ask-answered",
                    "phase": "verify",
                    "worker_role": "Verifier",
                    "question": "Already answered?",
                    "answer": "private answer do-not-print",
                    "status": "answered",
                    "answered_at": "2026-05-23T10:00:00Z"
                }]
            }
        });

        let view = project_questions_view(&body, "project-1", None, true, 20);
        let rendered = serde_json::to_string(&view).unwrap();

        assert_eq!(view["count"], 1);
        assert_eq!(view["questions"][0]["status"], "answered");
        assert!(view["questions"][0]["next_action"]
            .as_str()
            .unwrap()
            .is_empty());
        assert!(!rendered.contains("private answer"));
    }

    #[test]
    fn question_line_is_compact_with_options() {
        let question = json!({
            "ask_id": "ask-abcdef",
            "phase": "build",
            "worker_role": "Builder",
            "status": "pending",
            "question": "Pick one",
            "options": ["A", "B"]
        });

        let line = project_question_line(&question);
        let options = question_options_lines(&question);

        assert!(line.contains("ask-abcd -- build/pending -- Builder: Pick one"));
        assert_eq!(options, vec!["      [1] A", "      [2] B"]);
    }
}
