//! Durable isolated-test lifecycle for one exact workflow-learning revision.
//!
//! A test is deliberately not an installation. The proposal is locked while
//! the leased job runs, but no installation mirror or active registry state is
//! created. Completion atomically stores the report, returns the proposal to
//! operator review, completes the job, and enqueues its lifecycle notice.

use rusqlite::{params, types::Type, OptionalExtension, Transaction, TransactionBehavior};

use crate::workflow_learning_control::{
    proposal_by_id, transition_in_tx, validate_transition, WorkflowLearningControlError,
    WorkflowLearningStore, WorkflowProposalRecord, WorkflowProposalState,
    WorkflowProposalTransition,
};
use crate::workflow_learning_outbox::{
    insert_outbox_in_tx, NewWorkflowOutboxItem, WorkflowOutboxRecord,
};
use crate::workflow_learning_queue::{
    complete_job_in_tx, insert_job_in_tx, job_by_id, NewWorkflowJob, WorkflowJobKind,
    WorkflowJobRecord,
};
pub use crate::workflow_learning_types::{
    NewWorkflowIsolatedTest, WorkflowIsolatedTestRecord, WorkflowIsolatedTestStatus,
};
use crate::workflow_learning_validation::{validate_hash, validate_json, validate_token};

#[derive(Debug, Clone)]
pub struct WorkflowIsolatedTestCompletion {
    pub job_id: String,
    pub worker: String,
    pub passed: bool,
    pub result_json: String,
    pub proposal_transition: WorkflowProposalTransition,
    pub notification: Option<NewWorkflowOutboxItem>,
    pub completed_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowIsolatedTestLifecycleResult {
    pub proposal: WorkflowProposalRecord,
    pub job: WorkflowJobRecord,
    pub isolated_test: WorkflowIsolatedTestRecord,
    pub notification: Option<WorkflowOutboxRecord>,
}

impl WorkflowLearningStore {
    pub fn approve_and_enqueue_isolated_test(
        &self,
        transition: &WorkflowProposalTransition,
        job: &NewWorkflowJob,
        isolated_test: &NewWorkflowIsolatedTest,
    ) -> Result<WorkflowIsolatedTestLifecycleResult, WorkflowLearningControlError> {
        validate_transition(transition)?;
        validate_new_test(isolated_test)?;
        if transition.expected_state != WorkflowProposalState::Proposed
            || transition.to_state != WorkflowProposalState::ApprovedPendingInstall
            || job.kind != WorkflowJobKind::Install
            || job.proposal_id != transition.proposal_id
            || job.revision_sha256 != transition.expected_revision_sha256
            || isolated_test.proposal_id != transition.proposal_id
            || Some(isolated_test.revision_sha256.as_str())
                != transition.expected_revision_sha256.as_deref()
            || isolated_test.job_id != job.id
            || isolated_test.requested_by != transition.actor
            || isolated_test.requested_at_unix_ms != transition.occurred_at_unix_ms
        {
            return Err(WorkflowLearningControlError::InvalidInput(
                "isolated test approval identities do not match".to_string(),
            ));
        }

        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let proposal = transition_in_tx(&tx, transition)?;
        let job = insert_job_in_tx(&tx, job)?;
        let isolated_test = insert_test_in_tx(&tx, isolated_test)?;
        tx.commit()?;
        Ok(WorkflowIsolatedTestLifecycleResult {
            proposal,
            job,
            isolated_test,
            notification: None,
        })
    }

    pub fn complete_isolated_test(
        &self,
        request: &WorkflowIsolatedTestCompletion,
    ) -> Result<WorkflowIsolatedTestLifecycleResult, WorkflowLearningControlError> {
        validate_transition(&request.proposal_transition)?;
        validate_token("isolated test job id", &request.job_id, 96)?;
        validate_token("isolated test worker", &request.worker, 96)?;
        validate_json("isolated test result_json", &request.result_json, 64 * 1024)?;
        if request.proposal_transition.expected_state
            != WorkflowProposalState::ApprovedPendingInstall
            || request.proposal_transition.to_state != WorkflowProposalState::Proposed
        {
            return Err(WorkflowLearningControlError::InvalidInput(
                "isolated test completion must return the locked proposal to review".to_string(),
            ));
        }
        let revision = request
            .proposal_transition
            .expected_revision_sha256
            .as_deref()
            .ok_or_else(|| {
                WorkflowLearningControlError::InvalidInput(
                    "isolated test completion requires a revision".to_string(),
                )
            })?;
        if let Some(notification) = &request.notification {
            if notification.proposal_id != request.proposal_transition.proposal_id
                || notification.revision_sha256.as_deref() != Some(revision)
            {
                return Err(WorkflowLearningControlError::InvalidInput(
                    "isolated test notification identifies another revision".to_string(),
                ));
            }
        }

        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let test = test_by_job_id(&tx, &request.job_id)?
            .ok_or_else(|| WorkflowLearningControlError::NotFound(request.job_id.clone()))?;
        if test.proposal_id != request.proposal_transition.proposal_id
            || test.revision_sha256 != revision
        {
            return Err(WorkflowLearningControlError::Conflict(
                "isolated test job belongs to another proposal revision".to_string(),
            ));
        }
        let job = job_by_id(&tx, &request.job_id)?
            .ok_or_else(|| WorkflowLearningControlError::NotFound(request.job_id.clone()))?;
        if job.kind != WorkflowJobKind::Install
            || job.proposal_id != test.proposal_id
            || job.revision_sha256.as_deref() != Some(test.revision_sha256.as_str())
        {
            return Err(WorkflowLearningControlError::Conflict(
                "isolated test job identity is inconsistent".to_string(),
            ));
        }

        transition_in_tx(&tx, &request.proposal_transition)?;
        let job = complete_job_in_tx(
            &tx,
            &request.job_id,
            &request.worker,
            Some(&request.result_json),
            request.completed_at_unix_ms,
        )?;
        let isolated_test = complete_test_in_tx(&tx, request)?;
        let notification = request
            .notification
            .as_ref()
            .map(|item| insert_outbox_in_tx(&tx, item))
            .transpose()?;
        let proposal =
            proposal_by_id(&tx, &request.proposal_transition.proposal_id)?.ok_or_else(|| {
                WorkflowLearningControlError::NotFound(
                    request.proposal_transition.proposal_id.clone(),
                )
            })?;
        tx.commit()?;
        Ok(WorkflowIsolatedTestLifecycleResult {
            proposal,
            job,
            isolated_test,
            notification,
        })
    }

    pub fn isolated_test_by_job_id(
        &self,
        job_id: &str,
    ) -> Result<Option<WorkflowIsolatedTestRecord>, WorkflowLearningControlError> {
        validate_token("isolated test job id", job_id, 96)?;
        let conn = self.lock_conn()?;
        test_by_job_id(&conn, job_id).map_err(Into::into)
    }
}

fn insert_test_in_tx(
    tx: &Transaction<'_>,
    input: &NewWorkflowIsolatedTest,
) -> Result<WorkflowIsolatedTestRecord, WorkflowLearningControlError> {
    validate_new_test(input)?;
    if let Some(existing) = test_by_idempotency(tx, &input.idempotency_key)? {
        if existing.id == input.id
            && existing.proposal_id == input.proposal_id
            && existing.revision_sha256 == input.revision_sha256
            && existing.job_id == input.job_id
            && existing.requested_by == input.requested_by
            && existing.requested_at_unix_ms == input.requested_at_unix_ms
        {
            return Ok(existing);
        }
        return Err(WorkflowLearningControlError::Conflict(
            "isolated test idempotency key was reused".to_string(),
        ));
    }
    let proposal = proposal_by_id(tx, &input.proposal_id)?
        .ok_or_else(|| WorkflowLearningControlError::NotFound(input.proposal_id.clone()))?;
    if proposal.state != WorkflowProposalState::ApprovedPendingInstall
        || proposal.revision_sha256.as_deref() != Some(input.revision_sha256.as_str())
    {
        return Err(WorkflowLearningControlError::Conflict(
            "isolated test does not match the locked proposal revision".to_string(),
        ));
    }
    tx.execute(
        "INSERT INTO workflow_learning_tests (
             id, idempotency_key, proposal_id, revision_sha256, job_id,
             requested_by, requested_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![
            input.id,
            input.idempotency_key,
            input.proposal_id,
            input.revision_sha256,
            input.job_id,
            input.requested_by,
            input.requested_at_unix_ms,
        ],
    )?;
    test_by_job_id(tx, &input.job_id)?
        .ok_or_else(|| WorkflowLearningControlError::NotFound(input.id.clone()))
}

fn complete_test_in_tx(
    tx: &Transaction<'_>,
    request: &WorkflowIsolatedTestCompletion,
) -> Result<WorkflowIsolatedTestRecord, WorkflowLearningControlError> {
    let expected_status = if request.passed {
        WorkflowIsolatedTestStatus::Passed
    } else {
        WorkflowIsolatedTestStatus::Failed
    };
    let current = test_by_job_id(tx, &request.job_id)?
        .ok_or_else(|| WorkflowLearningControlError::NotFound(request.job_id.clone()))?;
    if current.status == expected_status
        && current.result_json.as_deref() == Some(request.result_json.as_str())
        && current.completed_at_unix_ms == Some(request.completed_at_unix_ms)
    {
        return Ok(current);
    }
    let changed = tx.execute(
        "UPDATE workflow_learning_tests
         SET status = ?1, result_json = ?2, completed_at = ?3, updated_at = ?3
         WHERE job_id = ?4 AND status = 'queued' AND result_json IS NULL
           AND completed_at IS NULL",
        params![
            expected_status.as_str(),
            request.result_json,
            request.completed_at_unix_ms,
            request.job_id,
        ],
    )?;
    if changed != 1 {
        return Err(WorkflowLearningControlError::Conflict(
            "isolated test completion changed concurrently".to_string(),
        ));
    }
    test_by_job_id(tx, &request.job_id)?
        .ok_or_else(|| WorkflowLearningControlError::NotFound(request.job_id.clone()))
}

fn validate_new_test(input: &NewWorkflowIsolatedTest) -> Result<(), WorkflowLearningControlError> {
    validate_token("isolated test id", &input.id, 96)?;
    validate_token("isolated test idempotency_key", &input.idempotency_key, 192)?;
    validate_token("isolated test proposal_id", &input.proposal_id, 96)?;
    validate_hash("isolated test revision_sha256", &input.revision_sha256)?;
    validate_token("isolated test job_id", &input.job_id, 96)?;
    validate_token("isolated test requested_by", &input.requested_by, 128)?;
    Ok(())
}

const TEST_SELECT: &str =
    "SELECT id, idempotency_key, proposal_id, revision_sha256, job_id, status,
            requested_by, result_json, requested_at, completed_at, updated_at
     FROM workflow_learning_tests";

fn test_by_job_id(
    conn: &rusqlite::Connection,
    job_id: &str,
) -> rusqlite::Result<Option<WorkflowIsolatedTestRecord>> {
    conn.query_row(
        &format!("{TEST_SELECT} WHERE job_id = ?1"),
        params![job_id],
        test_from_row,
    )
    .optional()
}

fn test_by_idempotency(
    conn: &rusqlite::Connection,
    idempotency_key: &str,
) -> rusqlite::Result<Option<WorkflowIsolatedTestRecord>> {
    conn.query_row(
        &format!("{TEST_SELECT} WHERE idempotency_key = ?1"),
        params![idempotency_key],
        test_from_row,
    )
    .optional()
}

fn test_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowIsolatedTestRecord> {
    let status_value: String = row.get(5)?;
    let status = WorkflowIsolatedTestStatus::parse(&status_value).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            5,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown isolated test status {status_value}"),
            )),
        )
    })?;
    Ok(WorkflowIsolatedTestRecord {
        id: row.get(0)?,
        idempotency_key: row.get(1)?,
        proposal_id: row.get(2)?,
        revision_sha256: row.get(3)?,
        job_id: row.get(4)?,
        status,
        requested_by: row.get(6)?,
        result_json: row.get(7)?,
        requested_at_unix_ms: row.get(8)?,
        completed_at_unix_ms: row.get(9)?,
        updated_at_unix_ms: row.get(10)?,
    })
}
