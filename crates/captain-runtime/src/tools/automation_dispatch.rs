//! Automation tool dispatch.

use std::sync::Arc;

use crate::kernel_handle::KernelHandle;

use super::{
    tool_cron_cancel, tool_cron_create, tool_cron_list, tool_cron_update, tool_file_trigger_list,
    tool_file_trigger_register, tool_file_trigger_remove, tool_file_trigger_set_enabled,
    tool_reminder_set, tool_schedule_create, tool_schedule_delete, tool_schedule_list,
    tool_todo_complete, tool_todo_create, tool_todo_delete, tool_todo_list, tool_todo_reopen,
};

pub(crate) async fn dispatch_automation_tool(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    match tool_name {
        "schedule_create" => tool_schedule_create(input, kernel).await,
        "schedule_list" => tool_schedule_list(kernel).await,
        "schedule_delete" => tool_schedule_delete(input, kernel).await,
        "cron_create" => tool_cron_create(input, kernel, caller_agent_id).await,
        "cron_list" => tool_cron_list(kernel, caller_agent_id).await,
        "cron_update" => tool_cron_update(input, kernel, caller_agent_id).await,
        "cron_cancel" => tool_cron_cancel(input, kernel).await,
        "reminder_set" => tool_reminder_set(input, kernel, caller_agent_id).await,
        "file_trigger_register" => tool_file_trigger_register(input, kernel, caller_agent_id).await,
        "file_trigger_list" => tool_file_trigger_list(input, kernel, caller_agent_id).await,
        "file_trigger_set_enabled" => tool_file_trigger_set_enabled(input, kernel).await,
        "file_trigger_remove" => tool_file_trigger_remove(input, kernel).await,
        "todo_create" => tool_todo_create(input, kernel),
        "todo_list" => tool_todo_list(input, kernel),
        "todo_complete" => tool_todo_complete(input, kernel),
        "todo_reopen" => tool_todo_reopen(input, kernel),
        "todo_delete" => tool_todo_delete(input, kernel),
        other => Err(format!("Unknown automation tool: {other}")),
    }
}
