//! Project, task, milestone, and checkpoint dispatch.

use std::sync::Arc;

use crate::kernel_handle::KernelHandle;

use super::{
    tool_checkpoint_save, tool_milestone_complete, tool_milestone_create, tool_milestone_list,
    tool_milestone_progress, tool_project_archive, tool_project_create, tool_project_delete,
    tool_project_get, tool_project_list, tool_project_resume, tool_project_task_create,
    tool_project_task_list, tool_project_task_update,
};

pub(crate) fn dispatch_project_tool(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    match tool_name {
        "project_create" => tool_project_create(input, kernel),
        "project_list" => tool_project_list(input, kernel),
        "project_get" => tool_project_get(input, kernel),
        "project_archive" => tool_project_archive(input, kernel),
        "project_delete" => tool_project_delete(input, kernel),
        "project_resume" => tool_project_resume(input, kernel),
        "project_task_create" => tool_project_task_create(input, kernel),
        "project_task_list" => tool_project_task_list(input, kernel),
        "project_task_update" => tool_project_task_update(input, kernel),
        "milestone_create" => tool_milestone_create(input, kernel),
        "milestone_list" => tool_milestone_list(input, kernel),
        "milestone_complete" => tool_milestone_complete(input, kernel),
        "milestone_progress" => tool_milestone_progress(input, kernel),
        "checkpoint_save" => tool_checkpoint_save(input, kernel, caller_agent_id),
        other => Err(format!("Unknown project tool: {other}")),
    }
}
