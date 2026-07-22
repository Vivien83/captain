//! Refinement-specific execution for the durable workflow-learning engine.

use captain_memory::workflow_learning_control::{
    PublishValidatedDraft, WorkflowProposalRecord, WorkflowProposalState,
    WorkflowProposalTransition,
};
use captain_memory::workflow_learning_outbox::NewWorkflowOutboxItem;
use captain_memory::workflow_learning_pipeline::{
    WorkflowPipelineRejection, WorkflowValidationCompletion,
};
use captain_memory::workflow_learning_queue::WorkflowJobRecord;
use captain_memory::workflow_learning_refinement::{
    WorkflowRefinementRecord, WorkflowRefinementState,
};
use captain_memory::workflow_learning_refinement_lifecycle::{
    WorkflowRefinementRejection, WorkflowRefinementValidationCompletion,
};

use crate::workflow_learning_engine::{
    WorkflowJobRunOutcome, WorkflowLearningEngine, WorkflowLearningEngineError,
};
use crate::workflow_learning_engine_support::{
    artifact_kind, bounded_error, bounded_json, refinement_draft_completion_request, require_state,
    required_proposal, safe_model_error_message, transition, verify_refinement_validation_identity,
    RefinementDraftJobPayload, RefinementValidationJobPayload, WorkflowRefinementJobLink,
    PAYLOAD_SCHEMA_VERSION,
};
use crate::workflow_learning_proposer::WorkflowProposerOutcome;
use crate::workflow_learning_staging::{LoadedStagedWorkflowDraft, StageWorkflowDraftRequest};

pub(crate) struct RefinementExecutionContext {
    request: WorkflowRefinementRecord,
    parent: WorkflowProposalRecord,
    staged_parent: LoadedStagedWorkflowDraft,
}

impl WorkflowLearningEngine {
    pub(crate) async fn run_refinement_draft(
        &self,
        job: WorkflowJobRecord,
        proposal: WorkflowProposalRecord,
        payload: RefinementDraftJobPayload,
        now_unix_ms: i64,
    ) -> Result<WorkflowJobRunOutcome, WorkflowLearningEngineError> {
        let context = self.refinement_context(&proposal, &job.id, &payload.refinement)?;
        let instruction = context.request.instruction.as_deref().ok_or_else(|| {
            WorkflowLearningEngineError::InvalidPayload(
                "queued refinement has no captured instruction".to_string(),
            )
        })?;
        self.control
            .mark_job_effect_started(&job.id, &self.config.worker_id, now_unix_ms)?;
        match self
            .proposer
            .refine(
                &context.staged_parent.manifest.draft,
                instruction,
                &context.request.language,
            )
            .await
        {
            Ok(WorkflowProposerOutcome::Draft(draft)) => {
                let receipt = match self.staging.stage(StageWorkflowDraftRequest {
                    job_id: &job.id,
                    workflow_signature: &payload.group.signature,
                    draft: &draft,
                    active_model: self.proposer.active_model(),
                }) {
                    Ok(receipt) => receipt,
                    Err(_) => {
                        self.reject_refinement_job(
                            &job,
                            &proposal,
                            &payload.refinement,
                            "staging_write_failed",
                            "refined draft could not be staged immutably",
                            now_unix_ms,
                        )?;
                        return Ok(WorkflowJobRunOutcome::rejected(&job));
                    }
                };
                let staged = match self.staging.load_exact(&job.id, &receipt.revision_sha256) {
                    Ok(staged) => staged,
                    Err(_) => {
                        self.reject_refinement_job(
                            &job,
                            &proposal,
                            &payload.refinement,
                            "staging_verification_failed",
                            "refined staging revision failed exact verification",
                            now_unix_ms,
                        )?;
                        return Ok(WorkflowJobRunOutcome::rejected(&job));
                    }
                };
                let request = refinement_draft_completion_request(
                    &self.config.worker_id,
                    &job,
                    &proposal,
                    &payload,
                    &staged,
                    now_unix_ms,
                    false,
                )?;
                let outcome = WorkflowJobRunOutcome::advanced(&job);
                self.control
                    .complete_draft_and_enqueue_validation(&request)?;
                Ok(outcome)
            }
            Ok(WorkflowProposerOutcome::Declined { reason }) => {
                self.reject_refinement_job(
                    &job,
                    &proposal,
                    &payload.refinement,
                    "model_declined",
                    &reason,
                    now_unix_ms,
                )?;
                Ok(WorkflowJobRunOutcome::rejected(&job))
            }
            Err(error) => {
                let code = error.code();
                let message = safe_model_error_message(code);
                if error.retryable() && job.attempt_count < job.max_attempts {
                    self.settle_model_error(&job, code, message, true, now_unix_ms)?;
                    Ok(WorkflowJobRunOutcome::retrying(&job))
                } else {
                    self.reject_refinement_job(
                        &job,
                        &proposal,
                        &payload.refinement,
                        code,
                        message,
                        now_unix_ms,
                    )?;
                    Ok(WorkflowJobRunOutcome::rejected(&job))
                }
            }
        }
    }

    pub(crate) fn run_refinement_validate(
        &self,
        job: WorkflowJobRecord,
        payload: RefinementValidationJobPayload,
        now_unix_ms: i64,
    ) -> Result<WorkflowJobRunOutcome, WorkflowLearningEngineError> {
        let proposal = required_proposal(&self.control, &job.proposal_id)?;
        require_state(&proposal, WorkflowProposalState::Validating)?;
        let context =
            self.refinement_context(&proposal, &payload.draft_job_id, &payload.refinement)?;
        let staged = match self
            .staging
            .load_exact(&payload.draft_job_id, &payload.revision_sha256)
        {
            Ok(staged) if verify_refinement_validation_identity(&staged, &payload).is_ok() => {
                staged
            }
            _ => {
                self.reject_refinement_job(
                    &job,
                    &proposal,
                    &payload.refinement,
                    "staged_revision_invalid",
                    "refined staged revision failed deterministic validation",
                    now_unix_ms,
                )?;
                return Ok(WorkflowJobRunOutcome::rejected(&job));
            }
        };
        let validation_json = bounded_json(&serde_json::json!({
            "schema_version": PAYLOAD_SCHEMA_VERSION,
            "checks": [
                "whole_response_schema",
                "immutable_parent_identity",
                "native_artifact_parser",
                "secret_scan",
                "authority_non_escalation",
                "immutable_staging_hashes"
            ],
            "model": staged.manifest.model,
            "limitations": staged.manifest.draft.limitations,
            "parent_revision_sha256": payload.refinement.parent_revision_sha256,
        }))?;
        let notification_payload = bounded_json(&serde_json::json!({
            "schema_version": PAYLOAD_SCHEMA_VERSION,
            "proposal_id": proposal.id,
            "revision_sha256": staged.manifest.revision_sha256,
            "state": "proposed",
            "refines_proposal_id": context.parent.id,
        }))?;
        let outcome = WorkflowJobRunOutcome::advanced(&job);
        self.control
            .complete_refinement_validation(&WorkflowRefinementValidationCompletion {
                request_id: context.request.id.clone(),
                expected_request_version: payload.refinement.expected_request_version,
                parent_transition: WorkflowProposalTransition {
                    proposal_id: context.parent.id,
                    expected_state: WorkflowProposalState::Proposed,
                    expected_version: payload.refinement.parent_proposal_version,
                    expected_revision_sha256: Some(
                        payload.refinement.parent_revision_sha256.clone(),
                    ),
                    to_state: WorkflowProposalState::Superseded,
                    actor: self.config.worker_id.clone(),
                    reason: format!("validated refinement {} superseded parent", proposal.id),
                    idempotency_key: format!(
                        "{}:superseded:{}",
                        payload.refinement.parent_proposal_id, proposal.id
                    ),
                    snoozed_until_unix_ms: None,
                    occurred_at_unix_ms: now_unix_ms,
                },
                validation: WorkflowValidationCompletion {
                    job_id: job.id,
                    worker: self.config.worker_id.clone(),
                    result_json: Some(validation_json.clone()),
                    publish: PublishValidatedDraft {
                        proposal_id: proposal.id.clone(),
                        expected_version: proposal.state_version,
                        staging_job_id: payload.draft_job_id,
                        revision_sha256: staged.manifest.revision_sha256.clone(),
                        artifact_sha256: staged.manifest.artifact_sha256.clone(),
                        kind: artifact_kind(staged.manifest.kind),
                        name: staged.manifest.name,
                        validation_json,
                        actor: self.config.worker_id.clone(),
                        reason: "refined revision passed deterministic validation".to_string(),
                        idempotency_key: format!("{}:proposed", proposal.id),
                        occurred_at_unix_ms: now_unix_ms,
                    },
                    notification: Some(NewWorkflowOutboxItem {
                        id: format!("{}-proposed", proposal.id),
                        idempotency_key: format!("{}:proposed-notification", proposal.id),
                        proposal_id: proposal.id,
                        revision_sha256: Some(staged.manifest.revision_sha256),
                        topic: "workflow_learning.proposed".to_string(),
                        payload_json: notification_payload,
                        max_attempts: 8,
                        run_after_unix_ms: now_unix_ms,
                        created_at_unix_ms: now_unix_ms,
                    }),
                    completed_at_unix_ms: now_unix_ms,
                },
                actor: self.config.worker_id.clone(),
                idempotency_key: format!("{}:completed", context.request.id),
                completed_at_unix_ms: now_unix_ms,
            })?;
        Ok(outcome)
    }

    pub(crate) fn refinement_context(
        &self,
        child: &WorkflowProposalRecord,
        draft_job_id: &str,
        link: &WorkflowRefinementJobLink,
    ) -> Result<RefinementExecutionContext, WorkflowLearningEngineError> {
        let request = self
            .control
            .get_refinement_request(&link.request_id)?
            .ok_or_else(|| {
                WorkflowLearningEngineError::InvalidPayload(
                    "refinement request vanished".to_string(),
                )
            })?;
        if request.state != WorkflowRefinementState::Queued
            || request.state_version != link.expected_request_version
            || request.child_proposal_id.as_deref() != Some(child.id.as_str())
            || request.draft_job_id.as_deref() != Some(draft_job_id)
            || request.proposal_id != link.parent_proposal_id
            || request.expected_proposal_version != link.parent_proposal_version
            || request.revision_sha256 != link.parent_revision_sha256
            || request.instruction.is_none()
        {
            return Err(WorkflowLearningEngineError::InvalidPayload(
                "refinement request does not match its durable job".to_string(),
            ));
        }
        let parent = required_proposal(&self.control, &link.parent_proposal_id)?;
        if parent.state != WorkflowProposalState::Proposed
            || parent.state_version != link.parent_proposal_version
            || parent.revision_sha256.as_deref() != Some(link.parent_revision_sha256.as_str())
            || parent.artifact_sha256.as_deref() != Some(link.parent_artifact_sha256.as_str())
            || parent.staging_job_id.as_deref() != Some(link.parent_staging_job_id.as_str())
        {
            return Err(WorkflowLearningEngineError::InvalidPayload(
                "refinement parent changed after capture".to_string(),
            ));
        }
        let staged_parent = self
            .staging
            .load_exact(&link.parent_staging_job_id, &link.parent_revision_sha256)?;
        if staged_parent.manifest.workflow_signature != parent.workflow_signature
            || staged_parent.manifest.artifact_sha256 != link.parent_artifact_sha256
            || Some(artifact_kind(staged_parent.manifest.kind)) != parent.kind
            || Some(staged_parent.manifest.name.as_str()) != parent.name.as_deref()
        {
            return Err(WorkflowLearningEngineError::InvalidPayload(
                "refinement parent staging differs from SQLite".to_string(),
            ));
        }
        Ok(RefinementExecutionContext {
            request,
            parent,
            staged_parent,
        })
    }

    fn reject_refinement_job(
        &self,
        job: &WorkflowJobRecord,
        proposal: &WorkflowProposalRecord,
        link: &WorkflowRefinementJobLink,
        code: &str,
        reason: &str,
        now_unix_ms: i64,
    ) -> Result<(), WorkflowLearningEngineError> {
        let reason = bounded_error(reason);
        let result_json = bounded_json(&serde_json::json!({
            "schema_version": PAYLOAD_SCHEMA_VERSION,
            "decision": "reject",
            "code": code,
            "reason": reason,
            "refinement_request_id": link.request_id,
        }))?;
        self.control
            .reject_refinement(&WorkflowRefinementRejection {
                request_id: link.request_id.clone(),
                expected_request_version: link.expected_request_version,
                rejection: WorkflowPipelineRejection {
                    job_id: job.id.clone(),
                    worker: self.config.worker_id.clone(),
                    job_kind: job.kind,
                    result_json: Some(result_json),
                    proposal_transition: transition(
                        proposal,
                        WorkflowProposalState::Rejected,
                        code,
                        &self.config.worker_id,
                        now_unix_ms,
                    ),
                    notification: None,
                    completed_at_unix_ms: now_unix_ms,
                },
                actor: self.config.worker_id.clone(),
                reason,
                idempotency_key: format!("{}:failed", link.request_id),
                completed_at_unix_ms: now_unix_ms,
            })?;
        Ok(())
    }
}
