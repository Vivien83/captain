use super::status_project_attention_render::{
    project_attention_action_body_hint, project_attention_action_hint,
    project_attention_action_reason_hint, project_attention_hidden_count,
    project_attention_last_event_hint, project_attention_question_hint,
    project_attention_tool_request_hint, project_attention_worker_hint,
    PROJECT_ATTENTION_VISIBLE_LIMIT,
};
use crate::{truncate_display, ui};

pub(super) use super::status_project_attention_metadata::project_attention_from_metadata;

pub(super) fn print_project_attention_rows(workload: &serde_json::Value, verbose: bool) {
    let Some(items) = workload["projects"]["attention"].as_array() else {
        return;
    };
    if items.is_empty() {
        return;
    }

    ui::blank();
    ui::section("Project Attention");
    for item in items.iter().take(PROJECT_ATTENTION_VISIBLE_LIMIT) {
        let slug = item["project_slug"]
            .as_str()
            .or_else(|| item["slug"].as_str())
            .unwrap_or("?");
        let state = item["state"]
            .as_str()
            .or_else(|| item["operator_state"].as_str())
            .unwrap_or("?");
        let phase = item["phase"].as_str().unwrap_or("?");
        let pending = item["pending_questions"].as_u64().unwrap_or(0);
        let summary = truncate_display(item["summary"].as_str().unwrap_or(""), 84);
        if verbose {
            println!("    {slug} -- {state} -- phase {phase} -- pending {pending} -- {summary}");
            if let Some(event) = project_attention_last_event_hint(item) {
                println!("        last_event: {event}");
            }
            if let Some(workers) = project_attention_worker_hint(item) {
                println!("        workers: {workers}");
            }
            if let Some(question) = project_attention_question_hint(item) {
                println!("        question: {question}");
            }
            if let Some(request) = project_attention_tool_request_hint(item) {
                println!("        tool_request: {request}");
            }
            if let Some(action) = project_attention_action_hint(item) {
                println!("        action: {action}");
            }
            if let Some(reason) = project_attention_action_reason_hint(item) {
                println!("        reason: {reason}");
            }
            if let Some(body) = project_attention_action_body_hint(item) {
                println!("        body: {body}");
            }
        } else {
            println!("    {slug} -- {state} -- phase {phase} -- {summary}");
        }
    }
    let hidden = project_attention_hidden_count(
        items.len(),
        workload["projects"]["attention_count"].as_u64(),
    );
    if hidden > 0 {
        println!("    ... and {hidden} more");
    }
    let has_action = items
        .iter()
        .any(|item| project_attention_action_hint(item).is_some());
    if !verbose && has_action {
        ui::hint("Run `captain status --verbose` to see project action details.");
    }
}

#[cfg(test)]
#[path = "status_project_attention_tests.rs"]
mod status_project_attention_tests;
