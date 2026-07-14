//! Persistent process dispatch.

use super::{
    tool_process_kill, tool_process_list, tool_process_poll, tool_process_start, tool_process_write,
};

pub(crate) async fn dispatch_process_tool(
    tool_name: &str,
    input: &serde_json::Value,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    match tool_name {
        "process_start" => tool_process_start(input, process_manager, caller_agent_id).await,
        "process_poll" => tool_process_poll(input, process_manager).await,
        "process_write" => tool_process_write(input, process_manager).await,
        "process_kill" => tool_process_kill(input, process_manager).await,
        "process_list" => tool_process_list(process_manager, caller_agent_id).await,
        other => Err(format!("Unknown process tool: {other}")),
    }
}
