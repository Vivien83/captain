//! Channel delivery and topic dispatch.

use std::path::Path;
use std::sync::Arc;

use crate::kernel_handle::KernelHandle;

use super::{
    tool_channel_delivery_batch, tool_channel_reconfigure, tool_channel_send,
    tool_telegram_get_topic, tool_telegram_set_topic,
};

pub(crate) async fn dispatch_channel_tool(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    workspace_root: Option<&Path>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    match tool_name {
        "channel_reconfigure" => tool_channel_reconfigure(input, kernel).await,
        "channel_delivery_batch" => {
            tool_channel_delivery_batch(input, kernel, workspace_root, caller_agent_id).await
        }
        "channel_send" => tool_channel_send(input, kernel, workspace_root, caller_agent_id).await,
        "telegram_set_topic" => tool_telegram_set_topic(input, kernel).await,
        "telegram_get_topic" => tool_telegram_get_topic(input, kernel).await,
        other => Err(format!("Unknown channel tool: {other}")),
    }
}
