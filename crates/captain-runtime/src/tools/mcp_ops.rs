use crate::kernel_handle::KernelHandle;
use crate::tools::require_kernel;
use std::sync::Arc;

pub(crate) async fn tool_mcp_catalog_search(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let query = input.get("query").and_then(|v| v.as_str());
    let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
    let out = kh.mcp_catalog_search(query, limit).await?;
    serde_json::to_string_pretty(&out).map_err(|e| format!("serialize mcp catalog: {e}"))
}

pub(crate) async fn tool_mcp_integration_install(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = input
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'id' parameter")?;
    let credentials = input
        .get("credentials")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let reload = input
        .get("reload")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let out = kh.mcp_integration_install(id, credentials, reload).await?;
    serde_json::to_string_pretty(&out).map_err(|e| format!("serialize mcp install: {e}"))
}

pub(crate) async fn tool_mcp_status(
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let out = kh.mcp_status().await?;
    serde_json::to_string_pretty(&out).map_err(|e| format!("serialize mcp status: {e}"))
}
