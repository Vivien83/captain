//! Durable, idempotent notification outbox for Skill Learning V2.

use rusqlite::{params, types::Type, OptionalExtension, Transaction, TransactionBehavior};

use crate::workflow_learning_control::{
    proposal_by_id, WorkflowLearningControlError, WorkflowLearningStore,
};
pub use crate::workflow_learning_types::{
    NewWorkflowOutboxItem, WorkflowOutboxRecord, WorkflowOutboxRecoverySummary,
    WorkflowOutboxStatus,
};
use crate::workflow_learning_validation::{
    validate_hash, validate_json, validate_text, validate_token,
};

impl WorkflowLearningStore {
    pub fn enqueue_outbox(
        &self,
        input: &NewWorkflowOutboxItem,
    ) -> Result<WorkflowOutboxRecord, WorkflowLearningControlError> {
        validate_outbox(input)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let item = insert_outbox_in_tx(&tx, input)?;
        tx.commit()?;
        Ok(item)
    }

    pub fn get_outbox(
        &self,
        id: &str,
    ) -> Result<Option<WorkflowOutboxRecord>, WorkflowLearningControlError> {
        validate_token("outbox id", id, 96)?;
        let conn = self.lock_conn()?;
        outbox_by_id(&conn, id).map_err(Into::into)
    }

    pub fn claim_due_outbox(
        &self,
        worker: &str,
        now_unix_ms: i64,
        lease_duration_ms: i64,
    ) -> Result<Option<WorkflowOutboxRecord>, WorkflowLearningControlError> {
        self.claim_due_outbox_matching(worker, None, now_unix_ms, lease_duration_ms)
    }

    pub fn claim_due_outbox_for_topic(
        &self,
        worker: &str,
        topic: &str,
        now_unix_ms: i64,
        lease_duration_ms: i64,
    ) -> Result<Option<WorkflowOutboxRecord>, WorkflowLearningControlError> {
        validate_token("outbox topic", topic, 96)?;
        self.claim_due_outbox_matching(worker, Some(&[topic]), now_unix_ms, lease_duration_ms)
    }

    pub fn claim_due_outbox_for_topics(
        &self,
        worker: &str,
        topics: &[&str],
        now_unix_ms: i64,
        lease_duration_ms: i64,
    ) -> Result<Option<WorkflowOutboxRecord>, WorkflowLearningControlError> {
        if topics.is_empty() || topics.len() > 16 {
            return Err(WorkflowLearningControlError::InvalidInput(
                "outbox topic selection must contain 1 to 16 topics".to_string(),
            ));
        }
        for topic in topics {
            validate_token("outbox topic", topic, 96)?;
        }
        self.claim_due_outbox_matching(worker, Some(topics), now_unix_ms, lease_duration_ms)
    }

    fn claim_due_outbox_matching(
        &self,
        worker: &str,
        topics: Option<&[&str]>,
        now_unix_ms: i64,
        lease_duration_ms: i64,
    ) -> Result<Option<WorkflowOutboxRecord>, WorkflowLearningControlError> {
        validate_token("outbox worker", worker, 96)?;
        if !(1_000..=3_600_000).contains(&lease_duration_ms) {
            return Err(WorkflowLearningControlError::InvalidInput(
                "outbox lease must be between 1 second and 1 hour".to_string(),
            ));
        }
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        reconcile_outbox_in_tx(&tx, now_unix_ms, false)?;
        let id: Option<String> = if let Some(topics) = topics {
            let placeholders = vec!["?"; topics.len()].join(", ");
            let sql = format!(
                "SELECT id FROM workflow_learning_outbox
                 WHERE status IN ('pending', 'retry_wait') AND run_after <= ?
                   AND topic IN ({placeholders})
                 ORDER BY run_after, created_at, id LIMIT 1"
            );
            let mut values = Vec::<rusqlite::types::Value>::with_capacity(topics.len() + 1);
            values.push(now_unix_ms.into());
            values.extend(
                topics
                    .iter()
                    .map(|topic| rusqlite::types::Value::Text((*topic).to_string())),
            );
            tx.query_row(&sql, rusqlite::params_from_iter(values), |row| row.get(0))
                .optional()?
        } else {
            tx.query_row(
                "SELECT id FROM workflow_learning_outbox
                 WHERE status IN ('pending', 'retry_wait') AND run_after <= ?1
                 ORDER BY run_after, created_at, id LIMIT 1",
                params![now_unix_ms],
                |row| row.get(0),
            )
            .optional()?
        };
        let Some(id) = id else {
            tx.commit()?;
            return Ok(None);
        };
        let changed = tx.execute(
            "UPDATE workflow_learning_outbox
             SET status = 'delivering', attempt_count = attempt_count + 1,
                 lease_owner = ?1, lease_expires_at = ?2, updated_at = ?3
             WHERE id = ?4 AND status IN ('pending', 'retry_wait') AND run_after <= ?3",
            params![worker, now_unix_ms + lease_duration_ms, now_unix_ms, id],
        )?;
        if changed != 1 {
            return Err(WorkflowLearningControlError::Conflict(
                "outbox item changed while claiming".to_string(),
            ));
        }
        let claimed = outbox_by_id(&tx, &id)?.ok_or_else(|| {
            WorkflowLearningControlError::CorruptData("claimed outbox item vanished".to_string())
        })?;
        tx.commit()?;
        Ok(Some(claimed))
    }

    pub fn complete_outbox(
        &self,
        id: &str,
        worker: &str,
        delivery_result_json: Option<&str>,
        completed_at_unix_ms: i64,
    ) -> Result<WorkflowOutboxRecord, WorkflowLearningControlError> {
        validate_token("outbox id", id, 96)?;
        validate_token("outbox worker", worker, 96)?;
        if let Some(result) = delivery_result_json {
            validate_json("delivery_result_json", result, 32 * 1024)?;
        }
        let conn = self.lock_conn()?;
        let changed = conn.execute(
            "UPDATE workflow_learning_outbox
             SET status = 'delivered', delivery_result_json = ?1, delivered_at = ?2,
                 lease_owner = NULL, lease_expires_at = NULL, last_error = NULL,
                 updated_at = ?2
             WHERE id = ?3 AND status = 'delivering' AND lease_owner = ?4
               AND lease_expires_at > ?2",
            params![delivery_result_json, completed_at_unix_ms, id, worker],
        )?;
        if changed != 1 {
            return Err(WorkflowLearningControlError::Conflict(
                "outbox completion requires the current delivery lease".to_string(),
            ));
        }
        outbox_by_id(&conn, id)?.ok_or_else(|| WorkflowLearningControlError::NotFound(id.into()))
    }

    pub fn fail_outbox(
        &self,
        id: &str,
        worker: &str,
        error: &str,
        retry_at_unix_ms: i64,
        failed_at_unix_ms: i64,
    ) -> Result<WorkflowOutboxRecord, WorkflowLearningControlError> {
        validate_token("outbox id", id, 96)?;
        validate_token("outbox worker", worker, 96)?;
        validate_text("outbox error", error, 1, 2_048)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = outbox_by_id(&tx, id)?
            .ok_or_else(|| WorkflowLearningControlError::NotFound(id.to_string()))?;
        if current.status != WorkflowOutboxStatus::Delivering
            || current.lease_owner.as_deref() != Some(worker)
            || current.lease_expires_at_unix_ms <= Some(failed_at_unix_ms)
        {
            return Err(WorkflowLearningControlError::Conflict(
                "outbox failure requires the current delivery lease".to_string(),
            ));
        }
        let (status, run_after) = if current.attempt_count < current.max_attempts {
            (WorkflowOutboxStatus::RetryWait, retry_at_unix_ms)
        } else {
            (WorkflowOutboxStatus::Dead, failed_at_unix_ms)
        };
        tx.execute(
            "UPDATE workflow_learning_outbox
             SET status = ?1, run_after = ?2, lease_owner = NULL,
                 lease_expires_at = NULL, last_error = ?3, updated_at = ?4
             WHERE id = ?5 AND status = 'delivering' AND lease_owner = ?6",
            params![
                status.as_str(),
                run_after,
                error,
                failed_at_unix_ms,
                id,
                worker
            ],
        )?;
        let failed = outbox_by_id(&tx, id)?
            .ok_or_else(|| WorkflowLearningControlError::NotFound(id.to_string()))?;
        tx.commit()?;
        Ok(failed)
    }

    /// Permanently reject an item that cannot become deliverable by retrying,
    /// such as an unsupported topic or a payload that no longer matches its
    /// immutable proposal revision.
    pub fn dead_letter_outbox(
        &self,
        id: &str,
        worker: &str,
        error: &str,
        failed_at_unix_ms: i64,
    ) -> Result<WorkflowOutboxRecord, WorkflowLearningControlError> {
        validate_token("outbox id", id, 96)?;
        validate_token("outbox worker", worker, 96)?;
        validate_text("outbox error", error, 1, 2_048)?;
        let conn = self.lock_conn()?;
        let changed = conn.execute(
            "UPDATE workflow_learning_outbox
             SET status = 'dead', run_after = ?1, lease_owner = NULL,
                 lease_expires_at = NULL, last_error = ?2, updated_at = ?1
             WHERE id = ?3 AND status = 'delivering' AND lease_owner = ?4
               AND lease_expires_at > ?1",
            params![failed_at_unix_ms, error, id, worker],
        )?;
        if changed != 1 {
            return Err(WorkflowLearningControlError::Conflict(
                "outbox dead-letter requires the current delivery lease".to_string(),
            ));
        }
        outbox_by_id(&conn, id)?.ok_or_else(|| WorkflowLearningControlError::NotFound(id.into()))
    }

    pub fn reconcile_expired_outbox(
        &self,
        now_unix_ms: i64,
    ) -> Result<WorkflowOutboxRecoverySummary, WorkflowLearningControlError> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let summary = reconcile_outbox_in_tx(&tx, now_unix_ms, false)?;
        tx.commit()?;
        Ok(summary)
    }

    /// Reclaim every delivery lease owned by the previous process. The same
    /// durable item and idempotency key are retained for transports that can
    /// consume them; non-idempotent transports remain at-least-once.
    pub fn reconcile_outbox_after_restart(
        &self,
        now_unix_ms: i64,
    ) -> Result<WorkflowOutboxRecoverySummary, WorkflowLearningControlError> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let summary = reconcile_outbox_in_tx(&tx, now_unix_ms, true)?;
        tx.commit()?;
        Ok(summary)
    }
}

pub(crate) fn insert_outbox_in_tx(
    tx: &Transaction<'_>,
    input: &NewWorkflowOutboxItem,
) -> Result<WorkflowOutboxRecord, WorkflowLearningControlError> {
    validate_outbox(input)?;
    if let Some(existing) = outbox_by_idempotency(tx, &input.idempotency_key)? {
        if existing.id == input.id
            && existing.proposal_id == input.proposal_id
            && existing.revision_sha256 == input.revision_sha256
            && existing.topic == input.topic
            && existing.payload_json == input.payload_json
        {
            return Ok(existing);
        }
        return Err(WorkflowLearningControlError::Conflict(
            "outbox idempotency key was reused with different input".to_string(),
        ));
    }
    let proposal = proposal_by_id(tx, &input.proposal_id)?
        .ok_or_else(|| WorkflowLearningControlError::NotFound(input.proposal_id.clone()))?;
    if input.revision_sha256.is_some() && input.revision_sha256 != proposal.revision_sha256 {
        return Err(WorkflowLearningControlError::Conflict(
            "outbox revision does not match the proposal revision".to_string(),
        ));
    }
    tx.execute(
        "INSERT INTO workflow_learning_outbox (
             id, idempotency_key, proposal_id, revision_sha256, topic,
             payload_json, max_attempts, run_after, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
        params![
            input.id,
            input.idempotency_key,
            input.proposal_id,
            input.revision_sha256,
            input.topic,
            input.payload_json,
            input.max_attempts,
            input.run_after_unix_ms,
            input.created_at_unix_ms,
        ],
    )?;
    outbox_by_id(tx, &input.id)?
        .ok_or_else(|| WorkflowLearningControlError::NotFound(input.id.clone()))
}

fn reconcile_outbox_in_tx(
    tx: &Transaction<'_>,
    now_unix_ms: i64,
    include_unexpired: bool,
) -> Result<WorkflowOutboxRecoverySummary, WorkflowLearningControlError> {
    let interrupted = i64::from(include_unexpired);
    let retry_message = if include_unexpired {
        "process restarted during delivery; retry uses the same idempotency key"
    } else {
        "delivery lease expired; retry uses the same idempotency key"
    };
    let retried = tx.execute(
        "UPDATE workflow_learning_outbox
         SET status = 'retry_wait', run_after = ?1, lease_owner = NULL,
             lease_expires_at = NULL, last_error = ?2,
             updated_at = ?1
         WHERE status = 'delivering' AND (?3 = 1 OR lease_expires_at <= ?1)
           AND attempt_count < max_attempts",
        params![now_unix_ms, retry_message, interrupted],
    )?;
    let dead_message = if include_unexpired {
        "process restarted during the final delivery attempt"
    } else {
        "delivery lease expired after final attempt"
    };
    let dead = tx.execute(
        "UPDATE workflow_learning_outbox
         SET status = 'dead', run_after = ?1, lease_owner = NULL,
             lease_expires_at = NULL, last_error = ?2,
             updated_at = ?1
         WHERE status = 'delivering' AND (?3 = 1 OR lease_expires_at <= ?1)
           AND attempt_count >= max_attempts",
        params![now_unix_ms, dead_message, interrupted],
    )?;
    Ok(WorkflowOutboxRecoverySummary { retried, dead })
}

const OUTBOX_SELECT: &str = "SELECT id, idempotency_key, proposal_id, revision_sha256, topic,
            payload_json, status, attempt_count, max_attempts, run_after,
            lease_owner, lease_expires_at, delivery_result_json, last_error,
            delivered_at, created_at, updated_at
     FROM workflow_learning_outbox";

fn outbox_by_id(
    conn: &rusqlite::Connection,
    id: &str,
) -> rusqlite::Result<Option<WorkflowOutboxRecord>> {
    conn.query_row(
        &format!("{OUTBOX_SELECT} WHERE id = ?1"),
        params![id],
        outbox_from_row,
    )
    .optional()
}

fn outbox_by_idempotency(
    conn: &rusqlite::Connection,
    idempotency_key: &str,
) -> rusqlite::Result<Option<WorkflowOutboxRecord>> {
    conn.query_row(
        &format!("{OUTBOX_SELECT} WHERE idempotency_key = ?1"),
        params![idempotency_key],
        outbox_from_row,
    )
    .optional()
}

fn outbox_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowOutboxRecord> {
    let status_value: String = row.get(6)?;
    let status = WorkflowOutboxStatus::parse(&status_value).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown outbox status {status_value}"),
            )),
        )
    })?;
    Ok(WorkflowOutboxRecord {
        id: row.get(0)?,
        idempotency_key: row.get(1)?,
        proposal_id: row.get(2)?,
        revision_sha256: row.get(3)?,
        topic: row.get(4)?,
        payload_json: row.get(5)?,
        status,
        attempt_count: row.get::<_, i64>(7)?.max(0) as u32,
        max_attempts: row.get::<_, i64>(8)?.max(0) as u32,
        run_after_unix_ms: row.get(9)?,
        lease_owner: row.get(10)?,
        lease_expires_at_unix_ms: row.get(11)?,
        delivery_result_json: row.get(12)?,
        last_error: row.get(13)?,
        delivered_at_unix_ms: row.get(14)?,
        created_at_unix_ms: row.get(15)?,
        updated_at_unix_ms: row.get(16)?,
    })
}

fn validate_outbox(input: &NewWorkflowOutboxItem) -> Result<(), WorkflowLearningControlError> {
    validate_token("outbox id", &input.id, 96)?;
    validate_token("outbox idempotency_key", &input.idempotency_key, 192)?;
    validate_token("proposal_id", &input.proposal_id, 96)?;
    if let Some(revision) = &input.revision_sha256 {
        validate_hash("outbox revision_sha256", revision)?;
    }
    validate_token("outbox topic", &input.topic, 96)?;
    validate_json("outbox payload_json", &input.payload_json, 64 * 1024)?;
    if !(1..=20).contains(&input.max_attempts) {
        return Err(WorkflowLearningControlError::InvalidInput(
            "outbox max_attempts must be between 1 and 20".to_string(),
        ));
    }
    Ok(())
}
