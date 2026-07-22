//! Atomic publication or rejection of one captured workflow refinement.

use rusqlite::{params, types::Type, OptionalExtension, Transaction, TransactionBehavior};

use crate::workflow_learning_control::{
    proposal_by_id, publish_validated_draft_in_tx, transition_in_tx, validate_transition,
    WorkflowLearningControlError, WorkflowLearningStore, WorkflowProposalRecord,
    WorkflowProposalState, WorkflowProposalTransition,
};
use crate::workflow_learning_outbox::{insert_outbox_in_tx, WorkflowOutboxRecord};
use crate::workflow_learning_pipeline::{publish_is_fresh, WorkflowPipelineRejection};
use crate::workflow_learning_pipeline_types::WorkflowValidationCompletion;
use crate::workflow_learning_pipeline_validation::{
    require_pipeline_job_identity, validate_pipeline_rejection, validate_validation_completion,
};
use crate::workflow_learning_queue::{
    complete_job_in_tx, WorkflowJobEffectState, WorkflowJobKind, WorkflowJobRecord,
};
use crate::workflow_learning_refinement::{
    insert_event, refinement_by_id, WorkflowRefinementEvent, WorkflowRefinementRecord,
    WorkflowRefinementState,
};
use crate::workflow_learning_validation::{validate_text, validate_token};

#[derive(Debug, Clone)]
pub struct WorkflowRefinementValidationCompletion {
    pub request_id: String,
    pub expected_request_version: u64,
    pub parent_transition: WorkflowProposalTransition,
    pub validation: WorkflowValidationCompletion,
    pub actor: String,
    pub idempotency_key: String,
    pub completed_at_unix_ms: i64,
}

#[derive(Debug, Clone)]
pub struct WorkflowRefinementRejection {
    pub request_id: String,
    pub expected_request_version: u64,
    pub rejection: WorkflowPipelineRejection,
    pub actor: String,
    pub reason: String,
    pub idempotency_key: String,
    pub completed_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRefinementLifecycleResult {
    pub request: WorkflowRefinementRecord,
    pub parent_proposal: WorkflowProposalRecord,
    pub child_proposal: WorkflowProposalRecord,
    pub job: WorkflowJobRecord,
    pub notification: Option<WorkflowOutboxRecord>,
}

impl WorkflowLearningStore {
    pub fn complete_refinement_validation(
        &self,
        input: &WorkflowRefinementValidationCompletion,
    ) -> Result<WorkflowRefinementLifecycleResult, WorkflowLearningControlError> {
        validate_completion(input)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let request = refinement_by_id(&tx, &input.request_id)?
            .ok_or_else(|| WorkflowLearningControlError::NotFound(input.request_id.clone()))?;
        let fresh = request_is_fresh(
            &request,
            input.expected_request_version,
            WorkflowRefinementState::Queued,
        );
        verify_completion_links(&request, input)?;
        require_request_state(
            &tx,
            &request,
            fresh,
            WorkflowRefinementState::Completed,
            None,
            &input.idempotency_key,
            &input.actor,
            "refinement validated and parent superseded",
            input.completed_at_unix_ms,
        )?;

        let child_fresh = publish_is_fresh(&tx, &input.validation)?;
        require_pipeline_job_identity(
            &tx,
            &input.validation.job_id,
            &input.validation.publish.proposal_id,
            WorkflowJobKind::Validate,
            WorkflowJobEffectState::None,
            child_fresh,
        )?;
        let child = publish_validated_draft_in_tx(&tx, &input.validation.publish)?;
        let parent = transition_in_tx(&tx, &input.parent_transition)?;
        let job = complete_job_in_tx(
            &tx,
            &input.validation.job_id,
            &input.validation.worker,
            input.validation.result_json.as_deref(),
            input.validation.completed_at_unix_ms,
        )?;
        let notification = input
            .validation
            .notification
            .as_ref()
            .map(|item| insert_outbox_in_tx(&tx, item))
            .transpose()?;
        let request = finish_request(
            &tx,
            &request,
            fresh,
            WorkflowRefinementState::Completed,
            None,
            &input.idempotency_key,
            &input.actor,
            "refinement validated and parent superseded",
            input.completed_at_unix_ms,
        )?;
        tx.commit()?;
        Ok(WorkflowRefinementLifecycleResult {
            request,
            parent_proposal: parent,
            child_proposal: child,
            job,
            notification,
        })
    }

    pub fn reject_refinement(
        &self,
        input: &WorkflowRefinementRejection,
    ) -> Result<WorkflowRefinementLifecycleResult, WorkflowLearningControlError> {
        validate_rejection(input)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let request = refinement_by_id(&tx, &input.request_id)?
            .ok_or_else(|| WorkflowLearningControlError::NotFound(input.request_id.clone()))?;
        let fresh = request_is_fresh(
            &request,
            input.expected_request_version,
            WorkflowRefinementState::Queued,
        );
        verify_rejection_links(&request, input)?;
        require_request_state(
            &tx,
            &request,
            fresh,
            WorkflowRefinementState::Failed,
            Some(&input.reason),
            &input.idempotency_key,
            &input.actor,
            &input.reason,
            input.completed_at_unix_ms,
        )?;

        let parent = proposal_by_id(&tx, &request.proposal_id)?
            .ok_or_else(|| WorkflowLearningControlError::NotFound(request.proposal_id.clone()))?;
        if parent.state != WorkflowProposalState::Proposed
            || parent.state_version != request.expected_proposal_version
            || parent.revision_sha256.as_deref() != Some(request.revision_sha256.as_str())
        {
            return Err(WorkflowLearningControlError::Conflict(
                "refinement rejection requires its parent revision to remain proposed".to_string(),
            ));
        }

        let child_before = proposal_by_id(&tx, &input.rejection.proposal_transition.proposal_id)?
            .ok_or_else(|| {
            WorkflowLearningControlError::NotFound(
                input.rejection.proposal_transition.proposal_id.clone(),
            )
        })?;
        let child_fresh = child_before.state == input.rejection.proposal_transition.expected_state
            && child_before.state_version == input.rejection.proposal_transition.expected_version
            && child_before.revision_sha256
                == input.rejection.proposal_transition.expected_revision_sha256;
        let expected_effect = if input.rejection.job_kind == WorkflowJobKind::Draft {
            WorkflowJobEffectState::Started
        } else {
            WorkflowJobEffectState::None
        };
        require_pipeline_job_identity(
            &tx,
            &input.rejection.job_id,
            &input.rejection.proposal_transition.proposal_id,
            input.rejection.job_kind,
            expected_effect,
            child_fresh,
        )?;
        let child = transition_in_tx(&tx, &input.rejection.proposal_transition)?;
        let job = complete_job_in_tx(
            &tx,
            &input.rejection.job_id,
            &input.rejection.worker,
            input.rejection.result_json.as_deref(),
            input.rejection.completed_at_unix_ms,
        )?;
        let notification = input
            .rejection
            .notification
            .as_ref()
            .map(|item| insert_outbox_in_tx(&tx, item))
            .transpose()?;
        let request = finish_request(
            &tx,
            &request,
            fresh,
            WorkflowRefinementState::Failed,
            Some(&input.reason),
            &input.idempotency_key,
            &input.actor,
            &input.reason,
            input.completed_at_unix_ms,
        )?;
        tx.commit()?;
        Ok(WorkflowRefinementLifecycleResult {
            request,
            parent_proposal: parent,
            child_proposal: child,
            job,
            notification,
        })
    }
}

fn validate_completion(
    input: &WorkflowRefinementValidationCompletion,
) -> Result<(), WorkflowLearningControlError> {
    validate_token("refinement request id", &input.request_id, 96)?;
    validate_token("refinement lifecycle actor", &input.actor, 128)?;
    validate_token(
        "refinement lifecycle idempotency_key",
        &input.idempotency_key,
        192,
    )?;
    validate_transition(&input.parent_transition)?;
    validate_validation_completion(&input.validation)?;
    if input.parent_transition.expected_state != WorkflowProposalState::Proposed
        || input.parent_transition.to_state != WorkflowProposalState::Superseded
        || input.parent_transition.expected_revision_sha256.is_none()
        || input.parent_transition.snoozed_until_unix_ms.is_some()
        || input.parent_transition.actor != input.actor
        || input.validation.worker != input.actor
        || input.validation.publish.actor != input.actor
        || input.parent_transition.occurred_at_unix_ms != input.completed_at_unix_ms
        || input.validation.publish.occurred_at_unix_ms != input.completed_at_unix_ms
        || input.validation.completed_at_unix_ms != input.completed_at_unix_ms
    {
        return Err(WorkflowLearningControlError::InvalidInput(
            "refinement completion requires exact proposed -> superseded parent and one validation time"
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_rejection(
    input: &WorkflowRefinementRejection,
) -> Result<(), WorkflowLearningControlError> {
    validate_token("refinement request id", &input.request_id, 96)?;
    validate_token("refinement lifecycle actor", &input.actor, 128)?;
    validate_token(
        "refinement lifecycle idempotency_key",
        &input.idempotency_key,
        192,
    )?;
    validate_text("refinement failure reason", &input.reason, 1, 2_048)?;
    validate_pipeline_rejection(&input.rejection)?;
    if input.rejection.worker != input.actor
        || input.rejection.proposal_transition.actor != input.actor
        || input.rejection.proposal_transition.occurred_at_unix_ms != input.completed_at_unix_ms
        || input.rejection.completed_at_unix_ms != input.completed_at_unix_ms
    {
        return Err(WorkflowLearningControlError::InvalidInput(
            "refinement rejection requires one actor and one completion time".to_string(),
        ));
    }
    Ok(())
}

fn verify_completion_links(
    request: &WorkflowRefinementRecord,
    input: &WorkflowRefinementValidationCompletion,
) -> Result<(), WorkflowLearningControlError> {
    if request.proposal_id != input.parent_transition.proposal_id
        || request.revision_sha256
            != input
                .parent_transition
                .expected_revision_sha256
                .as_deref()
                .unwrap_or_default()
        || request.expected_proposal_version != input.parent_transition.expected_version
        || request.child_proposal_id.as_deref()
            != Some(input.validation.publish.proposal_id.as_str())
    {
        return Err(WorkflowLearningControlError::Conflict(
            "refinement completion does not match parent and child identities".to_string(),
        ));
    }
    Ok(())
}

fn verify_rejection_links(
    request: &WorkflowRefinementRecord,
    input: &WorkflowRefinementRejection,
) -> Result<(), WorkflowLearningControlError> {
    if request.child_proposal_id.as_deref()
        != Some(input.rejection.proposal_transition.proposal_id.as_str())
    {
        return Err(WorkflowLearningControlError::Conflict(
            "refinement rejection does not match its child proposal".to_string(),
        ));
    }
    Ok(())
}

fn request_is_fresh(
    request: &WorkflowRefinementRecord,
    expected_version: u64,
    expected_state: WorkflowRefinementState,
) -> bool {
    request.state == expected_state && request.state_version == expected_version
}

#[allow(clippy::too_many_arguments)]
fn require_request_state(
    tx: &Transaction<'_>,
    request: &WorkflowRefinementRecord,
    fresh: bool,
    terminal: WorkflowRefinementState,
    expected_last_error: Option<&str>,
    idempotency_key: &str,
    actor: &str,
    reason: &str,
    completed_at_unix_ms: i64,
) -> Result<(), WorkflowLearningControlError> {
    if fresh {
        return Ok(());
    }
    let event = request_event_by_idempotency(tx, idempotency_key)?.ok_or_else(|| {
        WorkflowLearningControlError::Conflict(
            "refinement lifecycle request is stale without a matching event".to_string(),
        )
    })?;
    if request.state != terminal
        || request.last_error.as_deref() != expected_last_error
        || request.updated_at_unix_ms != completed_at_unix_ms
        || event.request_id != request.id
        || event.from_state != Some(WorkflowRefinementState::Queued)
        || event.to_state != terminal
        || event.resulting_version != request.state_version
        || event.actor != actor
        || event.reason != reason
        || event.created_at_unix_ms != completed_at_unix_ms
    {
        return Err(WorkflowLearningControlError::Conflict(
            "refinement lifecycle replay differs from the committed result".to_string(),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn finish_request(
    tx: &Transaction<'_>,
    request: &WorkflowRefinementRecord,
    fresh: bool,
    terminal: WorkflowRefinementState,
    last_error: Option<&str>,
    idempotency_key: &str,
    actor: &str,
    reason: &str,
    completed_at_unix_ms: i64,
) -> Result<WorkflowRefinementRecord, WorkflowLearningControlError> {
    if fresh {
        let next_version = request.state_version.saturating_add(1);
        let changed = tx.execute(
            "UPDATE workflow_learning_refinements
             SET state = ?1, state_version = ?2, last_error = ?3, updated_at = ?4
             WHERE id = ?5 AND state = 'queued' AND state_version = ?6",
            params![
                terminal.as_str(),
                next_version as i64,
                last_error,
                completed_at_unix_ms,
                request.id,
                request.state_version as i64,
            ],
        )?;
        if changed != 1 {
            return Err(WorkflowLearningControlError::Conflict(
                "refinement lifecycle changed concurrently".to_string(),
            ));
        }
        insert_event(
            tx,
            idempotency_key,
            &request.id,
            Some(WorkflowRefinementState::Queued),
            terminal,
            next_version,
            actor,
            reason,
            completed_at_unix_ms,
        )?;
    }
    refinement_by_id(tx, &request.id)?.ok_or_else(|| {
        WorkflowLearningControlError::CorruptData(
            "completed refinement request vanished".to_string(),
        )
    })
}

fn request_event_by_idempotency(
    conn: &rusqlite::Connection,
    idempotency_key: &str,
) -> rusqlite::Result<Option<WorkflowRefinementEvent>> {
    conn.query_row(
        "SELECT sequence, idempotency_key, request_id, from_state, to_state,
                resulting_version, actor, reason, created_at
         FROM workflow_learning_refinement_events WHERE idempotency_key = ?1",
        params![idempotency_key],
        |row| {
            let from_value: Option<String> = row.get(3)?;
            let to_value: String = row.get(4)?;
            Ok(WorkflowRefinementEvent {
                sequence: row.get::<_, i64>(0)?.max(0) as u64,
                idempotency_key: row.get(1)?,
                request_id: row.get(2)?,
                from_state: from_value
                    .map(|value| parse_state_column(3, &value))
                    .transpose()?,
                to_state: parse_state_column(4, &to_value)?,
                resulting_version: row.get::<_, i64>(5)?.max(0) as u64,
                actor: row.get(6)?,
                reason: row.get(7)?,
                created_at_unix_ms: row.get(8)?,
            })
        },
    )
    .optional()
}

fn parse_state_column(column: usize, value: &str) -> rusqlite::Result<WorkflowRefinementState> {
    WorkflowRefinementState::parse(value).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown refinement state {value}"),
            )),
        )
    })
}
