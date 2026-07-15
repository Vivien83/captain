//! Write-through orchestrator (v3.12a).
//!
//! This module sits between callers that want to persist a semantic
//! triple to MemPalace (the `memory_store` tool, `mirror_to_mempalace`,
//! the future LearningCommitter) and the MCP backend. It guarantees:
//!
//! - Local persistence **always** succeeds (except IO). The caller gets
//!   `Ok(id)` regardless of MemPalace availability.
//! - MemPalace is attempted synchronously; protocol-level success flips the
//!   row to `synced`, while transport or tool failure enters durable backoff.
//! - A background worker replays both `pending` and degraded `error` rows.
//! - One backend outage cannot consume the retry budget of an entire batch.
//!
//! The sender is abstracted behind `MemPalaceSender` so the orchestrator
//! is trivially mockable in unit tests without spinning up an MCP
//! subprocess.

use async_trait::async_trait;
use captain_memory::memory_writer::{
    self, MemoryOperation, MemoryWrite, NewMemoryWrite, SyncStatus,
};
use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tracing::{info, warn};

use crate::mcp::McpConnection;

/// Abstraction over "push a row to MemPalace" so tests can mock it.
#[async_trait]
pub trait MemPalaceSender: Send + Sync {
    async fn send(&self, row: &MemoryWrite) -> Result<(), String>;
}

/// Thin wrapper over the shared MCP connection pool that targets the
/// `mempalace` server specifically.
pub struct McpMemPalaceSender<'a> {
    pub mcp_conns: &'a AsyncMutex<Vec<McpConnection>>,
}

#[async_trait]
impl<'a> MemPalaceSender for McpMemPalaceSender<'a> {
    async fn send(&self, row: &MemoryWrite) -> Result<(), String> {
        let mut conns = self.mcp_conns.lock().await;
        let conn = conns
            .iter_mut()
            .find(|c| c.name() == "mempalace")
            .ok_or("mempalace MCP server not connected")?;

        // MemPalace KG operations accept only the bare triple. The wing/room
        // taxonomy lives in Captain's local journal.
        let input = serde_json::json!({
            "subject": row.subject,
            "predicate": row.predicate,
            "object": row.object,
        });
        let tool_name = match row.operation {
            MemoryOperation::Add => "mcp_mempalace_mempalace_kg_add",
            MemoryOperation::Invalidate => "mcp_mempalace_mempalace_kg_invalidate",
        };
        let response = conn
            .call_tool(tool_name, &input)
            .await
            .map_err(|e| format!("{tool_name} failed: {e}"))?;
        validate_mempalace_response(tool_name, &response)
    }
}

fn validate_mempalace_response(tool_name: &str, response: &str) -> Result<(), String> {
    let trimmed = response.trim();
    if trimmed.is_empty() {
        return Err(format!("{tool_name} returned an empty response"));
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return Ok(());
    };
    if value.get("success").and_then(serde_json::Value::as_bool) == Some(false)
        || value.get("error").is_some_and(|error| !error.is_null())
    {
        let detail = value
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("MemPalace rejected the operation");
        return Err(format!("{tool_name} rejected operation: {detail}"));
    }
    Ok(())
}

/// Core write-through entry point.
///
/// Always persists `record` locally first. Then attempts MemPalace via
/// `sender` and flips the row's `sync_status` accordingly. Returns the
/// row id on success (whether synced or pending).
pub async fn write_through(
    conn: Arc<Mutex<Connection>>,
    sender: Option<&dyn MemPalaceSender>,
    record: NewMemoryWrite,
) -> Result<String, String> {
    // Step 1 — local persistence (always)
    let row = {
        let guard = conn.lock().map_err(|e| format!("sqlite poisoned: {e}"))?;
        memory_writer::append(&guard, record).map_err(|e| format!("sqlite append: {e}"))?
    };

    sync_existing_write(Arc::clone(&conn), sender, &row).await?;

    Ok(row.id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncAttemptReport {
    pub attempted: bool,
    pub status: SyncStatus,
    pub error: Option<String>,
}

/// Attempt one already-journaled operation and persist the outcome. This is
/// used for invalidations created atomically by `memory_forget` as well as
/// normal add rows.
pub async fn sync_existing_write(
    conn: Arc<Mutex<Connection>>,
    sender: Option<&dyn MemPalaceSender>,
    row: &MemoryWrite,
) -> Result<SyncAttemptReport, String> {
    let Some(sender) = sender else {
        let guard = conn.lock().map_err(|e| format!("sqlite poisoned: {e}"))?;
        memory_writer::mark_deferred(&guard, &row.id, "mempalace sender not ready")
            .map_err(|e| format!("sqlite mark_deferred: {e}"))?;
        return Ok(SyncAttemptReport {
            attempted: false,
            status: if row.sync_status == SyncStatus::Error {
                SyncStatus::Error
            } else {
                SyncStatus::Pending
            },
            error: None,
        });
    };

    match sender.send(row).await {
        Ok(()) => {
            let guard = conn.lock().map_err(|e| format!("sqlite poisoned: {e}"))?;
            memory_writer::mark_synced(&guard, &row.id)
                .map_err(|e| format!("sqlite mark_synced: {e}"))?;
            Ok(SyncAttemptReport {
                attempted: true,
                status: SyncStatus::Synced,
                error: None,
            })
        }
        Err(error) => {
            let guard = conn.lock().map_err(|e| format!("sqlite poisoned: {e}"))?;
            let status = memory_writer::mark_error(&guard, &row.id, &error)
                .map_err(|e| format!("sqlite mark_error: {e}"))?;
            Ok(SyncAttemptReport {
                attempted: true,
                status,
                error: Some(error),
            })
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ResyncReport {
    pub attempted: usize,
    pub synced: usize,
    pub still_pending: usize,
    pub errored_out: usize,
    pub deferred: usize,
}

/// Retry due pending/degraded rows (up to `batch_limit`). After one failure,
/// the batch stops: a backend-wide outage must not age every queued fact at
/// once. Durable per-row backoff makes later ticks spread probes safely.
pub async fn resync_pending(
    conn: Arc<Mutex<Connection>>,
    sender: Option<&dyn MemPalaceSender>,
    batch_limit: usize,
) -> Result<ResyncReport, String> {
    let retryable = {
        let guard = conn.lock().map_err(|e| format!("sqlite poisoned: {e}"))?;
        memory_writer::list_retryable(&guard, memory_writer::now_ms(), batch_limit)
            .map_err(|e| format!("sqlite list_retryable: {e}"))?
    };

    let mut report = ResyncReport::default();
    let Some(sender) = sender else {
        report.deferred = retryable.len();
        return Ok(report);
    };

    for row in &retryable {
        let outcome = sync_existing_write(Arc::clone(&conn), Some(sender), row).await?;
        report.attempted += usize::from(outcome.attempted);
        match outcome.status {
            SyncStatus::Synced => report.synced += 1,
            SyncStatus::Pending => report.still_pending += 1,
            SyncStatus::Error => report.errored_out += 1,
        }
        if outcome.error.is_some() {
            report.deferred = retryable.len().saturating_sub(report.attempted);
            break;
        }
    }

    if report.attempted > 0 {
        info!(
            attempted = report.attempted,
            synced = report.synced,
            still_pending = report.still_pending,
            errored = report.errored_out,
            deferred = report.deferred,
            "memory_writer resync tick"
        );
    }
    Ok(report)
}

/// Spawn the background resync worker. It ticks every 30 seconds. Degraded
/// rows are never garbage-collected: they remain durable and recoverable.
///
/// The worker is fire-and-forget: it never panics, never blocks the
/// kernel, and silently drops if SQLite or MCP disappears.
pub fn spawn_resync_worker(
    conn: Arc<Mutex<Connection>>,
    mcp_conns: Arc<AsyncMutex<Vec<McpConnection>>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let tick = Duration::from_secs(30);
        let mut interval = tokio::time::interval(tick);
        loop {
            interval.tick().await;

            let sender = McpMemPalaceSender {
                mcp_conns: &mcp_conns,
            };
            if let Err(e) = resync_pending(Arc::clone(&conn), Some(&sender), 250).await {
                warn!(error = %e, "resync_pending failed");
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use captain_memory::memory_writer as store;
    use captain_memory::migration::run_migrations;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn fresh_db() -> Arc<Mutex<Connection>> {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        Arc::new(Mutex::new(conn))
    }

    fn sample() -> NewMemoryWrite {
        NewMemoryWrite {
            subject: "user".into(),
            predicate: "prefers".into(),
            object: "dark_mode".into(),
            wing: None,
            room: None,
            source: "unit_test".into(),
        }
    }

    fn make_all_retryable(db: &Arc<Mutex<Connection>>) {
        db.lock()
            .unwrap()
            .execute(
                "UPDATE memory_writes SET next_retry_at = 0 WHERE sync_status != 'synced'",
                [],
            )
            .unwrap();
    }

    struct OkSender {
        calls: AtomicUsize,
    }
    #[async_trait]
    impl MemPalaceSender for OkSender {
        async fn send(&self, _row: &MemoryWrite) -> Result<(), String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct FailSender;
    #[async_trait]
    impl MemPalaceSender for FailSender {
        async fn send(&self, _row: &MemoryWrite) -> Result<(), String> {
            Err("mempalace unreachable".into())
        }
    }

    /// A sender that fails N times then succeeds.
    struct FlakySender {
        remaining_failures: Arc<std::sync::atomic::AtomicI32>,
    }
    #[async_trait]
    impl MemPalaceSender for FlakySender {
        async fn send(&self, _row: &MemoryWrite) -> Result<(), String> {
            let left = self.remaining_failures.fetch_sub(1, Ordering::SeqCst);
            if left > 0 {
                Err("temporary".into())
            } else {
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn write_through_no_sender_returns_ok_without_spending_attempt() {
        let db = fresh_db();
        let id = write_through(Arc::clone(&db), None, sample())
            .await
            .unwrap();
        let guard = db.lock().unwrap();
        let row = store::get(&guard, &id).unwrap().unwrap();
        assert_eq!(row.sync_status, SyncStatus::Pending);
        assert_eq!(row.sync_attempts, 0);
        assert!(row.last_error.is_some());
    }

    #[tokio::test]
    async fn write_through_ok_sender_marks_synced() {
        let db = fresh_db();
        let sender = OkSender {
            calls: AtomicUsize::new(0),
        };
        let id = write_through(Arc::clone(&db), Some(&sender), sample())
            .await
            .unwrap();
        assert_eq!(sender.calls.load(Ordering::SeqCst), 1);
        let guard = db.lock().unwrap();
        let row = store::get(&guard, &id).unwrap().unwrap();
        assert_eq!(row.sync_status, SyncStatus::Synced);
        assert!(row.synced_at.is_some());
        assert!(row.last_error.is_none());
    }

    #[tokio::test]
    async fn write_through_failure_does_not_lose_data() {
        let db = fresh_db();
        let id = write_through(Arc::clone(&db), Some(&FailSender), sample())
            .await
            .unwrap();
        let guard = db.lock().unwrap();
        let row = store::get(&guard, &id).unwrap().unwrap();
        assert_eq!(row.sync_status, SyncStatus::Pending);
        assert_eq!(row.sync_attempts, 1);
    }

    #[tokio::test]
    async fn resync_flushes_pending_after_reconnect() {
        let db = fresh_db();

        // Write 3 rows while MemPalace is "down"
        for _ in 0..3 {
            write_through(Arc::clone(&db), Some(&FailSender), sample())
                .await
                .unwrap();
        }
        make_all_retryable(&db);
        {
            let guard = db.lock().unwrap();
            assert_eq!(
                store::count_by_status(&guard, SyncStatus::Pending).unwrap(),
                3
            );
        }

        // MemPalace comes back — resync should drain the queue
        let sender = OkSender {
            calls: AtomicUsize::new(0),
        };
        let report = resync_pending(Arc::clone(&db), Some(&sender), 100)
            .await
            .unwrap();
        assert_eq!(report.attempted, 3);
        assert_eq!(report.synced, 3);
        assert_eq!(sender.calls.load(Ordering::SeqCst), 3);
        let guard = db.lock().unwrap();
        assert_eq!(
            store::count_by_status(&guard, SyncStatus::Synced).unwrap(),
            3
        );
        assert_eq!(
            store::count_by_status(&guard, SyncStatus::Pending).unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn resync_retry_cap_transitions_to_error() {
        let db = fresh_db();

        // 1st attempt (write_through) fails: attempts=1
        let id = write_through(Arc::clone(&db), Some(&FailSender), sample())
            .await
            .unwrap();
        // 2nd resync pass: attempts=2, still pending
        make_all_retryable(&db);
        resync_pending(Arc::clone(&db), Some(&FailSender), 100)
            .await
            .unwrap();
        // 3rd resync pass: attempts=3, transitions to error
        make_all_retryable(&db);
        let report = resync_pending(Arc::clone(&db), Some(&FailSender), 100)
            .await
            .unwrap();
        assert_eq!(report.errored_out, 1);

        {
            let guard = db.lock().unwrap();
            let row = store::get(&guard, &id).unwrap().unwrap();
            assert_eq!(row.sync_status, SyncStatus::Error);
            assert_eq!(row.sync_attempts, 3);
        }

        // Error is degraded, not dead: after backoff it remains retryable.
        make_all_retryable(&db);
        let report4 = resync_pending(Arc::clone(&db), Some(&FailSender), 100)
            .await
            .unwrap();
        assert_eq!(report4.attempted, 1);
        assert_eq!(report4.errored_out, 1);
        let guard = db.lock().unwrap();
        assert_eq!(store::get(&guard, &id).unwrap().unwrap().sync_attempts, 4);
    }

    #[tokio::test]
    async fn missing_sender_does_not_hide_degraded_row() {
        let db = fresh_db();
        let row = store::append(&db.lock().unwrap(), sample()).unwrap();
        {
            let guard = db.lock().unwrap();
            for _ in 0..3 {
                store::mark_error(&guard, &row.id, "backend offline").unwrap();
            }
        }
        let degraded = store::get(&db.lock().unwrap(), &row.id).unwrap().unwrap();
        let outcome = sync_existing_write(Arc::clone(&db), None, &degraded)
            .await
            .unwrap();
        assert!(!outcome.attempted);
        assert_eq!(outcome.status, SyncStatus::Error);
        let persisted = store::get(&db.lock().unwrap(), &row.id).unwrap().unwrap();
        assert_eq!(persisted.sync_status, SyncStatus::Error);
        assert_eq!(persisted.sync_attempts, 3);
        assert_eq!(persisted.last_error.as_deref(), Some("backend offline"));
    }

    #[tokio::test]
    async fn flaky_sender_eventually_succeeds_before_cap() {
        let db = fresh_db();
        let fails_left = Arc::new(std::sync::atomic::AtomicI32::new(1));
        let sender = FlakySender {
            remaining_failures: Arc::clone(&fails_left),
        };

        // 1st attempt fails (write_through), still pending
        let id = write_through(Arc::clone(&db), Some(&sender), sample())
            .await
            .unwrap();
        // resync → 2nd attempt succeeds
        make_all_retryable(&db);
        let report = resync_pending(Arc::clone(&db), Some(&sender), 100)
            .await
            .unwrap();
        assert_eq!(report.synced, 1);

        let guard = db.lock().unwrap();
        let row = store::get(&guard, &id).unwrap().unwrap();
        assert_eq!(row.sync_status, SyncStatus::Synced);
    }

    #[tokio::test]
    async fn write_through_preserves_wing_and_room_in_sender_call() {
        type Captured = Option<(Option<String>, Option<String>)>;
        struct CaptureSender {
            last: Arc<Mutex<Captured>>,
        }
        #[async_trait]
        impl MemPalaceSender for CaptureSender {
            async fn send(&self, row: &MemoryWrite) -> Result<(), String> {
                *self.last.lock().unwrap() = Some((row.wing.clone(), row.room.clone()));
                Ok(())
            }
        }
        let captured = Arc::new(Mutex::new(None));
        let sender = CaptureSender {
            last: Arc::clone(&captured),
        };
        let db = fresh_db();
        let input = NewMemoryWrite {
            subject: "s".into(),
            predicate: "p".into(),
            object: "o".into(),
            wing: Some("learnings".into()),
            room: Some("general".into()),
            source: "test".into(),
        };
        write_through(Arc::clone(&db), Some(&sender), input)
            .await
            .unwrap();
        let got = captured.lock().unwrap().clone().unwrap();
        assert_eq!(got.0.as_deref(), Some("learnings"));
        assert_eq!(got.1.as_deref(), Some("general"));
    }

    #[tokio::test]
    async fn backend_outage_stops_batch_after_one_probe() {
        let db = fresh_db();
        for _ in 0..5 {
            store::append(&db.lock().unwrap(), sample()).unwrap();
        }
        let report = resync_pending(Arc::clone(&db), Some(&FailSender), 100)
            .await
            .unwrap();
        assert_eq!(report.attempted, 1);
        assert_eq!(report.still_pending, 1);
        assert_eq!(report.deferred, 4);

        let guard = db.lock().unwrap();
        let rows = store::list_pending(&guard, 100).unwrap();
        assert_eq!(rows.iter().filter(|row| row.sync_attempts == 1).count(), 1);
        assert_eq!(rows.iter().filter(|row| row.sync_attempts == 0).count(), 4);
    }

    #[test]
    fn mempalace_functional_error_is_not_treated_as_success() {
        let err = validate_mempalace_response(
            "mcp_mempalace_mempalace_kg_add",
            r#"{"success":false,"error":"predicate invalid"}"#,
        )
        .unwrap_err();
        assert!(err.contains("predicate invalid"));
        assert!(validate_mempalace_response(
            "mcp_mempalace_mempalace_kg_add",
            r#"{"success":true,"triple_id":"abc"}"#,
        )
        .is_ok());
    }
}
