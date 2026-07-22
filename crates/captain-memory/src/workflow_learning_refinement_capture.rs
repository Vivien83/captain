//! Atomic capture of one refinement instruction and its child draft job.

use rusqlite::{params, types::Type, OptionalExtension, TransactionBehavior};

use crate::workflow_learning_control::{
    create_observed_in_tx, proposal_by_id, transition_in_tx, validate_transition,
    NewWorkflowProposal, WorkflowLearningControlError, WorkflowLearningStore,
    WorkflowProposalRecord, WorkflowProposalState, WorkflowProposalTransition,
};
use crate::workflow_learning_queue::{
    insert_job_in_tx, job_by_id, NewWorkflowJob, WorkflowJobKind, WorkflowJobRecord,
};
use crate::workflow_learning_refinement::{
    expire_due_in_tx, insert_event, refinement_by_id, WorkflowRefinementEvent,
    WorkflowRefinementRecord, WorkflowRefinementState,
};
use crate::workflow_learning_validation::{validate_text, validate_token};

#[derive(Debug, Clone)]
pub struct CaptureWorkflowRefinement {
    pub request_id: String,
    pub expected_request_version: u64,
    pub actor: String,
    pub instruction: String,
    pub captured_message_id: String,
    pub child_proposal: NewWorkflowProposal,
    pub eligible_transition: WorkflowProposalTransition,
    pub drafting_transition: WorkflowProposalTransition,
    pub draft_job: NewWorkflowJob,
    pub idempotency_key: String,
    pub captured_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRefinementCaptureResult {
    pub request: WorkflowRefinementRecord,
    pub child_proposal: WorkflowProposalRecord,
    pub draft_job: WorkflowJobRecord,
}

impl WorkflowLearningStore {
    pub fn capture_refinement_and_enqueue_draft(
        &self,
        input: &CaptureWorkflowRefinement,
    ) -> Result<WorkflowRefinementCaptureResult, WorkflowLearningControlError> {
        validate_capture(input)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire_due_in_tx(&tx, input.captured_at_unix_ms)?;
        let current = refinement_by_id(&tx, &input.request_id)?
            .ok_or_else(|| WorkflowLearningControlError::NotFound(input.request_id.clone()))?;

        if current.actor != input.actor
            || input.eligible_transition.actor != input.actor
            || input.drafting_transition.actor != input.actor
            || input.child_proposal.origin_channel.as_deref() != Some(current.surface.as_str())
        {
            return Err(WorkflowLearningControlError::Conflict(
                "refinement capture actor or surface does not match its binding".to_string(),
            ));
        }

        if matches!(
            current.state,
            WorkflowRefinementState::Queued
                | WorkflowRefinementState::Completed
                | WorkflowRefinementState::Failed
        ) {
            let result = replay_result(&tx, input, current)?;
            tx.commit()?;
            return Ok(result);
        }
        if current.state != WorkflowRefinementState::AwaitingInput
            || current.state_version != input.expected_request_version
        {
            return Err(WorkflowLearningControlError::Conflict(format!(
                "refinement request is {}, version {}",
                current.state.as_str(),
                current.state_version
            )));
        }
        let parent = proposal_by_id(&tx, &current.proposal_id)?
            .ok_or_else(|| WorkflowLearningControlError::NotFound(current.proposal_id.clone()))?;
        if parent.state != WorkflowProposalState::Proposed
            || parent.state_version != current.expected_proposal_version
            || parent.revision_sha256.as_deref() != Some(current.revision_sha256.as_str())
        {
            return Err(WorkflowLearningControlError::Conflict(
                "proposal changed before refinement input was captured".to_string(),
            ));
        }
        if input.child_proposal.id == parent.id
            || input.child_proposal.workflow_signature == parent.workflow_signature
            || input.child_proposal.source_agent_id != parent.source_agent_id
        {
            return Err(WorkflowLearningControlError::InvalidInput(
                "refinement child must have a new identity and preserve its source agent"
                    .to_string(),
            ));
        }

        create_observed_in_tx(&tx, &input.child_proposal)?;
        transition_in_tx(&tx, &input.eligible_transition)?;
        let child = transition_in_tx(&tx, &input.drafting_transition)?;
        let job = insert_job_in_tx(&tx, &input.draft_job)?;
        let next_version = current.state_version.saturating_add(1);
        let changed = tx.execute(
            "UPDATE workflow_learning_refinements
             SET state = 'queued', state_version = ?1, instruction = ?2,
                 captured_message_id = ?3, child_proposal_id = ?4,
                 draft_job_id = ?5, updated_at = ?6
             WHERE id = ?7 AND state = 'awaiting_input' AND state_version = ?8",
            params![
                next_version as i64,
                input.instruction,
                input.captured_message_id,
                input.child_proposal.id,
                input.draft_job.id,
                input.captured_at_unix_ms,
                input.request_id,
                current.state_version as i64,
            ],
        )?;
        if changed != 1 {
            return Err(WorkflowLearningControlError::Conflict(
                "refinement request changed concurrently".to_string(),
            ));
        }
        insert_event(
            &tx,
            &input.idempotency_key,
            &input.request_id,
            Some(WorkflowRefinementState::AwaitingInput),
            WorkflowRefinementState::Queued,
            next_version,
            &input.actor,
            "operator refinement input captured and drafting queued",
            input.captured_at_unix_ms,
        )?;
        let request = refinement_by_id(&tx, &input.request_id)?.ok_or_else(|| {
            WorkflowLearningControlError::CorruptData(
                "captured refinement request vanished".to_string(),
            )
        })?;
        tx.commit()?;
        Ok(WorkflowRefinementCaptureResult {
            request,
            child_proposal: child,
            draft_job: job,
        })
    }
}

fn replay_result(
    tx: &rusqlite::Transaction<'_>,
    input: &CaptureWorkflowRefinement,
    request: WorkflowRefinementRecord,
) -> Result<WorkflowRefinementCaptureResult, WorkflowLearningControlError> {
    let event = refinement_event_by_idempotency(tx, &input.idempotency_key)?.ok_or_else(|| {
        WorkflowLearningControlError::Conflict(
            "queued refinement has no matching capture event".to_string(),
        )
    })?;
    if event.request_id != input.request_id
        || event.from_state != Some(WorkflowRefinementState::AwaitingInput)
        || event.to_state != WorkflowRefinementState::Queued
        || event.resulting_version != input.expected_request_version.saturating_add(1)
        || event.actor != input.actor
        || event.reason != "operator refinement input captured and drafting queued"
        || event.created_at_unix_ms != input.captured_at_unix_ms
        || request.instruction.as_deref() != Some(input.instruction.as_str())
        || request.captured_message_id.as_deref() != Some(input.captured_message_id.as_str())
        || request.child_proposal_id.as_deref() != Some(input.child_proposal.id.as_str())
        || request.draft_job_id.as_deref() != Some(input.draft_job.id.as_str())
    {
        return Err(WorkflowLearningControlError::Conflict(
            "refinement capture replay does not match the queued request".to_string(),
        ));
    }
    let child = proposal_by_id(tx, &input.child_proposal.id)?.ok_or_else(|| {
        WorkflowLearningControlError::CorruptData(
            "queued refinement child proposal vanished".to_string(),
        )
    })?;
    let job = job_by_id(tx, &input.draft_job.id)?.ok_or_else(|| {
        WorkflowLearningControlError::CorruptData(
            "queued refinement draft job vanished".to_string(),
        )
    })?;
    if child.idempotency_key != input.child_proposal.idempotency_key
        || child.workflow_signature != input.child_proposal.workflow_signature
        || child.source_agent_id != input.child_proposal.source_agent_id
        || child.origin_channel != input.child_proposal.origin_channel
        || child.evidence_json != input.child_proposal.evidence_json
        || child.created_at_unix_ms != input.child_proposal.created_at_unix_ms
        || job.idempotency_key != input.draft_job.idempotency_key
        || job.proposal_id != input.child_proposal.id
        || job.revision_sha256 != input.draft_job.revision_sha256
        || job.kind != WorkflowJobKind::Draft
        || job.payload_json != input.draft_job.payload_json
        || job.max_attempts != input.draft_job.max_attempts
        || job.run_after_unix_ms != input.draft_job.run_after_unix_ms
        || job.created_at_unix_ms != input.draft_job.created_at_unix_ms
    {
        return Err(WorkflowLearningControlError::Conflict(
            "refinement capture replay child or job differs".to_string(),
        ));
    }
    Ok(WorkflowRefinementCaptureResult {
        request,
        child_proposal: child,
        draft_job: job,
    })
}

fn validate_capture(input: &CaptureWorkflowRefinement) -> Result<(), WorkflowLearningControlError> {
    validate_token("refinement request id", &input.request_id, 96)?;
    validate_token("refinement capture actor", &input.actor, 128)?;
    validate_text("refinement instruction", &input.instruction, 1, 8_000)?;
    validate_token(
        "refinement captured_message_id",
        &input.captured_message_id,
        128,
    )?;
    validate_token(
        "refinement capture idempotency_key",
        &input.idempotency_key,
        192,
    )?;
    if input.child_proposal.id != input.eligible_transition.proposal_id
        || input.child_proposal.id != input.drafting_transition.proposal_id
        || input.child_proposal.id != input.draft_job.proposal_id
        || input.eligible_transition.expected_state != WorkflowProposalState::Observed
        || input.eligible_transition.expected_version != 0
        || input.eligible_transition.expected_revision_sha256.is_some()
        || input.eligible_transition.to_state != WorkflowProposalState::Eligible
        || input.drafting_transition.expected_state != WorkflowProposalState::Eligible
        || input.drafting_transition.expected_version != 1
        || input.drafting_transition.expected_revision_sha256.is_some()
        || input.drafting_transition.to_state != WorkflowProposalState::Drafting
        || input.draft_job.kind != WorkflowJobKind::Draft
        || input.draft_job.revision_sha256.is_some()
        || input.child_proposal.created_at_unix_ms != input.captured_at_unix_ms
        || input.eligible_transition.occurred_at_unix_ms != input.captured_at_unix_ms
        || input.drafting_transition.occurred_at_unix_ms != input.captured_at_unix_ms
        || input.draft_job.run_after_unix_ms != input.captured_at_unix_ms
        || input.draft_job.created_at_unix_ms != input.captured_at_unix_ms
    {
        return Err(WorkflowLearningControlError::InvalidInput(
            "refinement capture requires one exact observed -> eligible -> drafting child and Draft job"
                .to_string(),
        ));
    }
    validate_transition(&input.eligible_transition)?;
    validate_transition(&input.drafting_transition)?;
    Ok(())
}

fn refinement_event_by_idempotency(
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
