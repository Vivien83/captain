//! Atomic pre-approval commits for Skill Learning V2.
//!
//! Each operation settles the current lease, advances the proposal, and
//! schedules its durable continuation in one SQLite transaction.

use rusqlite::{Transaction, TransactionBehavior};

use crate::workflow_learning_control::{
    publish_validated_draft_in_tx, transition_in_tx, WorkflowLearningControlError,
    WorkflowLearningStore, WorkflowProposalState,
};
use crate::workflow_learning_lifecycle_validation::proposal_transition_is_fresh;
use crate::workflow_learning_outbox::insert_outbox_in_tx;
pub use crate::workflow_learning_pipeline_types::{
    WorkflowAnalysisCompletion, WorkflowDraftCompletion, WorkflowPipelineRejection,
    WorkflowPipelineResult, WorkflowValidationCompletion,
};
use crate::workflow_learning_pipeline_validation::{
    require_pipeline_job_identity, validate_analysis_completion, validate_draft_completion,
    validate_pipeline_rejection, validate_validation_completion,
};
use crate::workflow_learning_queue::{
    complete_job_in_tx, insert_job_in_tx, recover_uncertain_job_in_tx, WorkflowJobEffectState,
    WorkflowJobKind,
};

impl WorkflowLearningStore {
    pub fn complete_analysis_and_enqueue_draft(
        &self,
        request: &WorkflowAnalysisCompletion,
    ) -> Result<WorkflowPipelineResult, WorkflowLearningControlError> {
        validate_analysis_completion(request)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let fresh = proposal_transition_is_fresh(&tx, &request.eligibility_transition)?;
        require_pipeline_job_identity(
            &tx,
            &request.job_id,
            &request.eligibility_transition.proposal_id,
            WorkflowJobKind::Analyze,
            WorkflowJobEffectState::None,
            fresh,
        )?;
        transition_in_tx(&tx, &request.eligibility_transition)?;
        let proposal = transition_in_tx(&tx, &request.drafting_transition)?;
        let job = complete_job_in_tx(
            &tx,
            &request.job_id,
            &request.worker,
            request.result_json.as_deref(),
            request.completed_at_unix_ms,
        )?;
        let next_job = insert_job_in_tx(&tx, &request.draft_job)?;
        tx.commit()?;
        Ok(WorkflowPipelineResult {
            proposal,
            job,
            next_job: Some(next_job),
            notification: None,
        })
    }

    pub fn complete_draft_and_enqueue_validation(
        &self,
        request: &WorkflowDraftCompletion,
    ) -> Result<WorkflowPipelineResult, WorkflowLearningControlError> {
        validate_draft_completion(request)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let fresh = proposal_transition_is_fresh(&tx, &request.proposal_transition)?;
        require_pipeline_job_identity(
            &tx,
            &request.job_id,
            &request.proposal_transition.proposal_id,
            WorkflowJobKind::Draft,
            WorkflowJobEffectState::Started,
            fresh,
        )?;
        let proposal = transition_in_tx(&tx, &request.proposal_transition)?;
        let job = complete_job_in_tx(
            &tx,
            &request.job_id,
            &request.worker,
            request.result_json.as_deref(),
            request.completed_at_unix_ms,
        )?;
        let next_job = insert_job_in_tx(&tx, &request.validation_job)?;
        tx.commit()?;
        Ok(WorkflowPipelineResult {
            proposal,
            job,
            next_job: Some(next_job),
            notification: None,
        })
    }

    /// Finish a draft job after restart only when Runtime has recovered and
    /// reverified its unique staged revision. No model call is replayed.
    pub fn recover_staged_draft_and_enqueue_validation(
        &self,
        request: &WorkflowDraftCompletion,
    ) -> Result<WorkflowPipelineResult, WorkflowLearningControlError> {
        validate_draft_completion(request)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_pipeline_job_identity(
            &tx,
            &request.job_id,
            &request.proposal_transition.proposal_id,
            WorkflowJobKind::Draft,
            WorkflowJobEffectState::Started,
            false,
        )?;
        let proposal = transition_in_tx(&tx, &request.proposal_transition)?;
        let job = recover_uncertain_job_in_tx(
            &tx,
            &request.job_id,
            request.result_json.as_deref(),
            request.completed_at_unix_ms,
        )?;
        let next_job = insert_job_in_tx(&tx, &request.validation_job)?;
        tx.commit()?;
        Ok(WorkflowPipelineResult {
            proposal,
            job,
            next_job: Some(next_job),
            notification: None,
        })
    }

    pub fn complete_validation_and_publish(
        &self,
        request: &WorkflowValidationCompletion,
    ) -> Result<WorkflowPipelineResult, WorkflowLearningControlError> {
        validate_validation_completion(request)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let fresh = publish_is_fresh(&tx, request)?;
        require_pipeline_job_identity(
            &tx,
            &request.job_id,
            &request.publish.proposal_id,
            WorkflowJobKind::Validate,
            WorkflowJobEffectState::None,
            fresh,
        )?;
        let proposal = publish_validated_draft_in_tx(&tx, &request.publish)?;
        let job = complete_job_in_tx(
            &tx,
            &request.job_id,
            &request.worker,
            request.result_json.as_deref(),
            request.completed_at_unix_ms,
        )?;
        let notification = request
            .notification
            .as_ref()
            .map(|item| insert_outbox_in_tx(&tx, item))
            .transpose()?;
        tx.commit()?;
        Ok(WorkflowPipelineResult {
            proposal,
            job,
            next_job: None,
            notification,
        })
    }

    pub fn reject_pipeline_candidate(
        &self,
        request: &WorkflowPipelineRejection,
    ) -> Result<WorkflowPipelineResult, WorkflowLearningControlError> {
        validate_pipeline_rejection(request)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let fresh = proposal_transition_is_fresh(&tx, &request.proposal_transition)?;
        let expected_effect = if request.job_kind == WorkflowJobKind::Draft {
            WorkflowJobEffectState::Started
        } else {
            WorkflowJobEffectState::None
        };
        require_pipeline_job_identity(
            &tx,
            &request.job_id,
            &request.proposal_transition.proposal_id,
            request.job_kind,
            expected_effect,
            fresh,
        )?;
        let proposal = transition_in_tx(&tx, &request.proposal_transition)?;
        let job = complete_job_in_tx(
            &tx,
            &request.job_id,
            &request.worker,
            request.result_json.as_deref(),
            request.completed_at_unix_ms,
        )?;
        let notification = request
            .notification
            .as_ref()
            .map(|item| insert_outbox_in_tx(&tx, item))
            .transpose()?;
        tx.commit()?;
        Ok(WorkflowPipelineResult {
            proposal,
            job,
            next_job: None,
            notification,
        })
    }
}

pub(crate) fn publish_is_fresh(
    tx: &Transaction<'_>,
    request: &WorkflowValidationCompletion,
) -> Result<bool, WorkflowLearningControlError> {
    let proposal =
        crate::workflow_learning_control::proposal_by_id(tx, &request.publish.proposal_id)?
            .ok_or_else(|| {
                WorkflowLearningControlError::NotFound(request.publish.proposal_id.clone())
            })?;
    Ok(proposal.state == WorkflowProposalState::Validating
        && proposal.state_version == request.publish.expected_version
        && proposal.revision_sha256.is_none())
}
