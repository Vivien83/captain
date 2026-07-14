//! Dispatch for tool run supervision tools.

use crate::kernel_handle::KernelHandle;
use captain_types::config::ExecPolicy;
use std::path::Path;
use std::sync::Arc;

use super::{
    tool_run_cancel, tool_run_list, tool_run_result, tool_run_start, tool_run_status,
    ToolRunStartContext,
};

#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_tool_run_tool(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    allowed_tools: Option<&[String]>,
    caller_agent_id: Option<&str>,
    allowed_env_vars: Option<&[String]>,
    workspace_root: Option<&Path>,
    exec_policy: Option<&ExecPolicy>,
) -> Result<String, String> {
    match tool_name {
        "tool_run_start" => {
            tool_run_start(
                input,
                ToolRunStartContext {
                    kernel: kernel.cloned(),
                    allowed_tools: allowed_tools.map(|tools| tools.to_vec()),
                    caller_agent_id: caller_agent_id.map(str::to_string),
                    allowed_env_vars: allowed_env_vars.unwrap_or(&[]).to_vec(),
                    workspace_root: workspace_root.map(Path::to_path_buf),
                    exec_policy: exec_policy.cloned(),
                },
            )
            .await
        }
        "tool_run_status" => tool_run_status(input),
        "tool_run_result" => tool_run_result(input),
        "tool_run_cancel" => tool_run_cancel(input),
        "tool_run_list" => tool_run_list(input),
        other => Err(format!("Unknown tool run tool: {other}")),
    }
}
