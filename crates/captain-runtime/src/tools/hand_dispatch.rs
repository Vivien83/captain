//! Hand runtime dispatch.

use std::sync::Arc;

use crate::kernel_handle::KernelHandle;

use super::{
    tool_hand_activate, tool_hand_deactivate, tool_hand_list, tool_hand_status, tool_scaffold_hand,
};

pub(crate) async fn dispatch_hand_tool(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    match tool_name {
        "hand_list" => tool_hand_list(kernel).await,
        "hand_activate" => tool_hand_activate(input, kernel).await,
        "hand_status" => tool_hand_status(input, kernel).await,
        "hand_deactivate" => tool_hand_deactivate(input, kernel).await,
        "scaffold_hand" => tool_scaffold_hand(input, kernel).await,
        other => Err(format!("Unknown hand tool: {other}")),
    }
}
