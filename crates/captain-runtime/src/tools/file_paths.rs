//! Workspace-safe file path resolution shared by tool handlers.

use crate::kernel_handle::KernelHandle;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Reject path traversal attempts before falling back to legacy path handling.
pub(crate) fn validate_path(path: &str) -> Result<&str, String> {
    for component in Path::new(path).components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err("Path traversal denied: '..' components are forbidden".to_string());
        }
    }
    Ok(path)
}

/// Resolve a file path through the workspace sandbox when a root is available.
pub(crate) fn resolve_file_path(
    raw_path: &str,
    workspace_root: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(root) = workspace_root {
        crate::workspace_sandbox::resolve_sandbox_path(raw_path, root)
    } else {
        let _ = validate_path(raw_path)?;
        Ok(PathBuf::from(raw_path))
    }
}

/// Resolve a path with caller-specific extra roots and blocklisted paths.
pub(crate) fn resolve_file_path_for_caller(
    raw_path: &str,
    workspace_root: Option<&Path>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<PathBuf, String> {
    let additional = kernel
        .map(|k| k.additional_workspace_roots(caller_agent_id))
        .unwrap_or_default();
    if additional.is_empty() {
        return resolve_file_path(raw_path, workspace_root);
    }

    let mut allowed: Vec<&Path> = Vec::new();
    if let Some(root) = workspace_root {
        allowed.push(root);
    }
    for root in &additional {
        allowed.push(root.as_path());
    }

    let blocked_owned = kernel
        .map(|k| k.blocked_workspace_paths())
        .unwrap_or_default();
    let blocked_refs: Vec<&Path> = blocked_owned.iter().map(|path| path.as_path()).collect();
    crate::workspace_sandbox::resolve_sandbox_path_multi(raw_path, &allowed, &blocked_refs)
}
