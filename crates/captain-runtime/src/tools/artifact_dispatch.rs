//! Immutable artifact runtime dispatch.

use std::path::Path;
use std::sync::Arc;

use crate::kernel_handle::KernelHandle;

use super::{
    tool_artifact_deliver, tool_artifact_inspect, tool_artifact_list, tool_artifact_publish,
};

pub(crate) async fn dispatch_artifact_tool(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    workspace_root: Option<&Path>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    match tool_name {
        "artifact_publish" => {
            tool_artifact_publish(input, kernel, workspace_root, caller_agent_id).await
        }
        "artifact_list" => tool_artifact_list(input, kernel, caller_agent_id).await,
        "artifact_inspect" => tool_artifact_inspect(input, kernel, caller_agent_id).await,
        "artifact_deliver" => tool_artifact_deliver(input, kernel, caller_agent_id).await,
        other => Err(format!("Unknown artifact tool: {other}")),
    }
}
