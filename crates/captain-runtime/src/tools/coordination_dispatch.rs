//! Collaboration task/event dispatch.

use std::sync::Arc;

use crate::kernel_handle::KernelHandle;

use super::{
    tool_agent_find, tool_event_publish, tool_task_claim, tool_task_complete, tool_task_list,
    tool_task_post,
};

pub(crate) async fn dispatch_coordination_tool(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    match tool_name {
        "agent_find" => tool_agent_find(input, kernel),
        "task_post" => tool_task_post(input, kernel, caller_agent_id).await,
        "task_claim" => tool_task_claim(kernel, caller_agent_id).await,
        "task_complete" => tool_task_complete(input, kernel).await,
        "task_list" => tool_task_list(input, kernel).await,
        "event_publish" => tool_event_publish(input, kernel).await,
        other => Err(format!("Unknown coordination tool: {other}")),
    }
}
