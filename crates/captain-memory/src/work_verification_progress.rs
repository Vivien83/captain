//! Crash-safe active-operation registry for adaptive work verification.
//!
//! The append-only session event log is the durable history. This registry
//! contains only non-terminal operations so restart recovery never has to
//! infer an unfinished verification from free-form conversation text.

use crate::event_log;
use captain_types::agent::{AgentId, SessionId};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub const WORK_VERIFICATION_EVENT_TYPE: &str = "work_verification";
pub const WORK_VERIFICATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkVerificationState {
    Verifying,
    Correcting,
    Verified,
    Incomplete,
    Interrupted,
}

impl WorkVerificationState {
    fn is_active(self) -> bool {
        matches!(self, Self::Verifying | Self::Correcting)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableVerificationGap {
    pub code: String,
    pub tool_name: String,
    pub sequence: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkVerificationProgress {
    pub schema_version: u32,
    pub operation_id: String,
    pub runtime_instance_id: String,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub state: WorkVerificationState,
    pub correction_round: u8,
    pub receipt_digests: Vec<String>,
    pub gaps: Vec<DurableVerificationGap>,
    pub detail: String,
    pub started_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkVerificationStoreError {
    #[error("work verification SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("work verification JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("work verification operation {0} changed immutable identity")]
    IdentityMismatch(String),
    #[error("work verification operation {0} has no active owner")]
    MissingActive(String),
    #[error("invalid work verification progress: {0}")]
    Invalid(String),
}

/// Armed owner for one active verification operation. Normal terminal writes
/// disarm it. Task cancellation or unwind drops it and records an interrupted
/// event; a process kill is handled by the active registry on next boot.
pub struct WorkVerificationLease {
    conn: Arc<Mutex<Connection>>,
    progress: WorkVerificationProgress,
    armed: bool,
}

impl WorkVerificationLease {
    pub(crate) fn new(conn: Arc<Mutex<Connection>>, progress: WorkVerificationProgress) -> Self {
        Self {
            conn,
            progress,
            armed: true,
        }
    }

    pub fn operation_id(&self) -> &str {
        &self.progress.operation_id
    }

    pub fn progress(&self) -> &WorkVerificationProgress {
        &self.progress
    }

    pub fn record(
        &mut self,
        progress: WorkVerificationProgress,
    ) -> Result<(), WorkVerificationStoreError> {
        if progress.operation_id != self.progress.operation_id
            || progress.runtime_instance_id != self.progress.runtime_instance_id
            || progress.agent_id != self.progress.agent_id
            || progress.session_id != self.progress.session_id
            || progress.started_at_ms != self.progress.started_at_ms
        {
            return Err(WorkVerificationStoreError::IdentityMismatch(
                self.progress.operation_id.clone(),
            ));
        }
        let remains_active = progress.state.is_active();
        let mut guard = self.conn.lock().map_err(|error| {
            WorkVerificationStoreError::Invalid(format!("poisoned database lock: {error}"))
        })?;
        record(&mut guard, &progress)?;
        self.progress = progress;
        self.armed = remains_active;
        Ok(())
    }
}

impl Drop for WorkVerificationLease {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut interrupted = self.progress.clone();
        interrupted.state = WorkVerificationState::Interrupted;
        interrupted.detail =
            "Verification task was cancelled; current state must be inspected before any retry"
                .to_string();
        interrupted.updated_at_ms = now_unix_ms().max(interrupted.started_at_ms);
        let result = self
            .conn
            .lock()
            .map_err(|error| {
                WorkVerificationStoreError::Invalid(format!("poisoned database lock: {error}"))
            })
            .and_then(|mut guard| record(&mut guard, &interrupted).map(|_| ()));
        if let Err(error) = result {
            tracing::error!(%error, operation_id = %self.progress.operation_id, "Failed to close cancelled work verification lease; restart recovery remains armed");
        }
        self.armed = false;
    }
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

/// Persist one timeline event and update the active-operation registry in the
/// same SQLite transaction.
pub fn record(
    conn: &mut Connection,
    progress: &WorkVerificationProgress,
) -> Result<i64, WorkVerificationStoreError> {
    validate(progress)?;
    let payload = serde_json::to_value(progress)?;
    let payload_json = serde_json::to_string(progress)?;
    let tx = conn.transaction()?;

    if progress.state.is_active() {
        let changed = tx.execute(
            "INSERT INTO work_verification_active_operations (
                 operation_id, runtime_instance_id, agent_id, session_id,
                 payload, started_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(operation_id) DO UPDATE SET
                 payload = excluded.payload,
                 updated_at = excluded.updated_at
             WHERE work_verification_active_operations.runtime_instance_id = excluded.runtime_instance_id
               AND work_verification_active_operations.agent_id = excluded.agent_id
               AND work_verification_active_operations.session_id = excluded.session_id
               AND work_verification_active_operations.started_at = excluded.started_at",
            params![
                progress.operation_id,
                progress.runtime_instance_id,
                progress.agent_id.to_string(),
                progress.session_id.to_string(),
                payload_json,
                progress.started_at_ms,
                progress.updated_at_ms,
            ],
        )?;
        if changed != 1 {
            return Err(WorkVerificationStoreError::IdentityMismatch(
                progress.operation_id.clone(),
            ));
        }
    } else {
        ensure_matching_active_identity(&tx, progress)?;
        tx.execute(
            "DELETE FROM work_verification_active_operations
             WHERE operation_id = ?1",
            params![progress.operation_id],
        )?;
    }

    let event_id = event_log::append(
        &tx,
        &progress.session_id.to_string(),
        WORK_VERIFICATION_EVENT_TYPE,
        &payload,
    )?;
    tx.commit()?;
    Ok(event_id)
}

/// Close operations owned by an older runtime instance. No tool or external
/// effect is replayed; only an append-only interrupted event is written.
pub fn reconcile_after_restart(
    conn: &mut Connection,
    current_runtime_instance_id: &str,
    now_unix_ms: i64,
) -> Result<Vec<WorkVerificationProgress>, WorkVerificationStoreError> {
    let tx = conn.transaction()?;
    let rows = load_rows(
        &tx,
        "SELECT operation_id, runtime_instance_id, agent_id, session_id, payload
         FROM work_verification_active_operations
         WHERE runtime_instance_id <> ?1
         ORDER BY started_at ASC, operation_id ASC",
        current_runtime_instance_id,
    )?;
    let interrupted = interrupt_rows(&tx, rows, now_unix_ms)?;
    tx.commit()?;
    Ok(interrupted)
}

/// Close stale verification work before a new turn starts on the same
/// session. Kernel session serialization means there cannot be another live
/// owner for this session in the current runtime.
pub fn reconcile_session_before_start(
    conn: &mut Connection,
    session_id: &str,
    now_unix_ms: i64,
) -> Result<Vec<WorkVerificationProgress>, WorkVerificationStoreError> {
    let tx = conn.transaction()?;
    let rows = load_rows(
        &tx,
        "SELECT operation_id, runtime_instance_id, agent_id, session_id, payload
         FROM work_verification_active_operations
         WHERE session_id = ?1
         ORDER BY started_at ASC, operation_id ASC",
        session_id,
    )?;
    let interrupted = interrupt_rows(&tx, rows, now_unix_ms)?;
    tx.commit()?;
    Ok(interrupted)
}

fn validate(progress: &WorkVerificationProgress) -> Result<(), WorkVerificationStoreError> {
    if progress.schema_version != WORK_VERIFICATION_SCHEMA_VERSION {
        return Err(WorkVerificationStoreError::Invalid(
            "unsupported schema version".to_string(),
        ));
    }
    if progress.operation_id.is_empty()
        || progress.operation_id.len() > 128
        || progress.runtime_instance_id.is_empty()
        || progress.runtime_instance_id.len() > 128
        || progress.detail.len() > 512
        || progress.receipt_digests.len() > 128
        || progress.gaps.len() > 32
        || progress.updated_at_ms < progress.started_at_ms
    {
        return Err(WorkVerificationStoreError::Invalid(
            "identity, timestamp or evidence bounds".to_string(),
        ));
    }
    if progress
        .receipt_digests
        .iter()
        .any(|digest| digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        || progress.gaps.iter().any(|gap| {
            gap.code.len() > 64
                || gap.tool_name.len() > 128
                || gap
                    .scope_digest
                    .as_ref()
                    .is_some_and(|scope| scope.len() > 64)
        })
    {
        return Err(WorkVerificationStoreError::Invalid(
            "evidence encoding".to_string(),
        ));
    }
    Ok(())
}

fn ensure_matching_active_identity(
    tx: &Transaction<'_>,
    progress: &WorkVerificationProgress,
) -> Result<(), WorkVerificationStoreError> {
    let active_identity = tx
        .query_row(
            "SELECT runtime_instance_id, agent_id, session_id, started_at
             FROM work_verification_active_operations WHERE operation_id = ?1",
            params![progress.operation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((runtime, agent, session, started_at)) = active_identity else {
        return Err(WorkVerificationStoreError::MissingActive(
            progress.operation_id.clone(),
        ));
    };
    if runtime != progress.runtime_instance_id
        || agent != progress.agent_id.to_string()
        || session != progress.session_id.to_string()
        || started_at != progress.started_at_ms
    {
        return Err(WorkVerificationStoreError::IdentityMismatch(
            progress.operation_id.clone(),
        ));
    }
    Ok(())
}

type ActiveRow = (String, String, String, String, String);

fn load_rows(
    tx: &Transaction<'_>,
    sql: &str,
    parameter: &str,
) -> Result<Vec<ActiveRow>, WorkVerificationStoreError> {
    let mut statement = tx.prepare(sql)?;
    let rows = statement
        .query_map(params![parameter], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn interrupt_rows(
    tx: &Transaction<'_>,
    rows: Vec<ActiveRow>,
    now_unix_ms: i64,
) -> Result<Vec<WorkVerificationProgress>, WorkVerificationStoreError> {
    let mut interrupted = Vec::with_capacity(rows.len());
    for (operation_id, runtime_instance_id, agent_id, session_id, payload_json) in rows {
        let mut progress: WorkVerificationProgress = serde_json::from_str(&payload_json)?;
        if progress.operation_id != operation_id
            || progress.runtime_instance_id != runtime_instance_id
            || progress.agent_id.to_string() != agent_id
            || progress.session_id.to_string() != session_id
            || !progress.state.is_active()
        {
            return Err(WorkVerificationStoreError::IdentityMismatch(operation_id));
        }

        progress.state = WorkVerificationState::Interrupted;
        progress.detail =
            "Verification was interrupted; current state must be inspected before any retry"
                .to_string();
        progress.updated_at_ms = now_unix_ms.max(progress.started_at_ms);
        validate(&progress)?;
        event_log::append(
            tx,
            &progress.session_id.to_string(),
            WORK_VERIFICATION_EVENT_TYPE,
            &serde_json::to_value(&progress)?,
        )?;
        tx.execute(
            "DELETE FROM work_verification_active_operations
             WHERE operation_id = ?1 AND runtime_instance_id = ?2",
            params![progress.operation_id, progress.runtime_instance_id],
        )?;
        interrupted.push(progress);
    }
    Ok(interrupted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_log::RangeQuery;
    use crate::migration::run_migrations;

    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    fn progress(runtime: &str, operation: &str, session_id: SessionId) -> WorkVerificationProgress {
        WorkVerificationProgress {
            schema_version: WORK_VERIFICATION_SCHEMA_VERSION,
            operation_id: operation.to_string(),
            runtime_instance_id: runtime.to_string(),
            agent_id: AgentId::new(),
            session_id,
            state: WorkVerificationState::Verifying,
            correction_round: 0,
            receipt_digests: vec!["a".repeat(64)],
            gaps: Vec::new(),
            detail: "Checking post-condition evidence".to_string(),
            started_at_ms: 10,
            updated_at_ms: 10,
        }
    }

    #[test]
    fn active_and_terminal_states_share_the_timeline_transaction() {
        let mut conn = connection();
        let session_id = SessionId::new();
        let mut state = progress("runtime-a", "verify-a", session_id);
        record(&mut conn, &state).unwrap();
        state.state = WorkVerificationState::Verified;
        state.detail = "Evidence accepted".to_string();
        state.updated_at_ms = 20;
        record(&mut conn, &state).unwrap();

        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM work_verification_active_operations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active, 0);
        let events = event_log::range(
            &conn,
            &RangeQuery {
                session_id: session_id.to_string(),
                from_ts: None,
                to_ts: None,
                limit: Some(10),
            },
        )
        .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].payload["state"], "verified");
    }

    #[test]
    fn restart_marks_only_previous_runtime_operations_interrupted() {
        let mut conn = connection();
        let old_session = SessionId::new();
        let current_session = SessionId::new();
        record(&mut conn, &progress("runtime-old", "old", old_session)).unwrap();
        record(
            &mut conn,
            &progress("runtime-current", "current", current_session),
        )
        .unwrap();

        let interrupted = reconcile_after_restart(&mut conn, "runtime-current", 50).unwrap();
        assert_eq!(interrupted.len(), 1);
        assert_eq!(interrupted[0].operation_id, "old");
        assert_eq!(interrupted[0].state, WorkVerificationState::Interrupted);
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM work_verification_active_operations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn new_turn_closes_stale_same_session_operation_without_replay() {
        let mut conn = connection();
        let session_id = SessionId::new();
        record(&mut conn, &progress("runtime-a", "stale", session_id)).unwrap();

        let interrupted =
            reconcile_session_before_start(&mut conn, &session_id.to_string(), 25).unwrap();
        assert_eq!(interrupted.len(), 1);
        assert_eq!(interrupted[0].state, WorkVerificationState::Interrupted);
        assert!(interrupted[0].detail.contains("must be inspected"));
    }

    #[test]
    fn immutable_identity_cannot_be_reused_by_another_runtime() {
        let mut conn = connection();
        let session_id = SessionId::new();
        record(&mut conn, &progress("runtime-a", "verify-a", session_id)).unwrap();
        let error = record(&mut conn, &progress("runtime-b", "verify-a", session_id)).unwrap_err();
        assert!(matches!(
            error,
            WorkVerificationStoreError::IdentityMismatch(_)
        ));
    }

    #[test]
    fn auxiliary_reopen_does_not_interrupt_the_active_runtime() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("memory.db");
        let session_id = SessionId::new();
        let memory = crate::MemorySubstrate::open(&path, 0.01).unwrap();
        let state = progress(memory.runtime_instance_id(), "live-operation", session_id);
        memory.record_work_verification_progress(&state).unwrap();

        let auxiliary = crate::MemorySubstrate::open(&path, 0.01).unwrap();
        let events = auxiliary
            .read_session_events_tail_by_type(
                &RangeQuery {
                    session_id: session_id.to_string(),
                    from_ts: None,
                    to_ts: None,
                    limit: Some(10),
                },
                WORK_VERIFICATION_EVENT_TYPE,
            )
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload["state"], "verifying");
    }

    #[test]
    fn authoritative_reopen_closes_previous_owner_without_replay() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("memory.db");
        let session_id = SessionId::new();
        let memory = crate::MemorySubstrate::open(&path, 0.01).unwrap();
        let state = progress(
            memory.runtime_instance_id(),
            "power-loss-operation",
            session_id,
        );
        memory.record_work_verification_progress(&state).unwrap();
        drop(memory);

        let memory = crate::MemorySubstrate::open(&path, 0.01).unwrap();
        let interrupted = memory
            .reconcile_work_verification_after_restart(50)
            .unwrap();
        assert_eq!(interrupted.len(), 1);
        let events = memory
            .read_session_events_tail_by_type(
                &RangeQuery {
                    session_id: session_id.to_string(),
                    from_ts: None,
                    to_ts: None,
                    limit: Some(10),
                },
                WORK_VERIFICATION_EVENT_TYPE,
            )
            .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].payload["state"], "interrupted");
        assert!(events[1].payload["detail"]
            .as_str()
            .unwrap()
            .contains("must be inspected"));
    }

    #[test]
    fn terminal_state_requires_its_active_registry_row() {
        let mut conn = connection();
        let session_id = SessionId::new();
        let mut state = progress("runtime-a", "orphan", session_id);
        state.state = WorkVerificationState::Verified;
        let error = record(&mut conn, &state).unwrap_err();
        assert!(matches!(
            error,
            WorkVerificationStoreError::MissingActive(operation) if operation == "orphan"
        ));
        let events = event_log::range(
            &conn,
            &RangeQuery {
                session_id: session_id.to_string(),
                from_ts: None,
                to_ts: None,
                limit: Some(10),
            },
        )
        .unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn dropping_an_armed_lease_records_cancellation_as_interrupted() {
        let session_id = SessionId::new();
        let conn = Arc::new(Mutex::new(connection()));
        let state = progress("runtime-a", "cancelled-operation", session_id);
        {
            let mut guard = conn.lock().unwrap();
            record(&mut guard, &state).unwrap();
        }
        drop(WorkVerificationLease::new(Arc::clone(&conn), state));

        let guard = conn.lock().unwrap();
        let events = event_log::range(
            &guard,
            &RangeQuery {
                session_id: session_id.to_string(),
                from_ts: None,
                to_ts: None,
                limit: Some(10),
            },
        )
        .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].payload["state"], "interrupted");
        let active: i64 = guard
            .query_row(
                "SELECT COUNT(*) FROM work_verification_active_operations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active, 0);
    }
}
