use crate::kernel_handle::KernelHandle;
use std::path::Path;
use std::sync::Arc;

use super::tool_capability_forge;

pub(crate) fn dispatch_capspec_management_tool(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    workspace: Option<&Path>,
    caller_agent_id: Option<&str>,
) -> Option<Result<String, String>> {
    match tool_name {
        "capability_forge" => Some(tool_capability_forge(
            input,
            kernel,
            workspace,
            caller_agent_id,
        )),
        _ => None,
    }
}
