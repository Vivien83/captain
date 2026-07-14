use crate::kernel_handle::KernelHandle;
use crate::tools::{ensure_no_secret_literal, require_kernel};
use std::sync::Arc;

pub(crate) fn tool_goal_create(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let json = serde_json::to_string(input).map_err(|e| format!("serialize input: {e}"))?;
    ensure_no_secret_literal("goal_create", "input", &json)?;
    let id = kh.goal_create(&json)?;

    let ops: Arc<dyn crate::goal_loop::GoalLoopOps> =
        Arc::new(crate::goal_loop::KernelOps { kh: kh.clone() });
    crate::goal_loop::spawn_goal_loop_for(id.clone(), ops);

    Ok(format!("Goal '{id}' created and scheduled."))
}

pub(crate) fn tool_goal_list(kernel: Option<&Arc<dyn KernelHandle>>) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    kh.goal_list()
}

pub(crate) fn tool_goal_pause(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = input["id"].as_str().ok_or("Missing 'id' parameter")?;
    let existed = kh.goal_pause(id)?;
    if existed {
        Ok(format!("Goal '{id}' paused."))
    } else {
        Err(format!("Goal '{id}' not found"))
    }
}

pub(crate) fn tool_goal_resume(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = input["id"].as_str().ok_or("Missing 'id' parameter")?;
    let existed = kh.goal_resume(id)?;
    if existed {
        Ok(format!("Goal '{id}' resumed."))
    } else {
        Err(format!("Goal '{id}' not found"))
    }
}

pub(crate) fn tool_goal_status(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = input["id"].as_str().ok_or("Missing 'id' parameter")?;
    kh.goal_status(id)
}

pub(crate) fn tool_goal_delete(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = input["id"].as_str().ok_or("Missing 'id' parameter")?;
    let existed = kh.goal_delete(id)?;
    if existed {
        Ok(format!("Goal '{id}' deleted."))
    } else {
        Err(format!("Goal '{id}' not found"))
    }
}

pub(crate) fn tool_goal_list_suggestions(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = input["id"].as_str().ok_or("Missing 'id' parameter")?;
    kh.goal_list_suggestions(id)
}

pub(crate) fn tool_goal_apply_suggestion(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = input["id"].as_str().ok_or("Missing 'id' parameter")?;
    let sid = input["suggestion_id"]
        .as_str()
        .ok_or("Missing 'suggestion_id' parameter")?;
    let applied = kh.goal_apply_suggestion(id, sid)?;
    if applied {
        Ok(format!("Suggestion '{sid}' applied to goal '{id}'."))
    } else {
        Err(format!(
            "Suggestion '{sid}' not found or already resolved on goal '{id}'"
        ))
    }
}

pub(crate) fn tool_goal_reject_suggestion(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = input["id"].as_str().ok_or("Missing 'id' parameter")?;
    let sid = input["suggestion_id"]
        .as_str()
        .ok_or("Missing 'suggestion_id' parameter")?;
    let rejected = kh.goal_reject_suggestion(id, sid)?;
    if rejected {
        Ok(format!("Suggestion '{sid}' rejected on goal '{id}'."))
    } else {
        Err(format!(
            "Suggestion '{sid}' not found or already resolved on goal '{id}'"
        ))
    }
}
