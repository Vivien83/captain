use super::CaptainKernel;
use captain_capspec::{CapabilityScope, CompiledCapability};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, warn};

impl CaptainKernel {
    pub(super) fn active_capspecs_for_workspace(
        &self,
        workspace: Option<&Path>,
    ) -> Vec<Arc<CompiledCapability>> {
        self.ensure_capspec_project_registered(workspace);
        if self.capspec_watcher.is_none() {
            if let Err(error) = self.capspec_registry.reload_global() {
                warn!(error = %error, "CapSpec turn-boundary global reload failed");
            }
        }
        self.capspec_registry
            .active_capabilities(workspace)
            .unwrap_or_else(|error| {
                warn!(error = %error, "CapSpec active catalog unavailable");
                Vec::new()
            })
    }

    pub fn capspec_operational_status(&self) -> serde_json::Value {
        let definitions = self.capspec_registry.list();
        let active_global = self.active_capspecs_for_workspace(None).len();
        let watcher = self
            .capspec_watcher
            .as_ref()
            .map(|watcher| watcher.status())
            .transpose();
        let runs = self.capspec_executor.list_runs(100);
        match (definitions, watcher, runs) {
            (Ok(definitions), Ok(watcher), Ok(runs)) => serde_json::json!({
                "ready": true,
                "definitions": definitions,
                "active_global": active_global,
                "watcher": watcher,
                "fallback_reload": self.capspec_watcher.is_none(),
                "recent_runs": runs,
            }),
            (definitions, watcher, runs) => serde_json::json!({
                "ready": false,
                "error": definitions.err().map(|error| error.to_string())
                    .or_else(|| watcher.err().map(|error| error.to_string()))
                    .or_else(|| runs.err().map(|error| error.to_string())),
            }),
        }
    }

    fn ensure_capspec_project_registered(&self, workspace: Option<&Path>) {
        let Some(workspace) = workspace.and_then(canonical_project_with_capabilities) else {
            return;
        };
        match self.register_capspec_project_scope(&workspace, false) {
            Ok(_) => {}
            Err(error) => {
                warn!(error = %error, workspace = %workspace.display(), "CapSpec project registration failed");
            }
        }
    }

    pub(super) fn register_capspec_project_scope(
        &self,
        workspace: &Path,
        create: bool,
    ) -> Result<CapabilityScope, String> {
        let workspace = workspace
            .canonicalize()
            .map_err(|error| format!("invalid CapSpec project workspace: {error}"))?;
        let root = workspace.join(".captain").join("capabilities");
        if !create && !root.is_dir() {
            return Err(format!(
                "CapSpec project directory does not exist: {}",
                root.display()
            ));
        }

        let scope = CapabilityScope::Project(workspace.clone());
        let already_registered = self
            .capspec_registry
            .scope_registered(&scope)
            .map_err(|error| error.to_string())?;
        let (scope, discovered) = if already_registered {
            (scope, None)
        } else {
            let (scope, report) = self
                .capspec_registry
                .register_project(&workspace)
                .map_err(|error| error.to_string())?;
            (scope, Some(report.discovered))
        };
        if let Some(watcher) = &self.capspec_watcher {
            watcher
                .watch_scope(&scope)
                .map_err(|error| format!("CapSpec project watcher failed: {error}"))?;
        }
        if let Some(discovered) = discovered {
            debug!(
                workspace = %workspace.display(),
                discovered,
                "CapSpec project scope registered"
            );
        }
        Ok(scope)
    }
}

fn canonical_project_with_capabilities(workspace: &Path) -> Option<PathBuf> {
    if !workspace.join(".captain").join("capabilities").is_dir() {
        return None;
    }
    workspace.canonicalize().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn project_without_capability_directory_is_not_registered() {
        let temp = TempDir::new().unwrap();
        assert!(canonical_project_with_capabilities(temp.path()).is_none());
    }

    #[test]
    fn existing_project_capability_directory_is_canonicalized() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir_all(temp.path().join(".captain/capabilities")).unwrap();
        assert_eq!(
            canonical_project_with_capabilities(temp.path()),
            temp.path().canonicalize().ok()
        );
    }
}
