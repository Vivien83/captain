use crate::kernel_handle::KernelHandle;
use crate::tools::require_kernel;
use std::sync::Arc;

pub(crate) async fn tool_session_recall(input: &serde_json::Value) -> Result<String, String> {
    let query = input["query"].as_str().ok_or("Missing 'query' parameter")?;
    let max_results = input["max_results"]
        .as_u64()
        .map(|n| n.min(20) as usize)
        .unwrap_or(5);
    let agent_filter = input["agent_filter"].as_str();
    let sessions_root = dirs::home_dir()
        .map(|h| h.join(".captain").join("sessions"))
        .ok_or("HOME not resolvable")?;
    let hits = crate::session_summarizer::recall_checkpoints(
        &sessions_root,
        query,
        max_results,
        agent_filter,
    );
    Ok(serde_json::json!({
        "hits": hits,
        "total": hits.len(),
        "sessions_root": sessions_root.display().to_string(),
        "search": "sqlite_fts5_with_scan_fallback"
    })
    .to_string())
}

pub(crate) async fn tool_session_tool_call_summary(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("caller agent id not available")?;
    let limit = input["limit"]
        .as_u64()
        .map(|n| n.clamp(1, 2000) as usize)
        .unwrap_or(200);
    let summary = kh.session_tool_call_summary(agent_id, limit)?;
    Ok(summary.to_string())
}

pub(crate) async fn tool_workspace_add(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let raw = input["path"]
        .as_str()
        .ok_or("Missing 'path' parameter")?
        .trim();
    if raw.is_empty() {
        return Err("path cannot be empty".into());
    }
    let target = std::path::PathBuf::from(raw);
    kh.add_workspace_path(&target)?;
    Ok(serde_json::json!({
        "status": "ok",
        "path": raw,
        "message": "workspace path added; effective on next agent turn"
    })
    .to_string())
}
