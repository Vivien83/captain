//! Durable leased jobs for Skill Learning V2.
//!
//! A worker marks an external effect as started before invoking a model or
//! touching active state. If its lease then expires, the job becomes
//! `uncertain` instead of being called twice.

use rusqlite::{params, types::Type, OptionalExtension, Transaction, TransactionBehavior};

use crate::workflow_learning_control::{
    create_observed_in_tx, proposal_by_id, publish_validated_draft_in_tx, transition_in_tx,
    validate_publish, validate_transition, NewWorkflowProposal, PublishValidatedDraft,
    WorkflowLearningControlError, WorkflowLearningStore, WorkflowProposalRecord,
    WorkflowProposalState, WorkflowProposalTransition,
};
use crate::workflow_learning_outbox::{insert_outbox_in_tx, NewWorkflowOutboxItem};
pub use crate::workflow_learning_types::{
    NewWorkflowJob, WorkflowJobEffectState, WorkflowJobKind, WorkflowJobRecord,
    WorkflowJobRecoverySummary, WorkflowJobStatus,
};
use crate::workflow_learning_validation::{
    validate_hash, validate_json, validate_text, validate_token,
};

impl WorkflowLearningStore {
    pub fn enqueue_job(
        &self,
        input: &NewWorkflowJob,
    ) -> Result<WorkflowJobRecord, WorkflowLearningControlError> {
        validate_job(input)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job = insert_job_in_tx(&tx, input)?;
        tx.commit()?;
        Ok(job)
    }

    pub fn observe_and_enqueue_analysis(
        &self,
        proposal: &NewWorkflowProposal,
        job: &NewWorkflowJob,
    ) -> Result<(WorkflowProposalRecord, WorkflowJobRecord), WorkflowLearningControlError> {
        if job.kind != WorkflowJobKind::Analyze
            || job.proposal_id != proposal.id
            || job.revision_sha256.is_some()
        {
            return Err(WorkflowLearningControlError::InvalidInput(
                "observed proposals require a matching revisionless Analyze job".to_string(),
            ));
        }
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let proposal = create_observed_in_tx(&tx, proposal)?;
        let job = insert_job_in_tx(&tx, job)?;
        tx.commit()?;
        Ok((proposal, job))
    }

    pub fn get_job(
        &self,
        id: &str,
    ) -> Result<Option<WorkflowJobRecord>, WorkflowLearningControlError> {
        validate_token("job id", id, 96)?;
        let conn = self.lock_conn()?;
        job_by_id(&conn, id).map_err(Into::into)
    }

    pub fn list_uncertain_jobs(
        &self,
        limit: usize,
    ) -> Result<Vec<WorkflowJobRecord>, WorkflowLearningControlError> {
        let conn = self.lock_conn()?;
        let mut statement = conn.prepare(&format!(
            "{JOB_SELECT} WHERE status = 'uncertain' ORDER BY updated_at, id LIMIT ?1"
        ))?;
        let rows = statement.query_map(params![limit.clamp(1, 1_000) as i64], job_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn claim_due_job(
        &self,
        worker: &str,
        now_unix_ms: i64,
        lease_duration_ms: i64,
    ) -> Result<Option<WorkflowJobRecord>, WorkflowLearningControlError> {
        self.claim_due_job_scope(worker, now_unix_ms, lease_duration_ms, JobClaimScope::All)
    }

    pub fn claim_due_preapproval_job(
        &self,
        worker: &str,
        now_unix_ms: i64,
        lease_duration_ms: i64,
    ) -> Result<Option<WorkflowJobRecord>, WorkflowLearningControlError> {
        self.claim_due_job_scope(
            worker,
            now_unix_ms,
            lease_duration_ms,
            JobClaimScope::Preapproval,
        )
    }

    pub fn claim_due_isolated_test_job(
        &self,
        worker: &str,
        now_unix_ms: i64,
        lease_duration_ms: i64,
    ) -> Result<Option<WorkflowJobRecord>, WorkflowLearningControlError> {
        self.claim_due_job_scope(
            worker,
            now_unix_ms,
            lease_duration_ms,
            JobClaimScope::IsolatedTest,
        )
    }

    /// Claim only activation lifecycle work. Isolated tests deliberately use
    /// the same `Install` kind, so their durable test row must exclude them
    /// from this worker rather than relying on payload conventions.
    pub fn claim_due_activation_job(
        &self,
        worker: &str,
        now_unix_ms: i64,
        lease_duration_ms: i64,
    ) -> Result<Option<WorkflowJobRecord>, WorkflowLearningControlError> {
        self.claim_due_job_scope(
            worker,
            now_unix_ms,
            lease_duration_ms,
            JobClaimScope::Activation,
        )
    }

    /// Reclaim one interrupted activation effect for deterministic journal
    /// reconciliation. This does not increment the attempt counter: the
    /// worker is verifying the already-started effect, not starting a new one.
    pub fn claim_uncertain_activation_job(
        &self,
        worker: &str,
        now_unix_ms: i64,
        lease_duration_ms: i64,
    ) -> Result<Option<WorkflowJobRecord>, WorkflowLearningControlError> {
        validate_job_claim(worker, lease_duration_ms)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        reconcile_jobs_in_tx(&tx, now_unix_ms, false)?;
        let id: Option<String> = tx
            .query_row(
                "SELECT jobs.id
                 FROM workflow_learning_jobs jobs
                 WHERE jobs.status = 'uncertain' AND jobs.effect_state = 'started'
                   AND jobs.run_after <= ?1
                   AND jobs.kind IN ('install', 'canary', 'rollback')
                   AND NOT EXISTS (
                       SELECT 1 FROM workflow_learning_tests tests
                       WHERE tests.job_id = jobs.id
                   )
                 ORDER BY jobs.run_after, jobs.updated_at, jobs.id LIMIT 1",
                params![now_unix_ms],
                |row| row.get(0),
            )
            .optional()?;
        let Some(id) = id else {
            tx.commit()?;
            return Ok(None);
        };
        let changed = tx.execute(
            "UPDATE workflow_learning_jobs
             SET status = 'running', lease_owner = ?1, lease_expires_at = ?2,
                 updated_at = ?3
             WHERE id = ?4 AND status = 'uncertain' AND effect_state = 'started'",
            params![worker, now_unix_ms + lease_duration_ms, now_unix_ms, id],
        )?;
        if changed != 1 {
            return Err(WorkflowLearningControlError::Conflict(
                "uncertain activation changed while claiming".to_string(),
            ));
        }
        let claimed = job_by_id(&tx, &id)?.ok_or_else(|| {
            WorkflowLearningControlError::CorruptData(
                "reclaimed activation job vanished".to_string(),
            )
        })?;
        tx.commit()?;
        Ok(Some(claimed))
    }

    fn claim_due_job_scope(
        &self,
        worker: &str,
        now_unix_ms: i64,
        lease_duration_ms: i64,
        scope: JobClaimScope,
    ) -> Result<Option<WorkflowJobRecord>, WorkflowLearningControlError> {
        validate_job_claim(worker, lease_duration_ms)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        reconcile_jobs_in_tx(&tx, now_unix_ms, false)?;
        let selection = match scope {
            JobClaimScope::Preapproval => {
                "SELECT id FROM workflow_learning_jobs
             WHERE status IN ('pending', 'retry_wait') AND run_after <= ?1
               AND kind IN ('analyze', 'draft', 'validate')
             ORDER BY run_after, created_at, id LIMIT 1"
            }
            JobClaimScope::IsolatedTest => {
                "SELECT jobs.id
             FROM workflow_learning_jobs jobs
             INNER JOIN workflow_learning_tests tests ON tests.job_id = jobs.id
             WHERE jobs.status IN ('pending', 'retry_wait') AND jobs.run_after <= ?1
               AND jobs.kind = 'install' AND tests.status = 'queued'
             ORDER BY jobs.run_after, jobs.created_at, jobs.id LIMIT 1"
            }
            JobClaimScope::Activation => {
                "SELECT jobs.id
             FROM workflow_learning_jobs jobs
             WHERE jobs.status IN ('pending', 'retry_wait') AND jobs.run_after <= ?1
               AND jobs.kind IN ('install', 'canary', 'rollback')
               AND NOT EXISTS (
                   SELECT 1 FROM workflow_learning_tests tests
                   WHERE tests.job_id = jobs.id
               )
             ORDER BY jobs.run_after, jobs.created_at, jobs.id LIMIT 1"
            }
            JobClaimScope::All => {
                "SELECT id FROM workflow_learning_jobs
             WHERE status IN ('pending', 'retry_wait') AND run_after <= ?1
             ORDER BY run_after, created_at, id LIMIT 1"
            }
        };
        let id: Option<String> = tx
            .query_row(selection, params![now_unix_ms], |row| row.get(0))
            .optional()?;
        let Some(id) = id else {
            tx.commit()?;
            return Ok(None);
        };
        let changed = tx.execute(
            "UPDATE workflow_learning_jobs
             SET status = 'running', attempt_count = attempt_count + 1,
                 lease_owner = ?1, lease_expires_at = ?2, updated_at = ?3
             WHERE id = ?4 AND status IN ('pending', 'retry_wait')
               AND effect_state = 'none' AND run_after <= ?3",
            params![worker, now_unix_ms + lease_duration_ms, now_unix_ms, id],
        )?;
        if changed != 1 {
            return Err(WorkflowLearningControlError::Conflict(
                "job changed while claiming".to_string(),
            ));
        }
        let claimed = job_by_id(&tx, &id)?.ok_or_else(|| {
            WorkflowLearningControlError::CorruptData("claimed job vanished".to_string())
        })?;
        tx.commit()?;
        Ok(Some(claimed))
    }

    pub fn mark_job_effect_started(
        &self,
        id: &str,
        worker: &str,
        started_at_unix_ms: i64,
    ) -> Result<WorkflowJobRecord, WorkflowLearningControlError> {
        validate_token("job id", id, 96)?;
        validate_token("job worker", worker, 96)?;
        let conn = self.lock_conn()?;
        let current = job_by_id(&conn, id)?
            .ok_or_else(|| WorkflowLearningControlError::NotFound(id.to_string()))?;
        if current.status == WorkflowJobStatus::Running
            && current.lease_owner.as_deref() == Some(worker)
            && current.effect_state == WorkflowJobEffectState::Started
            && current.lease_expires_at_unix_ms > Some(started_at_unix_ms)
        {
            return Ok(current);
        }
        let changed = conn.execute(
            "UPDATE workflow_learning_jobs SET effect_state = 'started', updated_at = ?1
             WHERE id = ?2 AND status = 'running' AND lease_owner = ?3
               AND effect_state = 'none' AND lease_expires_at > ?1",
            params![started_at_unix_ms, id, worker],
        )?;
        if changed != 1 {
            return Err(WorkflowLearningControlError::Conflict(
                "effect start requires a live job lease".to_string(),
            ));
        }
        job_by_id(&conn, id)?.ok_or_else(|| WorkflowLearningControlError::NotFound(id.into()))
    }

    pub fn complete_job(
        &self,
        id: &str,
        worker: &str,
        result_json: Option<&str>,
        completed_at_unix_ms: i64,
    ) -> Result<WorkflowJobRecord, WorkflowLearningControlError> {
        validate_token("job id", id, 96)?;
        validate_token("job worker", worker, 96)?;
        if let Some(result) = result_json {
            validate_json("job result_json", result, 64 * 1024)?;
        }
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let completed = complete_job_in_tx(&tx, id, worker, result_json, completed_at_unix_ms)?;
        tx.commit()?;
        Ok(completed)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fail_job(
        &self,
        id: &str,
        worker: &str,
        error_code: &str,
        error_message: &str,
        retryable: bool,
        retry_at_unix_ms: i64,
        failed_at_unix_ms: i64,
    ) -> Result<WorkflowJobRecord, WorkflowLearningControlError> {
        validate_token("job id", id, 96)?;
        validate_token("job worker", worker, 96)?;
        validate_token("job error_code", error_code, 96)?;
        validate_text("job error_message", error_message, 1, 2_048)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = job_by_id(&tx, id)?
            .ok_or_else(|| WorkflowLearningControlError::NotFound(id.to_string()))?;
        if current.status != WorkflowJobStatus::Running
            || current.lease_owner.as_deref() != Some(worker)
            || current.lease_expires_at_unix_ms <= Some(failed_at_unix_ms)
        {
            return Err(WorkflowLearningControlError::Conflict(
                "job failure requires the current lease".to_string(),
            ));
        }
        let (status, run_after) = if current.effect_state == WorkflowJobEffectState::Started {
            (WorkflowJobStatus::Uncertain, failed_at_unix_ms)
        } else if retryable && current.attempt_count < current.max_attempts {
            (WorkflowJobStatus::RetryWait, retry_at_unix_ms)
        } else {
            (WorkflowJobStatus::Dead, failed_at_unix_ms)
        };
        tx.execute(
            "UPDATE workflow_learning_jobs
             SET status = ?1, run_after = ?2, error_code = ?3, error_message = ?4,
                 lease_owner = NULL, lease_expires_at = NULL, updated_at = ?5
             WHERE id = ?6 AND status = 'running' AND lease_owner = ?7",
            params![
                status.as_str(),
                run_after,
                error_code,
                error_message,
                failed_at_unix_ms,
                id,
                worker,
            ],
        )?;
        let failed = job_by_id(&tx, id)?
            .ok_or_else(|| WorkflowLearningControlError::NotFound(id.to_string()))?;
        tx.commit()?;
        Ok(failed)
    }

    /// Settle a model or other external call whose failure was observed by the
    /// worker. Unlike a crash after `effect_state=started`, this known result
    /// may be retried because the previous call has conclusively returned.
    #[allow(clippy::too_many_arguments)]
    pub fn fail_job_after_known_effect(
        &self,
        id: &str,
        worker: &str,
        error_code: &str,
        error_message: &str,
        retryable: bool,
        retry_at_unix_ms: i64,
        failed_at_unix_ms: i64,
        notification: Option<&NewWorkflowOutboxItem>,
    ) -> Result<WorkflowJobRecord, WorkflowLearningControlError> {
        validate_token("job id", id, 96)?;
        validate_token("job worker", worker, 96)?;
        validate_token("job error_code", error_code, 96)?;
        validate_text("job error_message", error_message, 1, 2_048)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = job_by_id(&tx, id)?
            .ok_or_else(|| WorkflowLearningControlError::NotFound(id.to_string()))?;
        let retry = retryable && current.attempt_count < current.max_attempts;
        let expected_status = if retry {
            WorkflowJobStatus::RetryWait
        } else {
            WorkflowJobStatus::Dead
        };
        let expected_effect = if retry {
            WorkflowJobEffectState::None
        } else {
            WorkflowJobEffectState::Completed
        };
        let expected_run_after = if retry {
            retry_at_unix_ms
        } else {
            failed_at_unix_ms
        };
        if retry && notification.is_some() {
            return Err(WorkflowLearningControlError::InvalidInput(
                "known-effect retry cannot emit a terminal notification".to_string(),
            ));
        }
        if current.status == expected_status
            && current.effect_state == expected_effect
            && current.error_code.as_deref() == Some(error_code)
            && current.error_message.as_deref() == Some(error_message)
            && current.run_after_unix_ms == expected_run_after
        {
            if let Some(notification) = notification {
                insert_outbox_in_tx(&tx, notification)?;
            }
            tx.commit()?;
            return Ok(current);
        }
        if current.status != WorkflowJobStatus::Running
            || current.lease_owner.as_deref() != Some(worker)
            || current.effect_state != WorkflowJobEffectState::Started
            || current.lease_expires_at_unix_ms <= Some(failed_at_unix_ms)
        {
            return Err(WorkflowLearningControlError::Conflict(
                "known effect failure requires the current started job lease".to_string(),
            ));
        }
        let changed = tx.execute(
            "UPDATE workflow_learning_jobs
             SET status = ?1, effect_state = ?2, run_after = ?3,
                 error_code = ?4, error_message = ?5, lease_owner = NULL,
                 lease_expires_at = NULL, updated_at = ?6
             WHERE id = ?7 AND status = 'running' AND lease_owner = ?8
               AND effect_state = 'started' AND lease_expires_at > ?6",
            params![
                expected_status.as_str(),
                expected_effect.as_str(),
                expected_run_after,
                error_code,
                error_message,
                failed_at_unix_ms,
                id,
                worker,
            ],
        )?;
        if changed != 1 {
            return Err(WorkflowLearningControlError::Conflict(
                "known effect failure changed concurrently".to_string(),
            ));
        }
        let settled = job_by_id(&tx, id)?
            .ok_or_else(|| WorkflowLearningControlError::NotFound(id.to_string()))?;
        if let Some(notification) = notification {
            insert_outbox_in_tx(&tx, notification)?;
        }
        tx.commit()?;
        Ok(settled)
    }

    pub fn reconcile_expired_jobs(
        &self,
        now_unix_ms: i64,
    ) -> Result<WorkflowJobRecoverySummary, WorkflowLearningControlError> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let summary = reconcile_jobs_in_tx(&tx, now_unix_ms, false)?;
        tx.commit()?;
        Ok(summary)
    }

    /// Reconcile every job leased by the previous Captain process.
    ///
    /// Call only during boot, before any workflow-learning worker starts.
    pub fn reconcile_jobs_after_restart(
        &self,
        now_unix_ms: i64,
    ) -> Result<WorkflowJobRecoverySummary, WorkflowLearningControlError> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let summary = reconcile_jobs_in_tx(&tx, now_unix_ms, true)?;
        tx.commit()?;
        Ok(summary)
    }

    pub fn publish_and_enqueue_notification(
        &self,
        publish: &PublishValidatedDraft,
        notification: &NewWorkflowOutboxItem,
    ) -> Result<WorkflowProposalRecord, WorkflowLearningControlError> {
        validate_publish(publish)?;
        if notification.proposal_id != publish.proposal_id
            || notification.revision_sha256.as_deref() != Some(&publish.revision_sha256)
        {
            return Err(WorkflowLearningControlError::InvalidInput(
                "proposal notification must identify the published revision".to_string(),
            ));
        }
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let proposal = publish_validated_draft_in_tx(&tx, publish)?;
        insert_outbox_in_tx(&tx, notification)?;
        tx.commit()?;
        Ok(proposal)
    }

    pub fn approve_and_enqueue_install(
        &self,
        transition: &WorkflowProposalTransition,
        install_job: &NewWorkflowJob,
        notification: Option<&NewWorkflowOutboxItem>,
    ) -> Result<WorkflowProposalRecord, WorkflowLearningControlError> {
        validate_transition(transition)?;
        validate_job(install_job)?;
        if transition.to_state != WorkflowProposalState::ApprovedPendingInstall
            || install_job.kind != WorkflowJobKind::Install
            || install_job.proposal_id != transition.proposal_id
            || install_job.revision_sha256 != transition.expected_revision_sha256
        {
            return Err(WorkflowLearningControlError::InvalidInput(
                "approval must atomically enqueue the matching install revision".to_string(),
            ));
        }
        if let Some(notification) = notification {
            if notification.proposal_id != transition.proposal_id
                || notification.revision_sha256 != transition.expected_revision_sha256
            {
                return Err(WorkflowLearningControlError::InvalidInput(
                    "approval notification must identify the approved revision".to_string(),
                ));
            }
        }
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let proposal = transition_in_tx(&tx, transition)?;
        insert_job_in_tx(&tx, install_job)?;
        if let Some(notification) = notification {
            insert_outbox_in_tx(&tx, notification)?;
        }
        tx.commit()?;
        Ok(proposal)
    }
}

#[derive(Debug, Clone, Copy)]
enum JobClaimScope {
    All,
    Preapproval,
    IsolatedTest,
    Activation,
}

fn validate_job_claim(
    worker: &str,
    lease_duration_ms: i64,
) -> Result<(), WorkflowLearningControlError> {
    validate_token("job worker", worker, 96)?;
    if !(1_000..=3_600_000).contains(&lease_duration_ms) {
        return Err(WorkflowLearningControlError::InvalidInput(
            "job lease must be between 1 second and 1 hour".to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn insert_job_in_tx(
    tx: &Transaction<'_>,
    input: &NewWorkflowJob,
) -> Result<WorkflowJobRecord, WorkflowLearningControlError> {
    validate_job(input)?;
    if let Some(existing) = job_by_idempotency(tx, &input.idempotency_key)? {
        if existing.id == input.id
            && existing.proposal_id == input.proposal_id
            && existing.revision_sha256 == input.revision_sha256
            && existing.kind == input.kind
            && existing.payload_json == input.payload_json
        {
            return Ok(existing);
        }
        return Err(WorkflowLearningControlError::Conflict(
            "job idempotency key was reused with different input".to_string(),
        ));
    }
    let proposal = proposal_by_id(tx, &input.proposal_id)?
        .ok_or_else(|| WorkflowLearningControlError::NotFound(input.proposal_id.clone()))?;
    if input.revision_sha256.is_some() && input.revision_sha256 != proposal.revision_sha256 {
        return Err(WorkflowLearningControlError::Conflict(
            "job revision does not match the proposal revision".to_string(),
        ));
    }
    if !job_allowed_in_state(input.kind, proposal.state) {
        return Err(WorkflowLearningControlError::Conflict(format!(
            "{} job is not valid while proposal is {}",
            input.kind.as_str(),
            proposal.state.as_str()
        )));
    }
    tx.execute(
        "INSERT INTO workflow_learning_jobs (
             id, idempotency_key, proposal_id, revision_sha256, kind,
             payload_json, max_attempts, run_after, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
        params![
            input.id,
            input.idempotency_key,
            input.proposal_id,
            input.revision_sha256,
            input.kind.as_str(),
            input.payload_json,
            input.max_attempts,
            input.run_after_unix_ms,
            input.created_at_unix_ms,
        ],
    )?;
    job_by_id(tx, &input.id)?
        .ok_or_else(|| WorkflowLearningControlError::NotFound(input.id.clone()))
}

fn reconcile_jobs_in_tx(
    tx: &Transaction<'_>,
    now_unix_ms: i64,
    include_unexpired: bool,
) -> Result<WorkflowJobRecoverySummary, WorkflowLearningControlError> {
    let interrupted = i64::from(include_unexpired);
    let retry_code = if include_unexpired {
        "process_restarted"
    } else {
        "lease_expired"
    };
    let retry_message = if include_unexpired {
        "previous process stopped before external effect"
    } else {
        "worker lease expired before external effect"
    };
    let retried_without_effect = tx.execute(
        "UPDATE workflow_learning_jobs
         SET status = 'retry_wait', run_after = ?1, lease_owner = NULL,
             lease_expires_at = NULL, error_code = ?2,
             error_message = ?3, updated_at = ?1
         WHERE status = 'running' AND (?4 = 1 OR lease_expires_at <= ?1)
           AND effect_state = 'none' AND attempt_count < max_attempts",
        params![now_unix_ms, retry_code, retry_message, interrupted],
    )?;
    let dead_message = if include_unexpired {
        "previous process stopped after final attempt"
    } else {
        "worker lease expired after final attempt"
    };
    let dead = tx.execute(
        "UPDATE workflow_learning_jobs
         SET status = 'dead', run_after = ?1, lease_owner = NULL,
             lease_expires_at = NULL, error_code = 'attempts_exhausted',
             error_message = ?2, updated_at = ?1
         WHERE status = 'running' AND (?3 = 1 OR lease_expires_at <= ?1)
           AND effect_state = 'none' AND attempt_count >= max_attempts",
        params![now_unix_ms, dead_message, interrupted],
    )?;
    let uncertain_message = if include_unexpired {
        "process stopped after effect start; automatic replay blocked"
    } else {
        "lease expired after effect start; automatic replay blocked"
    };
    let uncertain_effects = tx.execute(
        "UPDATE workflow_learning_jobs
         SET status = 'uncertain', run_after = ?1, lease_owner = NULL,
             lease_expires_at = NULL, error_code = 'effect_interrupted',
             error_message = ?2,
             updated_at = ?1
         WHERE status = 'running' AND (?3 = 1 OR lease_expires_at <= ?1)
           AND effect_state = 'started'",
        params![now_unix_ms, uncertain_message, interrupted],
    )?;
    Ok(WorkflowJobRecoverySummary {
        retried_without_effect,
        uncertain_effects,
        dead,
    })
}

const JOB_SELECT: &str = "SELECT id, idempotency_key, proposal_id, revision_sha256, kind, status,
            payload_json, attempt_count, max_attempts, run_after, lease_owner,
            lease_expires_at, effect_state, result_json, error_code,
            error_message, created_at, updated_at
     FROM workflow_learning_jobs";

pub(crate) fn job_by_id(
    conn: &rusqlite::Connection,
    id: &str,
) -> rusqlite::Result<Option<WorkflowJobRecord>> {
    conn.query_row(
        &format!("{JOB_SELECT} WHERE id = ?1"),
        params![id],
        job_from_row,
    )
    .optional()
}

pub(crate) fn complete_job_in_tx(
    tx: &Transaction<'_>,
    id: &str,
    worker: &str,
    result_json: Option<&str>,
    completed_at_unix_ms: i64,
) -> Result<WorkflowJobRecord, WorkflowLearningControlError> {
    validate_token("job id", id, 96)?;
    validate_token("job worker", worker, 96)?;
    if let Some(result) = result_json {
        validate_json("job result_json", result, 64 * 1024)?;
    }
    let current =
        job_by_id(tx, id)?.ok_or_else(|| WorkflowLearningControlError::NotFound(id.to_string()))?;
    if current.status == WorkflowJobStatus::Succeeded
        && current.effect_state == WorkflowJobEffectState::Completed
        && current.result_json.as_deref() == result_json
    {
        return Ok(current);
    }
    let changed = tx.execute(
        "UPDATE workflow_learning_jobs
         SET status = 'succeeded', effect_state = 'completed', result_json = ?1,
             error_code = NULL, error_message = NULL, lease_owner = NULL,
             lease_expires_at = NULL, updated_at = ?2
         WHERE id = ?3 AND status = 'running' AND lease_owner = ?4
           AND lease_expires_at > ?2",
        params![result_json, completed_at_unix_ms, id, worker],
    )?;
    if changed != 1 {
        return Err(WorkflowLearningControlError::Conflict(
            "job completion requires the current lease".to_string(),
        ));
    }
    job_by_id(tx, id)?.ok_or_else(|| WorkflowLearningControlError::NotFound(id.into()))
}

pub(crate) fn recover_uncertain_job_in_tx(
    tx: &Transaction<'_>,
    id: &str,
    result_json: Option<&str>,
    recovered_at_unix_ms: i64,
) -> Result<WorkflowJobRecord, WorkflowLearningControlError> {
    validate_token("job id", id, 96)?;
    if let Some(result) = result_json {
        validate_json("job result_json", result, 64 * 1024)?;
    }
    let current =
        job_by_id(tx, id)?.ok_or_else(|| WorkflowLearningControlError::NotFound(id.to_string()))?;
    if current.status == WorkflowJobStatus::Succeeded
        && current.effect_state == WorkflowJobEffectState::Completed
        && current.result_json.as_deref() == result_json
    {
        return Ok(current);
    }
    let changed = tx.execute(
        "UPDATE workflow_learning_jobs
         SET status = 'succeeded', effect_state = 'completed', result_json = ?1,
             error_code = NULL, error_message = NULL, lease_owner = NULL,
             lease_expires_at = NULL, updated_at = ?2
         WHERE id = ?3 AND status = 'uncertain' AND effect_state = 'started'",
        params![result_json, recovered_at_unix_ms, id],
    )?;
    if changed != 1 {
        return Err(WorkflowLearningControlError::Conflict(
            "job recovery requires an uncertain started effect".to_string(),
        ));
    }
    job_by_id(tx, id)?.ok_or_else(|| WorkflowLearningControlError::NotFound(id.into()))
}

fn job_by_idempotency(
    conn: &rusqlite::Connection,
    idempotency_key: &str,
) -> rusqlite::Result<Option<WorkflowJobRecord>> {
    conn.query_row(
        &format!("{JOB_SELECT} WHERE idempotency_key = ?1"),
        params![idempotency_key],
        job_from_row,
    )
    .optional()
}

fn job_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowJobRecord> {
    let kind_value: String = row.get(4)?;
    let status_value: String = row.get(5)?;
    let effect_value: String = row.get(12)?;
    Ok(WorkflowJobRecord {
        id: row.get(0)?,
        idempotency_key: row.get(1)?,
        proposal_id: row.get(2)?,
        revision_sha256: row.get(3)?,
        kind: WorkflowJobKind::parse(&kind_value)
            .ok_or_else(|| corrupt_column(4, format!("unknown job kind {kind_value}")))?,
        status: WorkflowJobStatus::parse(&status_value)
            .ok_or_else(|| corrupt_column(5, format!("unknown job status {status_value}")))?,
        payload_json: row.get(6)?,
        attempt_count: row.get::<_, i64>(7)?.max(0) as u32,
        max_attempts: row.get::<_, i64>(8)?.max(0) as u32,
        run_after_unix_ms: row.get(9)?,
        lease_owner: row.get(10)?,
        lease_expires_at_unix_ms: row.get(11)?,
        effect_state: WorkflowJobEffectState::parse(&effect_value)
            .ok_or_else(|| corrupt_column(12, format!("unknown effect state {effect_value}")))?,
        result_json: row.get(13)?,
        error_code: row.get(14)?,
        error_message: row.get(15)?,
        created_at_unix_ms: row.get(16)?,
        updated_at_unix_ms: row.get(17)?,
    })
}

fn corrupt_column(column: usize, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message,
        )),
    )
}

fn validate_job(input: &NewWorkflowJob) -> Result<(), WorkflowLearningControlError> {
    validate_token("job id", &input.id, 96)?;
    validate_token("job idempotency_key", &input.idempotency_key, 192)?;
    validate_token("proposal_id", &input.proposal_id, 96)?;
    if let Some(revision) = &input.revision_sha256 {
        validate_hash("job revision_sha256", revision)?;
    }
    validate_json("job payload_json", &input.payload_json, 64 * 1024)?;
    if !(1..=20).contains(&input.max_attempts) {
        return Err(WorkflowLearningControlError::InvalidInput(
            "job max_attempts must be between 1 and 20".to_string(),
        ));
    }
    Ok(())
}

fn job_allowed_in_state(kind: WorkflowJobKind, state: WorkflowProposalState) -> bool {
    match kind {
        WorkflowJobKind::Analyze => matches!(
            state,
            WorkflowProposalState::Observed | WorkflowProposalState::Eligible
        ),
        WorkflowJobKind::Draft => state == WorkflowProposalState::Drafting,
        WorkflowJobKind::Validate => state == WorkflowProposalState::Validating,
        WorkflowJobKind::Install => state == WorkflowProposalState::ApprovedPendingInstall,
        WorkflowJobKind::Canary => state == WorkflowProposalState::ActiveCanary,
        WorkflowJobKind::Rollback => matches!(
            state,
            WorkflowProposalState::ActiveCanary
                | WorkflowProposalState::Active
                | WorkflowProposalState::InstallFailed
        ),
    }
}
