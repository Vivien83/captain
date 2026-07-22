use rusqlite::Transaction;

use crate::workflow_learning_control::{
    validate_publish, validate_transition, WorkflowLearningControlError, WorkflowProposalState,
    WorkflowProposalTransition,
};
use crate::workflow_learning_outbox::NewWorkflowOutboxItem;
use crate::workflow_learning_pipeline_types::{
    WorkflowAnalysisCompletion, WorkflowDraftCompletion, WorkflowPipelineRejection,
    WorkflowValidationCompletion,
};
use crate::workflow_learning_queue::{
    job_by_id, NewWorkflowJob, WorkflowJobEffectState, WorkflowJobKind, WorkflowJobRecord,
    WorkflowJobStatus,
};

pub(crate) fn validate_analysis_completion(
    request: &WorkflowAnalysisCompletion,
) -> Result<(), WorkflowLearningControlError> {
    validate_transition(&request.eligibility_transition)?;
    validate_transition(&request.drafting_transition)?;
    require_unrevisioned_pair(
        &request.eligibility_transition,
        WorkflowProposalState::Observed,
        WorkflowProposalState::Eligible,
    )?;
    require_unrevisioned_pair(
        &request.drafting_transition,
        WorkflowProposalState::Eligible,
        WorkflowProposalState::Drafting,
    )?;
    if request.eligibility_transition.proposal_id != request.drafting_transition.proposal_id
        || request.drafting_transition.expected_version
            != request
                .eligibility_transition
                .expected_version
                .saturating_add(1)
    {
        return Err(WorkflowLearningControlError::InvalidInput(
            "analysis transitions must advance one proposal consecutively".to_string(),
        ));
    }
    require_next_job(
        &request.draft_job,
        &request.eligibility_transition.proposal_id,
        WorkflowJobKind::Draft,
    )
}

pub(crate) fn validate_draft_completion(
    request: &WorkflowDraftCompletion,
) -> Result<(), WorkflowLearningControlError> {
    validate_transition(&request.proposal_transition)?;
    require_unrevisioned_pair(
        &request.proposal_transition,
        WorkflowProposalState::Drafting,
        WorkflowProposalState::Validating,
    )?;
    require_next_job(
        &request.validation_job,
        &request.proposal_transition.proposal_id,
        WorkflowJobKind::Validate,
    )
}

pub(crate) fn validate_validation_completion(
    request: &WorkflowValidationCompletion,
) -> Result<(), WorkflowLearningControlError> {
    validate_publish(&request.publish)?;
    require_notification(
        request.notification.as_ref(),
        &request.publish.proposal_id,
        Some(&request.publish.revision_sha256),
    )
}

pub(crate) fn validate_pipeline_rejection(
    request: &WorkflowPipelineRejection,
) -> Result<(), WorkflowLearningControlError> {
    validate_transition(&request.proposal_transition)?;
    let expected_state = match request.job_kind {
        WorkflowJobKind::Draft => WorkflowProposalState::Drafting,
        WorkflowJobKind::Validate => WorkflowProposalState::Validating,
        _ => {
            return Err(WorkflowLearningControlError::InvalidInput(
                "pipeline rejection only accepts draft or validate jobs".to_string(),
            ))
        }
    };
    require_unrevisioned_pair(
        &request.proposal_transition,
        expected_state,
        WorkflowProposalState::Rejected,
    )?;
    require_notification(
        request.notification.as_ref(),
        &request.proposal_transition.proposal_id,
        None,
    )
}

pub(crate) fn require_pipeline_job_identity(
    tx: &Transaction<'_>,
    id: &str,
    proposal_id: &str,
    kind: WorkflowJobKind,
    expected_effect: WorkflowJobEffectState,
    fresh: bool,
) -> Result<WorkflowJobRecord, WorkflowLearningControlError> {
    let job =
        job_by_id(tx, id)?.ok_or_else(|| WorkflowLearningControlError::NotFound(id.to_string()))?;
    if job.proposal_id != proposal_id || job.revision_sha256.is_some() || job.kind != kind {
        return Err(WorkflowLearningControlError::Conflict(
            "pipeline job does not match the unrevisioned proposal phase".to_string(),
        ));
    }
    if fresh && (job.status != WorkflowJobStatus::Running || job.effect_state != expected_effect) {
        return Err(WorkflowLearningControlError::Conflict(format!(
            "fresh {} completion requires a running job with {} effect",
            kind.as_str(),
            expected_effect.as_str()
        )));
    }
    Ok(job)
}

fn require_unrevisioned_pair(
    transition: &WorkflowProposalTransition,
    from: WorkflowProposalState,
    to: WorkflowProposalState,
) -> Result<(), WorkflowLearningControlError> {
    if transition.expected_state == from
        && transition.to_state == to
        && transition.expected_revision_sha256.is_none()
        && transition.snoozed_until_unix_ms.is_none()
    {
        Ok(())
    } else {
        Err(WorkflowLearningControlError::InvalidInput(format!(
            "pipeline operation requires {} -> {} without a revision",
            from.as_str(),
            to.as_str()
        )))
    }
}

fn require_next_job(
    job: &NewWorkflowJob,
    proposal_id: &str,
    kind: WorkflowJobKind,
) -> Result<(), WorkflowLearningControlError> {
    if job.proposal_id == proposal_id && job.revision_sha256.is_none() && job.kind == kind {
        Ok(())
    } else {
        Err(WorkflowLearningControlError::InvalidInput(
            "next pipeline job does not match the unrevisioned proposal phase".to_string(),
        ))
    }
}

fn require_notification(
    notification: Option<&NewWorkflowOutboxItem>,
    proposal_id: &str,
    revision: Option<&str>,
) -> Result<(), WorkflowLearningControlError> {
    if let Some(notification) = notification {
        if notification.proposal_id != proposal_id
            || notification.revision_sha256.as_deref() != revision
        {
            return Err(WorkflowLearningControlError::InvalidInput(
                "pipeline notification does not match the proposal revision".to_string(),
            ));
        }
    }
    Ok(())
}
