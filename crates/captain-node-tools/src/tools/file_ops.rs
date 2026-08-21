//! Basic file read/write/list/edit handlers.

use crate::kernel_handle::KernelHandle;
use crate::tools::{ensure_no_secret_literal, resolve_file_path_for_caller};
use std::path::Path;
use std::sync::Arc;

pub(crate) async fn tool_file_read(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let resolved = resolve_file_path_for_caller(raw_path, workspace_root, kernel, caller_agent_id)?;
    tokio::fs::read_to_string(&resolved)
        .await
        .map_err(|e| format!("Failed to read file: {e}"))
}

pub(crate) async fn tool_file_write(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let resolved = resolve_file_path_for_caller(raw_path, workspace_root, kernel, caller_agent_id)?;
    let content = input["content"]
        .as_str()
        .ok_or("Missing 'content' parameter")?;
    ensure_no_secret_literal("file_write", "content", content)?;
    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create directories: {e}"))?;
    }
    tokio::fs::write(&resolved, content)
        .await
        .map_err(|e| format!("Failed to write file: {e}"))?;
    Ok(format!(
        "Successfully wrote {} bytes to {}",
        content.len(),
        resolved.display()
    ))
}

pub(crate) async fn tool_file_list(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let resolved = resolve_file_path_for_caller(raw_path, workspace_root, kernel, caller_agent_id)?;
    let mut entries = tokio::fs::read_dir(&resolved)
        .await
        .map_err(|e| format!("Failed to list directory: {e}"))?;
    let mut files = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("Failed to read entry: {e}"))?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        let metadata = entry.metadata().await;
        let suffix = match metadata {
            Ok(m) if m.is_dir() => "/",
            _ => "",
        };
        files.push(format!("{name}{suffix}"));
    }
    files.sort();
    Ok(files.join("\n"))
}

pub(crate) async fn tool_edit_file(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let old = input["old_string"]
        .as_str()
        .ok_or("Missing 'old_string' parameter")?;
    let new = input["new_string"]
        .as_str()
        .ok_or("Missing 'new_string' parameter")?;
    ensure_no_secret_literal("edit_file", "new_string", new)?;
    let replace_all = input["replace_all"].as_bool().unwrap_or(false);
    let resolved = resolve_file_path_for_caller(raw_path, workspace_root, kernel, caller_agent_id)?;

    let content = tokio::fs::read_to_string(&resolved)
        .await
        .map_err(|e| format!("Failed to read {}: {e}", resolved.display()))?;

    let result = crate::edit_strategies::try_edit(&content, old, new, replace_all)?;
    tokio::fs::write(&resolved, &result.new_content)
        .await
        .map_err(|e| format!("Failed to write {}: {e}", resolved.display()))?;

    Ok(format!(
        "Edited {} via `{}` ({} replacement{}, {}→{} bytes)",
        resolved.display(),
        result.strategy,
        result.replacements,
        if result.replacements > 1 { "s" } else { "" },
        content.len(),
        result.new_content.len()
    ))
}

pub(crate) async fn tool_multi_edit(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let edits = input["edits"]
        .as_array()
        .ok_or("Missing 'edits' parameter (must be an array)")?;
    if edits.is_empty() {
        return Err("'edits' array is empty — nothing to do".into());
    }
    let resolved = resolve_file_path_for_caller(raw_path, workspace_root, kernel, caller_agent_id)?;

    let original = tokio::fs::read_to_string(&resolved)
        .await
        .map_err(|e| format!("Failed to read {}: {e}", resolved.display()))?;

    let mut working = original.clone();
    let mut applied: Vec<(&str, usize)> = Vec::with_capacity(edits.len());

    for (idx, edit) in edits.iter().enumerate() {
        let old = edit["old_string"]
            .as_str()
            .ok_or_else(|| format!("edit[{idx}] missing 'old_string'"))?;
        let new = edit["new_string"]
            .as_str()
            .ok_or_else(|| format!("edit[{idx}] missing 'new_string'"))?;
        ensure_no_secret_literal("multi_edit", &format!("edits[{idx}].new_string"), new)?;
        let replace_all = edit["replace_all"].as_bool().unwrap_or(false);

        let result =
            crate::edit_strategies::try_edit(&working, old, new, replace_all).map_err(|e| {
                format!(
                    "edit[{idx}] failed: {e}. \
                     Atomic abort — {} prior edit{} rolled back, file untouched.",
                    applied.len(),
                    if applied.len() == 1 { "" } else { "s" }
                )
            })?;
        working = result.new_content;
        applied.push((result.strategy, result.replacements));
    }

    tokio::fs::write(&resolved, &working)
        .await
        .map_err(|e| format!("Failed to write {}: {e}", resolved.display()))?;

    let strategies: Vec<String> = applied
        .iter()
        .enumerate()
        .map(|(i, (s, n))| format!("[{i}]={s}({n})"))
        .collect();
    Ok(format!(
        "Atomically applied {} edit{} to {} ({}→{} bytes). Strategies: {}",
        applied.len(),
        if applied.len() == 1 { "" } else { "s" },
        resolved.display(),
        original.len(),
        working.len(),
        strategies.join(", ")
    ))
}
