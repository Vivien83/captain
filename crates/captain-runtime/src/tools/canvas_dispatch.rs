//! Canvas presentation dispatch.

use std::path::Path;

use super::tool_canvas_present;

pub(crate) async fn dispatch_canvas_tool(
    tool_name: &str,
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    match tool_name {
        "canvas_present" => tool_canvas_present(input, workspace_root).await,
        other => Err(format!("Unknown canvas tool: {other}")),
    }
}
