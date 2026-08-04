//! Durable control plane for detached sub-agent delegations.
//!
//! A job becomes `uncertain` when Captain stops after the delegated model
//! turn started. Captain never replays that ambiguous effect automatically;
//! an agent or operator must explicitly resume it.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use captain_types::agent_delegation::{
    AgentDelegationEffectState, AgentDelegationJobRecord, AgentDelegationRecoverySummary,
    AgentDelegationStatus, NewAgentDelegationJob, AGENT_DELEGATION_MAX_ACTIVE_PER_CALLER,
    AGENT_DELEGATION_MAX_ATTEMPTS, AGENT_DELEGATION_MAX_DEPENDENCIES, AGENT_DELEGATION_MAX_DEPTH,
    AGENT_DELEGATION_MAX_LINEAGE_TOKENS, AGENT_DELEGATION_MAX_RESULT_BYTES,
    AGENT_DELEGATION_MAX_TASK_BYTES, AGENT_DELEGATION_MAX_TOKENS,
};
use captain_types::error::{CaptainError, CaptainResult};
use rusqlite::{
    params, types::Type, Connection, OptionalExtension, Transaction, TransactionBehavior,
};

const JOB_SELECT: &str = "SELECT id, idempotency_key, caller_agent_id, target_agent_id,
    title, task, max_tokens, status, state_version, attempt_count, lease_owner,
    lease_expires_at, effect_state, result, result_truncated, used_tokens,
    error_code, error_message, cancel_requested_at, started_at, completed_at,
    created_at, updated_at, root_job_id, parent_job_id, depth,
    (SELECT reserved_tokens FROM agent_delegation_lineages
     WHERE root_job_id = agent_delegation_jobs.root_job_id)
    FROM agent_delegation_jobs";

#[derive(Clone)]
pub struct AgentDelegationJobStore {
    conn: Arc<Mutex<Connection>>,
}

impl AgentDelegationJobStore {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn enqueue(
        &self,
        input: &NewAgentDelegationJob,
    ) -> CaptainResult<AgentDelegationJobRecord> {
        validate_new_job(input)?;
        let dependencies = normalized_dependencies(&input.depends_on)?;
        let mut conn = self.lock_conn()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(memory_error)?;

        if let Some(existing) = job_by_idempotency(&tx, &input.idempotency_key)? {
            if same_job_input(&existing, input, &dependencies) {
                tx.commit().map_err(memory_error)?;
                return Ok(existing);
            }
            return Err(CaptainError::InvalidInput(
                "agent delegation idempotency key was reused with different input".to_string(),
            ));
        }
        if job_by_id(&tx, &input.id)?.is_some() {
            return Err(CaptainError::InvalidInput(format!(
                "agent delegation job id already exists: {}",
                input.id
            )));
        }

        let active: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM agent_delegation_jobs
                 WHERE caller_agent_id = ?1
                   AND status IN ('blocked', 'queued', 'running', 'cancel_requested')",
                params![input.caller_agent_id],
                |row| row.get(0),
            )
            .map_err(memory_error)?;
        if active >= AGENT_DELEGATION_MAX_ACTIVE_PER_CALLER as i64 {
            return Err(CaptainError::InvalidInput(format!(
                "agent delegation active-job limit reached ({AGENT_DELEGATION_MAX_ACTIVE_PER_CALLER})"
            )));
        }

        validate_and_reserve_lineage(&tx, input)?;
        let initial_status = dependency_status(&tx, &input.caller_agent_id, &dependencies)?;
        tx.execute(
            "INSERT INTO agent_delegation_jobs (
                 id, idempotency_key, caller_agent_id, target_agent_id, title,
                 task, max_tokens, status, error_code, error_message,
                 completed_at, created_at, updated_at, root_job_id,
                 parent_job_id, depth
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12,
                 ?13, ?14, ?15
             )",
            params![
                input.id,
                input.idempotency_key,
                input.caller_agent_id,
                input.target_agent_id,
                input.title,
                input.task,
                input.max_tokens as i64,
                initial_status.as_str(),
                (initial_status == AgentDelegationStatus::DependencyFailed)
                    .then_some("dependency_failed"),
                (initial_status == AgentDelegationStatus::DependencyFailed)
                    .then_some("one or more prerequisite delegation jobs did not succeed"),
                initial_status
                    .is_terminal()
                    .then_some(input.created_at_unix_ms),
                input.created_at_unix_ms,
                input.root_job_id,
                input.parent_job_id,
                input.depth,
            ],
        )
        .map_err(memory_error)?;
        for dependency in &dependencies {
            tx.execute(
                "INSERT INTO agent_delegation_dependencies
                     (job_id, depends_on_job_id, created_at) VALUES (?1, ?2, ?3)",
                params![input.id, dependency, input.created_at_unix_ms],
            )
            .map_err(memory_error)?;
        }
        let created = job_by_id(&tx, &input.id)?.ok_or_else(|| {
            CaptainError::Internal("created agent delegation job vanished".to_string())
        })?;
        tx.commit().map_err(memory_error)?;
        Ok(created)
    }

    pub fn get_for_caller(
        &self,
        caller_agent_id: &str,
        job_id: &str,
    ) -> CaptainResult<Option<AgentDelegationJobRecord>> {
        validate_token("caller agent id", caller_agent_id, 96)?;
        validate_token("delegation job id", job_id, 96)?;
        let conn = self.lock_conn()?;
        let job = job_by_id(&conn, job_id)?;
        match job {
            Some(job) if job.caller_agent_id == caller_agent_id => Ok(Some(job)),
            Some(_) => Err(CaptainError::AuthDenied(
                "delegation job belongs to another agent".to_string(),
            )),
            None => Ok(None),
        }
    }

    pub fn get(&self, job_id: &str) -> CaptainResult<Option<AgentDelegationJobRecord>> {
        validate_token("delegation job id", job_id, 96)?;
        let conn = self.lock_conn()?;
        job_by_id(&conn, job_id)
    }

    pub fn list_for_caller(
        &self,
        caller_agent_id: &str,
        status: Option<AgentDelegationStatus>,
        limit: usize,
    ) -> CaptainResult<Vec<AgentDelegationJobRecord>> {
        validate_token("caller agent id", caller_agent_id, 96)?;
        let conn = self.lock_conn()?;
        let limit = limit.clamp(1, 200) as i64;
        let mut records = Vec::new();
        if let Some(status) = status {
            let mut statement = conn
                .prepare(&format!(
                    "{JOB_SELECT} WHERE caller_agent_id = ?1 AND status = ?2
                     ORDER BY updated_at DESC, id DESC LIMIT ?3"
                ))
                .map_err(memory_error)?;
            let rows = statement
                .query_map(params![caller_agent_id, status.as_str(), limit], row_to_job)
                .map_err(memory_error)?;
            for row in rows {
                records.push(row.map_err(memory_error)?);
            }
        } else {
            let mut statement = conn
                .prepare(&format!(
                    "{JOB_SELECT} WHERE caller_agent_id = ?1
                     ORDER BY updated_at DESC, id DESC LIMIT ?2"
                ))
                .map_err(memory_error)?;
            let rows = statement
                .query_map(params![caller_agent_id, limit], row_to_job)
                .map_err(memory_error)?;
            for row in rows {
                records.push(row.map_err(memory_error)?);
            }
        }
        attach_dependencies(&conn, &mut records)?;
        Ok(records)
    }

    pub fn claim_ready(
        &self,
        worker: &str,
        now_unix_ms: i64,
        lease_duration_ms: i64,
    ) -> CaptainResult<Option<AgentDelegationJobRecord>> {
        validate_worker(worker, lease_duration_ms)?;
        let mut conn = self.lock_conn()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(memory_error)?;
        reconcile_in_tx(&tx, now_unix_ms, false)?;
        refresh_dependencies_in_tx(&tx, now_unix_ms)?;
        tx.execute(
            "UPDATE agent_delegation_jobs
             SET status = 'failed', state_version = state_version + 1,
                 error_code = 'attempts_exhausted',
                 error_message = 'delegation reached the explicit resume limit',
                 completed_at = ?1, updated_at = ?1
             WHERE status = 'queued' AND attempt_count >= ?2",
            params![now_unix_ms, AGENT_DELEGATION_MAX_ATTEMPTS],
        )
        .map_err(memory_error)?;
        refresh_dependencies_in_tx(&tx, now_unix_ms)?;
        let id: Option<String> = tx
            .query_row(
                "SELECT id FROM agent_delegation_jobs
                 WHERE status = 'queued' AND attempt_count < ?1
                 ORDER BY created_at, id LIMIT 1",
                params![AGENT_DELEGATION_MAX_ATTEMPTS],
                |row| row.get(0),
            )
            .optional()
            .map_err(memory_error)?;
        let Some(id) = id else {
            tx.commit().map_err(memory_error)?;
            return Ok(None);
        };
        let changed = tx
            .execute(
                "UPDATE agent_delegation_jobs
                 SET status = 'running', state_version = state_version + 1,
                     attempt_count = attempt_count + 1, lease_owner = ?1,
                     lease_expires_at = ?2, started_at = ?3, updated_at = ?3
                 WHERE id = ?4 AND status = 'queued' AND attempt_count < ?5",
                params![
                    worker,
                    now_unix_ms.saturating_add(lease_duration_ms),
                    now_unix_ms,
                    id,
                    AGENT_DELEGATION_MAX_ATTEMPTS,
                ],
            )
            .map_err(memory_error)?;
        if changed != 1 {
            return Err(CaptainError::Internal(
                "delegation job changed while claiming".to_string(),
            ));
        }
        let claimed = job_by_id(&tx, &id)?
            .ok_or_else(|| CaptainError::Internal("claimed delegation job vanished".to_string()))?;
        tx.commit().map_err(memory_error)?;
        Ok(Some(claimed))
    }

    pub fn mark_effect_started(
        &self,
        job_id: &str,
        worker: &str,
        now_unix_ms: i64,
    ) -> CaptainResult<AgentDelegationJobRecord> {
        validate_token("delegation job id", job_id, 96)?;
        validate_token("delegation worker", worker, 96)?;
        let conn = self.lock_conn()?;
        if let Some(current) = job_by_id(&conn, job_id)? {
            if current.status == AgentDelegationStatus::Running
                && current.lease_owner.as_deref() == Some(worker)
                && current.effect_state == AgentDelegationEffectState::Started
                && current.lease_expires_at_unix_ms > Some(now_unix_ms)
            {
                return Ok(current);
            }
        }
        let changed = conn
            .execute(
                "UPDATE agent_delegation_jobs
                 SET effect_state = 'started', state_version = state_version + 1,
                     updated_at = ?1
                 WHERE id = ?2 AND status = 'running' AND lease_owner = ?3
                   AND effect_state = 'not_started' AND lease_expires_at > ?1",
                params![now_unix_ms, job_id, worker],
            )
            .map_err(memory_error)?;
        if changed != 1 {
            return Err(CaptainError::InvalidState {
                current: "not running with a live unstarted lease".to_string(),
                operation: "start delegated agent effect".to_string(),
            });
        }
        job_by_id(&conn, job_id)?.ok_or_else(|| CaptainError::AgentNotFound(job_id.to_string()))
    }

    pub fn renew_lease(
        &self,
        job_id: &str,
        worker: &str,
        now_unix_ms: i64,
        lease_duration_ms: i64,
    ) -> CaptainResult<bool> {
        validate_token("delegation job id", job_id, 96)?;
        validate_worker(worker, lease_duration_ms)?;
        let conn = self.lock_conn()?;
        let changed = conn
            .execute(
                "UPDATE agent_delegation_jobs
                 SET lease_expires_at = ?1, updated_at = ?2
                 WHERE id = ?3 AND status IN ('running', 'cancel_requested')
                   AND lease_owner = ?4 AND lease_expires_at > ?2",
                params![
                    now_unix_ms.saturating_add(lease_duration_ms),
                    now_unix_ms,
                    job_id,
                    worker,
                ],
            )
            .map_err(memory_error)?;
        Ok(changed == 1)
    }

    pub fn complete(
        &self,
        job_id: &str,
        worker: &str,
        result: &str,
        used_tokens: u64,
        now_unix_ms: i64,
    ) -> CaptainResult<AgentDelegationJobRecord> {
        validate_token("delegation job id", job_id, 96)?;
        validate_token("delegation worker", worker, 96)?;
        let (result, result_truncated) = bounded_result(result);
        let mut conn = self.lock_conn()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(memory_error)?;
        let changed = tx
            .execute(
                "UPDATE agent_delegation_jobs
                 SET status = 'succeeded', state_version = state_version + 1,
                     effect_state = 'completed', result = ?1, result_truncated = ?2,
                     used_tokens = ?3, error_code = NULL, error_message = NULL,
                     lease_owner = NULL, lease_expires_at = NULL,
                     completed_at = ?4, updated_at = ?4
                 WHERE id = ?5 AND status IN ('running', 'cancel_requested')
                   AND lease_owner = ?6
                   AND effect_state = 'started' AND lease_expires_at > ?4",
                params![
                    result,
                    result_truncated,
                    to_i64("used tokens", used_tokens)?,
                    now_unix_ms,
                    job_id,
                    worker,
                ],
            )
            .map_err(memory_error)?;
        if changed != 1 {
            return Err(CaptainError::InvalidState {
                current: "not running with a live started lease".to_string(),
                operation: "complete delegation job".to_string(),
            });
        }
        refresh_dependencies_in_tx(&tx, now_unix_ms)?;
        let completed = job_by_id(&tx, job_id)?
            .ok_or_else(|| CaptainError::AgentNotFound(job_id.to_string()))?;
        tx.commit().map_err(memory_error)?;
        Ok(completed)
    }

    pub fn fail_before_effect(
        &self,
        job_id: &str,
        worker: &str,
        error_code: &str,
        error_message: &str,
        now_unix_ms: i64,
    ) -> CaptainResult<AgentDelegationJobRecord> {
        validate_token("delegation job id", job_id, 96)?;
        validate_token("delegation worker", worker, 96)?;
        validate_token("delegation error code", error_code, 96)?;
        validate_text("delegation error message", error_message, 1, 4_096)?;
        let mut conn = self.lock_conn()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(memory_error)?;
        let changed = tx
            .execute(
                "UPDATE agent_delegation_jobs
                 SET status = 'failed', state_version = state_version + 1,
                     effect_state = 'completed', error_code = ?1,
                     error_message = ?2, lease_owner = NULL,
                     lease_expires_at = NULL, completed_at = ?3, updated_at = ?3
                 WHERE id = ?4 AND status = 'running' AND lease_owner = ?5
                   AND effect_state = 'not_started' AND lease_expires_at > ?3",
                params![error_code, error_message, now_unix_ms, job_id, worker],
            )
            .map_err(memory_error)?;
        if changed != 1 {
            return Err(CaptainError::InvalidState {
                current: "not running with a live unstarted lease".to_string(),
                operation: "fail delegation before effect".to_string(),
            });
        }
        refresh_dependencies_in_tx(&tx, now_unix_ms)?;
        let failed = job_by_id(&tx, job_id)?
            .ok_or_else(|| CaptainError::AgentNotFound(job_id.to_string()))?;
        tx.commit().map_err(memory_error)?;
        Ok(failed)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fail_known(
        &self,
        job_id: &str,
        worker: &str,
        error_code: &str,
        error_message: &str,
        used_tokens: Option<u64>,
        now_unix_ms: i64,
    ) -> CaptainResult<AgentDelegationJobRecord> {
        validate_token("delegation job id", job_id, 96)?;
        validate_token("delegation worker", worker, 96)?;
        validate_token("delegation error code", error_code, 96)?;
        validate_text("delegation error message", error_message, 1, 4_096)?;
        let mut conn = self.lock_conn()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(memory_error)?;
        let changed = tx
            .execute(
                "UPDATE agent_delegation_jobs
                 SET status = 'failed', state_version = state_version + 1,
                     effect_state = 'completed', used_tokens = ?1,
                     error_code = ?2, error_message = ?3,
                     lease_owner = NULL, lease_expires_at = NULL,
                     completed_at = ?4, updated_at = ?4
                 WHERE id = ?5 AND status IN ('running', 'cancel_requested')
                   AND lease_owner = ?6
                   AND effect_state = 'started' AND lease_expires_at > ?4",
                params![
                    used_tokens
                        .map(|value| to_i64("used tokens", value))
                        .transpose()?,
                    error_code,
                    error_message,
                    now_unix_ms,
                    job_id,
                    worker,
                ],
            )
            .map_err(memory_error)?;
        if changed != 1 {
            return Err(CaptainError::InvalidState {
                current: "not running with a live started lease".to_string(),
                operation: "fail delegation job".to_string(),
            });
        }
        refresh_dependencies_in_tx(&tx, now_unix_ms)?;
        let failed = job_by_id(&tx, job_id)?
            .ok_or_else(|| CaptainError::AgentNotFound(job_id.to_string()))?;
        tx.commit().map_err(memory_error)?;
        Ok(failed)
    }

    pub fn request_cancel(
        &self,
        caller_agent_id: &str,
        job_id: &str,
        now_unix_ms: i64,
    ) -> CaptainResult<AgentDelegationJobRecord> {
        validate_token("caller agent id", caller_agent_id, 96)?;
        validate_token("delegation job id", job_id, 96)?;
        let mut conn = self.lock_conn()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(memory_error)?;
        let current = owned_job(&tx, caller_agent_id, job_id)?;
        match current.status {
            AgentDelegationStatus::Blocked | AgentDelegationStatus::Queued => {
                tx.execute(
                    "UPDATE agent_delegation_jobs
                     SET status = 'cancelled', state_version = state_version + 1,
                         effect_state = 'completed', error_code = 'cancelled',
                         error_message = 'delegation cancelled before execution',
                         cancel_requested_at = ?1, completed_at = ?1, updated_at = ?1
                     WHERE id = ?2 AND status IN ('blocked', 'queued')",
                    params![now_unix_ms, job_id],
                )
                .map_err(memory_error)?;
                refresh_dependencies_in_tx(&tx, now_unix_ms)?;
            }
            AgentDelegationStatus::Running => {
                tx.execute(
                    "UPDATE agent_delegation_jobs
                     SET status = 'cancel_requested', state_version = state_version + 1,
                         cancel_requested_at = ?1, updated_at = ?1
                     WHERE id = ?2 AND status = 'running'",
                    params![now_unix_ms, job_id],
                )
                .map_err(memory_error)?;
            }
            _ => {}
        }
        let updated = job_by_id(&tx, job_id)?
            .ok_or_else(|| CaptainError::AgentNotFound(job_id.to_string()))?;
        tx.commit().map_err(memory_error)?;
        Ok(updated)
    }

    pub fn settle_cancel_request(
        &self,
        job_id: &str,
        worker: &str,
        now_unix_ms: i64,
    ) -> CaptainResult<AgentDelegationJobRecord> {
        validate_token("delegation job id", job_id, 96)?;
        validate_token("delegation worker", worker, 96)?;
        let mut conn = self.lock_conn()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(memory_error)?;
        let current = job_by_id(&tx, job_id)?
            .ok_or_else(|| CaptainError::AgentNotFound(job_id.to_string()))?;
        if current.status != AgentDelegationStatus::CancelRequested
            || current.lease_owner.as_deref() != Some(worker)
            || current.lease_expires_at_unix_ms <= Some(now_unix_ms)
        {
            return Err(CaptainError::InvalidState {
                current: current.status.as_str().to_string(),
                operation: "settle delegation cancellation".to_string(),
            });
        }
        let (status, code, message) = if current.effect_state
            == AgentDelegationEffectState::NotStarted
        {
            (
                AgentDelegationStatus::Cancelled,
                "cancelled",
                "delegation cancelled before the model turn started",
            )
        } else {
            (
                AgentDelegationStatus::Uncertain,
                "cancelled_after_effect_start",
                "delegation cancellation interrupted a started model turn; replay requires explicit resume",
            )
        };
        tx.execute(
            "UPDATE agent_delegation_jobs
             SET status = ?1, state_version = state_version + 1,
                 error_code = ?2, error_message = ?3,
                 lease_owner = NULL, lease_expires_at = NULL,
                 completed_at = ?4, updated_at = ?4
             WHERE id = ?5 AND status = 'cancel_requested' AND lease_owner = ?6",
            params![status.as_str(), code, message, now_unix_ms, job_id, worker],
        )
        .map_err(memory_error)?;
        refresh_dependencies_in_tx(&tx, now_unix_ms)?;
        let settled = job_by_id(&tx, job_id)?
            .ok_or_else(|| CaptainError::AgentNotFound(job_id.to_string()))?;
        tx.commit().map_err(memory_error)?;
        Ok(settled)
    }

    /// Reconcile one worker that panicked or lost its local execution task.
    /// An unstarted job is safe to queue again; a started model turn is
    /// ambiguous and therefore requires explicit resume.
    pub fn interrupt_worker_job(
        &self,
        job_id: &str,
        worker: &str,
        detail: &str,
        now_unix_ms: i64,
    ) -> CaptainResult<AgentDelegationJobRecord> {
        validate_token("delegation job id", job_id, 96)?;
        validate_token("delegation worker", worker, 96)?;
        validate_text("delegation interruption detail", detail, 1, 4_096)?;
        let mut conn = self.lock_conn()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(memory_error)?;
        let current = job_by_id(&tx, job_id)?
            .ok_or_else(|| CaptainError::AgentNotFound(job_id.to_string()))?;
        if !matches!(
            current.status,
            AgentDelegationStatus::Running | AgentDelegationStatus::CancelRequested
        ) || current.lease_owner.as_deref() != Some(worker)
        {
            return Err(CaptainError::InvalidState {
                current: current.status.as_str().to_string(),
                operation: "reconcile interrupted delegation worker".to_string(),
            });
        }
        let (status, effect_state, code, message, completed_at) = match (
            current.status,
            current.effect_state,
        ) {
            (AgentDelegationStatus::CancelRequested, AgentDelegationEffectState::NotStarted) => (
                AgentDelegationStatus::Cancelled,
                AgentDelegationEffectState::Completed,
                "cancelled",
                "delegation cancellation completed before effect",
                Some(now_unix_ms),
            ),
            (_, AgentDelegationEffectState::NotStarted) => (
                AgentDelegationStatus::Queued,
                AgentDelegationEffectState::NotStarted,
                "worker_interrupted",
                detail,
                None,
            ),
            _ => (
                AgentDelegationStatus::Uncertain,
                AgentDelegationEffectState::Started,
                "worker_interrupted_after_effect",
                "delegation worker stopped after the model turn started; replay requires explicit resume",
                Some(now_unix_ms),
            ),
        };
        tx.execute(
            "UPDATE agent_delegation_jobs
             SET status = ?1, state_version = state_version + 1,
                 effect_state = ?2, error_code = ?3, error_message = ?4,
                 lease_owner = NULL, lease_expires_at = NULL,
                 completed_at = ?5, updated_at = ?6
             WHERE id = ?7 AND status IN ('running', 'cancel_requested')
               AND lease_owner = ?8",
            params![
                status.as_str(),
                effect_state.as_str(),
                code,
                message,
                completed_at,
                now_unix_ms,
                job_id,
                worker,
            ],
        )
        .map_err(memory_error)?;
        refresh_dependencies_in_tx(&tx, now_unix_ms)?;
        let reconciled = job_by_id(&tx, job_id)?
            .ok_or_else(|| CaptainError::AgentNotFound(job_id.to_string()))?;
        tx.commit().map_err(memory_error)?;
        Ok(reconciled)
    }

    pub fn resume(
        &self,
        caller_agent_id: &str,
        job_id: &str,
        now_unix_ms: i64,
    ) -> CaptainResult<AgentDelegationJobRecord> {
        validate_token("caller agent id", caller_agent_id, 96)?;
        validate_token("delegation job id", job_id, 96)?;
        let mut conn = self.lock_conn()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(memory_error)?;
        let current = owned_job(&tx, caller_agent_id, job_id)?;
        if !matches!(
            current.status,
            AgentDelegationStatus::Failed
                | AgentDelegationStatus::Uncertain
                | AgentDelegationStatus::DependencyFailed
        ) {
            return Err(CaptainError::InvalidState {
                current: current.status.as_str().to_string(),
                operation: "explicitly resume delegation job".to_string(),
            });
        }
        if current.attempt_count >= AGENT_DELEGATION_MAX_ATTEMPTS {
            return Err(CaptainError::InvalidState {
                current: "attempt limit reached".to_string(),
                operation: "explicitly resume delegation job".to_string(),
            });
        }
        let status = dependency_status(&tx, caller_agent_id, &current.depends_on)?;
        tx.execute(
            "UPDATE agent_delegation_jobs
             SET status = ?1, state_version = state_version + 1,
                 lease_owner = NULL, lease_expires_at = NULL,
                 effect_state = 'not_started', result = NULL,
                 result_truncated = 0, used_tokens = NULL,
                 error_code = ?2, error_message = ?3,
                 cancel_requested_at = NULL, started_at = NULL,
                 completed_at = NULL, updated_at = ?4
             WHERE id = ?5 AND status IN ('failed', 'uncertain', 'dependency_failed')",
            params![
                status.as_str(),
                (status == AgentDelegationStatus::DependencyFailed).then_some("dependency_failed"),
                (status == AgentDelegationStatus::DependencyFailed)
                    .then_some("one or more prerequisite delegation jobs still did not succeed"),
                now_unix_ms,
                job_id,
            ],
        )
        .map_err(memory_error)?;
        let resumed = job_by_id(&tx, job_id)?
            .ok_or_else(|| CaptainError::AgentNotFound(job_id.to_string()))?;
        tx.commit().map_err(memory_error)?;
        Ok(resumed)
    }

    pub fn reconcile_after_restart(
        &self,
        now_unix_ms: i64,
    ) -> CaptainResult<AgentDelegationRecoverySummary> {
        self.reconcile(now_unix_ms, true)
    }

    pub fn reconcile_expired(
        &self,
        now_unix_ms: i64,
    ) -> CaptainResult<AgentDelegationRecoverySummary> {
        self.reconcile(now_unix_ms, false)
    }

    /// Bound terminal history while preserving every live job and every job
    /// still referenced by another delegation dependency or lineage child. A
    /// lineage budget is removed only after its final job is gone, so partially
    /// retained or resumable lineages never regain already-reserved tokens.
    pub fn prune_terminal_history(&self, keep: usize) -> CaptainResult<usize> {
        let mut conn = self.lock_conn()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(memory_error)?;
        let deleted = tx
            .execute(
            "DELETE FROM agent_delegation_jobs
             WHERE status IN ('succeeded', 'failed', 'cancelled', 'uncertain', 'dependency_failed')
               AND id NOT IN (
                   SELECT id FROM agent_delegation_jobs
                   WHERE status IN ('succeeded', 'failed', 'cancelled', 'uncertain', 'dependency_failed')
                   ORDER BY updated_at DESC, id DESC LIMIT ?1
               )
               AND NOT EXISTS (
                   SELECT 1 FROM agent_delegation_dependencies dependencies
                   WHERE dependencies.depends_on_job_id = agent_delegation_jobs.id
               )
               AND NOT EXISTS (
                   SELECT 1 FROM agent_delegation_jobs children
                   WHERE children.parent_job_id = agent_delegation_jobs.id
               )",
            params![keep.clamp(100, 20_000) as i64],
        )
        .map_err(memory_error)?;
        tx.execute(
            "DELETE FROM agent_delegation_lineages
             WHERE NOT EXISTS (
                 SELECT 1 FROM agent_delegation_jobs jobs
                 WHERE jobs.root_job_id = agent_delegation_lineages.root_job_id
             )",
            [],
        )
        .map_err(memory_error)?;
        tx.commit().map_err(memory_error)?;
        Ok(deleted)
    }

    fn reconcile(
        &self,
        now_unix_ms: i64,
        include_unexpired: bool,
    ) -> CaptainResult<AgentDelegationRecoverySummary> {
        let mut conn = self.lock_conn()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(memory_error)?;
        let mut summary = reconcile_in_tx(&tx, now_unix_ms, include_unexpired)?;
        summary.dependency_failed = refresh_dependencies_in_tx(&tx, now_unix_ms)?;
        tx.commit().map_err(memory_error)?;
        Ok(summary)
    }

    fn lock_conn(&self) -> CaptainResult<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|error| CaptainError::Internal(format!("delegation store lock: {error}")))
    }
}

fn reconcile_in_tx(
    tx: &Transaction<'_>,
    now_unix_ms: i64,
    include_unexpired: bool,
) -> CaptainResult<AgentDelegationRecoverySummary> {
    let any_lease = i64::from(include_unexpired);
    let requeued_without_effect = tx
        .execute(
            "UPDATE agent_delegation_jobs
             SET status = 'queued', state_version = state_version + 1,
                 lease_owner = NULL, lease_expires_at = NULL,
                 error_code = ?1, error_message = ?2, updated_at = ?3
             WHERE status = 'running' AND (?4 = 1 OR lease_expires_at <= ?3)
               AND effect_state = 'not_started' AND attempt_count < ?5",
            params![
                if include_unexpired {
                    "process_restarted"
                } else {
                    "lease_expired"
                },
                if include_unexpired {
                    "previous Captain process stopped before the delegated model turn started"
                } else {
                    "delegation worker lease expired before the model turn started"
                },
                now_unix_ms,
                any_lease,
                AGENT_DELEGATION_MAX_ATTEMPTS,
            ],
        )
        .map_err(memory_error)?;
    tx.execute(
        "UPDATE agent_delegation_jobs
         SET status = 'failed', state_version = state_version + 1,
             lease_owner = NULL, lease_expires_at = NULL,
             effect_state = 'completed', error_code = 'attempts_exhausted',
             error_message = 'delegation stopped before effect after the final allowed attempt',
             completed_at = ?1, updated_at = ?1
         WHERE status = 'running' AND (?2 = 1 OR lease_expires_at <= ?1)
           AND effect_state = 'not_started' AND attempt_count >= ?3",
        params![now_unix_ms, any_lease, AGENT_DELEGATION_MAX_ATTEMPTS],
    )
    .map_err(memory_error)?;
    let cancelled_without_effect = tx
        .execute(
            "UPDATE agent_delegation_jobs
             SET status = 'cancelled', state_version = state_version + 1,
                 lease_owner = NULL, lease_expires_at = NULL,
                 effect_state = 'completed', error_code = 'cancelled',
                 error_message = 'cancellation completed before the delegated model turn started',
                 completed_at = ?1, updated_at = ?1
             WHERE status = 'cancel_requested' AND (?2 = 1 OR lease_expires_at <= ?1)
               AND effect_state = 'not_started'",
            params![now_unix_ms, any_lease],
        )
        .map_err(memory_error)?;
    let uncertain_after_effect = tx
        .execute(
            "UPDATE agent_delegation_jobs
             SET status = 'uncertain', state_version = state_version + 1,
                 lease_owner = NULL, lease_expires_at = NULL,
                 error_code = 'effect_interrupted',
                 error_message = ?1, completed_at = ?2, updated_at = ?2
             WHERE status IN ('running', 'cancel_requested')
               AND (?3 = 1 OR lease_expires_at <= ?2) AND effect_state = 'started'",
            params![
                if include_unexpired {
                    "Captain stopped after the delegated model turn started; automatic replay is blocked"
                } else {
                    "delegation lease expired after the model turn started; automatic replay is blocked"
                },
                now_unix_ms,
                any_lease,
            ],
        )
        .map_err(memory_error)?;
    Ok(AgentDelegationRecoverySummary {
        requeued_without_effect,
        cancelled_without_effect,
        uncertain_after_effect,
        dependency_failed: 0,
    })
}

fn refresh_dependencies_in_tx(tx: &Transaction<'_>, now_unix_ms: i64) -> CaptainResult<usize> {
    let mut total_failed = 0;
    loop {
        let ids = {
            let mut statement = tx
                .prepare(
                    "SELECT id, caller_agent_id FROM agent_delegation_jobs
                     WHERE status = 'blocked' ORDER BY created_at, id",
                )
                .map_err(memory_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(memory_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(memory_error)?
        };
        let mut changed = 0;
        for (job_id, caller) in ids {
            let dependencies = load_dependencies(tx, &job_id)?;
            let status = dependency_status(tx, &caller, &dependencies)?;
            if status == AgentDelegationStatus::Blocked {
                continue;
            }
            let dependency_failed = status == AgentDelegationStatus::DependencyFailed;
            changed += tx
                .execute(
                    "UPDATE agent_delegation_jobs
                     SET status = ?1, state_version = state_version + 1,
                         error_code = ?2, error_message = ?3,
                         completed_at = ?4, updated_at = ?5
                     WHERE id = ?6 AND status = 'blocked'",
                    params![
                        status.as_str(),
                        dependency_failed.then_some("dependency_failed"),
                        dependency_failed
                            .then_some("one or more prerequisite delegation jobs did not succeed"),
                        dependency_failed.then_some(now_unix_ms),
                        now_unix_ms,
                        job_id,
                    ],
                )
                .map_err(memory_error)?;
            total_failed += usize::from(dependency_failed);
        }
        if changed == 0 {
            break;
        }
    }
    Ok(total_failed)
}

fn dependency_status(
    conn: &Connection,
    caller_agent_id: &str,
    dependencies: &[String],
) -> CaptainResult<AgentDelegationStatus> {
    if dependencies.is_empty() {
        return Ok(AgentDelegationStatus::Queued);
    }
    let mut all_succeeded = true;
    for dependency in dependencies {
        let job = job_by_id(conn, dependency)?.ok_or_else(|| {
            CaptainError::InvalidInput(format!(
                "delegation dependency does not exist: {dependency}"
            ))
        })?;
        if job.caller_agent_id != caller_agent_id {
            return Err(CaptainError::AuthDenied(format!(
                "delegation dependency belongs to another agent: {dependency}"
            )));
        }
        if job.status.is_terminal() && !job.status.is_success() {
            return Ok(AgentDelegationStatus::DependencyFailed);
        }
        all_succeeded &= job.status.is_success();
    }
    Ok(if all_succeeded {
        AgentDelegationStatus::Queued
    } else {
        AgentDelegationStatus::Blocked
    })
}

fn owned_job(
    conn: &Connection,
    caller_agent_id: &str,
    job_id: &str,
) -> CaptainResult<AgentDelegationJobRecord> {
    let job =
        job_by_id(conn, job_id)?.ok_or_else(|| CaptainError::AgentNotFound(job_id.to_string()))?;
    if job.caller_agent_id != caller_agent_id {
        return Err(CaptainError::AuthDenied(
            "delegation job belongs to another agent".to_string(),
        ));
    }
    Ok(job)
}

fn job_by_id(conn: &Connection, id: &str) -> CaptainResult<Option<AgentDelegationJobRecord>> {
    let mut job = conn
        .query_row(
            &format!("{JOB_SELECT} WHERE id = ?1"),
            params![id],
            row_to_job,
        )
        .optional()
        .map_err(memory_error)?;
    if let Some(job) = &mut job {
        job.depends_on = load_dependencies(conn, id)?;
    }
    Ok(job)
}

fn job_by_idempotency(
    conn: &Connection,
    idempotency_key: &str,
) -> CaptainResult<Option<AgentDelegationJobRecord>> {
    let mut job = conn
        .query_row(
            &format!("{JOB_SELECT} WHERE idempotency_key = ?1"),
            params![idempotency_key],
            row_to_job,
        )
        .optional()
        .map_err(memory_error)?;
    if let Some(job) = &mut job {
        job.depends_on = load_dependencies(conn, &job.id)?;
    }
    Ok(job)
}

fn load_dependencies(conn: &Connection, job_id: &str) -> CaptainResult<Vec<String>> {
    let mut statement = conn
        .prepare(
            "SELECT depends_on_job_id FROM agent_delegation_dependencies
             WHERE job_id = ?1 ORDER BY depends_on_job_id",
        )
        .map_err(memory_error)?;
    let rows = statement
        .query_map(params![job_id], |row| row.get::<_, String>(0))
        .map_err(memory_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(memory_error)
}

fn attach_dependencies(
    conn: &Connection,
    jobs: &mut [AgentDelegationJobRecord],
) -> CaptainResult<()> {
    for job in jobs {
        job.depends_on = load_dependencies(conn, &job.id)?;
    }
    Ok(())
}

fn row_to_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentDelegationJobRecord> {
    let max_tokens = nonnegative_u64(row, 6, "max_tokens")?;
    let status_value: String = row.get(7)?;
    let state_version = nonnegative_u64(row, 8, "state_version")?;
    let attempt_count = nonnegative_u32(row, 9, "attempt_count")?;
    let effect_value: String = row.get(12)?;
    let used_tokens = optional_nonnegative_u64(row, 15, "used_tokens")?;
    let depth = nonnegative_u32(row, 25, "depth")?;
    let lineage_reserved_tokens = nonnegative_u64(row, 26, "lineage_reserved_tokens")?;
    Ok(AgentDelegationJobRecord {
        id: row.get(0)?,
        idempotency_key: row.get(1)?,
        root_job_id: row.get(23)?,
        parent_job_id: row.get(24)?,
        depth,
        lineage_reserved_tokens,
        caller_agent_id: row.get(2)?,
        target_agent_id: row.get(3)?,
        title: row.get(4)?,
        task: row.get(5)?,
        max_tokens,
        depends_on: Vec::new(),
        status: AgentDelegationStatus::parse(&status_value).ok_or_else(|| {
            corrupt_column(7, format!("unknown delegation status {status_value}"))
        })?,
        state_version,
        attempt_count,
        lease_owner: row.get(10)?,
        lease_expires_at_unix_ms: row.get(11)?,
        effect_state: AgentDelegationEffectState::parse(&effect_value).ok_or_else(|| {
            corrupt_column(
                12,
                format!("unknown delegation effect state {effect_value}"),
            )
        })?,
        result: row.get(13)?,
        result_truncated: row.get(14)?,
        used_tokens,
        error_code: row.get(16)?,
        error_message: row.get(17)?,
        cancel_requested_at_unix_ms: row.get(18)?,
        started_at_unix_ms: row.get(19)?,
        completed_at_unix_ms: row.get(20)?,
        created_at_unix_ms: row.get(21)?,
        updated_at_unix_ms: row.get(22)?,
    })
}

fn nonnegative_u64(row: &rusqlite::Row<'_>, column: usize, name: &str) -> rusqlite::Result<u64> {
    let value: i64 = row.get(column)?;
    u64::try_from(value).map_err(|_| corrupt_column(column, format!("negative {name}")))
}

fn nonnegative_u32(row: &rusqlite::Row<'_>, column: usize, name: &str) -> rusqlite::Result<u32> {
    let value = nonnegative_u64(row, column, name)?;
    u32::try_from(value).map_err(|_| corrupt_column(column, format!("oversized {name}")))
}

fn optional_nonnegative_u64(
    row: &rusqlite::Row<'_>,
    column: usize,
    name: &str,
) -> rusqlite::Result<Option<u64>> {
    let value: Option<i64> = row.get(column)?;
    value
        .map(|value| {
            u64::try_from(value).map_err(|_| corrupt_column(column, format!("negative {name}")))
        })
        .transpose()
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

fn validate_new_job(input: &NewAgentDelegationJob) -> CaptainResult<()> {
    validate_token("delegation job id", &input.id, 96)?;
    validate_token("delegation idempotency key", &input.idempotency_key, 192)?;
    validate_token("delegation root job id", &input.root_job_id, 96)?;
    if let Some(parent_job_id) = input.parent_job_id.as_deref() {
        validate_token("delegation parent job id", parent_job_id, 96)?;
    }
    if !(1..=AGENT_DELEGATION_MAX_DEPTH).contains(&input.depth) {
        return Err(CaptainError::InvalidInput(format!(
            "delegation depth must be between 1 and {AGENT_DELEGATION_MAX_DEPTH}"
        )));
    }
    match input.parent_job_id.as_deref() {
        None if input.root_job_id != input.id || input.depth != 1 => {
            return Err(CaptainError::InvalidInput(
                "a root delegation must use its own id as root_job_id and depth 1".to_string(),
            ));
        }
        Some(parent_job_id)
            if parent_job_id == input.id || input.root_job_id == input.id || input.depth <= 1 =>
        {
            return Err(CaptainError::InvalidInput(
                "a nested delegation must reference a different parent and root at depth 2 or greater"
                    .to_string(),
            ));
        }
        _ => {}
    }
    validate_token("caller agent id", &input.caller_agent_id, 96)?;
    validate_token("target agent id", &input.target_agent_id, 96)?;
    if input.caller_agent_id == input.target_agent_id {
        return Err(CaptainError::InvalidInput(
            "an agent cannot delegate a job to itself".to_string(),
        ));
    }
    validate_text("delegation title", &input.title, 1, 200)?;
    validate_text(
        "delegation task",
        &input.task,
        1,
        AGENT_DELEGATION_MAX_TASK_BYTES,
    )?;
    if !(1..=AGENT_DELEGATION_MAX_TOKENS).contains(&input.max_tokens) {
        return Err(CaptainError::InvalidInput(format!(
            "delegation max_tokens must be between 1 and {AGENT_DELEGATION_MAX_TOKENS}"
        )));
    }
    if input.depends_on.len() > AGENT_DELEGATION_MAX_DEPENDENCIES {
        return Err(CaptainError::InvalidInput(format!(
            "delegation supports at most {AGENT_DELEGATION_MAX_DEPENDENCIES} dependencies"
        )));
    }
    Ok(())
}

fn validate_and_reserve_lineage(
    tx: &Transaction<'_>,
    input: &NewAgentDelegationJob,
) -> CaptainResult<()> {
    let requested = to_i64("delegation max_tokens", input.max_tokens)?;
    let Some(parent_job_id) = input.parent_job_id.as_deref() else {
        let existing: bool = tx
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM agent_delegation_lineages WHERE root_job_id = ?1
                 )",
                params![input.root_job_id],
                |row| row.get(0),
            )
            .map_err(memory_error)?;
        if existing {
            return Err(CaptainError::InvalidInput(format!(
                "agent delegation lineage already exists: {}",
                input.root_job_id
            )));
        }
        tx.execute(
            "INSERT INTO agent_delegation_lineages (
                 root_job_id, reserved_tokens, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?3)",
            params![input.root_job_id, requested, input.created_at_unix_ms],
        )
        .map_err(memory_error)?;
        return Ok(());
    };

    let parent = job_by_id(tx, parent_job_id)?.ok_or_else(|| {
        CaptainError::InvalidInput(format!("delegation parent does not exist: {parent_job_id}"))
    })?;
    if parent.target_agent_id != input.caller_agent_id {
        return Err(CaptainError::AuthDenied(format!(
            "delegation parent {parent_job_id} did not run as the caller agent"
        )));
    }
    if parent.root_job_id != input.root_job_id {
        return Err(CaptainError::InvalidInput(format!(
            "delegation parent {parent_job_id} belongs to another lineage"
        )));
    }
    let expected_depth = parent
        .depth
        .checked_add(1)
        .ok_or_else(|| CaptainError::InvalidInput("delegation depth overflowed".to_string()))?;
    if input.depth != expected_depth || input.depth > AGENT_DELEGATION_MAX_DEPTH {
        return Err(CaptainError::InvalidInput(format!(
            "delegation depth {} does not follow parent depth {} or exceeds maximum {}",
            input.depth, parent.depth, AGENT_DELEGATION_MAX_DEPTH
        )));
    }
    if parent.status != AgentDelegationStatus::Running
        || parent.effect_state != AgentDelegationEffectState::Started
    {
        return Err(CaptainError::InvalidState {
            current: format!(
                "{} / {}",
                parent.status.as_str(),
                parent.effect_state.as_str()
            ),
            operation: "create a nested delegation from an actively running parent".to_string(),
        });
    }

    let reserved: i64 = tx
        .query_row(
            "SELECT reserved_tokens FROM agent_delegation_lineages
             WHERE root_job_id = ?1",
            params![input.root_job_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(memory_error)?
        .ok_or_else(|| {
            CaptainError::Internal(format!(
                "delegation lineage budget is missing: {}",
                input.root_job_id
            ))
        })?;
    let reserved = u64::try_from(reserved).map_err(|_| {
        CaptainError::Internal(format!(
            "delegation lineage budget is corrupt: {}",
            input.root_job_id
        ))
    })?;
    let next_reserved = reserved.checked_add(input.max_tokens).ok_or_else(|| {
        CaptainError::InvalidInput("delegation lineage token budget overflowed".to_string())
    })?;
    if next_reserved > AGENT_DELEGATION_MAX_LINEAGE_TOKENS {
        return Err(CaptainError::InvalidInput(format!(
            "delegation lineage token budget exceeded: {next_reserved} / {AGENT_DELEGATION_MAX_LINEAGE_TOKENS}"
        )));
    }
    tx.execute(
        "UPDATE agent_delegation_lineages
         SET reserved_tokens = ?1, updated_at = MAX(updated_at, ?2)
         WHERE root_job_id = ?3",
        params![
            to_i64("delegation lineage reserved tokens", next_reserved)?,
            input.created_at_unix_ms,
            input.root_job_id
        ],
    )
    .map_err(memory_error)?;
    Ok(())
}

fn normalized_dependencies(dependencies: &[String]) -> CaptainResult<Vec<String>> {
    let mut normalized = BTreeSet::new();
    for dependency in dependencies {
        validate_token("delegation dependency id", dependency, 96)?;
        if !normalized.insert(dependency.clone()) {
            return Err(CaptainError::InvalidInput(format!(
                "duplicate delegation dependency: {dependency}"
            )));
        }
    }
    Ok(normalized.into_iter().collect())
}

fn same_job_input(
    existing: &AgentDelegationJobRecord,
    input: &NewAgentDelegationJob,
    dependencies: &[String],
) -> bool {
    let same_root = if existing.parent_job_id.is_none() && input.parent_job_id.is_none() {
        existing.root_job_id == existing.id && input.root_job_id == input.id
    } else {
        existing.root_job_id == input.root_job_id
    };
    same_root
        && existing.parent_job_id == input.parent_job_id
        && existing.depth == input.depth
        && existing.caller_agent_id == input.caller_agent_id
        && existing.target_agent_id == input.target_agent_id
        && existing.title == input.title
        && existing.task == input.task
        && existing.max_tokens == input.max_tokens
        && existing.depends_on == dependencies
}

fn validate_worker(worker: &str, lease_duration_ms: i64) -> CaptainResult<()> {
    validate_token("delegation worker", worker, 96)?;
    if !(1_000..=3_600_000).contains(&lease_duration_ms) {
        return Err(CaptainError::InvalidInput(
            "delegation lease must be between 1 second and 1 hour".to_string(),
        ));
    }
    Ok(())
}

fn validate_token(label: &str, value: &str, max_bytes: usize) -> CaptainResult<()> {
    if value.is_empty()
        || value.len() > max_bytes
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
    {
        return Err(CaptainError::InvalidInput(format!(
            "{label} must be a non-empty token of at most {max_bytes} bytes"
        )));
    }
    Ok(())
}

fn validate_text(
    label: &str,
    value: &str,
    min_bytes: usize,
    max_bytes: usize,
) -> CaptainResult<()> {
    if value.len() < min_bytes || value.len() > max_bytes || value.contains('\0') {
        return Err(CaptainError::InvalidInput(format!(
            "{label} must contain {min_bytes}..={max_bytes} UTF-8 bytes without NUL"
        )));
    }
    Ok(())
}

fn bounded_result(result: &str) -> (String, bool) {
    let truncated = result.len() > AGENT_DELEGATION_MAX_RESULT_BYTES;
    (
        captain_types::truncate_str(result, AGENT_DELEGATION_MAX_RESULT_BYTES).to_string(),
        truncated,
    )
}

fn to_i64(label: &str, value: u64) -> CaptainResult<i64> {
    i64::try_from(value)
        .map_err(|_| CaptainError::InvalidInput(format!("{label} exceeds SQLite range")))
}

fn memory_error(error: rusqlite::Error) -> CaptainError {
    CaptainError::Memory(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn setup() -> AgentDelegationJobStore {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        AgentDelegationJobStore::new(Arc::new(Mutex::new(conn)))
    }

    fn new_job(
        id: &str,
        caller: &str,
        target: &str,
        dependencies: &[&str],
    ) -> NewAgentDelegationJob {
        NewAgentDelegationJob {
            id: id.to_string(),
            idempotency_key: format!("idem:{id}"),
            root_job_id: id.to_string(),
            parent_job_id: None,
            depth: 1,
            caller_agent_id: caller.to_string(),
            target_agent_id: target.to_string(),
            title: format!("Job {id}"),
            task: format!("Complete {id} with evidence"),
            max_tokens: 5_000,
            depends_on: dependencies
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            created_at_unix_ms: 1_000,
        }
    }

    fn nested_job(
        id: &str,
        root_job_id: &str,
        parent_job_id: &str,
        depth: u32,
        caller: &str,
        target: &str,
    ) -> NewAgentDelegationJob {
        let mut job = new_job(id, caller, target, &[]);
        job.root_job_id = root_job_id.to_string();
        job.parent_job_id = Some(parent_job_id.to_string());
        job.depth = depth;
        job
    }

    fn start_effect(store: &AgentDelegationJobStore, job_id: &str, worker: &str, now_unix_ms: i64) {
        let claimed = store
            .claim_ready(worker, now_unix_ms, 60_000)
            .unwrap()
            .expect("delegation should be ready");
        assert_eq!(claimed.id, job_id);
        store
            .mark_effect_started(job_id, worker, now_unix_ms + 1)
            .unwrap();
    }

    #[test]
    fn enqueue_is_exactly_idempotent_and_owner_scoped() {
        let store = setup();
        let job = new_job("job-a", "caller-a", "worker-a", &[]);
        let first = store.enqueue(&job).unwrap();
        let replay = store.enqueue(&job).unwrap();
        assert_eq!(first, replay);
        assert_eq!(first.status, AgentDelegationStatus::Queued);

        let mut regenerated = job.clone();
        regenerated.id = "new-candidate-id".to_string();
        regenerated.root_job_id = regenerated.id.clone();
        assert_eq!(store.enqueue(&regenerated).unwrap(), first);

        let mut changed = job.clone();
        changed.task = "different".to_string();
        assert!(store
            .enqueue(&changed)
            .unwrap_err()
            .to_string()
            .contains("idempotency"));
        assert!(store
            .get_for_caller("caller-b", "job-a")
            .unwrap_err()
            .to_string()
            .contains("another agent"));
    }

    #[test]
    fn nested_delegation_persists_lineage_depth_and_reserved_budget() {
        let store = setup();
        let root = store
            .enqueue(&new_job("lineage-root", "captain", "worker-a", &[]))
            .unwrap();
        assert_eq!(root.root_job_id, root.id);
        assert_eq!(root.parent_job_id, None);
        assert_eq!(root.depth, 1);
        assert_eq!(root.lineage_reserved_tokens, 5_000);
        start_effect(&store, "lineage-root", "scheduler-root", 2_000);

        let child = store
            .enqueue(&nested_job(
                "lineage-child",
                "lineage-root",
                "lineage-root",
                2,
                "worker-a",
                "worker-b",
            ))
            .unwrap();
        assert_eq!(child.root_job_id, "lineage-root");
        assert_eq!(child.parent_job_id.as_deref(), Some("lineage-root"));
        assert_eq!(child.depth, 2);
        assert_eq!(child.lineage_reserved_tokens, 10_000);
        assert_eq!(
            store
                .get("lineage-root")
                .unwrap()
                .unwrap()
                .lineage_reserved_tokens,
            10_000
        );
    }

    #[test]
    fn nested_delegation_requires_the_running_parent_target_as_caller() {
        let store = setup();
        store
            .enqueue(&new_job("lineage-root", "captain", "worker-a", &[]))
            .unwrap();

        let before_effect = store
            .enqueue(&nested_job(
                "too-early",
                "lineage-root",
                "lineage-root",
                2,
                "worker-a",
                "worker-b",
            ))
            .unwrap_err();
        assert!(before_effect
            .to_string()
            .contains("actively running parent"));

        start_effect(&store, "lineage-root", "scheduler-root", 2_000);
        let wrong_caller = store
            .enqueue(&nested_job(
                "wrong-caller",
                "lineage-root",
                "lineage-root",
                2,
                "intruder",
                "worker-b",
            ))
            .unwrap_err();
        assert!(wrong_caller
            .to_string()
            .contains("did not run as the caller agent"));
    }

    #[test]
    fn nested_delegation_enforces_depth_and_cumulative_lineage_budget() {
        let store = setup();
        let mut root = new_job("lineage-root", "captain", "worker-a", &[]);
        root.max_tokens = AGENT_DELEGATION_MAX_LINEAGE_TOKENS - 1_000;
        store.enqueue(&root).unwrap();
        start_effect(&store, "lineage-root", "scheduler-root", 2_000);

        let too_deep = nested_job(
            "too-deep",
            "lineage-root",
            "lineage-root",
            AGENT_DELEGATION_MAX_DEPTH + 1,
            "worker-a",
            "worker-b",
        );
        assert!(store
            .enqueue(&too_deep)
            .unwrap_err()
            .to_string()
            .contains("depth"));

        let mut over_budget = nested_job(
            "over-budget",
            "lineage-root",
            "lineage-root",
            2,
            "worker-a",
            "worker-b",
        );
        over_budget.max_tokens = 1_001;
        let error = store.enqueue(&over_budget).unwrap_err();
        assert!(error.to_string().contains("lineage token budget exceeded"));
        assert_eq!(
            store
                .get("lineage-root")
                .unwrap()
                .unwrap()
                .lineage_reserved_tokens,
            AGENT_DELEGATION_MAX_LINEAGE_TOKENS - 1_000
        );
    }

    #[test]
    fn nested_idempotency_replay_survives_parent_completion() {
        let store = setup();
        store
            .enqueue(&new_job("lineage-root", "captain", "worker-a", &[]))
            .unwrap();
        start_effect(&store, "lineage-root", "scheduler-root", 2_000);
        let child = nested_job(
            "lineage-child",
            "lineage-root",
            "lineage-root",
            2,
            "worker-a",
            "worker-b",
        );
        let first = store.enqueue(&child).unwrap();
        store
            .complete("lineage-root", "scheduler-root", "done", 50, 2_002)
            .unwrap();

        let mut replay = child;
        replay.id = "regenerated-child-id".to_string();
        assert_eq!(store.enqueue(&replay).unwrap(), first);
        assert_eq!(first.lineage_reserved_tokens, 10_000);
    }

    #[test]
    fn pruning_keeps_a_terminal_parent_while_its_lineage_child_exists() {
        let store = setup();
        store
            .enqueue(&new_job("lineage-root", "captain", "worker-a", &[]))
            .unwrap();
        start_effect(&store, "lineage-root", "scheduler-root", 2_000);

        store
            .enqueue(&new_job("child-gate", "worker-a", "gate-worker", &[]))
            .unwrap();
        start_effect(&store, "child-gate", "scheduler-gate", 2_002);

        let mut child = nested_job(
            "lineage-child",
            "lineage-root",
            "lineage-root",
            2,
            "worker-a",
            "worker-b",
        );
        child.depends_on = vec!["child-gate".to_string()];
        assert_eq!(
            store.enqueue(&child).unwrap().status,
            AgentDelegationStatus::Blocked
        );
        store
            .complete("lineage-root", "scheduler-root", "root done", 50, 2_004)
            .unwrap();

        for index in 0..101 {
            let id = format!("terminal-{index:03}");
            store
                .enqueue(&new_job(&id, "filler-caller", "filler-worker", &[]))
                .unwrap();
            start_effect(&store, &id, "scheduler-filler", 3_000 + index * 3);
            store
                .complete(&id, "scheduler-filler", "done", 1, 3_002 + index * 3)
                .unwrap();
        }

        assert_eq!(store.prune_terminal_history(100).unwrap(), 1);
        assert!(store.get("lineage-root").unwrap().is_some());
        assert!(store.get("lineage-child").unwrap().is_some());
    }

    #[test]
    fn dependencies_gate_execution_and_cascade_failure() {
        let store = setup();
        store
            .enqueue(&new_job("root-ok", "caller", "worker-a", &[]))
            .unwrap();
        let child = store
            .enqueue(&new_job("child-ok", "caller", "worker-b", &["root-ok"]))
            .unwrap();
        assert_eq!(child.status, AgentDelegationStatus::Blocked);

        let root = store
            .claim_ready("scheduler", 2_000, 60_000)
            .unwrap()
            .unwrap();
        assert_eq!(root.id, "root-ok");
        store
            .mark_effect_started("root-ok", "scheduler", 2_001)
            .unwrap();
        store
            .complete("root-ok", "scheduler", "evidence", 120, 2_002)
            .unwrap();
        let child = store.get("child-ok").unwrap().unwrap();
        assert_eq!(child.status, AgentDelegationStatus::Queued);

        store
            .enqueue(&new_job("root-fail", "caller", "worker-a", &[]))
            .unwrap();
        store
            .enqueue(&new_job("child-fail", "caller", "worker-b", &["root-fail"]))
            .unwrap();
        let root = store
            .claim_ready("scheduler", 3_000, 60_000)
            .unwrap()
            .unwrap();
        assert_eq!(root.id, "child-ok");
        store
            .mark_effect_started("child-ok", "scheduler", 3_001)
            .unwrap();
        store
            .complete("child-ok", "scheduler", "done", 20, 3_002)
            .unwrap();
        let root = store
            .claim_ready("scheduler", 3_003, 60_000)
            .unwrap()
            .unwrap();
        assert_eq!(root.id, "root-fail");
        store
            .mark_effect_started("root-fail", "scheduler", 3_004)
            .unwrap();
        store
            .fail_known(
                "root-fail",
                "scheduler",
                "worker_failed",
                "known failure",
                Some(42),
                3_005,
            )
            .unwrap();
        let child = store.get("child-fail").unwrap().unwrap();
        assert_eq!(child.status, AgentDelegationStatus::DependencyFailed);
    }

    #[test]
    fn restart_requeues_only_work_without_started_effect() {
        let store = setup();
        store
            .enqueue(&new_job("safe", "caller", "worker-a", &[]))
            .unwrap();
        store
            .enqueue(&new_job("ambiguous", "caller", "worker-b", &[]))
            .unwrap();
        let safe = store
            .claim_ready("old-boot", 2_000, 60_000)
            .unwrap()
            .unwrap();
        assert_eq!(safe.id, "ambiguous");
        let ambiguous = store
            .claim_ready("old-boot", 2_001, 60_000)
            .unwrap()
            .unwrap();
        assert_eq!(ambiguous.id, "safe");
        store
            .mark_effect_started("safe", "old-boot", 2_002)
            .unwrap();

        let recovery = store.reconcile_after_restart(3_000).unwrap();
        assert_eq!(recovery.requeued_without_effect, 1);
        assert_eq!(recovery.uncertain_after_effect, 1);
        assert_eq!(
            store.get("ambiguous").unwrap().unwrap().status,
            AgentDelegationStatus::Queued
        );
        assert_eq!(
            store.get("safe").unwrap().unwrap().status,
            AgentDelegationStatus::Uncertain
        );
        assert_eq!(
            store.resume("caller", "safe", 3_001).unwrap().status,
            AgentDelegationStatus::Queued
        );
    }

    #[test]
    fn cancel_before_effect_is_final_but_started_cancel_is_uncertain() {
        let store = setup();
        store
            .enqueue(&new_job("queued", "caller", "worker-a", &[]))
            .unwrap();
        let cancelled = store.request_cancel("caller", "queued", 2_000).unwrap();
        assert_eq!(cancelled.status, AgentDelegationStatus::Cancelled);

        store
            .enqueue(&new_job("running", "caller", "worker-a", &[]))
            .unwrap();
        store
            .claim_ready("scheduler", 3_000, 60_000)
            .unwrap()
            .unwrap();
        store
            .mark_effect_started("running", "scheduler", 3_001)
            .unwrap();
        assert_eq!(
            store
                .request_cancel("caller", "running", 3_002)
                .unwrap()
                .status,
            AgentDelegationStatus::CancelRequested
        );
        let settled = store
            .settle_cancel_request("running", "scheduler", 3_003)
            .unwrap();
        assert_eq!(settled.status, AgentDelegationStatus::Uncertain);
        assert!(settled.error_message.unwrap().contains("explicit resume"));
    }

    #[test]
    fn known_result_wins_a_late_cancel_race() {
        let store = setup();
        store
            .enqueue(&new_job("race", "caller", "worker-a", &[]))
            .unwrap();
        store
            .claim_ready("scheduler", 3_000, 60_000)
            .unwrap()
            .unwrap();
        store
            .mark_effect_started("race", "scheduler", 3_001)
            .unwrap();
        store.request_cancel("caller", "race", 3_002).unwrap();

        let completed = store
            .complete("race", "scheduler", "known result", 40, 3_003)
            .unwrap();
        assert_eq!(completed.status, AgentDelegationStatus::Succeeded);
        assert_eq!(completed.result.as_deref(), Some("known result"));
    }

    #[test]
    fn pre_effect_failure_is_terminal_and_unblocks_failure_dependents() {
        let store = setup();
        store
            .enqueue(&new_job("bad-input", "caller", "worker-a", &[]))
            .unwrap();
        store
            .enqueue(&new_job(
                "blocked-child",
                "caller",
                "worker-b",
                &["bad-input"],
            ))
            .unwrap();
        store
            .claim_ready("scheduler", 3_000, 60_000)
            .unwrap()
            .unwrap();

        let failed = store
            .fail_before_effect(
                "bad-input",
                "scheduler",
                "invalid_dependency_context",
                "dependency result is unavailable",
                3_001,
            )
            .unwrap();
        assert_eq!(failed.status, AgentDelegationStatus::Failed);
        assert_eq!(
            store.get("blocked-child").unwrap().unwrap().status,
            AgentDelegationStatus::DependencyFailed
        );
    }

    #[test]
    fn worker_interrupt_requeues_only_before_the_effect_boundary() {
        let store = setup();
        store
            .enqueue(&new_job("before-effect", "caller", "worker-a", &[]))
            .unwrap();
        store
            .claim_ready("scheduler", 3_000, 60_000)
            .unwrap()
            .unwrap();
        let requeued = store
            .interrupt_worker_job("before-effect", "scheduler", "worker panic", 3_001)
            .unwrap();
        assert_eq!(requeued.status, AgentDelegationStatus::Queued);

        let started_store = setup();
        started_store
            .enqueue(&new_job("after-effect", "caller", "worker-b", &[]))
            .unwrap();
        let claimed = started_store
            .claim_ready("scheduler", 4_000, 60_000)
            .unwrap()
            .unwrap();
        assert_eq!(claimed.id, "after-effect");
        started_store
            .mark_effect_started("after-effect", "scheduler", 4_001)
            .unwrap();
        let uncertain = started_store
            .interrupt_worker_job("after-effect", "scheduler", "worker panic", 4_002)
            .unwrap();
        assert_eq!(uncertain.status, AgentDelegationStatus::Uncertain);
        assert!(uncertain.error_message.unwrap().contains("explicit resume"));
    }

    #[test]
    fn dependencies_cannot_cross_caller_boundaries() {
        let store = setup();
        store
            .enqueue(&new_job("private", "caller-a", "worker-a", &[]))
            .unwrap();
        let error = store
            .enqueue(&new_job("intruder", "caller-b", "worker-b", &["private"]))
            .unwrap_err();
        assert!(error.to_string().contains("another agent"));
    }

    #[test]
    fn result_is_utf8_safe_and_bounded() {
        let store = setup();
        store
            .enqueue(&new_job("bounded", "caller", "worker", &[]))
            .unwrap();
        store
            .claim_ready("scheduler", 2_000, 60_000)
            .unwrap()
            .unwrap();
        store
            .mark_effect_started("bounded", "scheduler", 2_001)
            .unwrap();
        let result = format!("{}😀", "x".repeat(AGENT_DELEGATION_MAX_RESULT_BYTES));
        let completed = store
            .complete("bounded", "scheduler", &result, 50, 2_002)
            .unwrap();
        assert!(completed.result_truncated);
        assert!(completed.result.unwrap().len() <= AGENT_DELEGATION_MAX_RESULT_BYTES);
    }
}
