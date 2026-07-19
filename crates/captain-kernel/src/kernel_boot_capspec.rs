use crate::error::{KernelError, KernelResult};
use captain_capspec::{CapabilityExecutor, CapabilityRegistry, CapabilityWatcher};
use captain_types::config::KernelConfig;
use captain_types::error::CaptainError;
use std::sync::Arc;
use tracing::{info, warn};

pub(super) struct BootCapSpec {
    pub(super) registry: Arc<CapabilityRegistry>,
    pub(super) executor: Arc<CapabilityExecutor>,
    pub(super) watcher: Option<CapabilityWatcher>,
}

pub(super) fn build_boot_capspec(config: &KernelConfig) -> KernelResult<BootCapSpec> {
    let source_root = config.home_dir.join("capabilities");
    let database = config.data_dir.join("capabilities.db");
    let registry = Arc::new(CapabilityRegistry::open(&source_root, &database).map_err(
        |error| {
            KernelError::Captain(CaptainError::Config(format!(
                "initialize CapSpec registry: {error}"
            )))
        },
    )?);
    let state_key = config.data_dir.join("capabilities.key");
    let executor = Arc::new(
        CapabilityExecutor::open(Arc::clone(&registry), &database, &state_key).map_err(
            |error| {
                KernelError::Captain(CaptainError::Config(format!(
                    "initialize durable CapSpec executor: {error}"
                )))
            },
        )?,
    );
    let watcher = match CapabilityWatcher::new(Arc::clone(&registry), config.reload.debounce_ms) {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            warn!(error = %error, "CapSpec watcher unavailable; using turn-boundary catalog reload");
            None
        }
    };
    info!(
        root = %source_root.display(),
        watcher_ready = watcher.is_some(),
        "CapSpec registry initialized"
    );
    Ok(BootCapSpec {
        registry,
        executor,
        watcher,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn boot_capspec_creates_private_durable_registry_and_owned_watcher() {
        let temp = TempDir::new().unwrap();
        let mut config = KernelConfig::default();
        config.home_dir = temp.path().join("home");
        config.data_dir = config.home_dir.join("data");

        let boot = build_boot_capspec(&config).unwrap();

        assert!(config.home_dir.join("capabilities").is_dir());
        assert!(config.data_dir.join("capabilities.db").is_file());
        assert!(config.data_dir.join("capabilities.key").is_file());
        assert!(boot.watcher.is_some());
        assert!(boot.registry.list().unwrap().is_empty());
        assert!(boot.executor.list_runs(10).unwrap().is_empty());
    }
}
