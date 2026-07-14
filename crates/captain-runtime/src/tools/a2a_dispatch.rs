//! A2A outbound dispatch.

use std::sync::Arc;

use crate::kernel_handle::KernelHandle;

use super::{tool_a2a_discover, tool_a2a_send};

pub(crate) async fn dispatch_a2a_tool(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    match tool_name {
        "a2a_discover" => tool_a2a_discover(input).await,
        "a2a_send" => tool_a2a_send(input, kernel).await,
        other => Err(format!("Unknown A2A tool: {other}")),
    }
}
