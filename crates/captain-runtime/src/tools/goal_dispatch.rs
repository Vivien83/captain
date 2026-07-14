//! Goal and suggestion dispatch.

use std::sync::Arc;

use crate::kernel_handle::KernelHandle;

use super::{
    tool_goal_apply_suggestion, tool_goal_create, tool_goal_delete, tool_goal_list,
    tool_goal_list_suggestions, tool_goal_pause, tool_goal_reject_suggestion, tool_goal_resume,
    tool_goal_status,
};

pub(crate) fn dispatch_goal_tool(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    match tool_name {
        "goal_create" => tool_goal_create(input, kernel),
        "goal_list" => tool_goal_list(kernel),
        "goal_pause" => tool_goal_pause(input, kernel),
        "goal_resume" => tool_goal_resume(input, kernel),
        "goal_status" => tool_goal_status(input, kernel),
        "goal_delete" => tool_goal_delete(input, kernel),
        "goal_list_suggestions" => tool_goal_list_suggestions(input, kernel),
        "goal_apply_suggestion" => tool_goal_apply_suggestion(input, kernel),
        "goal_reject_suggestion" => tool_goal_reject_suggestion(input, kernel),
        other => Err(format!("Unknown goal tool: {other}")),
    }
}
