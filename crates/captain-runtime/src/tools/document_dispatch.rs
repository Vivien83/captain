//! Document runtime dispatch.

use std::path::Path;
use std::sync::Arc;

use crate::kernel_handle::KernelHandle;

use super::{tool_document_extract, tool_document_pipeline};

pub(crate) async fn dispatch_document_tool(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    workspace_root: Option<&Path>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    match tool_name {
        "document_pipeline" => {
            tool_document_pipeline(input, kernel, workspace_root, caller_agent_id).await
        }
        "document_create" => crate::document_tools::create_document(input, workspace_root).await,
        "document_extract" => {
            tool_document_extract(input, workspace_root, kernel, caller_agent_id).await
        }
        other => Err(format!("Unknown document tool: {other}")),
    }
}
