use crate::kernel_handle::KernelHandle;
use crate::tools::{ensure_no_secret_literal, require_kernel};
use std::sync::Arc;

const SCHEDULES_KEY: &str = "__captain_schedules";

pub(crate) fn parse_schedule_to_cron(input: &str) -> Result<String, String> {
    let input = input.trim().to_lowercase();
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.len() == 5
        && parts
            .iter()
            .all(|p| p.chars().all(|c| c.is_ascii_digit() || "*/,-".contains(c)))
    {
        return Ok(input);
    }

    if let Some(rest) = input.strip_prefix("every ") {
        if rest == "minute" || rest == "1 minute" {
            return Ok("* * * * *".to_string());
        }
        if let Some(mins) = rest.strip_suffix(" minutes") {
            let n: u32 = mins
                .trim()
                .parse()
                .map_err(|_| format!("Invalid number in '{input}'"))?;
            if n == 0 || n > 59 {
                return Err(format!("Minutes must be 1-59, got {n}"));
            }
            return Ok(format!("*/{n} * * * *"));
        }
        if rest == "hour" || rest == "1 hour" {
            return Ok("0 * * * *".to_string());
        }
        if let Some(hrs) = rest.strip_suffix(" hours") {
            let n: u32 = hrs
                .trim()
                .parse()
                .map_err(|_| format!("Invalid number in '{input}'"))?;
            if n == 0 || n > 23 {
                return Err(format!("Hours must be 1-23, got {n}"));
            }
            return Ok(format!("0 */{n} * * *"));
        }
        if rest == "day" || rest == "1 day" {
            return Ok("0 0 * * *".to_string());
        }
        if rest == "week" || rest == "1 week" {
            return Ok("0 0 * * 0".to_string());
        }
    }

    if let Some(time_str) = input.strip_prefix("daily at ") {
        let hour = parse_time_to_hour(time_str)?;
        return Ok(format!("0 {hour} * * *"));
    }
    if let Some(time_str) = input.strip_prefix("weekdays at ") {
        let hour = parse_time_to_hour(time_str)?;
        return Ok(format!("0 {hour} * * 1-5"));
    }
    if let Some(time_str) = input.strip_prefix("weekends at ") {
        let hour = parse_time_to_hour(time_str)?;
        return Ok(format!("0 {hour} * * 0,6"));
    }

    match input.as_str() {
        "hourly" => return Ok("0 * * * *".to_string()),
        "daily" => return Ok("0 0 * * *".to_string()),
        "weekly" => return Ok("0 0 * * 0".to_string()),
        "monthly" => return Ok("0 0 1 * *".to_string()),
        _ => {}
    }

    Err(format!(
        "Could not parse schedule '{input}'. Try: 'every 5 minutes', 'daily at 9am', 'weekdays at 6pm', or a cron expression like '0 */5 * * *'"
    ))
}

fn parse_time_to_hour(s: &str) -> Result<u32, String> {
    let s = s.trim().to_lowercase();
    if let Some(h) = s.strip_suffix("am") {
        let hour: u32 = h.trim().parse().map_err(|_| format!("Invalid time: {s}"))?;
        return match hour {
            12 => Ok(0),
            1..=11 => Ok(hour),
            _ => Err(format!("Invalid hour: {hour}")),
        };
    }
    if let Some(h) = s.strip_suffix("pm") {
        let hour: u32 = h.trim().parse().map_err(|_| format!("Invalid time: {s}"))?;
        return match hour {
            12 => Ok(12),
            1..=11 => Ok(hour + 12),
            _ => Err(format!("Invalid hour: {hour}")),
        };
    }
    if let Some((h, _m)) = s.split_once(':') {
        let hour: u32 = h.trim().parse().map_err(|_| format!("Invalid time: {s}"))?;
        if hour > 23 {
            return Err(format!("Hour must be 0-23, got {hour}"));
        }
        return Ok(hour);
    }

    let hour: u32 = s.parse().map_err(|_| format!("Invalid time: {s}"))?;
    if hour > 23 {
        return Err(format!("Hour must be 0-23, got {hour}"));
    }
    Ok(hour)
}

pub(crate) async fn tool_schedule_create(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let description = input["description"]
        .as_str()
        .ok_or("Missing 'description' parameter")?;
    let schedule_str = input["schedule"]
        .as_str()
        .ok_or("Missing 'schedule' parameter")?;
    let agent = input["agent"].as_str().unwrap_or("");
    let cron_expr = parse_schedule_to_cron(schedule_str)?;
    let schedule_id = uuid::Uuid::new_v4().to_string();

    let entry = serde_json::json!({
        "id": schedule_id,
        "description": description,
        "schedule_input": schedule_str,
        "cron": cron_expr,
        "agent": agent,
        "created_at": chrono::Utc::now().to_rfc3339(),
        "enabled": true,
    });
    let mut schedules: Vec<serde_json::Value> = match kh.memory_recall(SCHEDULES_KEY)? {
        Some(serde_json::Value::Array(arr)) => arr,
        _ => Vec::new(),
    };
    schedules.push(entry);
    kh.memory_store(SCHEDULES_KEY, serde_json::Value::Array(schedules))?;

    Ok(format!(
        "Schedule created:\n  ID: {schedule_id}\n  Description: {description}\n  Cron: {cron_expr}\n  Original: {schedule_str}"
    ))
}

pub(crate) async fn tool_schedule_list(
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let schedules: Vec<serde_json::Value> = match kh.memory_recall(SCHEDULES_KEY)? {
        Some(serde_json::Value::Array(arr)) => arr,
        _ => Vec::new(),
    };
    if schedules.is_empty() {
        return Ok("No scheduled tasks.".to_string());
    }

    let mut output = format!("Scheduled tasks ({}):\n\n", schedules.len());
    for s in &schedules {
        let enabled = s["enabled"].as_bool().unwrap_or(true);
        let status = if enabled { "active" } else { "paused" };
        output.push_str(&format!(
            "  [{status}] {} — {}\n    Cron: {} | Agent: {}\n    Created: {}\n\n",
            s["id"].as_str().unwrap_or("?"),
            s["description"].as_str().unwrap_or("?"),
            s["cron"].as_str().unwrap_or("?"),
            s["agent"].as_str().unwrap_or("(self)"),
            s["created_at"].as_str().unwrap_or("?"),
        ));
    }
    Ok(output)
}

pub(crate) async fn tool_schedule_delete(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = input["id"].as_str().ok_or("Missing 'id' parameter")?;
    let mut schedules: Vec<serde_json::Value> = match kh.memory_recall(SCHEDULES_KEY)? {
        Some(serde_json::Value::Array(arr)) => arr,
        _ => Vec::new(),
    };
    let before = schedules.len();
    schedules.retain(|s| s["id"].as_str() != Some(id));
    if schedules.len() == before {
        return Err(format!("Schedule '{id}' not found."));
    }
    kh.memory_store(SCHEDULES_KEY, serde_json::Value::Array(schedules))?;
    Ok(format!("Schedule '{id}' deleted."))
}

pub(crate) async fn tool_cron_create(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Agent ID required for cron_create")?;
    ensure_no_secret_literal("cron_create", "input", &input.to_string())?;
    ensure_cron_webhook_url_is_public("cron_create", input)?;
    kh.cron_create(agent_id, input.clone()).await
}

pub(crate) async fn tool_reminder_set(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Agent ID required for reminder_set")?;
    let delay_minutes = input["delay_minutes"]
        .as_f64()
        .ok_or("Missing 'delay_minutes' (number)")?;
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' (string)")?;
    ensure_no_secret_literal("reminder_set", "message", message)?;
    let channel = input["channel"].as_str().unwrap_or("telegram");
    let trigger_at = chrono::Utc::now() + chrono::Duration::seconds((delay_minutes * 60.0) as i64);
    let cron_input = serde_json::json!({
        "name": format!("reminder-{}", &trigger_at.format("%H%M")),
        "schedule": { "kind": "at", "at": trigger_at.to_rfc3339() },
        "action": { "kind": "agent_turn", "message": format!("RAPPEL: {message}"), "timeout_secs": 60 },
        "delivery": { "kind": "channel", "channel": channel },
        "one_shot": true
    });
    kh.cron_create(agent_id, cron_input).await
}

pub(crate) async fn tool_cron_list(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Agent ID required for cron_list")?;
    let jobs = kh.cron_list(agent_id).await?;
    serde_json::to_string_pretty(&jobs).map_err(|e| format!("Failed to serialize cron jobs: {e}"))
}

pub(crate) async fn tool_cron_update(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Agent ID required for cron_update")?;
    let _ = input["job_id"]
        .as_str()
        .or_else(|| input["id"].as_str())
        .ok_or("Missing 'job_id' parameter")?;
    ensure_no_secret_literal("cron_update", "input", &input.to_string())?;
    ensure_cron_webhook_url_is_public("cron_update", input)?;
    kh.cron_update(agent_id, input.clone()).await
}

pub(crate) fn ensure_cron_webhook_url_is_public(
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<(), String> {
    let Some(delivery) = input.get("delivery") else {
        return Ok(());
    };
    if delivery.get("kind").and_then(|v| v.as_str()) != Some("webhook") {
        return Ok(());
    }
    let url = delivery
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{tool_name}.delivery webhook requires 'url'"))?;
    crate::web_fetch::check_ssrf(url)
        .map_err(|e| format!("SSRF blocked: {tool_name}.delivery.url is not public-safe: {e}"))
}

pub(crate) async fn tool_cron_cancel(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let job_id = input["job_id"]
        .as_str()
        .ok_or("Missing 'job_id' parameter")?;
    kh.cron_cancel(job_id).await?;
    Ok(format!("Cron job '{job_id}' cancelled."))
}

pub(crate) async fn tool_file_trigger_register(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Agent ID required for file_trigger_register")?;
    ensure_no_secret_literal("file_trigger_register", "input", &input.to_string())?;
    let trigger_id = kh.file_trigger_register(agent_id, input.clone()).await?;
    Ok(serde_json::json!({ "trigger_id": trigger_id, "agent_id": agent_id }).to_string())
}

pub(crate) async fn tool_file_trigger_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let scope = input
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("self");
    let agent_filter = match scope {
        "all" => None,
        _ => Some(caller_agent_id.ok_or("Agent ID required for file_trigger_list")?),
    };
    let list = kh.file_trigger_list(agent_filter).await?;
    serde_json::to_string_pretty(&list)
        .map_err(|e| format!("Failed to serialize file triggers: {e}"))
}

pub(crate) async fn tool_file_trigger_set_enabled(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let trigger_id = input["trigger_id"]
        .as_str()
        .ok_or("Missing 'trigger_id' parameter")?;
    let enabled = input["enabled"]
        .as_bool()
        .ok_or("Missing 'enabled' (bool)")?;
    let updated = kh.file_trigger_set_enabled(trigger_id, enabled).await?;
    if !updated {
        return Err(format!("File trigger '{trigger_id}' not found"));
    }
    Ok(serde_json::json!({ "trigger_id": trigger_id, "enabled": enabled }).to_string())
}

pub(crate) async fn tool_file_trigger_remove(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let trigger_id = input["trigger_id"]
        .as_str()
        .ok_or("Missing 'trigger_id' parameter")?;
    let removed = kh.file_trigger_remove(trigger_id).await?;
    if !removed {
        return Err(format!("File trigger '{trigger_id}' not found"));
    }
    Ok(format!("File trigger '{trigger_id}' removed."))
}

pub(crate) fn tool_todo_create(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let title = input["title"]
        .as_str()
        .ok_or("Missing 'title' parameter")?
        .trim();
    if title.is_empty() {
        return Err("'title' must be a non-empty string".into());
    }
    let body = input["body"].as_str().unwrap_or("");
    ensure_no_secret_literal("todo_create", "title", title)?;
    ensure_no_secret_literal("todo_create", "body", body)?;
    let row = kh.todo_create(title, body)?;
    serde_json::to_string_pretty(&row).map_err(|e| format!("Failed to serialize todo: {e}"))
}

pub(crate) fn tool_todo_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let filter = input["status"].as_str().unwrap_or("open");
    let limit = input
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n.min(u32::MAX as u64) as u32);
    let rows = kh.todo_list(filter, limit)?;
    serde_json::to_string_pretty(&rows).map_err(|e| format!("Failed to serialize todos: {e}"))
}

pub(crate) fn tool_todo_complete(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = input["id"].as_str().ok_or("Missing 'id' parameter")?;
    match kh.todo_complete(id)? {
        Some(row) => {
            serde_json::to_string_pretty(&row).map_err(|e| format!("Failed to serialize todo: {e}"))
        }
        None => Err(format!("Todo '{id}' not found")),
    }
}

pub(crate) fn tool_todo_reopen(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = input["id"].as_str().ok_or("Missing 'id' parameter")?;
    match kh.todo_reopen(id)? {
        Some(row) => {
            serde_json::to_string_pretty(&row).map_err(|e| format!("Failed to serialize todo: {e}"))
        }
        None => Err(format!("Todo '{id}' not found")),
    }
}

pub(crate) fn tool_todo_delete(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = input["id"].as_str().ok_or("Missing 'id' parameter")?;
    if !kh.todo_delete(id)? {
        return Err(format!("Todo '{id}' not found"));
    }
    Ok(format!("Todo '{id}' deleted."))
}
