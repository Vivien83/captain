//! Reload persisted detached tool runs (`tool_run_start`) at boot.
//!
//! `captain_runtime::tool_runs::global_registry()` is a process-wide
//! in-memory registry that starts empty on every process start. Without
//! this step, any detached run still `Running` when Captain last shut down
//! (or any recent finished run) would be invisible to
//! `tool_run_status`/`tool_run_result`/`tool_run_list` after a restart.

use super::CaptainKernel;
use captain_memory::detached_tool_runs::DetachedToolRunStore;
use captain_runtime::tool_runs::{global_registry, MAX_RUNS};
use tracing::{info, warn};

pub(super) fn restore_persisted_tool_runs(kernel: &CaptainKernel) {
    let store = DetachedToolRunStore::new(kernel.memory.usage_conn());
    let registry = global_registry();
    registry.configure_persistence(store.clone());

    let interrupted = match store.reconcile_running_as_interrupted() {
        Ok(rows) => rows.len(),
        Err(e) => {
            warn!("Failed to reconcile in-flight detached tool runs: {e}");
            0
        }
    };
    if let Err(e) = store.prune_terminal_history(MAX_RUNS) {
        warn!("Failed to prune persisted detached tool run history: {e}");
    }

    match store.list_recent(MAX_RUNS) {
        Ok(records) => {
            let count = records.len();
            registry.hydrate_from_persisted(records);
            if count > 0 {
                info!(
                    count,
                    interrupted, "Restored detached tool runs from persistent storage"
                );
            }
        }
        Err(e) => warn!("Failed to load persisted detached tool runs: {e}"),
    }
}
