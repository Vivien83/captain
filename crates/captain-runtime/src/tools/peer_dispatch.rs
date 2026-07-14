//! Peer federation dispatch.

use std::sync::Arc;

use crate::kernel_handle::KernelHandle;

use super::tool_peer_list;

pub(crate) fn dispatch_peer_tool(
    tool_name: &str,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    match tool_name {
        "peer_list" => tool_peer_list(kernel),
        other => Err(format!("Unknown peer tool: {other}")),
    }
}
