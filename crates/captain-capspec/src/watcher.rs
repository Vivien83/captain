use crate::{CapabilityRegistry, CapabilityScope, RegistryError, ReloadReport};
use chrono::Utc;
use notify_debouncer_mini::{
    new_debouncer_opt,
    notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher},
    Config as DebouncerConfig, DebounceEventResult, Debouncer,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{info, warn};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilityWatcherStatus {
    pub watched_roots: Vec<PathBuf>,
    pub successful_reloads: u64,
    pub failed_reloads: u64,
    pub last_reload_at: Option<String>,
    pub last_report: Option<ReloadReport>,
    pub last_error: Option<String>,
}

pub struct CapabilityWatcher<T: Watcher = RecommendedWatcher> {
    registry: Arc<CapabilityRegistry>,
    debouncer: Mutex<Debouncer<T>>,
    watched_roots: Mutex<BTreeSet<PathBuf>>,
    status: Arc<Mutex<CapabilityWatcherStatus>>,
}

impl CapabilityWatcher<RecommendedWatcher> {
    pub fn new(registry: Arc<CapabilityRegistry>, debounce_ms: u64) -> Result<Self, RegistryError> {
        Self::new_with_backend(registry, debounce_ms, NotifyConfig::default())
    }
}

impl<T> CapabilityWatcher<T>
where
    T: Watcher + 'static,
{
    fn new_with_backend(
        registry: Arc<CapabilityRegistry>,
        debounce_ms: u64,
        notify_config: NotifyConfig,
    ) -> Result<Self, RegistryError> {
        let status = Arc::new(Mutex::new(CapabilityWatcherStatus::default()));
        let registry_for_handler = Arc::clone(&registry);
        let status_for_handler = Arc::clone(&status);
        let config = DebouncerConfig::default()
            .with_timeout(Duration::from_millis(debounce_ms.max(50)))
            .with_notify_config(notify_config);
        let debouncer =
            new_debouncer_opt::<_, T>(config, move |events: DebounceEventResult| match events {
                Ok(events) if !events.is_empty() => {
                    record_reload(&registry_for_handler, &status_for_handler);
                }
                Ok(_) => {}
                Err(error) => record_watcher_error(&status_for_handler, error.to_string()),
            })
            .map_err(|error| watcher_error("initialize", error))?;
        let watcher = Self {
            registry,
            debouncer: Mutex::new(debouncer),
            watched_roots: Mutex::new(BTreeSet::new()),
            status,
        };
        for (_, root) in watcher.registry.source_roots()? {
            watcher.watch_root(&root)?;
        }
        Ok(watcher)
    }

    pub fn watch_scope(&self, scope: &CapabilityScope) -> Result<(), RegistryError> {
        let root = self
            .registry
            .source_roots()?
            .into_iter()
            .find_map(|(candidate, path)| (candidate == *scope).then_some(path))
            .ok_or_else(|| RegistryError::UnknownScope(scope.key()))?;
        self.watch_root(&root)
    }

    pub fn status(&self) -> Result<CapabilityWatcherStatus, RegistryError> {
        let mut status = self
            .status
            .lock()
            .map_err(|_| RegistryError::Poisoned)?
            .clone();
        status.watched_roots = self
            .watched_roots
            .lock()
            .map_err(|_| RegistryError::Poisoned)?
            .iter()
            .cloned()
            .collect();
        Ok(status)
    }

    fn watch_root(&self, root: &Path) -> Result<(), RegistryError> {
        let root = root.canonicalize()?;
        let metadata = std::fs::symlink_metadata(&root)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(RegistryError::InvalidSourcePath(root.display().to_string()));
        }
        let mut watched = self
            .watched_roots
            .lock()
            .map_err(|_| RegistryError::Poisoned)?;
        if watched.contains(&root) {
            return Ok(());
        }
        self.debouncer
            .lock()
            .map_err(|_| RegistryError::Poisoned)?
            .watcher()
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|error| watcher_error("watch", error))?;
        watched.insert(root.clone());
        info!(root = %root.display(), "CapSpec hot reload watcher armed");
        Ok(())
    }
}

fn record_reload(registry: &CapabilityRegistry, status: &Mutex<CapabilityWatcherStatus>) {
    let outcome = registry.reload_all();
    let Ok(mut state) = status.lock() else {
        return;
    };
    state.last_reload_at = Some(Utc::now().to_rfc3339());
    match outcome {
        Ok(report) => {
            state.successful_reloads = state.successful_reloads.saturating_add(1);
            state.last_error = None;
            state.last_report = Some(report);
        }
        Err(error) => {
            state.failed_reloads = state.failed_reloads.saturating_add(1);
            state.last_error = Some(error.to_string());
            warn!(error = %error, "CapSpec hot reload failed");
        }
    }
}

fn record_watcher_error(status: &Mutex<CapabilityWatcherStatus>, error: String) {
    if let Ok(mut state) = status.lock() {
        state.failed_reloads = state.failed_reloads.saturating_add(1);
        state.last_reload_at = Some(Utc::now().to_rfc3339());
        state.last_error = Some(error.clone());
    }
    warn!(error = %error, "CapSpec filesystem watcher failed");
}

fn watcher_error(action: &str, error: impl std::fmt::Display) -> RegistryError {
    RegistryError::Io(std::io::Error::other(format!(
        "CapSpec watcher {action}: {error}"
    )))
}

#[cfg(test)]
#[path = "watcher_tests.rs"]
mod tests;
