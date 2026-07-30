//! Durable active-operation registry for session compaction.
//!
//! Timeline events remain append-only. This registry contains only operations
//! that are currently running so a new runtime instance can close them as
//! interrupted after an abrupt process or host stop.

use crate::event_log;
use captain_types::compaction::{CompactionPhase, CompactionProgress, CompactionState};
use rusqlite::{params, Connection, OptionalExtension};

pub const COMPACTION_PROGRESS_EVENT_TYPE: &str = "compaction_progress";

#[derive(Debug, thiserror::Error)]
pub enum CompactionProgressStoreError {
    #[error("compaction progress SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("compaction progress JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("compaction operation {0} changed immutable identity")]
    IdentityMismatch(String),
}

/// Persist one timeline event and update the active-operation registry in the
/// same SQLite transaction.
pub fn record(
    conn: &mut Connection,
    progress: &CompactionProgress,
) -> Result<i64, CompactionProgressStoreError> {
    let payload = serde_json::to_value(progress)?;
    let payload_json = serde_json::to_string(progress)?;
    let tx = conn.transaction()?;

    if matches!(progress.state, CompactionState::Running) {
        let changed = tx.execute(
            "INSERT INTO compaction_active_operations (
                 operation_id, runtime_instance_id, agent_id, session_id,
                 payload, started_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(operation_id) DO UPDATE SET
                 payload = excluded.payload,
                 updated_at = excluded.updated_at
             WHERE compaction_active_operations.runtime_instance_id = excluded.runtime_instance_id
               AND compaction_active_operations.agent_id = excluded.agent_id
               AND compaction_active_operations.session_id = excluded.session_id
               AND compaction_active_operations.started_at = excluded.started_at",
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
            return Err(CompactionProgressStoreError::IdentityMismatch(
                progress.operation_id.clone(),
            ));
        }
    } else {
        let active_identity = tx
            .query_row(
                "SELECT runtime_instance_id, agent_id, session_id, started_at
                 FROM compaction_active_operations WHERE operation_id = ?1",
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
        if active_identity.is_some_and(|(runtime, agent, session, started_at)| {
            runtime != progress.runtime_instance_id
                || agent != progress.agent_id.to_string()
                || session != progress.session_id.to_string()
                || started_at != progress.started_at_ms
        }) {
            return Err(CompactionProgressStoreError::IdentityMismatch(
                progress.operation_id.clone(),
            ));
        }
        tx.execute(
            "DELETE FROM compaction_active_operations WHERE operation_id = ?1",
            params![progress.operation_id],
        )?;
    }

    let event_id = event_log::append(
        &tx,
        &progress.session_id.to_string(),
        COMPACTION_PROGRESS_EVENT_TYPE,
        &payload,
    )?;
    tx.commit()?;
    Ok(event_id)
}

/// Close every operation owned by a previous runtime instance. The returned
/// terminal states have already been persisted and are ready for live fanout.
pub fn reconcile_after_restart(
    conn: &mut Connection,
    current_runtime_instance_id: &str,
    now_unix_ms: i64,
) -> Result<Vec<CompactionProgress>, CompactionProgressStoreError> {
    let tx = conn.transaction()?;
    let rows = {
        let mut statement = tx.prepare(
            "SELECT operation_id, runtime_instance_id, agent_id, session_id, payload
             FROM compaction_active_operations
             WHERE runtime_instance_id <> ?1
             ORDER BY started_at ASC, operation_id ASC",
        )?;
        let rows = statement
            .query_map(params![current_runtime_instance_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    let mut interrupted = Vec::with_capacity(rows.len());
    for (operation_id, runtime_instance_id, agent_id, session_id, payload_json) in rows {
        let mut progress: CompactionProgress = serde_json::from_str(&payload_json)?;
        if progress.operation_id != operation_id
            || progress.runtime_instance_id != runtime_instance_id
            || progress.agent_id.to_string() != agent_id
            || progress.session_id.to_string() != session_id
            || !matches!(progress.state, CompactionState::Running)
        {
            return Err(CompactionProgressStoreError::IdentityMismatch(operation_id));
        }

        progress.phase = CompactionPhase::Interrupted;
        progress.state = CompactionState::Interrupted;
        progress.detail =
            "Compaction was interrupted by a runtime restart; the recoverable session was retained"
                .to_string();
        progress.completed_units = None;
        progress.total_units = None;
        progress.unit = None;
        progress.updated_at_ms = now_unix_ms.max(progress.started_at_ms);

        let payload = serde_json::to_value(&progress)?;
        event_log::append(
            &tx,
            &progress.session_id.to_string(),
            COMPACTION_PROGRESS_EVENT_TYPE,
            &payload,
        )?;
        tx.execute(
            "DELETE FROM compaction_active_operations
             WHERE operation_id = ?1 AND runtime_instance_id = ?2",
            params![progress.operation_id, progress.runtime_instance_id],
        )?;
        interrupted.push(progress);
    }

    tx.commit()?;
    Ok(interrupted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;
    use captain_types::agent::{AgentId, SessionId};
    use captain_types::compaction::{CompactionProgressUnit, COMPACTION_PROGRESS_SCHEMA_VERSION};

    fn progress(runtime: &str, operation: &str) -> CompactionProgress {
        CompactionProgress {
            schema_version: COMPACTION_PROGRESS_SCHEMA_VERSION,
            operation_id: operation.to_string(),
            runtime_instance_id: runtime.to_string(),
            agent_id: AgentId::new(),
            session_id: SessionId::new(),
            phase: CompactionPhase::Chunking,
            state: CompactionState::Running,
            detail: "Processed chunk 1 of 4".to_string(),
            message_count: 24,
            estimated_tokens: 12_000,
            context_window_tokens: 200_000,
            completed_units: Some(1),
            total_units: Some(4),
            unit: Some(CompactionProgressUnit::Chunks),
            started_at_ms: 10,
            updated_at_ms: 20,
        }
    }

    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn running_and_terminal_transitions_share_the_timeline_transaction() {
        let mut conn = connection();
        let mut state = progress("runtime-a", "operation-a");
        record(&mut conn, &state).unwrap();

        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM compaction_active_operations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active, 1);

        state.phase = CompactionPhase::Completed;
        state.state = CompactionState::Succeeded;
        state.completed_units = None;
        state.total_units = None;
        state.unit = None;
        state.updated_at_ms = 30;
        record(&mut conn, &state).unwrap();

        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM compaction_active_operations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions_events WHERE event_type = 'compaction_progress'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active, 0);
        assert_eq!(events, 2);
    }

    #[test]
    fn restart_interrupts_only_previous_runtime_operations_and_is_idempotent() {
        let mut conn = connection();
        let old = progress("runtime-old", "operation-old");
        let current = progress("runtime-current", "operation-current");
        record(&mut conn, &old).unwrap();
        record(&mut conn, &current).unwrap();

        let interrupted = reconcile_after_restart(&mut conn, "runtime-current", 50).unwrap();
        assert_eq!(interrupted.len(), 1);
        assert_eq!(interrupted[0].operation_id, "operation-old");
        assert_eq!(interrupted[0].state, CompactionState::Interrupted);
        assert_eq!(interrupted[0].phase, CompactionPhase::Interrupted);

        let remaining: String = conn
            .query_row(
                "SELECT operation_id FROM compaction_active_operations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, "operation-current");
        assert!(reconcile_after_restart(&mut conn, "runtime-current", 60)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn an_operation_id_cannot_change_session_identity() {
        let mut conn = connection();
        let original = progress("runtime-a", "operation-a");
        record(&mut conn, &original).unwrap();
        let mut conflicting = original.clone();
        conflicting.session_id = SessionId::new();

        assert!(matches!(
            record(&mut conn, &conflicting),
            Err(CompactionProgressStoreError::IdentityMismatch(operation))
                if operation == "operation-a"
        ));
        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions_events WHERE event_type = 'compaction_progress'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(events, 1);

        conflicting.state = CompactionState::Succeeded;
        conflicting.phase = CompactionPhase::Completed;
        assert!(matches!(
            record(&mut conn, &conflicting),
            Err(CompactionProgressStoreError::IdentityMismatch(operation))
                if operation == "operation-a"
        ));
        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM compaction_active_operations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active, 1);
    }
}
