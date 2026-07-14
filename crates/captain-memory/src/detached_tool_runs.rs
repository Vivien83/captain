//! Persistence for detached tool runs (`tool_run_start`) — survives a
//! Captain restart, unlike the in-memory-only registry in
//! `crates/captain-runtime/src/tool_runs.rs`.
//!
//! Only detached runs are persisted here. Foreground runs are tied to a
//! live request/response cycle and can't be resumed after a restart
//! regardless, so persisting them would be pointless.

use captain_types::error::{CaptainError, CaptainResult};
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

/// A persisted detached tool run row.
#[derive(Debug, Clone)]
pub struct DetachedToolRunRecord {
    pub run_id: String,
    pub tool_name: String,
    pub status: String,
    pub caller_agent_id: Option<String>,
    pub origin_tool_use_id: Option<String>,
    pub started_at_unix_ms: i64,
    pub finished_at_unix_ms: Option<i64>,
    pub is_error: Option<bool>,
    pub result: Option<String>,
    pub result_truncated: bool,
}

fn row_to_record(row: &rusqlite::Row) -> rusqlite::Result<DetachedToolRunRecord> {
    Ok(DetachedToolRunRecord {
        run_id: row.get(0)?,
        tool_name: row.get(1)?,
        status: row.get(2)?,
        caller_agent_id: row.get(3)?,
        origin_tool_use_id: row.get(4)?,
        started_at_unix_ms: row.get(5)?,
        finished_at_unix_ms: row.get(6)?,
        is_error: row.get(7)?,
        result: row.get(8)?,
        result_truncated: row.get(9)?,
    })
}

const SELECT_COLUMNS: &str = "run_id, tool_name, status, caller_agent_id, origin_tool_use_id, \
     started_at, finished_at, is_error, result, result_truncated";

/// Detached tool run store backed by SQLite.
#[derive(Clone)]
pub struct DetachedToolRunStore {
    conn: Arc<Mutex<Connection>>,
}

impl DetachedToolRunStore {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Record a newly started detached run as `running`. Best-effort by
    /// design — callers should log-and-continue on error rather than fail
    /// the tool call itself.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_running(
        &self,
        run_id: &str,
        tool_name: &str,
        caller_agent_id: Option<&str>,
        origin_tool_use_id: Option<&str>,
        started_at_unix_ms: i64,
    ) -> CaptainResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Internal(e.to_string()))?;
        conn.execute(
            "INSERT OR REPLACE INTO detached_tool_runs
             (run_id, tool_name, status, caller_agent_id, origin_tool_use_id, started_at, finished_at, is_error, result, result_truncated)
             VALUES (?1, ?2, 'running', ?3, ?4, ?5, NULL, NULL, NULL, 0)",
            rusqlite::params![
                run_id,
                tool_name,
                caller_agent_id,
                origin_tool_use_id,
                started_at_unix_ms
            ],
        )
        .map_err(|e| CaptainError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Update a run to a terminal status with its final result.
    pub fn mark_finished(
        &self,
        run_id: &str,
        status: &str,
        is_error: Option<bool>,
        result: Option<&str>,
        result_truncated: bool,
        finished_at_unix_ms: i64,
    ) -> CaptainResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Internal(e.to_string()))?;
        conn.execute(
            "UPDATE detached_tool_runs
             SET status = ?2, is_error = ?3, result = ?4, result_truncated = ?5, finished_at = ?6
             WHERE run_id = ?1",
            rusqlite::params![
                run_id,
                status,
                is_error,
                result,
                result_truncated,
                finished_at_unix_ms
            ],
        )
        .map_err(|e| CaptainError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Most recent runs, newest first, up to `limit`.
    pub fn list_recent(&self, limit: usize) -> CaptainResult<Vec<DetachedToolRunRecord>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Internal(e.to_string()))?;
        let sql = format!(
            "SELECT {SELECT_COLUMNS} FROM detached_tool_runs \
             ORDER BY started_at DESC, run_id DESC LIMIT ?1"
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| CaptainError::Memory(e.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![limit as i64], row_to_record)
            .map_err(|e| CaptainError::Memory(e.to_string()))?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| CaptainError::Memory(e.to_string()))?);
        }
        Ok(results)
    }

    /// Keep only the newest terminal rows while preserving every run that is
    /// still active. This bounds result retention without hiding work that an
    /// operator may still need to inspect or cancel.
    pub fn prune_terminal_history(&self, keep: usize) -> CaptainResult<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Internal(e.to_string()))?;
        conn.execute(
            "DELETE FROM detached_tool_runs
             WHERE status <> 'running'
               AND run_id NOT IN (
                 SELECT run_id FROM detached_tool_runs
                 WHERE status <> 'running'
                 ORDER BY started_at DESC, run_id DESC
                 LIMIT ?1
               )",
            rusqlite::params![keep as i64],
        )
        .map_err(|e| CaptainError::Memory(e.to_string()))
    }

    /// Any row still `running` in the DB is a crash signature: the process
    /// died mid-run, so it was never marked finished. Reclassifies them as
    /// `interrupted` with an explanatory result and returns the updated
    /// records so the caller can reload them into the in-memory registry.
    pub fn reconcile_running_as_interrupted(&self) -> CaptainResult<Vec<DetachedToolRunRecord>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Internal(e.to_string()))?;
        let now = chrono::Utc::now().timestamp_millis();
        const INTERRUPTED_MESSAGE: &str =
            "Tool run was interrupted by a Captain restart; no result was recorded.";

        // Select the target run_ids first, then update by id — avoids
        // relying on `finished_at = now` to find "what we just touched",
        // which can spuriously match rows updated in the same millisecond
        // on a fast repeated call (millisecond-resolution timestamps).
        let run_ids: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT run_id FROM detached_tool_runs WHERE status = 'running'")
                .map_err(|e| CaptainError::Memory(e.to_string()))?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| CaptainError::Memory(e.to_string()))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row.map_err(|e| CaptainError::Memory(e.to_string()))?);
            }
            ids
        };
        if run_ids.is_empty() {
            return Ok(Vec::new());
        }

        conn.execute(
            "UPDATE detached_tool_runs
             SET status = 'interrupted', is_error = 1, result = ?1, finished_at = ?2
             WHERE status = 'running'",
            rusqlite::params![INTERRUPTED_MESSAGE, now],
        )
        .map_err(|e| CaptainError::Memory(e.to_string()))?;

        let placeholders = vec!["?"; run_ids.len()].join(",");
        let sql = format!(
            "SELECT {SELECT_COLUMNS} FROM detached_tool_runs WHERE run_id IN ({placeholders})"
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| CaptainError::Memory(e.to_string()))?;
        let params: Vec<&dyn rusqlite::types::ToSql> = run_ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt
            .query_map(params.as_slice(), row_to_record)
            .map_err(|e| CaptainError::Memory(e.to_string()))?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| CaptainError::Memory(e.to_string()))?);
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn setup() -> DetachedToolRunStore {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        DetachedToolRunStore::new(Arc::new(Mutex::new(conn)))
    }

    #[test]
    fn records_and_finishes_a_run() {
        let store = setup();
        store
            .upsert_running("run-1", "shell_exec", Some("agent-1"), Some("tc-1"), 1000)
            .unwrap();

        let recent = store.list_recent(10).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].status, "running");

        store
            .mark_finished("run-1", "completed", Some(false), Some("ok"), false, 2000)
            .unwrap();

        let recent = store.list_recent(10).unwrap();
        assert_eq!(recent[0].status, "completed");
        assert_eq!(recent[0].result.as_deref(), Some("ok"));
        assert_eq!(recent[0].finished_at_unix_ms, Some(2000));
    }

    #[test]
    fn reconciles_still_running_rows_as_interrupted() {
        let store = setup();
        store
            .upsert_running("run-crash", "ssh_exec", Some("agent-1"), None, 1000)
            .unwrap();
        store
            .upsert_running("run-done", "cargo", Some("agent-1"), None, 1000)
            .unwrap();
        store
            .mark_finished(
                "run-done",
                "completed",
                Some(false),
                Some("ok"),
                false,
                1500,
            )
            .unwrap();

        let interrupted = store.reconcile_running_as_interrupted().unwrap();
        assert_eq!(interrupted.len(), 1);
        assert_eq!(interrupted[0].run_id, "run-crash");
        assert_eq!(interrupted[0].status, "interrupted");
        assert_eq!(interrupted[0].is_error, Some(true));

        // Running twice must not re-flag the already-completed run or
        // double-count the already-interrupted one.
        let interrupted_again = store.reconcile_running_as_interrupted().unwrap();
        assert!(interrupted_again.is_empty());
    }

    #[test]
    fn list_recent_orders_newest_first() {
        let store = setup();
        store
            .upsert_running("run-old", "cargo", None, None, 1000)
            .unwrap();
        store
            .upsert_running("run-new", "npm", None, None, 2000)
            .unwrap();

        let recent = store.list_recent(10).unwrap();
        assert_eq!(recent[0].run_id, "run-new");
        assert_eq!(recent[1].run_id, "run-old");
    }

    #[test]
    fn prune_terminal_history_keeps_newest_rows_and_all_running_rows() {
        let store = setup();
        for (run_id, started_at) in [("run-old", 1_000), ("run-mid", 2_000), ("run-new", 3_000)] {
            store
                .upsert_running(run_id, "cargo", None, None, started_at)
                .unwrap();
            store
                .mark_finished(
                    run_id,
                    "completed",
                    Some(false),
                    Some("ok"),
                    false,
                    started_at + 100,
                )
                .unwrap();
        }
        store
            .upsert_running("run-active", "ssh_exec", None, None, 4_000)
            .unwrap();

        assert_eq!(store.prune_terminal_history(2).unwrap(), 1);
        let recent = store.list_recent(10).unwrap();
        let ids: Vec<_> = recent.iter().map(|record| record.run_id.as_str()).collect();
        assert_eq!(ids, vec!["run-active", "run-new", "run-mid"]);
    }
}
