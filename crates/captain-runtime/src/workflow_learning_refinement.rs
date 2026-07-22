//! Channel-neutral capture of one durable operator refinement instruction.

use captain_memory::workflow_learning_control::{
    NewWorkflowProposal, WorkflowLearningControlError, WorkflowLearningStore,
    WorkflowProposalRecord, WorkflowProposalState, WorkflowProposalTransition,
};
use captain_memory::workflow_learning_refinement::WorkflowRefinementRecord;
use captain_memory::workflow_learning_refinement_capture::{
    CaptureWorkflowRefinement, WorkflowRefinementCaptureResult,
};
use sha2::{Digest, Sha256};

use crate::workflow_learning_analysis::WorkflowGroupAnalysis;
use crate::workflow_learning_engine_support::{
    bounded_json, new_refinement_draft_job, WorkflowRefinementJobLink,
};
use crate::workflow_learning_proposer::WorkflowDraftKind;
use crate::workflow_learning_staging::{
    LoadedStagedWorkflowDraft, WorkflowStagingError, WorkflowStagingRoot,
};

const MAX_INSTRUCTION_CHARS: usize = 8_000;
const AWAITING_INPUT_VERSION: u64 = 0;
const QUEUED_REFINEMENT_VERSION: u64 = 1;

#[derive(Debug, Clone)]
pub struct WorkflowRefinementCaptureInput {
    pub actor: String,
    pub surface: String,
    pub conversation_key: String,
    pub captured_message_id: String,
    pub instruction: String,
    pub captured_at_unix_ms: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkflowRefinementCoordinatorError {
    #[error("invalid workflow refinement input: {0}")]
    InvalidInput(String),
    #[error(transparent)]
    Control(#[from] WorkflowLearningControlError),
    #[error(transparent)]
    Staging(#[from] WorkflowStagingError),
    #[error("workflow refinement evidence is invalid: {0}")]
    Evidence(#[from] serde_json::Error),
    #[error(transparent)]
    Engine(#[from] crate::workflow_learning_engine::WorkflowLearningEngineError),
}

#[derive(Clone)]
pub struct WorkflowRefinementCoordinator {
    control: WorkflowLearningStore,
    staging: WorkflowStagingRoot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRefinementCoordinatorResult {
    pub capture: WorkflowRefinementCaptureResult,
    pub replayed: bool,
}

impl WorkflowRefinementCoordinator {
    pub fn new(control: WorkflowLearningStore, staging: WorkflowStagingRoot) -> Self {
        Self { control, staging }
    }

    pub fn capture_pending(
        &self,
        input: &WorkflowRefinementCaptureInput,
    ) -> Result<Option<WorkflowRefinementCaptureResult>, WorkflowRefinementCoordinatorError> {
        self.capture_pending_with_status(input)
            .map(|result| result.map(|result| result.capture))
    }

    pub fn capture_pending_with_status(
        &self,
        input: &WorkflowRefinementCaptureInput,
    ) -> Result<Option<WorkflowRefinementCoordinatorResult>, WorkflowRefinementCoordinatorError>
    {
        let pending = self.control.pending_refinement_for_binding(
            &input.surface,
            &input.conversation_key,
            &input.actor,
            input.captured_at_unix_ms,
        )?;
        let (request, replayed) = match pending {
            Some(request) => (request, false),
            None => match self.control.refinement_for_captured_message(
                &input.surface,
                &input.conversation_key,
                &input.actor,
                &input.captured_message_id,
            )? {
                Some(request) => (request, true),
                None => return Ok(None),
            },
        };
        let instruction = validate_instruction(&input.instruction)?;
        let parent = self
            .control
            .get(&request.proposal_id)?
            .ok_or_else(|| WorkflowLearningControlError::NotFound(request.proposal_id.clone()))?;
        let staged_parent = load_exact_parent(&self.staging, &request, &parent)?;
        let mut group: WorkflowGroupAnalysis = serde_json::from_str(&parent.evidence_json)?;
        if group.signature != parent.workflow_signature
            || WorkflowDraftKind::from_classification(group.classification)
                != Some(staged_parent.manifest.kind)
        {
            return Err(WorkflowRefinementCoordinatorError::InvalidInput(
                "parent evidence does not match its staged draft".to_string(),
            ));
        }

        let child_signature = child_signature(&request, instruction);
        group.signature.clone_from(&child_signature);
        group.eligible = true;
        group.reasons.clear();
        group.explicit_reuse_request = true;
        let evidence_json = bounded_json(&group)?;
        let identity_hash = digest_hex(request.id.as_bytes());
        let child_id = format!("wl-ref-{identity_hash}");
        let draft_job_id = format!("wlr-{identity_hash}-draft");
        let actor = request.actor.clone();
        let transition = |from, version, to, suffix| WorkflowProposalTransition {
            proposal_id: child_id.clone(),
            expected_state: from,
            expected_version: version,
            expected_revision_sha256: None,
            to_state: to,
            actor: actor.clone(),
            reason: format!("operator refinement child {suffix}"),
            idempotency_key: format!("{child_id}:{suffix}"),
            snoozed_until_unix_ms: None,
            occurred_at_unix_ms: input.captured_at_unix_ms,
        };
        let draft_job = new_refinement_draft_job(
            &draft_job_id,
            &child_id,
            &group,
            WorkflowRefinementJobLink {
                request_id: request.id.clone(),
                expected_request_version: QUEUED_REFINEMENT_VERSION,
                parent_proposal_id: parent.id.clone(),
                parent_proposal_version: parent.state_version,
                parent_revision_sha256: staged_parent.manifest.revision_sha256.clone(),
                parent_artifact_sha256: staged_parent.manifest.artifact_sha256.clone(),
                parent_staging_job_id: staged_parent.manifest.job_id.clone(),
            },
            input.captured_at_unix_ms,
        )?;
        let message_hash = digest_hex(input.captured_message_id.as_bytes());
        let request_id = request.id.clone();
        let capture_idempotency_key = format!("{request_id}:capture:{message_hash}");
        let eligible_transition = transition(
            WorkflowProposalState::Observed,
            0,
            WorkflowProposalState::Eligible,
            "eligible",
        );
        let drafting_transition = transition(
            WorkflowProposalState::Eligible,
            1,
            WorkflowProposalState::Drafting,
            "drafting",
        );
        let result =
            self.control
                .capture_refinement_and_enqueue_draft(&CaptureWorkflowRefinement {
                    request_id,
                    expected_request_version: AWAITING_INPUT_VERSION,
                    actor: actor.clone(),
                    instruction: instruction.to_string(),
                    captured_message_id: input.captured_message_id.clone(),
                    child_proposal: NewWorkflowProposal {
                        id: child_id.clone(),
                        idempotency_key: format!("{child_id}:observed"),
                        workflow_signature: child_signature,
                        source_agent_id: parent.source_agent_id,
                        origin_channel: Some(request.surface),
                        evidence_json,
                        created_at_unix_ms: input.captured_at_unix_ms,
                    },
                    eligible_transition,
                    drafting_transition,
                    draft_job,
                    idempotency_key: capture_idempotency_key,
                    captured_at_unix_ms: input.captured_at_unix_ms,
                })?;
        Ok(Some(WorkflowRefinementCoordinatorResult {
            capture: result,
            replayed,
        }))
    }
}

fn validate_instruction(value: &str) -> Result<&str, WorkflowRefinementCoordinatorError> {
    let value = value.trim();
    let len = value.chars().count();
    if len < 4 || len > MAX_INSTRUCTION_CHARS || value.contains('\0') {
        return Err(WorkflowRefinementCoordinatorError::InvalidInput(
            "instruction length or content is invalid".to_string(),
        ));
    }
    if let Some(secret_kind) = crate::memory_policy::scan_for_secrets(value) {
        return Err(WorkflowRefinementCoordinatorError::InvalidInput(format!(
            "instruction contains secret-like material ({secret_kind})"
        )));
    }
    Ok(value)
}

fn load_exact_parent(
    staging: &WorkflowStagingRoot,
    request: &WorkflowRefinementRecord,
    parent: &WorkflowProposalRecord,
) -> Result<LoadedStagedWorkflowDraft, WorkflowRefinementCoordinatorError> {
    if parent.state != WorkflowProposalState::Proposed
        || parent.state_version != request.expected_proposal_version
        || parent.revision_sha256.as_deref() != Some(request.revision_sha256.as_str())
    {
        return Err(WorkflowRefinementCoordinatorError::InvalidInput(
            "parent proposal changed before refinement capture".to_string(),
        ));
    }
    let staging_job_id = parent.staging_job_id.as_deref().ok_or_else(|| {
        WorkflowRefinementCoordinatorError::InvalidInput(
            "parent proposal has no staging identity".to_string(),
        )
    })?;
    let staged = staging.load_exact(staging_job_id, &request.revision_sha256)?;
    if staged.manifest.workflow_signature != parent.workflow_signature
        || staged.manifest.artifact_sha256.as_str()
            != parent.artifact_sha256.as_deref().unwrap_or_default()
        || Some(staged.manifest.kind) != parent.kind.map(artifact_draft_kind)
        || Some(staged.manifest.name.as_str()) != parent.name.as_deref()
    {
        return Err(WorkflowRefinementCoordinatorError::InvalidInput(
            "parent staging identity does not match SQLite".to_string(),
        ));
    }
    Ok(staged)
}

fn artifact_draft_kind(
    kind: captain_memory::workflow_learning_control::WorkflowArtifactKind,
) -> WorkflowDraftKind {
    match kind {
        captain_memory::workflow_learning_control::WorkflowArtifactKind::Skill => {
            WorkflowDraftKind::Skill
        }
        captain_memory::workflow_learning_control::WorkflowArtifactKind::Capspec => {
            WorkflowDraftKind::Capspec
        }
        captain_memory::workflow_learning_control::WorkflowArtifactKind::Automation => {
            WorkflowDraftKind::Automation
        }
        captain_memory::workflow_learning_control::WorkflowArtifactKind::Refinement => {
            WorkflowDraftKind::Refinement
        }
    }
}

fn child_signature(request: &WorkflowRefinementRecord, instruction: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"captain-workflow-refinement-v1\0");
    hasher.update(request.proposal_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(request.revision_sha256.as_bytes());
    hasher.update(b"\0");
    hasher.update(instruction.as_bytes());
    hex::encode(hasher.finalize())
}

fn digest_hex(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}
