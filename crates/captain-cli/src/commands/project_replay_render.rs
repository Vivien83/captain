use serde_json::Value;

use crate::ui;

const DEFAULT_REPLAY_EVENTS: u64 = 20;
const DEFAULT_REPLAY_WORKERS: u64 = 8;
const DEFAULT_REPLAY_QUESTIONS: u64 = 6;

pub(super) fn print_project_replay(replay: &Value) {
    let project = replay["project_slug"]
        .as_str()
        .or_else(|| replay["project_id"].as_str())
        .unwrap_or("project");
    let state = replay["state"].as_str().unwrap_or("unknown");
    let phase = replay["phase"].as_str().unwrap_or("unknown");
    let progress = replay["progress"].as_u64().unwrap_or(0);
    ui::section("Project Replay");
    ui::blank();
    println!("    Project {project} -- {state}/{phase} -- progress {progress}%");
    println!("{}", replay_transcript_line(replay));
    print_replay_workers(&replay["workers"]);
    print_replay_questions(&replay["pending_questions"]);
    print_replay_events(&replay["events"]);
    for action in replay["next_actions"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        if let Some(action) = action.as_str() {
            ui::hint(&format!("Next: {action}"));
        }
    }
}

fn replay_transcript_line(replay: &Value) -> String {
    let session = replay["session_id"].as_str().unwrap_or("unknown-session");
    let count = replay
        .pointer("/transcript/count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let stored = replay
        .pointer("/transcript/stored_count")
        .and_then(Value::as_u64)
        .unwrap_or(count);
    let pending = replay
        .pointer("/pending_questions/total")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    format!("    Transcript {session} -- events {count}/{stored} -- pending_questions {pending}")
}

fn print_replay_workers(group: &Value) {
    let workers = group["items"].as_array().cloned().unwrap_or_default();
    if workers.is_empty() {
        println!("    Workers: none");
        return;
    }
    println!("    Workers:");
    for worker in workers {
        println!("{}", replay_worker_line(&worker));
    }
    if group["truncated"].as_bool().unwrap_or(false) {
        let total = group["total"].as_u64().unwrap_or(0);
        let limit = group["limit"].as_u64().unwrap_or(DEFAULT_REPLAY_WORKERS);
        ui::hint(&format!(
            "Showing priority/recent {limit} of {total} worker(s)."
        ));
    }
}

pub(super) fn replay_worker_line(worker: &Value) -> String {
    let phase = worker["phase"].as_str().unwrap_or("?");
    let status = worker["status"].as_str().unwrap_or("unknown");
    let role = worker["role"].as_str().unwrap_or("worker");
    let tool_calls = worker["tool_calls"]
        .as_u64()
        .map(|count| format!(" -- tool_calls {count}"))
        .unwrap_or_default();
    let cost = replay_worker_cost(worker);
    let decisions = replay_tool_decisions_line(worker);
    let request = replay_tool_request_line(&worker["tool_request"]);
    let summary = worker["summary"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(|value| format!(" -- {value}"))
        .unwrap_or_default();
    format!("      {phase} -- {status} -- {role}{tool_calls}{cost}{decisions}{request}{summary}")
}

fn replay_worker_cost(worker: &Value) -> String {
    worker["cost_usd"]
        .as_f64()
        .map(|cost| format!(" -- cost ${cost:.4}"))
        .unwrap_or_default()
}

fn replay_tool_decisions_line(worker: &Value) -> String {
    let decisions = worker["tool_decisions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if decisions.is_empty() {
        return String::new();
    }
    let text = decisions
        .iter()
        .take(3)
        .filter_map(replay_tool_decision_fragment)
        .collect::<Vec<_>>()
        .join("; ");
    if text.is_empty() {
        String::new()
    } else {
        format!(" -- decisions {text}")
    }
}

fn replay_tool_decision_fragment(decision: &Value) -> Option<String> {
    let tool = decision["tool"].as_str()?.trim();
    if tool.is_empty() {
        return None;
    }
    let status = decision["status"].as_str().unwrap_or("unknown");
    let reason = decision["reason"].as_str().unwrap_or("no reason recorded");
    let duration = decision["duration_ms"]
        .as_u64()
        .map(|ms| format!(", {ms}ms"))
        .unwrap_or_default();
    Some(format!("{tool} [{status}{duration}]: {reason}"))
}

fn replay_tool_request_line(request: &Value) -> String {
    let Some(tools) = request.get("tools").and_then(Value::as_array) else {
        return String::new();
    };
    let names = tools
        .iter()
        .filter_map(Value::as_str)
        .take(4)
        .collect::<Vec<_>>()
        .join(", ");
    if names.is_empty() {
        String::new()
    } else {
        format!(" -- needs {names}")
    }
}

fn print_replay_questions(group: &Value) {
    let questions = group["items"].as_array().cloned().unwrap_or_default();
    if questions.is_empty() {
        return;
    }
    println!("    Pending Questions:");
    for question in questions {
        println!("{}", replay_question_line(&question));
        for option in question["options"].as_array().cloned().unwrap_or_default() {
            if let Some(option) = option.as_str() {
                println!("        - {option}");
            }
        }
        if let Some(next) = question["next_action"]
            .as_str()
            .filter(|value| !value.is_empty())
        {
            println!("        next: {next}");
        }
    }
    if group["truncated"].as_bool().unwrap_or(false) {
        let total = group["total"].as_u64().unwrap_or(0);
        let limit = group["limit"].as_u64().unwrap_or(DEFAULT_REPLAY_QUESTIONS);
        ui::hint(&format!(
            "Showing first {limit} of {total} pending question(s)."
        ));
    }
}

pub(super) fn replay_question_line(question: &Value) -> String {
    let ask_id = question["ask_id"].as_str().unwrap_or("ask");
    let phase = question["phase"].as_str().unwrap_or("?");
    let role = question["worker_role"].as_str().unwrap_or("worker");
    let text = question["question"]
        .as_str()
        .unwrap_or("(no question text)");
    format!("      {ask_id} -- {phase} -- {role}: {text}")
}

fn print_replay_events(group: &Value) {
    let events = group["items"].as_array().cloned().unwrap_or_default();
    if events.is_empty() {
        println!("    Events: none");
        return;
    }
    println!("    Events:");
    for event in events {
        println!("{}", replay_event_line(&event));
    }
    if group["truncated"].as_bool().unwrap_or(false) {
        let total = group["total"].as_u64().unwrap_or(0);
        let limit = group["limit"].as_u64().unwrap_or(DEFAULT_REPLAY_EVENTS);
        ui::hint(&format!("Showing newest {limit} of {total} event(s)."));
    }
}

pub(super) fn replay_event_line(event: &Value) -> String {
    let ts = event["ts"].as_str().unwrap_or("?");
    let phase = event["phase"].as_str().unwrap_or("?");
    let status = event["status"].as_str().unwrap_or("?");
    let kind = event["kind"].as_str().unwrap_or("event");
    let title = event["title"].as_str().unwrap_or("Untitled event");
    let detail = event["detail"].as_str().unwrap_or("");
    if detail.is_empty() {
        format!("      {ts} -- {phase}/{status} -- {kind} -- {title}")
    } else {
        format!("      {ts} -- {phase}/{status} -- {kind} -- {title}: {detail}")
    }
}
