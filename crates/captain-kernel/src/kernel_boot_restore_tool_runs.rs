//! Reload persisted observable tool runs at boot.
//!
//! `captain_runtime::tool_runs::global_registry()` is a process-wide
//! in-memory registry that starts empty on every process start. Without
//! this step, any run still `Running` when Captain last shut down
//! (or any recent finished run) would be invisible to
//! `tool_run_status`/`tool_run_result`/`tool_run_list` after a restart.

use super::CaptainKernel;
use captain_memory::detached_tool_runs::{
    DetachedToolRunCompletion, DetachedToolRunRecord, DetachedToolRunStore,
};
use captain_runtime::tool_run_output::ToolRunOutputStore;
use captain_runtime::tool_runs::{global_registry, MAX_RUNS};
use tracing::{info, warn};

pub(super) fn restore_persisted_tool_runs(kernel: &CaptainKernel) {
    let store = DetachedToolRunStore::new(kernel.memory.usage_conn());
    let registry = global_registry();
    registry.configure_persistence(store.clone());
    let output_store = match captain_runtime::tool_run_output::ToolRunOutputStore::new(
        kernel.config.data_dir.join("tool-runs"),
    ) {
        Ok(output_store) => Some(output_store),
        Err(error) => {
            warn!("Failed to initialize owner-only tool-run output store: {error}");
            None
        }
    };

    let (interrupted, reconciliation_succeeded) = match store.reconcile_running_as_interrupted() {
        Ok(rows) => (rows.len(), true),
        Err(e) => {
            warn!("Failed to reconcile in-flight tool runs: {e}");
            (0, false)
        }
    };

    if reconciliation_succeeded {
        if let Some(output_store) = output_store.as_ref() {
            match store.list_recent(MAX_RUNS) {
                Ok(records) => {
                    let recoverable = records
                        .into_iter()
                        .filter(|record| {
                            record.status == "interrupted" && record.output_file_name.is_none()
                        })
                        .collect::<Vec<_>>();
                    if recover_interrupted_outputs(&store, output_store, &recoverable) {
                        if let Err(error) = output_store.discard_orphaned_captures() {
                            warn!(
                                "Failed to discard orphaned tool-run captures after recovery: {error}"
                            );
                        }
                    }
                }
                Err(error) => warn!(
                    "Failed to list interrupted tool runs; preserving partial captures: {error}"
                ),
            }
        }
    }
    if let Some(output_store) = output_store {
        registry.configure_output_store(output_store);
    }

    if let Err(e) = store.prune_terminal_history(MAX_RUNS) {
        warn!("Failed to prune persisted tool run history: {e}");
    }

    match store.list_recent(MAX_RUNS) {
        Ok(records) => {
            let count = records.len();
            registry.hydrate_from_persisted(records);
            if count > 0 {
                info!(
                    count,
                    interrupted, "Restored tool runs from persistent storage"
                );
            }
        }
        Err(e) => warn!("Failed to load persisted tool runs: {e}"),
    }
}

fn recover_interrupted_outputs(
    store: &DetachedToolRunStore,
    output_store: &ToolRunOutputStore,
    records: &[DetachedToolRunRecord],
) -> bool {
    let mut recovered_cleanly = true;
    for record in records {
        match output_store.recover_interrupted_output(&record.run_id) {
            Ok(Some(output)) => {
                if let Err(error) = store.mark_finished(
                    &record.run_id,
                    DetachedToolRunCompletion {
                        status: &record.status,
                        is_error: record.is_error,
                        result: record.result.as_deref(),
                        result_truncated: record.result_truncated,
                        output_file_name: Some(&output.file_name),
                        output_stored_bytes: Some(output.stored_bytes),
                        output_total_bytes: Some(output.total_bytes),
                        output_sha256: Some(&output.sha256),
                        output_capped: output.capped,
                        output_redacted: output.redacted,
                        finished_at_unix_ms: record
                            .finished_at_unix_ms
                            .unwrap_or(record.started_at_unix_ms),
                    },
                ) {
                    recovered_cleanly = false;
                    warn!(
                        run_id = %record.run_id,
                        "Recovered interrupted tool-run output but failed to attach its metadata: {error}"
                    );
                }
            }
            Ok(None) => {}
            Err(error) => {
                recovered_cleanly = false;
                warn!(
                    run_id = %record.run_id,
                    "Failed to recover interrupted tool-run output: {error}"
                );
            }
        }
    }
    recovered_cleanly
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_memory::migration::run_migrations;
    use rusqlite::Connection;
    use std::sync::{Arc, Mutex};

    #[test]
    fn interrupted_capture_is_attached_before_orphan_cleanup() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let store = DetachedToolRunStore::new(Arc::new(Mutex::new(conn)));
        store
            .upsert_running(
                "toolrun-power-cut",
                "shell_exec",
                Some("captain"),
                None,
                false,
                Some("digest"),
                1_000,
            )
            .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let output_store = ToolRunOutputStore::new(dir.path().join("tool-runs")).unwrap();
        let capture = output_store.begin_capture("toolrun-power-cut").unwrap();
        output_store
            .append_capture(
                &Mutex::new(capture),
                "password=power-cut-secret\nuseful partial\n",
            )
            .unwrap();

        let interrupted = store.reconcile_running_as_interrupted().unwrap();
        recover_interrupted_outputs(&store, &output_store, &interrupted);
        output_store.discard_orphaned_captures().unwrap();

        let record = store.list_recent(1).unwrap().remove(0);
        assert_eq!(record.status, "interrupted");
        assert!(record.output_redacted);
        let metadata = captain_runtime::tool_run_output::ToolRunOutputMetadata {
            file_name: record.output_file_name.unwrap(),
            stored_bytes: record.output_stored_bytes.unwrap(),
            total_bytes: record.output_total_bytes.unwrap(),
            sha256: record.output_sha256.unwrap(),
            capped: record.output_capped,
            redacted: record.output_redacted,
        };
        let page = output_store.read_lines(&metadata, 1, 10).unwrap();
        assert!(page.content.contains("password=[REDACTED]"));
        assert!(page.content.contains("useful partial"));
        assert!(!page.content.contains("power-cut-secret"));
    }

    #[test]
    fn interrupted_row_recovers_final_file_left_by_prior_attach_failure() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let store = DetachedToolRunStore::new(Arc::new(Mutex::new(conn)));
        store
            .upsert_running(
                "toolrun-prior-boot",
                "shell_exec",
                Some("captain"),
                None,
                true,
                None,
                2_000,
            )
            .unwrap();
        store.reconcile_running_as_interrupted().unwrap();

        let dir = tempfile::tempdir().unwrap();
        let output_store = ToolRunOutputStore::new(dir.path().join("tool-runs")).unwrap();
        output_store
            .persist_content("toolrun-prior-boot", "already committed evidence\n")
            .unwrap();

        let interrupted = store.list_recent(1).unwrap();
        assert!(recover_interrupted_outputs(
            &store,
            &output_store,
            &interrupted
        ));

        let record = store.list_recent(1).unwrap().remove(0);
        assert_eq!(record.status, "interrupted");
        assert!(record.output_file_name.is_some());
        assert_eq!(record.output_sha256.unwrap().len(), 64);
    }
}
