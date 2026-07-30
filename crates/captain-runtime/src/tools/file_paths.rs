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
    let blocked_owned = kernel
        .map(|k| k.blocked_workspace_paths())
        .unwrap_or_default();
    if additional.is_empty() && blocked_owned.is_empty() {
        return resolve_file_path(raw_path, workspace_root);
    }

    let mut allowed: Vec<&Path> = Vec::new();
    if let Some(root) = workspace_root {
        allowed.push(root);
    }
    for root in &additional {
        allowed.push(root.as_path());
    }

    let blocked_refs: Vec<&Path> = blocked_owned.iter().map(|path| path.as_path()).collect();
    crate::workspace_sandbox::resolve_sandbox_path_multi(raw_path, &allowed, &blocked_refs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct BlockedPathKernel {
        blocked: PathBuf,
    }

    #[async_trait]
    impl KernelHandle for BlockedPathKernel {
        async fn spawn_agent(
            &self,
            _manifest_toml: &str,
            _parent_id: Option<&str>,
        ) -> Result<(String, String), String> {
            Err("not available in this test".to_string())
        }

        async fn send_to_agent(&self, _agent_id: &str, _message: &str) -> Result<String, String> {
            Err("not available in this test".to_string())
        }

        fn list_agents(&self) -> Vec<crate::kernel_handle::AgentInfo> {
            Vec::new()
        }

        fn kill_agent(&self, _agent_id: &str) -> Result<(), String> {
            Err("not available in this test".to_string())
        }

        fn memory_store(&self, _key: &str, _value: serde_json::Value) -> Result<(), String> {
            Ok(())
        }

        fn memory_recall(&self, _key: &str) -> Result<Option<serde_json::Value>, String> {
            Ok(None)
        }

        fn find_agents(&self, _query: &str) -> Vec<crate::kernel_handle::AgentInfo> {
            Vec::new()
        }

        async fn task_post(
            &self,
            _title: &str,
            _description: &str,
            _assigned_to: Option<&str>,
            _created_by: Option<&str>,
        ) -> Result<String, String> {
            Err("not available in this test".to_string())
        }

        async fn task_claim(&self, _agent_id: &str) -> Result<Option<serde_json::Value>, String> {
            Ok(None)
        }

        async fn task_complete(&self, _task_id: &str, _result: &str) -> Result<(), String> {
            Ok(())
        }

        fn blocked_workspace_paths(&self) -> Vec<PathBuf> {
            vec![self.blocked.clone()]
        }
    }

    #[test]
    fn blocklist_applies_to_ordinary_agents_without_extra_roots() {
        let workspace = tempfile::tempdir().unwrap();
        let secret = workspace.path().join("mounted-secret");
        let ordinary = workspace.path().join("ordinary.txt");
        std::fs::write(&secret, "secret").unwrap();
        std::fs::write(&ordinary, "ordinary").unwrap();
        let kernel: Arc<dyn KernelHandle> = Arc::new(BlockedPathKernel {
            blocked: secret.clone(),
        });

        let denied = resolve_file_path_for_caller(
            secret.to_str().unwrap(),
            Some(workspace.path()),
            Some(&kernel),
            Some("ordinary-agent"),
        )
        .unwrap_err();
        assert!(denied.contains("protected zone"), "{denied}");

        let resolved = resolve_file_path_for_caller(
            ordinary.to_str().unwrap(),
            Some(workspace.path()),
            Some(&kernel),
            Some("ordinary-agent"),
        )
        .unwrap();
        assert_eq!(resolved, ordinary.canonicalize().unwrap());
    }
}
