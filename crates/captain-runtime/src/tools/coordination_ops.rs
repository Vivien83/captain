use crate::kernel_handle::KernelHandle;
use crate::tools::require_kernel;
use std::sync::Arc;

pub(crate) fn tool_agent_find(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let query = input["query"].as_str().ok_or("Missing 'query' parameter")?;
    let agents = kh.find_agents(query);
    if agents.is_empty() {
        return Ok(format!("No agents found matching '{query}'."));
    }
    let result: Vec<serde_json::Value> = agents
        .iter()
        .map(|a| {
            serde_json::json!({
                "id": a.id,
                "name": a.name,
                "state": a.state,
                "description": a.description,
                "tags": a.tags,
                "tools": a.tools,
                "model": format!("{}:{}", a.model_provider, a.model_name),
            })
        })
        .collect();
    serde_json::to_string_pretty(&result).map_err(|e| format!("Serialize error: {e}"))
}

pub(crate) async fn tool_task_post(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let title = input["title"].as_str().ok_or("Missing 'title' parameter")?;
    let description = input["description"]
        .as_str()
        .ok_or("Missing 'description' parameter")?;
    let assigned_to = input["assigned_to"].as_str();
    let task_id = kh
        .task_post(title, description, assigned_to, caller_agent_id)
        .await?;
    Ok(format!("Task created with ID: {task_id}"))
}

pub(crate) async fn tool_task_claim(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.unwrap_or("");
    match kh.task_claim(agent_id).await? {
        Some(task) => {
            serde_json::to_string_pretty(&task).map_err(|e| format!("Serialize error: {e}"))
        }
        None => Ok("No tasks available.".to_string()),
    }
}

pub(crate) async fn tool_task_complete(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let task_id = input["task_id"]
        .as_str()
        .ok_or("Missing 'task_id' parameter")?;
    let result = input["result"]
        .as_str()
        .ok_or("Missing 'result' parameter")?;
    kh.task_complete(task_id, result).await?;
    Ok(format!("Task {task_id} marked as completed."))
}

pub(crate) async fn tool_task_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let status = input["status"].as_str();
    let tasks = kh.task_list(status).await?;
    if tasks.is_empty() {
        return Ok("No tasks found.".to_string());
    }
    serde_json::to_string_pretty(&tasks).map_err(|e| format!("Serialize error: {e}"))
}

pub(crate) async fn tool_event_publish(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let event_type = input["event_type"]
        .as_str()
        .ok_or("Missing 'event_type' parameter")?;
    let payload = input
        .get("payload")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    kh.publish_event(event_type, payload).await?;
    Ok(format!("Event '{event_type}' published successfully."))
}
