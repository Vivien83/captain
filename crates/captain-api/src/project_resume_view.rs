use serde_json::{json, Value};

const ID_LIMIT: usize = 120;
const TITLE_LIMIT: usize = 180;
const SUMMARY_LIMIT: usize = 600;
const LIST_LIMIT: usize = 50;

pub(crate) fn checkpoint_view(value: Value) -> Value {
    let Some(checkpoint) = value.as_object() else {
        return Value::Null;
    };
    json!({
        "id": safe_scalar(checkpoint.get("id"), ID_LIMIT),
        "created_at": safe_scalar(checkpoint.get("created_at"), ID_LIMIT),
        "session_id": safe_scalar(checkpoint.get("session_id"), ID_LIMIT),
        "summary": safe_scalar(checkpoint.get("summary"), SUMMARY_LIMIT),
    })
}

pub(crate) fn task_list_view(value: Value) -> Value {
    let Some(tasks) = value.as_array() else {
        return Value::Array(Vec::new());
    };
    Value::Array(tasks.iter().take(LIST_LIMIT).map(task_view_ref).collect())
}

pub(crate) fn task_item_view(value: Value) -> Value {
    task_view_ref(&value)
}

pub(crate) fn goal_list_view(value: Value) -> Value {
    let Some(goals) = value.as_array() else {
        return Value::Array(Vec::new());
    };
    Value::Array(goals.iter().take(LIST_LIMIT).map(goal_view_ref).collect())
}

pub(crate) fn goal_item_view(value: Value) -> Value {
    goal_view_ref(&value)
}

pub(crate) fn milestone_list_view(value: Value) -> Value {
    let Some(milestones) = value.as_array() else {
        return Value::Array(Vec::new());
    };
    Value::Array(
        milestones
            .iter()
            .take(LIST_LIMIT)
            .map(milestone_view_ref)
            .collect(),
    )
}

pub(crate) fn milestone_item_view(value: Value) -> Value {
    milestone_view_ref(&value)
}

pub(crate) fn checkpoint_list_view(value: Value) -> Value {
    let Some(checkpoints) = value.as_array() else {
        return Value::Array(Vec::new());
    };
    Value::Array(
        checkpoints
            .iter()
            .take(LIST_LIMIT)
            .map(checkpoint_view_ref)
            .collect(),
    )
}

pub(crate) fn milestone_progress_view(value: Value) -> Value {
    let Some(progress) = value.as_object() else {
        return Value::Null;
    };
    json!({
        "total": safe_scalar(progress.get("total"), ID_LIMIT),
        "completed": safe_scalar(progress.get("completed"), ID_LIMIT),
        "missed": safe_scalar(progress.get("missed"), ID_LIMIT),
        "pct": safe_scalar(progress.get("pct"), ID_LIMIT),
    })
}

fn task_view_ref(task: &Value) -> Value {
    json!({
        "id": safe_scalar(task.get("id"), ID_LIMIT),
        "project_id": safe_scalar(task.get("project_id"), ID_LIMIT),
        "title": safe_scalar(task.get("title"), TITLE_LIMIT),
        "status": safe_task_status(task.get("status")),
        "priority": safe_scalar(task.get("priority"), ID_LIMIT),
        "deadline": safe_scalar(task.get("deadline"), ID_LIMIT),
        "created_at": safe_scalar(task.get("created_at"), ID_LIMIT),
        "updated_at": safe_scalar(task.get("updated_at"), ID_LIMIT),
        "completed_at": safe_scalar(task.get("completed_at"), ID_LIMIT),
    })
}

fn goal_view_ref(goal: &Value) -> Value {
    json!({
        "id": safe_scalar(goal.get("id"), ID_LIMIT),
        "project_id": safe_scalar(goal.get("project_id"), ID_LIMIT),
        "project_slug": safe_scalar(goal.get("project_slug"), ID_LIMIT),
        "name": safe_scalar(goal.get("name"), TITLE_LIMIT),
        "status": safe_goal_status(goal.get("status")),
        "interval_secs": safe_scalar(goal.get("interval_secs"), ID_LIMIT),
        "check_command_configured": command_configured(goal.get("check_command")),
        "recovery_command_configured": command_configured(goal.get("recovery_command")),
        "escalation_threshold": safe_scalar(goal.get("escalation_threshold"), ID_LIMIT),
        "max_llm_calls_per_hour": safe_scalar(goal.get("max_llm_calls_per_hour"), ID_LIMIT),
        "consecutive_fails": safe_scalar(goal.get("consecutive_fails"), ID_LIMIT),
        "last_check_ts": safe_scalar(goal.get("last_check_ts"), ID_LIMIT),
        "updated_at": safe_scalar(goal.get("updated_at"), ID_LIMIT),
        "escalated_at": safe_scalar(goal.get("escalated_at"), ID_LIMIT),
    })
}

fn milestone_view_ref(milestone: &Value) -> Value {
    json!({
        "id": safe_scalar(milestone.get("id"), ID_LIMIT),
        "project_id": safe_scalar(milestone.get("project_id"), ID_LIMIT),
        "name": safe_scalar(milestone.get("name"), TITLE_LIMIT),
        "status": safe_milestone_status(milestone.get("status")),
        "due_date": safe_scalar(milestone.get("due_date"), ID_LIMIT),
        "created_at": safe_scalar(milestone.get("created_at"), ID_LIMIT),
        "updated_at": safe_scalar(milestone.get("updated_at"), ID_LIMIT),
        "completed_at": safe_scalar(milestone.get("completed_at"), ID_LIMIT),
        "deliverable_count": milestone
            .get("deliverables")
            .and_then(Value::as_array)
            .map(|items| json!(items.len()))
            .unwrap_or(Value::Null),
    })
}

fn checkpoint_view_ref(checkpoint: &Value) -> Value {
    json!({
        "id": safe_scalar(checkpoint.get("id"), ID_LIMIT),
        "created_at": safe_scalar(checkpoint.get("created_at"), ID_LIMIT),
        "session_id": safe_scalar(checkpoint.get("session_id"), ID_LIMIT),
        "summary": safe_scalar(checkpoint.get("summary"), SUMMARY_LIMIT),
    })
}

fn safe_task_status(value: Option<&Value>) -> Value {
    let status = match value.and_then(Value::as_str).unwrap_or("todo") {
        "todo" => "todo",
        "doing" => "doing",
        "blocked" => "blocked",
        "done" => "done",
        _ => "todo",
    };
    Value::String(status.to_string())
}

fn safe_goal_status(value: Option<&Value>) -> Value {
    let status = match value.and_then(Value::as_str).unwrap_or("active") {
        "active" => "active",
        "paused" => "paused",
        "completed" => "completed",
        "failed" => "failed",
        "escalated" => "escalated",
        _ => "active",
    };
    Value::String(status.to_string())
}

fn command_configured(value: Option<&Value>) -> Value {
    Value::Bool(
        value
            .and_then(Value::as_str)
            .is_some_and(|command| !command.trim().is_empty()),
    )
}

fn safe_milestone_status(value: Option<&Value>) -> Value {
    let status = match value.and_then(Value::as_str).unwrap_or("open") {
        "open" => "open",
        "completed" => "completed",
        "missed" => "missed",
        _ => "open",
    };
    Value::String(status.to_string())
}

fn safe_scalar(value: Option<&Value>, limit: usize) -> Value {
    match value {
        Some(Value::String(text)) => bounded_text(text, limit)
            .filter(|text| !text.is_empty())
            .map(Value::String)
            .unwrap_or(Value::Null),
        Some(Value::Number(number)) => Value::Number(number.clone()),
        Some(Value::Bool(flag)) => Value::Bool(*flag),
        _ => Value::Null,
    }
}

fn bounded_text(value: &str, limit: usize) -> Option<String> {
    let cleaned = value
        .trim()
        .chars()
        .filter(|ch| !ch.is_control() || matches!(*ch, '\n' | '\t'))
        .collect::<String>();
    if cleaned.chars().count() <= limit {
        return Some(cleaned);
    }
    let keep = limit.saturating_sub(3);
    let mut truncated = cleaned.chars().take(keep).collect::<String>();
    truncated.push_str("...");
    Some(truncated)
}

#[cfg(test)]
#[path = "project_resume_view_tests.rs"]
mod project_resume_view_tests;
