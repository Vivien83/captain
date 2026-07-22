//! Atomic wake-up of snoozed workflow-learning proposals.

use rusqlite::TransactionBehavior;

use crate::workflow_learning_control::{
    proposal_by_id, transition_in_tx, validate_transition, WorkflowLearningControlError,
    WorkflowLearningStore, WorkflowProposalRecord, WorkflowProposalState,
    WorkflowProposalTransition,
};
use crate::workflow_learning_outbox::{
    insert_outbox_in_tx, NewWorkflowOutboxItem, WorkflowOutboxRecord,
};

#[derive(Debug, Clone)]
pub struct WorkflowSnoozeWake {
    pub proposal_transition: WorkflowProposalTransition,
    pub notification: NewWorkflowOutboxItem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowSnoozeWakeResult {
    pub proposal: WorkflowProposalRecord,
    pub notification: WorkflowOutboxRecord,
}

impl WorkflowLearningStore {
    pub fn wake_snoozed_and_notify(
        &self,
        request: &WorkflowSnoozeWake,
    ) -> Result<WorkflowSnoozeWakeResult, WorkflowLearningControlError> {
        validate_wake(request)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current =
            proposal_by_id(&tx, &request.proposal_transition.proposal_id)?.ok_or_else(|| {
                WorkflowLearningControlError::NotFound(
                    request.proposal_transition.proposal_id.clone(),
                )
            })?;
        if current.state == WorkflowProposalState::Snoozed
            && current.snoozed_until_unix_ms.map_or(true, |deadline| {
                deadline > request.proposal_transition.occurred_at_unix_ms
            })
        {
            return Err(WorkflowLearningControlError::Conflict(
                "snoozed proposal is not due yet".to_string(),
            ));
        }
        let proposal = transition_in_tx(&tx, &request.proposal_transition)?;
        let notification = insert_outbox_in_tx(&tx, &request.notification)?;
        tx.commit()?;
        Ok(WorkflowSnoozeWakeResult {
            proposal,
            notification,
        })
    }
}

fn validate_wake(request: &WorkflowSnoozeWake) -> Result<(), WorkflowLearningControlError> {
    validate_transition(&request.proposal_transition)?;
    let transition = &request.proposal_transition;
    let revision = transition
        .expected_revision_sha256
        .as_deref()
        .ok_or_else(|| {
            WorkflowLearningControlError::InvalidInput(
                "snooze wake requires the exact proposal revision".to_string(),
            )
        })?;
    if transition.expected_state != WorkflowProposalState::Snoozed
        || transition.to_state != WorkflowProposalState::Proposed
        || transition.snoozed_until_unix_ms.is_some()
    {
        return Err(WorkflowLearningControlError::InvalidInput(
            "snooze wake requires snoozed -> proposed without a new deadline".to_string(),
        ));
    }
    let notification = &request.notification;
    if notification.proposal_id != transition.proposal_id
        || notification.revision_sha256.as_deref() != Some(revision)
        || notification.run_after_unix_ms != transition.occurred_at_unix_ms
        || notification.created_at_unix_ms != transition.occurred_at_unix_ms
    {
        return Err(WorkflowLearningControlError::InvalidInput(
            "snooze wake notification does not match the exact transition".to_string(),
        ));
    }
    Ok(())
}
