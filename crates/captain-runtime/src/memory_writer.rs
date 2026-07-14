//! Write-through orchestrator (v3.12a).
//!
//! This module sits between callers that want to persist a semantic
//! triple to MemPalace (the `memory_store` tool, `mirror_to_mempalace`,
//! the future LearningCommitter) and the MCP backend. It guarantees:
//!
//! - Local persistence **always** succeeds (except IO). The caller gets
//!   `Ok(id)` regardless of MemPalace availability.
//! - MemPalace is attempted synchronously; success flips the row to
//!   `synced`, failure to `pending` (or `error` after 3 attempts).
//! - A background worker replays `pending` rows on a timer or when an
//!   MCP reconnect event fires.
//!
//! The sender is abstracted behind `MemPalaceSender` so the orchestrator
//! is trivially mockable in unit tests without spinning up an MCP
//! subprocess.

use async_trait::async_trait;
use captain_memory::memory_writer::{self, MemoryWrite, NewMemoryWrite, SyncStatus};
use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tracing::{debug, info, warn};

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

        // MemPalace `kg_add` accepts only the bare (subject, predicate,
        // object) triple. The wing/room taxonomy lives in the local
        // `memory_writes` audit row — server-side routing is a separate
        // concern (and passing wing/room here returned -32000 Internal
        // tool error).
        let input = serde_json::json!({
            "subject": row.subject,
            "predicate": row.predicate,
            "object": row.object,
        });
        let tool_name = "mcp_mempalace_mempalace_kg_add";
        conn.call_tool(tool_name, &input)
            .await
            .map(|_| ())
            .map_err(|e| format!("{tool_name} failed: {e}"))
    }
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

    // Step 2 — attempt MemPalace (best-effort)
    let sync_result = match sender {
        Some(s) => s.send(&row).await,
        None => Err("no mempalace sender".to_string()),
    };

    // Step 3 — update status
    {
        let guard = conn.lock().map_err(|e| format!("sqlite poisoned: {e}"))?;
        match &sync_result {
            Ok(()) => {
                memory_writer::mark_synced(&guard, &row.id)
                    .map_err(|e| format!("sqlite mark_synced: {e}"))?;
            }
            Err(e) => {
                memory_writer::mark_error(&guard, &row.id, e)
                    .map_err(|e| format!("sqlite mark_error: {e}"))?;
            }
        }
    }

    Ok(row.id)
}

#[derive(Debug, Clone, Default)]
pub struct ResyncReport {
    pub attempted: usize,
    pub synced: usize,
    pub still_pending: usize,
    pub errored_out: usize,
}

/// Retry every pending row (up to `batch_limit`). Called by the
/// background worker on tick and on MCP reconnect.
pub async fn resync_pending(
    conn: Arc<Mutex<Connection>>,
    sender: Option<&dyn MemPalaceSender>,
    batch_limit: usize,
) -> Result<ResyncReport, String> {
    let pending = {
        let guard = conn.lock().map_err(|e| format!("sqlite poisoned: {e}"))?;
        memory_writer::list_pending(&guard, batch_limit)
            .map_err(|e| format!("sqlite list_pending: {e}"))?
    };

    let mut report = ResyncReport {
        attempted: pending.len(),
        ..Default::default()
    };

    for row in pending {
        let res = match sender {
            Some(s) => s.send(&row).await,
            None => Err("no mempalace sender".to_string()),
        };
        let guard = conn.lock().map_err(|e| format!("sqlite poisoned: {e}"))?;
        match res {
            Ok(()) => {
                if memory_writer::mark_synced(&guard, &row.id).is_ok() {
                    report.synced += 1;
                }
            }
            Err(e) => match memory_writer::mark_error(&guard, &row.id, &e) {
                Ok(SyncStatus::Error) => report.errored_out += 1,
                Ok(_) => report.still_pending += 1,
                Err(err) => debug!(row_id = %row.id, %err, "mark_error failed"),
            },
        }
    }

    if report.attempted > 0 {
        info!(
            attempted = report.attempted,
            synced = report.synced,
            still_pending = report.still_pending,
            errored = report.errored_out,
            "memory_writer resync tick"
        );
    }
    Ok(report)
}

/// Spawn the background resync worker. It ticks every 30 seconds and
/// also runs an error-row GC pass once an hour.
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
        let mut tick_count: u64 = 0;

        loop {
            interval.tick().await;
            tick_count = tick_count.wrapping_add(1);

            let sender = McpMemPalaceSender {
                mcp_conns: &mcp_conns,
            };
            if let Err(e) = resync_pending(Arc::clone(&conn), Some(&sender), 100).await {
                warn!(error = %e, "resync_pending failed");
            }

            // GC error rows older than 30d, once per ~hour (120 ticks × 30s)
            if tick_count.is_multiple_of(120) {
                let thirty_days_ms: i64 = 30 * 24 * 60 * 60 * 1000;
                if let Ok(guard) = conn.lock() {
                    match memory_writer::cleanup_errors(&guard, thirty_days_ms) {
                        Ok(n) if n > 0 => info!(removed = n, "memory_writes cleanup"),
                        Ok(_) => {}
                        Err(e) => warn!(error = %e, "memory_writes cleanup failed"),
                    }
                }
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
    async fn write_through_no_sender_returns_ok_and_marks_pending() {
        let db = fresh_db();
        let id = write_through(Arc::clone(&db), None, sample())
            .await
            .unwrap();
        let guard = db.lock().unwrap();
        let row = store::get(&guard, &id).unwrap().unwrap();
        assert_eq!(row.sync_status, SyncStatus::Pending);
        assert_eq!(row.sync_attempts, 1); // one failed send attempt counted
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
        resync_pending(Arc::clone(&db), Some(&FailSender), 100)
            .await
            .unwrap();
        // 3rd resync pass: attempts=3, transitions to error
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

        // 4th resync: error rows excluded, nothing to attempt
        let report4 = resync_pending(Arc::clone(&db), Some(&FailSender), 100)
            .await
            .unwrap();
        assert_eq!(report4.attempted, 0);
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
}
