use crate::kernel_handle::KernelHandle;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

mod file_ops;
mod file_search;
mod patch_ops;

pub(crate) use file_ops::{
    tool_edit_file, tool_file_list, tool_file_read, tool_file_write, tool_multi_edit,
};
pub(crate) use file_search::{tool_file_inspect_batch, tool_glob, tool_grep};
pub(crate) use patch_ops::tool_apply_patch;

pub(crate) fn resolve_file_path_for_caller(
    raw_path: &str,
    workspace_root: Option<&Path>,
    _kernel: Option<&Arc<dyn KernelHandle>>,
    _caller_agent_id: Option<&str>,
) -> Result<PathBuf, String> {
    let root =
        workspace_root.ok_or_else(|| "A local Node workspace root is required".to_string())?;
    crate::workspace_sandbox::resolve_sandbox_path(raw_path, root)
}

pub(crate) fn ensure_no_secret_literal(
    _tool_name: &str,
    _field: &str,
    text: &str,
) -> Result<(), String> {
    crate::output_security::ensure_no_secret_literal(text)
}

pub(crate) fn truncate_owned(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str("\n...[truncated]");
    truncated
}
