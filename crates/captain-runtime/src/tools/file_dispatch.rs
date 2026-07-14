//! Filesystem dispatch.

use std::path::Path;
use std::sync::Arc;

use crate::kernel_handle::KernelHandle;

use super::{
    tool_apply_patch, tool_edit_file, tool_file_inspect_batch, tool_file_list, tool_file_read,
    tool_file_write, tool_glob, tool_grep, tool_multi_edit,
};

pub(crate) async fn dispatch_file_tool(
    tool_name: &str,
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    match tool_name {
        "file_inspect_batch" => {
            tool_file_inspect_batch(input, workspace_root, kernel, caller_agent_id).await
        }
        "file_read" => tool_file_read(input, workspace_root, kernel, caller_agent_id).await,
        "file_write" => tool_file_write(input, workspace_root, kernel, caller_agent_id).await,
        "file_list" => tool_file_list(input, workspace_root, kernel, caller_agent_id).await,
        "apply_patch" => tool_apply_patch(input, workspace_root, kernel, caller_agent_id).await,
        "edit_file" => tool_edit_file(input, workspace_root, kernel, caller_agent_id).await,
        "multi_edit" => tool_multi_edit(input, workspace_root, kernel, caller_agent_id).await,
        "grep" => tool_grep(input, workspace_root, kernel, caller_agent_id).await,
        "glob" => tool_glob(input, workspace_root, kernel, caller_agent_id).await,
        other => Err(format!("Unknown file tool: {other}")),
    }
}
