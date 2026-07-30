//! Durable operational snapshot for Skill Learning V2.

use captain_types::workflow_learning::{
    WorkflowLearningJobQueueView, WorkflowLearningModelIdentity,
    WorkflowLearningNotificationQueueView, WorkflowLearningWorkerPhase,
    WorkflowLearningWorkloadView,
};
use rusqlite::{params, OptionalExtension, TransactionBehavior};

use crate::workflow_learning_control::{WorkflowLearningControlError, WorkflowLearningStore};
use crate::workflow_learning_validation::{validate_text, validate_token};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowLearningWorkerHeartbeat {
    pub worker_id: String,
    pub phase: WorkflowLearningWorkerPhase,
    pub bound_model: Option<WorkflowLearningModelIdentity>,
    pub started_at_unix_ms: i64,
    pub heartbeat_at_unix_ms: i64,
    pub last_scan_at_unix_ms: Option<i64>,
    pub last_progress_at_unix_ms: Option<i64>,
    pub last_error_scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowLearningOperationalSnapshot {
    pub worker: Option<WorkflowLearningWorkerHeartbeat>,
    pub jobs: WorkflowLearningJobQueueView,
    pub notifications: WorkflowLearningNotificationQueueView,
    pub workflows: WorkflowLearningWorkloadView,
}

impl WorkflowLearningStore {
    pub fn record_worker_heartbeat(
        &self,
        heartbeat: &WorkflowLearningWorkerHeartbeat,
    ) -> Result<(), WorkflowLearningControlError> {
        validate_heartbeat(heartbeat)?;
        let conn = self.lock_conn()?;
        let (provider, model) = heartbeat
            .bound_model
            .as_ref()
            .map(|identity| {
                (
                    Some(identity.provider.as_str()),
                    Some(identity.model.as_str()),
                )
            })
            .unwrap_or((None, None));
        conn.execute(
            "INSERT INTO workflow_learning_runtime (
                 singleton, worker_id, phase, provider, model, started_at,
                 heartbeat_at, last_scan_at, last_progress_at, last_error_scope
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(singleton) DO UPDATE SET
                 worker_id = excluded.worker_id,
                 phase = excluded.phase,
                 provider = excluded.provider,
                 model = excluded.model,
                 started_at = excluded.started_at,
                 heartbeat_at = excluded.heartbeat_at,
                 last_scan_at = excluded.last_scan_at,
                 last_progress_at = excluded.last_progress_at,
                 last_error_scope = excluded.last_error_scope",
            params![
                heartbeat.worker_id,
                worker_phase_str(heartbeat.phase),
                provider,
                model,
                heartbeat.started_at_unix_ms,
                heartbeat.heartbeat_at_unix_ms,
                heartbeat.last_scan_at_unix_ms,
                heartbeat.last_progress_at_unix_ms,
                heartbeat.last_error_scope,
            ],
        )?;
        Ok(())
    }

    /// Read worker heartbeat, queues and workflow counts from one SQLite snapshot.
    pub fn operational_snapshot(
        &self,
    ) -> Result<WorkflowLearningOperationalSnapshot, WorkflowLearningControlError> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let worker = tx
            .query_row(
                "SELECT worker_id, phase, provider, model, started_at, heartbeat_at,
                        last_scan_at, last_progress_at, last_error_scope
                 FROM workflow_learning_runtime WHERE singleton = 1",
                [],
                worker_from_row,
            )
            .optional()?;
        let jobs = job_queue_view(&tx)?;
        let notifications = notification_queue_view(&tx)?;
        let workflows = workflow_view(&tx)?;
        tx.commit()?;
        Ok(WorkflowLearningOperationalSnapshot {
            worker,
            jobs,
            notifications,
            workflows,
        })
    }
}

fn validate_heartbeat(
    heartbeat: &WorkflowLearningWorkerHeartbeat,
) -> Result<(), WorkflowLearningControlError> {
    validate_token("workflow-learning worker_id", &heartbeat.worker_id, 160)?;
    if let Some(identity) = &heartbeat.bound_model {
        validate_text("workflow-learning provider", &identity.provider, 1, 128)?;
        validate_text("workflow-learning model", &identity.model, 1, 192)?;
    }
    if let Some(scope) = &heartbeat.last_error_scope {
        validate_token("workflow-learning error scope", scope, 96)?;
    }
    if heartbeat.started_at_unix_ms < 0
        || heartbeat.heartbeat_at_unix_ms < heartbeat.started_at_unix_ms
        || heartbeat
            .last_scan_at_unix_ms
            .is_some_and(|at| at < 0 || at > heartbeat.heartbeat_at_unix_ms)
        || heartbeat
            .last_progress_at_unix_ms
            .is_some_and(|at| at < 0 || at > heartbeat.heartbeat_at_unix_ms)
    {
        return Err(WorkflowLearningControlError::InvalidInput(
            "workflow-learning heartbeat timestamps are inconsistent".to_string(),
        ));
    }
    Ok(())
}

fn worker_phase_str(phase: WorkflowLearningWorkerPhase) -> &'static str {
    match phase {
        WorkflowLearningWorkerPhase::Starting => "starting",
        WorkflowLearningWorkerPhase::Running => "running",
        WorkflowLearningWorkerPhase::Degraded => "degraded",
    }
}

fn worker_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowLearningWorkerHeartbeat> {
    let phase: String = row.get(1)?;
    let phase = match phase.as_str() {
        "starting" => WorkflowLearningWorkerPhase::Starting,
        "running" => WorkflowLearningWorkerPhase::Running,
        "degraded" => WorkflowLearningWorkerPhase::Degraded,
        other => return Err(invalid_data(1, format!("unknown worker phase {other}"))),
    };
    let provider: Option<String> = row.get(2)?;
    let model: Option<String> = row.get(3)?;
    let bound_model = match (provider, model) {
        (Some(provider), Some(model)) => Some(WorkflowLearningModelIdentity { provider, model }),
        (None, None) => None,
        _ => return Err(invalid_data(2, "partial workflow model identity")),
    };
    Ok(WorkflowLearningWorkerHeartbeat {
        worker_id: row.get(0)?,
        phase,
        bound_model,
        started_at_unix_ms: row.get(4)?,
        heartbeat_at_unix_ms: row.get(5)?,
        last_scan_at_unix_ms: row.get(6)?,
        last_progress_at_unix_ms: row.get(7)?,
        last_error_scope: row.get(8)?,
    })
}

fn job_queue_view(
    conn: &rusqlite::Connection,
) -> Result<WorkflowLearningJobQueueView, WorkflowLearningControlError> {
    let mut view = conn.query_row(
        "SELECT
             COALESCE(SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN status = 'retry_wait' THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN status = 'uncertain' THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN status = 'dead' THEN 1 ELSE 0 END), 0),
             MIN(CASE WHEN status IN ('pending','running','retry_wait','uncertain','dead')
                      THEN created_at END),
             MIN(CASE WHEN status = 'retry_wait' THEN run_after END),
             MAX(updated_at)
         FROM workflow_learning_jobs",
        [],
        |row| {
            Ok(WorkflowLearningJobQueueView {
                pending: nonnegative(row.get(0)?),
                running: nonnegative(row.get(1)?),
                retry_wait: nonnegative(row.get(2)?),
                uncertain: nonnegative(row.get(3)?),
                dead: nonnegative(row.get(4)?),
                oldest_actionable_at_unix_ms: row.get(5)?,
                next_retry_at_unix_ms: row.get(6)?,
                last_activity_at_unix_ms: row.get(7)?,
                last_error_code: None,
            })
        },
    )?;
    view.last_error_code = conn
        .query_row(
            "SELECT error_code FROM workflow_learning_jobs
             WHERE error_code IS NOT NULL
               AND status IN ('retry_wait','uncertain','dead')
             ORDER BY updated_at DESC, id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(view)
}

fn notification_queue_view(
    conn: &rusqlite::Connection,
) -> Result<WorkflowLearningNotificationQueueView, WorkflowLearningControlError> {
    conn.query_row(
        "SELECT
             COALESCE(SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN status = 'delivering' THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN status = 'retry_wait' THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN status = 'dead' THEN 1 ELSE 0 END), 0),
             MIN(CASE WHEN status IN ('pending','delivering','retry_wait','dead')
                      THEN created_at END),
             MIN(CASE WHEN status = 'retry_wait' THEN run_after END),
             MAX(updated_at)
         FROM workflow_learning_outbox",
        [],
        |row| {
            Ok(WorkflowLearningNotificationQueueView {
                pending: nonnegative(row.get(0)?),
                delivering: nonnegative(row.get(1)?),
                retry_wait: nonnegative(row.get(2)?),
                dead: nonnegative(row.get(3)?),
                oldest_actionable_at_unix_ms: row.get(4)?,
                next_retry_at_unix_ms: row.get(5)?,
                last_activity_at_unix_ms: row.get(6)?,
            })
        },
    )
    .map_err(Into::into)
}

fn workflow_view(
    conn: &rusqlite::Connection,
) -> Result<WorkflowLearningWorkloadView, WorkflowLearningControlError> {
    conn.query_row(
        "SELECT
             COUNT(*),
             COALESCE(SUM(CASE WHEN state IN (
                 'observed','eligible','drafting','validating',
                 'approved_pending_install','active_canary'
             ) THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN state = 'proposed' THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN state = 'active' THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN state IN (
                 'rejected','install_failed','rolled_back'
             ) THEN 1 ELSE 0 END), 0),
             MAX(updated_at)
         FROM workflow_learning_proposals",
        [],
        |row| {
            Ok(WorkflowLearningWorkloadView {
                total: nonnegative(row.get(0)?),
                processing: nonnegative(row.get(1)?),
                awaiting_decision: nonnegative(row.get(2)?),
                active: nonnegative(row.get(3)?),
                attention: nonnegative(row.get(4)?),
                last_activity_at_unix_ms: row.get(5)?,
            })
        },
    )
    .map_err(Into::into)
}

fn nonnegative(value: i64) -> u64 {
    value.max(0) as u64
}

fn invalid_data(column: usize, message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message.into(),
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_learning_control::NewWorkflowProposal;
    use crate::workflow_learning_outbox::NewWorkflowOutboxItem;
    use crate::workflow_learning_queue::{NewWorkflowJob, WorkflowJobKind};
    use crate::MemorySubstrate;

    fn store() -> WorkflowLearningStore {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        WorkflowLearningStore::new(memory.usage_conn())
    }

    fn observed(store: &WorkflowLearningStore) {
        store
            .create_observed(&NewWorkflowProposal {
                id: "proposal-status".to_string(),
                idempotency_key: "observe:proposal-status".to_string(),
                workflow_signature: "a".repeat(64),
                source_agent_id: "captain".to_string(),
                origin_channel: Some("telegram".to_string()),
                evidence_json: "{}".to_string(),
                created_at_unix_ms: 100,
            })
            .unwrap();
    }

    #[test]
    fn snapshot_reports_exact_worker_queue_and_outbox_state() {
        let store = store();
        observed(&store);
        store
            .enqueue_job(&NewWorkflowJob {
                id: "job-status".to_string(),
                idempotency_key: "job:status".to_string(),
                proposal_id: "proposal-status".to_string(),
                revision_sha256: None,
                kind: WorkflowJobKind::Analyze,
                payload_json: "{}".to_string(),
                max_attempts: 3,
                run_after_unix_ms: 200,
                created_at_unix_ms: 100,
            })
            .unwrap();
        store
            .enqueue_outbox(&NewWorkflowOutboxItem {
                id: "outbox-status".to_string(),
                idempotency_key: "outbox:status".to_string(),
                proposal_id: "proposal-status".to_string(),
                revision_sha256: None,
                topic: "workflow.proposed".to_string(),
                payload_json: "{}".to_string(),
                max_attempts: 3,
                run_after_unix_ms: 200,
                created_at_unix_ms: 100,
            })
            .unwrap();
        store
            .record_worker_heartbeat(&WorkflowLearningWorkerHeartbeat {
                worker_id: "captain:workflow-learning-v2:test".to_string(),
                phase: WorkflowLearningWorkerPhase::Running,
                bound_model: Some(WorkflowLearningModelIdentity {
                    provider: "codex".to_string(),
                    model: "gpt-5.6-sol".to_string(),
                }),
                started_at_unix_ms: 100,
                heartbeat_at_unix_ms: 300,
                last_scan_at_unix_ms: Some(250),
                last_progress_at_unix_ms: Some(275),
                last_error_scope: None,
            })
            .unwrap();

        let snapshot = store.operational_snapshot().unwrap();
        assert_eq!(snapshot.worker.unwrap().heartbeat_at_unix_ms, 300);
        assert_eq!(snapshot.jobs.pending, 1);
        assert_eq!(snapshot.notifications.pending, 1);
        assert_eq!(snapshot.workflows.total, 1);
        assert_eq!(snapshot.workflows.processing, 1);
    }

    #[test]
    fn heartbeat_upsert_replaces_stale_worker_identity_and_clears_model() {
        let store = store();
        for (worker_id, model, heartbeat_at) in [
            ("worker:old", Some("gpt-5.5"), 20),
            ("worker:new", None, 40),
        ] {
            store
                .record_worker_heartbeat(&WorkflowLearningWorkerHeartbeat {
                    worker_id: worker_id.to_string(),
                    phase: if model.is_some() {
                        WorkflowLearningWorkerPhase::Running
                    } else {
                        WorkflowLearningWorkerPhase::Starting
                    },
                    bound_model: model.map(|model| WorkflowLearningModelIdentity {
                        provider: "codex".to_string(),
                        model: model.to_string(),
                    }),
                    started_at_unix_ms: 10,
                    heartbeat_at_unix_ms: heartbeat_at,
                    last_scan_at_unix_ms: None,
                    last_progress_at_unix_ms: None,
                    last_error_scope: None,
                })
                .unwrap();
        }

        let worker = store.operational_snapshot().unwrap().worker.unwrap();
        assert_eq!(worker.worker_id, "worker:new");
        assert_eq!(worker.phase, WorkflowLearningWorkerPhase::Starting);
        assert!(worker.bound_model.is_none());
    }
}
