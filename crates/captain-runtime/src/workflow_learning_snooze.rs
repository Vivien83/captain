//! Runtime policy for waking due workflow-learning proposals.

use captain_memory::workflow_learning_control::{
    WorkflowLearningControlError, WorkflowLearningStore, WorkflowProposalState,
    WorkflowProposalTransition,
};
use captain_memory::workflow_learning_outbox::NewWorkflowOutboxItem;
use captain_memory::workflow_learning_snooze::{WorkflowSnoozeWake, WorkflowSnoozeWakeResult};

use crate::workflow_learning_delivery::WORKFLOW_PROPOSAL_OUTBOX_TOPIC;

const OUTBOX_MAX_ATTEMPTS: u32 = 8;

#[derive(Debug, thiserror::Error)]
pub enum WorkflowSnoozeError {
    #[error("invalid workflow snooze worker: {0}")]
    InvalidWorker(String),
    #[error("snoozed workflow proposal is incomplete: {0}")]
    IncompleteProposal(String),
    #[error(transparent)]
    Control(#[from] WorkflowLearningControlError),
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct WorkflowSnoozeScheduler {
    control: WorkflowLearningStore,
    actor: String,
}

impl WorkflowSnoozeScheduler {
    pub fn new(
        control: WorkflowLearningStore,
        actor: impl Into<String>,
    ) -> Result<Self, WorkflowSnoozeError> {
        let actor = actor.into();
        if actor.is_empty()
            || actor.len() > 128
            || !actor.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
        {
            return Err(WorkflowSnoozeError::InvalidWorker(
                "actor must be a safe 1..=128 byte identifier".to_string(),
            ));
        }
        Ok(Self { control, actor })
    }

    pub fn wake_next_due(
        &self,
        now_unix_ms: i64,
    ) -> Result<Option<WorkflowSnoozeWakeResult>, WorkflowSnoozeError> {
        let Some(proposal) = self
            .control
            .list_due_snoozed(now_unix_ms, 1)?
            .into_iter()
            .next()
        else {
            return Ok(None);
        };
        let revision = proposal.revision_sha256.clone().ok_or_else(|| {
            WorkflowSnoozeError::IncompleteProposal("revision is missing".to_string())
        })?;
        let token = proposal.operator_token.clone().ok_or_else(|| {
            WorkflowSnoozeError::IncompleteProposal("operator token is missing".to_string())
        })?;
        let proposal_id = proposal.id.clone();
        let next_version = proposal.state_version.checked_add(1).ok_or_else(|| {
            WorkflowSnoozeError::IncompleteProposal("state version overflow".to_string())
        })?;
        let payload_json = serde_json::to_string(&serde_json::json!({
            "schema_version": 1,
            "proposal_id": &proposal_id,
            "revision_sha256": &revision,
            "state": "proposed",
        }))?;
        let request = WorkflowSnoozeWake {
            proposal_transition: WorkflowProposalTransition {
                proposal_id: proposal_id.clone(),
                expected_state: WorkflowProposalState::Snoozed,
                expected_version: proposal.state_version,
                expected_revision_sha256: Some(revision.clone()),
                to_state: WorkflowProposalState::Proposed,
                actor: self.actor.clone(),
                reason: "workflow proposal snooze elapsed".to_string(),
                idempotency_key: format!(
                    "snooze-wake-transition:{token}:v{}",
                    proposal.state_version
                ),
                snoozed_until_unix_ms: None,
                occurred_at_unix_ms: now_unix_ms,
            },
            notification: NewWorkflowOutboxItem {
                id: format!("snooze-wake-{token}-v{next_version}"),
                idempotency_key: format!("snooze-wake-notification:{token}:v{next_version}"),
                proposal_id,
                revision_sha256: Some(revision),
                topic: WORKFLOW_PROPOSAL_OUTBOX_TOPIC.to_string(),
                payload_json,
                max_attempts: OUTBOX_MAX_ATTEMPTS,
                run_after_unix_ms: now_unix_ms,
                created_at_unix_ms: now_unix_ms,
            },
        };
        Ok(Some(self.control.wake_snoozed_and_notify(&request)?))
    }
}
