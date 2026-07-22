//! Durable Skill Learning V2 scanner and pre-approval worker.

use crate::workflow_learning_engine_support::{
    artifact_kind, bounded_error, bounded_json, draft_completion_request, new_group_job,
    parse_any_validation_payload, parse_draft_payload, parse_group_payload,
    refinement_draft_completion_request, require_state, required_proposal,
    safe_model_error_message, transition, verify_staged_identity, verify_validation_identity,
    AnyValidationJobPayload, DraftJobPayload, GroupJobPayload, PAYLOAD_SCHEMA_VERSION,
};
use crate::workflow_learning_proposer::{WorkflowDraftProposer, WorkflowProposerOutcome};
use crate::workflow_learning_staging::{
    StageWorkflowDraftRequest, WorkflowStagingError, WorkflowStagingRoot,
};
use captain_memory::workflow_learning::WorkflowEpisodeStore;
use captain_memory::workflow_learning_control::{
    PublishValidatedDraft, WorkflowLearningControlError, WorkflowLearningStore,
    WorkflowProposalRecord, WorkflowProposalState, WorkflowProposalTransition,
};
use captain_memory::workflow_learning_outbox::NewWorkflowOutboxItem;
use captain_memory::workflow_learning_pipeline::{
    WorkflowAnalysisCompletion, WorkflowPipelineRejection, WorkflowValidationCompletion,
};
use captain_memory::workflow_learning_queue::{WorkflowJobKind, WorkflowJobRecord};
use captain_types::error::CaptainError;

#[derive(Debug, Clone)]
pub struct WorkflowLearningEngineConfig {
    pub worker_id: String,
    pub scan_limit: usize,
    pub daily_proposal_limit: u32,
    pub lease_duration_ms: i64,
}

impl Default for WorkflowLearningEngineConfig {
    fn default() -> Self {
        Self {
            worker_id: "captain:workflow-learning-v2".to_string(),
            scan_limit: 1_000,
            daily_proposal_limit: 3,
            lease_duration_ms: 120_000,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkflowScanSummary {
    pub episodes_seen: usize,
    pub rejected: usize,
    pub deferred: usize,
    pub linked_existing: usize,
    pub proposals_created: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkflowDraftRecoverySummary {
    pub recovered: usize,
    pub unresolved: usize,
    pub blocked: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowJobRunOutcome {
    Idle,
    Advanced {
        kind: WorkflowJobKind,
        job_id: String,
        proposal_id: String,
    },
    Retrying {
        kind: WorkflowJobKind,
        job_id: String,
        proposal_id: String,
    },
    Rejected {
        kind: WorkflowJobKind,
        job_id: String,
        proposal_id: String,
    },
}

impl WorkflowJobRunOutcome {
    pub(crate) fn advanced(job: &WorkflowJobRecord) -> Self {
        Self::Advanced {
            kind: job.kind,
            job_id: job.id.clone(),
            proposal_id: job.proposal_id.clone(),
        }
    }

    pub(crate) fn retrying(job: &WorkflowJobRecord) -> Self {
        Self::Retrying {
            kind: job.kind,
            job_id: job.id.clone(),
            proposal_id: job.proposal_id.clone(),
        }
    }

    pub(crate) fn rejected(job: &WorkflowJobRecord) -> Self {
        Self::Rejected {
            kind: job.kind,
            job_id: job.id.clone(),
            proposal_id: job.proposal_id.clone(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WorkflowLearningEngineError {
    #[error("workflow episode storage failed: {0}")]
    Memory(#[from] CaptainError),
    #[error(transparent)]
    Control(#[from] WorkflowLearningControlError),
    #[error(transparent)]
    Staging(#[from] WorkflowStagingError),
    #[error("workflow-learning payload is invalid: {0}")]
    InvalidPayload(String),
    #[error("workflow-learning serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub struct WorkflowLearningEngine {
    pub(crate) episodes: WorkflowEpisodeStore,
    pub(crate) control: WorkflowLearningStore,
    pub(crate) proposer: WorkflowDraftProposer,
    pub(crate) staging: WorkflowStagingRoot,
    pub(crate) config: WorkflowLearningEngineConfig,
}

impl WorkflowLearningEngine {
    pub fn new(
        episodes: WorkflowEpisodeStore,
        control: WorkflowLearningStore,
        proposer: WorkflowDraftProposer,
        staging: WorkflowStagingRoot,
        config: WorkflowLearningEngineConfig,
    ) -> Result<Self, WorkflowLearningEngineError> {
        if config.scan_limit == 0 || config.scan_limit > 1_000 {
            return Err(WorkflowLearningEngineError::InvalidPayload(
                "scan_limit must be within 1..=1000".to_string(),
            ));
        }
        if !(1_000..=3_600_000).contains(&config.lease_duration_ms) {
            return Err(WorkflowLearningEngineError::InvalidPayload(
                "lease_duration_ms must be within 1000..=3600000".to_string(),
            ));
        }
        Ok(Self {
            episodes,
            control,
            proposer,
            staging,
            config,
        })
    }

    pub fn recover_staged_drafts(
        &self,
        now_unix_ms: i64,
    ) -> Result<WorkflowDraftRecoverySummary, WorkflowLearningEngineError> {
        let mut summary = WorkflowDraftRecoverySummary::default();
        for job in self.control.list_uncertain_jobs(1_000)? {
            if job.kind != WorkflowJobKind::Draft {
                continue;
            }
            let payload = match parse_draft_payload(&job.payload_json) {
                Ok(payload) => payload,
                Err(_) => {
                    summary.blocked += 1;
                    continue;
                }
            };
            let staged = match self.staging.recover_job(&job.id) {
                Ok(Some(staged)) => staged,
                Ok(None) => {
                    summary.unresolved += 1;
                    continue;
                }
                Err(_) => {
                    summary.blocked += 1;
                    continue;
                }
            };
            let proposal = required_proposal(&self.control, &job.proposal_id)?;
            let request = match &payload {
                DraftJobPayload::Discovery(payload) => {
                    if verify_staged_identity(&staged, &payload.group).is_err() {
                        summary.blocked += 1;
                        continue;
                    }
                    draft_completion_request(
                        &self.config.worker_id,
                        &job,
                        &proposal,
                        &payload.group,
                        &staged,
                        now_unix_ms,
                        true,
                    )?
                }
                DraftJobPayload::Refinement(payload) => {
                    if self
                        .refinement_context(&proposal, &job.id, &payload.refinement)
                        .is_err()
                        || verify_staged_identity(&staged, &payload.group).is_err()
                    {
                        summary.blocked += 1;
                        continue;
                    }
                    refinement_draft_completion_request(
                        &self.config.worker_id,
                        &job,
                        &proposal,
                        payload,
                        &staged,
                        now_unix_ms,
                        true,
                    )?
                }
            };
            self.control
                .recover_staged_draft_and_enqueue_validation(&request)?;
            summary.recovered += 1;
        }
        Ok(summary)
    }

    pub async fn run_next_job(
        &self,
        now_unix_ms: i64,
    ) -> Result<WorkflowJobRunOutcome, WorkflowLearningEngineError> {
        let Some(job) = self.control.claim_due_preapproval_job(
            &self.config.worker_id,
            now_unix_ms,
            self.config.lease_duration_ms,
        )?
        else {
            return Ok(WorkflowJobRunOutcome::Idle);
        };
        match job.kind {
            WorkflowJobKind::Analyze => self.run_analyze(job, now_unix_ms),
            WorkflowJobKind::Draft => self.run_draft(job, now_unix_ms).await,
            WorkflowJobKind::Validate => self.run_validate(job, now_unix_ms),
            _ => Err(WorkflowLearningEngineError::InvalidPayload(format!(
                "preapproval claim returned {}",
                job.kind.as_str()
            ))),
        }
    }

    fn run_analyze(
        &self,
        job: WorkflowJobRecord,
        now_unix_ms: i64,
    ) -> Result<WorkflowJobRunOutcome, WorkflowLearningEngineError> {
        let payload = self.parse_or_settle_invalid(&job, now_unix_ms)?;
        let proposal = required_proposal(&self.control, &job.proposal_id)?;
        require_state(&proposal, WorkflowProposalState::Observed)?;
        let draft_job = new_group_job(
            &format!("{}-draft", proposal.id),
            &proposal.id,
            WorkflowJobKind::Draft,
            &payload.group,
            now_unix_ms,
        )?;
        let result_json = bounded_json(&serde_json::json!({
            "schema_version": PAYLOAD_SCHEMA_VERSION,
            "classification": payload.group.classification,
            "signature": payload.group.signature,
        }))?;
        let outcome = WorkflowJobRunOutcome::advanced(&job);
        self.control
            .complete_analysis_and_enqueue_draft(&WorkflowAnalysisCompletion {
                job_id: job.id,
                worker: self.config.worker_id.clone(),
                result_json: Some(result_json),
                eligibility_transition: transition(
                    &proposal,
                    WorkflowProposalState::Eligible,
                    "eligible",
                    &self.config.worker_id,
                    now_unix_ms,
                ),
                drafting_transition: WorkflowProposalTransition {
                    proposal_id: proposal.id.clone(),
                    expected_state: WorkflowProposalState::Eligible,
                    expected_version: proposal.state_version + 1,
                    expected_revision_sha256: None,
                    to_state: WorkflowProposalState::Drafting,
                    actor: self.config.worker_id.clone(),
                    reason: "deterministic workflow is eligible for drafting".to_string(),
                    idempotency_key: format!("{}:drafting", proposal.id),
                    snoozed_until_unix_ms: None,
                    occurred_at_unix_ms: now_unix_ms,
                },
                draft_job,
                completed_at_unix_ms: now_unix_ms,
            })?;
        Ok(outcome)
    }

    async fn run_draft(
        &self,
        job: WorkflowJobRecord,
        now_unix_ms: i64,
    ) -> Result<WorkflowJobRunOutcome, WorkflowLearningEngineError> {
        let payload = self.parse_draft_or_settle_invalid(&job, now_unix_ms)?;
        let proposal = required_proposal(&self.control, &job.proposal_id)?;
        require_state(&proposal, WorkflowProposalState::Drafting)?;
        match payload {
            DraftJobPayload::Discovery(payload) => {
                self.run_discovery_draft(job, proposal, payload, now_unix_ms)
                    .await
            }
            DraftJobPayload::Refinement(payload) => {
                self.run_refinement_draft(job, proposal, payload, now_unix_ms)
                    .await
            }
        }
    }

    async fn run_discovery_draft(
        &self,
        job: WorkflowJobRecord,
        proposal: WorkflowProposalRecord,
        payload: GroupJobPayload,
        now_unix_ms: i64,
    ) -> Result<WorkflowJobRunOutcome, WorkflowLearningEngineError> {
        self.control
            .mark_job_effect_started(&job.id, &self.config.worker_id, now_unix_ms)?;
        match self.proposer.draft(&payload.group).await {
            Ok(WorkflowProposerOutcome::Draft(draft)) => {
                let staged = self.staging.stage(StageWorkflowDraftRequest {
                    job_id: &job.id,
                    workflow_signature: &payload.group.signature,
                    draft: &draft,
                    active_model: self.proposer.active_model(),
                });
                let receipt = match staged {
                    Ok(receipt) => receipt,
                    Err(_) => {
                        self.reject_draft_after_staging_failure(
                            &job,
                            &proposal,
                            "staging_write_failed",
                            now_unix_ms,
                        )?;
                        return Ok(WorkflowJobRunOutcome::rejected(&job));
                    }
                };
                let staged = match self.staging.load_exact(&job.id, &receipt.revision_sha256) {
                    Ok(staged) => staged,
                    Err(_) => {
                        self.reject_draft_after_staging_failure(
                            &job,
                            &proposal,
                            "staging_verification_failed",
                            now_unix_ms,
                        )?;
                        return Ok(WorkflowJobRunOutcome::rejected(&job));
                    }
                };
                let request = draft_completion_request(
                    &self.config.worker_id,
                    &job,
                    &proposal,
                    &payload.group,
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
                let result_json = bounded_json(&serde_json::json!({
                    "schema_version": PAYLOAD_SCHEMA_VERSION,
                    "decision": "decline",
                    "reason": reason,
                }))?;
                let outcome = WorkflowJobRunOutcome::rejected(&job);
                self.control
                    .reject_pipeline_candidate(&WorkflowPipelineRejection {
                        job_id: job.id,
                        worker: self.config.worker_id.clone(),
                        job_kind: WorkflowJobKind::Draft,
                        result_json: Some(result_json),
                        proposal_transition: transition(
                            &proposal,
                            WorkflowProposalState::Rejected,
                            "model-declined",
                            &self.config.worker_id,
                            now_unix_ms,
                        ),
                        notification: None,
                        completed_at_unix_ms: now_unix_ms,
                    })?;
                Ok(outcome)
            }
            Err(error) => {
                self.settle_model_error(
                    &job,
                    error.code(),
                    safe_model_error_message(error.code()),
                    error.retryable(),
                    now_unix_ms,
                )?;
                Ok(WorkflowJobRunOutcome::retrying(&job))
            }
        }
    }

    fn run_validate(
        &self,
        job: WorkflowJobRecord,
        now_unix_ms: i64,
    ) -> Result<WorkflowJobRunOutcome, WorkflowLearningEngineError> {
        let payload = match parse_any_validation_payload(&job.payload_json) {
            Ok(payload) => payload,
            Err(error) => {
                self.control.fail_job(
                    &job.id,
                    &self.config.worker_id,
                    "invalid_job_payload",
                    &error.to_string(),
                    false,
                    now_unix_ms,
                    now_unix_ms,
                )?;
                return Err(error);
            }
        };
        match payload {
            AnyValidationJobPayload::Discovery(payload) => {
                self.run_discovery_validate(job, payload, now_unix_ms)
            }
            AnyValidationJobPayload::Refinement(payload) => {
                self.run_refinement_validate(job, payload, now_unix_ms)
            }
        }
    }

    fn run_discovery_validate(
        &self,
        job: WorkflowJobRecord,
        payload: crate::workflow_learning_engine_support::ValidationJobPayload,
        now_unix_ms: i64,
    ) -> Result<WorkflowJobRunOutcome, WorkflowLearningEngineError> {
        let proposal = required_proposal(&self.control, &job.proposal_id)?;
        require_state(&proposal, WorkflowProposalState::Validating)?;
        let staged = match self
            .staging
            .load_exact(&payload.draft_job_id, &payload.revision_sha256)
        {
            Ok(staged) if verify_validation_identity(&staged, &payload).is_ok() => staged,
            _ => {
                let result_json = bounded_json(&serde_json::json!({
                    "schema_version": PAYLOAD_SCHEMA_VERSION,
                    "valid": false,
                    "code": "staged_revision_invalid",
                }))?;
                let outcome = WorkflowJobRunOutcome::rejected(&job);
                self.control
                    .reject_pipeline_candidate(&WorkflowPipelineRejection {
                        job_id: job.id,
                        worker: self.config.worker_id.clone(),
                        job_kind: WorkflowJobKind::Validate,
                        result_json: Some(result_json),
                        proposal_transition: transition(
                            &proposal,
                            WorkflowProposalState::Rejected,
                            "staged-revision-invalid",
                            &self.config.worker_id,
                            now_unix_ms,
                        ),
                        notification: None,
                        completed_at_unix_ms: now_unix_ms,
                    })?;
                return Ok(outcome);
            }
        };
        let validation_json = bounded_json(&serde_json::json!({
            "schema_version": PAYLOAD_SCHEMA_VERSION,
            "checks": [
                "whole_response_schema",
                "native_artifact_parser",
                "secret_scan",
                "path_and_identifier_policy",
                "immutable_staging_hashes"
            ],
            "model": staged.manifest.model,
            "limitations": staged.manifest.draft.limitations,
        }))?;
        let kind = artifact_kind(staged.manifest.kind);
        let notification_payload = bounded_json(&serde_json::json!({
            "schema_version": PAYLOAD_SCHEMA_VERSION,
            "proposal_id": proposal.id,
            "revision_sha256": staged.manifest.revision_sha256,
            "state": "proposed",
        }))?;
        let outcome = WorkflowJobRunOutcome::advanced(&job);
        self.control
            .complete_validation_and_publish(&WorkflowValidationCompletion {
                job_id: job.id,
                worker: self.config.worker_id.clone(),
                result_json: Some(validation_json.clone()),
                publish: PublishValidatedDraft {
                    proposal_id: proposal.id.clone(),
                    expected_version: proposal.state_version,
                    staging_job_id: payload.draft_job_id,
                    revision_sha256: staged.manifest.revision_sha256.clone(),
                    artifact_sha256: staged.manifest.artifact_sha256.clone(),
                    kind,
                    name: staged.manifest.name,
                    validation_json,
                    actor: self.config.worker_id.clone(),
                    reason: "exact staged revision passed deterministic validation".to_string(),
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
            })?;
        Ok(outcome)
    }

    fn parse_or_settle_invalid(
        &self,
        job: &WorkflowJobRecord,
        now_unix_ms: i64,
    ) -> Result<GroupJobPayload, WorkflowLearningEngineError> {
        match parse_group_payload(&job.payload_json) {
            Ok(payload) => Ok(payload),
            Err(error) => {
                self.control.fail_job(
                    &job.id,
                    &self.config.worker_id,
                    "invalid_job_payload",
                    &error.to_string(),
                    false,
                    now_unix_ms,
                    now_unix_ms,
                )?;
                Err(error)
            }
        }
    }

    fn parse_draft_or_settle_invalid(
        &self,
        job: &WorkflowJobRecord,
        now_unix_ms: i64,
    ) -> Result<DraftJobPayload, WorkflowLearningEngineError> {
        match parse_draft_payload(&job.payload_json) {
            Ok(payload) => Ok(payload),
            Err(error) => {
                self.control.fail_job(
                    &job.id,
                    &self.config.worker_id,
                    "invalid_job_payload",
                    &error.to_string(),
                    false,
                    now_unix_ms,
                    now_unix_ms,
                )?;
                Err(error)
            }
        }
    }

    pub(crate) fn settle_model_error(
        &self,
        job: &WorkflowJobRecord,
        code: &str,
        message: &str,
        retryable: bool,
        now_unix_ms: i64,
    ) -> Result<(), WorkflowLearningEngineError> {
        let exponent = job.attempt_count.saturating_sub(1).min(6);
        let backoff_ms = 30_000_i64.saturating_mul(1_i64 << exponent);
        self.control.fail_job_after_known_effect(
            &job.id,
            &self.config.worker_id,
            code,
            &bounded_error(message),
            retryable,
            now_unix_ms.saturating_add(backoff_ms.min(600_000)),
            now_unix_ms,
            None,
        )?;
        Ok(())
    }

    fn reject_draft_after_staging_failure(
        &self,
        job: &WorkflowJobRecord,
        proposal: &WorkflowProposalRecord,
        code: &str,
        now_unix_ms: i64,
    ) -> Result<(), WorkflowLearningEngineError> {
        let result_json = bounded_json(&serde_json::json!({
            "schema_version": PAYLOAD_SCHEMA_VERSION,
            "decision": "reject",
            "code": code,
        }))?;
        self.control
            .reject_pipeline_candidate(&WorkflowPipelineRejection {
                job_id: job.id.clone(),
                worker: self.config.worker_id.clone(),
                job_kind: WorkflowJobKind::Draft,
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
            })?;
        Ok(())
    }
}
