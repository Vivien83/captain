use rusqlite::Transaction;

use crate::workflow_learning_control::{
    proposal_by_id, WorkflowLearningControlError, WorkflowProposalState, WorkflowProposalTransition,
};
use crate::workflow_learning_installation::{
    WorkflowInstallationPhase, WorkflowInstallationTransition,
};
use crate::workflow_learning_outbox::NewWorkflowOutboxItem;
use crate::workflow_learning_queue::{
    job_by_id, NewWorkflowJob, WorkflowJobEffectState, WorkflowJobKind, WorkflowJobRecord,
    WorkflowJobStatus,
};
use crate::workflow_learning_validation::{validate_text, validate_token};

pub(crate) fn proposal_transition_is_fresh(
    tx: &Transaction<'_>,
    transition: &WorkflowProposalTransition,
) -> Result<bool, WorkflowLearningControlError> {
    let proposal = proposal_by_id(tx, &transition.proposal_id)?
        .ok_or_else(|| WorkflowLearningControlError::NotFound(transition.proposal_id.clone()))?;
    Ok(proposal.state == transition.expected_state
        && proposal.state_version == transition.expected_version
        && proposal.revision_sha256 == transition.expected_revision_sha256)
}

pub(crate) fn require_proposal_pair(
    transition: &WorkflowProposalTransition,
    from: &[WorkflowProposalState],
    to: WorkflowProposalState,
) -> Result<(), WorkflowLearningControlError> {
    if from.contains(&transition.expected_state)
        && transition.to_state == to
        && transition.expected_revision_sha256.is_some()
    {
        Ok(())
    } else {
        Err(WorkflowLearningControlError::InvalidInput(format!(
            "lifecycle operation requires {:?} -> {} with an immutable revision",
            from.iter().map(|state| state.as_str()).collect::<Vec<_>>(),
            to.as_str()
        )))
    }
}

pub(crate) fn require_installation_pair(
    transition: &WorkflowInstallationTransition,
    from: &[WorkflowInstallationPhase],
    to: WorkflowInstallationPhase,
) -> Result<(), WorkflowLearningControlError> {
    if from.contains(&transition.expected_phase) && transition.to_phase == to {
        Ok(())
    } else {
        Err(WorkflowLearningControlError::InvalidInput(format!(
            "lifecycle operation requires installation {:?} -> {}",
            from.iter().map(|phase| phase.as_str()).collect::<Vec<_>>(),
            to.as_str()
        )))
    }
}

pub(crate) fn require_job_identity(
    tx: &Transaction<'_>,
    id: &str,
    proposal_id: &str,
    revision_sha256: &str,
    kind: WorkflowJobKind,
    fresh: bool,
) -> Result<WorkflowJobRecord, WorkflowLearningControlError> {
    let job =
        job_by_id(tx, id)?.ok_or_else(|| WorkflowLearningControlError::NotFound(id.to_string()))?;
    if job.proposal_id != proposal_id
        || job.revision_sha256.as_deref() != Some(revision_sha256)
        || job.kind != kind
    {
        return Err(WorkflowLearningControlError::Conflict(
            "lifecycle job does not match proposal revision and operation".to_string(),
        ));
    }
    if fresh
        && (job.status != WorkflowJobStatus::Running
            || job.effect_state != WorkflowJobEffectState::Started)
    {
        return Err(WorkflowLearningControlError::Conflict(
            "fresh lifecycle completion requires a running job with a started effect".to_string(),
        ));
    }
    Ok(job)
}

pub(crate) fn require_next_job(
    job: &NewWorkflowJob,
    proposal_id: &str,
    revision_sha256: &str,
    kind: WorkflowJobKind,
) -> Result<(), WorkflowLearningControlError> {
    if job.proposal_id == proposal_id
        && job.revision_sha256.as_deref() == Some(revision_sha256)
        && job.kind == kind
    {
        Ok(())
    } else {
        Err(WorkflowLearningControlError::InvalidInput(
            "next lifecycle job does not match proposal revision and phase".to_string(),
        ))
    }
}

pub(crate) fn require_notification(
    notification: Option<&NewWorkflowOutboxItem>,
    proposal_id: &str,
    revision_sha256: &str,
) -> Result<(), WorkflowLearningControlError> {
    if let Some(notification) = notification {
        if notification.proposal_id != proposal_id
            || notification.revision_sha256.as_deref() != Some(revision_sha256)
        {
            return Err(WorkflowLearningControlError::InvalidInput(
                "lifecycle notification does not match proposal revision".to_string(),
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_known_failure(
    error_code: &str,
    error_message: &str,
) -> Result<(), WorkflowLearningControlError> {
    validate_token("lifecycle error_code", error_code, 96)?;
    validate_text("lifecycle error_message", error_message, 1, 2_048)
}
