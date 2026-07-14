//! Location and local time dispatch.

use super::{tool_location_get, tool_system_time};

pub(crate) async fn dispatch_location_tool(tool_name: &str) -> Result<String, String> {
    match tool_name {
        "location_get" => tool_location_get().await,
        "system_time" => Ok(tool_system_time()),
        other => Err(format!("Unknown location tool: {other}")),
    }
}
