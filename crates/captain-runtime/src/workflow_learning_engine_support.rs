use std::collections::{BTreeMap, HashMap};

use captain_memory::workflow_learning_control::{
    WorkflowArtifactKind, WorkflowLearningStore, WorkflowProposalRecord, WorkflowProposalState,
    WorkflowProposalTransition,
};
use captain_memory::workflow_learning_pipeline::WorkflowDraftCompletion;
use captain_memory::workflow_learning_queue::{NewWorkflowJob, WorkflowJobKind, WorkflowJobRecord};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::workflow_learning_analysis::{
    ExistingCapabilityKind, WorkflowAnalysisCatalog, WorkflowClassification, WorkflowGroupAnalysis,
    WorkflowRejectionReason,
};
use crate::workflow_learning_engine::WorkflowLearningEngineError;
use crate::workflow_learning_proposer::WorkflowDraftKind;
use crate::workflow_learning_staging::LoadedStagedWorkflowDraft;

pub(crate) const PAYLOAD_SCHEMA_VERSION: u16 = 1;
const RESULT_MAX_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GroupJobPayload {
    pub(crate) schema_version: u16,
    pub(crate) group: WorkflowGroupAnalysis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkflowRefinementJobLink {
    pub(crate) request_id: String,
    pub(crate) expected_request_version: u64,
    pub(crate) parent_proposal_id: String,
    pub(crate) parent_proposal_version: u64,
    pub(crate) parent_revision_sha256: String,
    pub(crate) parent_artifact_sha256: String,
    pub(crate) parent_staging_job_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RefinementDraftJobPayload {
    pub(crate) schema_version: u16,
    pub(crate) refinement: WorkflowRefinementJobLink,
    pub(crate) group: WorkflowGroupAnalysis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum DraftJobPayload {
    Discovery(GroupJobPayload),
    Refinement(RefinementDraftJobPayload),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ValidationJobPayload {
    pub(crate) schema_version: u16,
    pub(crate) group: WorkflowGroupAnalysis,
    pub(crate) draft_job_id: String,
    pub(crate) revision_sha256: String,
    pub(crate) artifact_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RefinementValidationJobPayload {
    pub(crate) schema_version: u16,
    pub(crate) refinement: WorkflowRefinementJobLink,
    pub(crate) group: WorkflowGroupAnalysis,
    pub(crate) draft_job_id: String,
    pub(crate) revision_sha256: String,
    pub(crate) artifact_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum AnyValidationJobPayload {
    Discovery(ValidationJobPayload),
    Refinement(RefinementValidationJobPayload),
}

pub(crate) fn new_group_job(
    id: &str,
    proposal_id: &str,
    kind: WorkflowJobKind,
    group: &WorkflowGroupAnalysis,
    now_unix_ms: i64,
) -> Result<NewWorkflowJob, WorkflowLearningEngineError> {
    Ok(NewWorkflowJob {
        id: id.to_string(),
        idempotency_key: format!("{id}:enqueue"),
        proposal_id: proposal_id.to_string(),
        revision_sha256: None,
        kind,
        payload_json: bounded_json(&GroupJobPayload {
            schema_version: PAYLOAD_SCHEMA_VERSION,
            group: group.clone(),
        })?,
        max_attempts: 3,
        run_after_unix_ms: now_unix_ms,
        created_at_unix_ms: now_unix_ms,
    })
}

pub(crate) fn new_refinement_draft_job(
    id: &str,
    proposal_id: &str,
    group: &WorkflowGroupAnalysis,
    refinement: WorkflowRefinementJobLink,
    now_unix_ms: i64,
) -> Result<NewWorkflowJob, WorkflowLearningEngineError> {
    let payload = RefinementDraftJobPayload {
        schema_version: PAYLOAD_SCHEMA_VERSION,
        refinement,
        group: group.clone(),
    };
    validate_refinement_draft_payload(&payload)?;
    Ok(NewWorkflowJob {
        id: id.to_string(),
        idempotency_key: format!("{id}:enqueue"),
        proposal_id: proposal_id.to_string(),
        revision_sha256: None,
        kind: WorkflowJobKind::Draft,
        payload_json: bounded_json(&DraftJobPayload::Refinement(payload))?,
        max_attempts: 3,
        run_after_unix_ms: now_unix_ms,
        created_at_unix_ms: now_unix_ms,
    })
}

pub(crate) fn draft_completion_request(
    worker_id: &str,
    job: &WorkflowJobRecord,
    proposal: &WorkflowProposalRecord,
    group: &WorkflowGroupAnalysis,
    staged: &LoadedStagedWorkflowDraft,
    now_unix_ms: i64,
    recovered: bool,
) -> Result<WorkflowDraftCompletion, WorkflowLearningEngineError> {
    require_state(proposal, WorkflowProposalState::Drafting)?;
    verify_staged_identity(staged, group)?;
    let validation_id = format!("{}-validate", proposal.id);
    let validation_payload = bounded_json(&ValidationJobPayload {
        schema_version: PAYLOAD_SCHEMA_VERSION,
        group: group.clone(),
        draft_job_id: job.id.clone(),
        revision_sha256: staged.manifest.revision_sha256.clone(),
        artifact_sha256: staged.manifest.artifact_sha256.clone(),
    })?;
    let result_json = bounded_json(&serde_json::json!({
        "schema_version": PAYLOAD_SCHEMA_VERSION,
        "staged": true,
        "recovered_from_staging": recovered,
        "revision_sha256": staged.manifest.revision_sha256,
        "artifact_sha256": staged.manifest.artifact_sha256,
        "model": staged.manifest.model,
    }))?;
    Ok(WorkflowDraftCompletion {
        job_id: job.id.clone(),
        worker: worker_id.to_string(),
        result_json: Some(result_json),
        proposal_transition: WorkflowProposalTransition {
            proposal_id: proposal.id.clone(),
            expected_state: WorkflowProposalState::Drafting,
            expected_version: proposal.state_version,
            expected_revision_sha256: None,
            to_state: WorkflowProposalState::Validating,
            actor: worker_id.to_string(),
            reason: if recovered {
                "unique staged draft recovered after interrupted model job".to_string()
            } else {
                "model draft staged immutably for deterministic validation".to_string()
            },
            idempotency_key: format!("{}:validating", proposal.id),
            snoozed_until_unix_ms: None,
            occurred_at_unix_ms: now_unix_ms,
        },
        validation_job: NewWorkflowJob {
            id: validation_id.clone(),
            idempotency_key: format!("{validation_id}:enqueue"),
            proposal_id: proposal.id.clone(),
            revision_sha256: None,
            kind: WorkflowJobKind::Validate,
            payload_json: validation_payload,
            max_attempts: 3,
            run_after_unix_ms: now_unix_ms,
            created_at_unix_ms: now_unix_ms,
        },
        completed_at_unix_ms: now_unix_ms,
    })
}

pub(crate) fn refinement_draft_completion_request(
    worker_id: &str,
    job: &WorkflowJobRecord,
    proposal: &WorkflowProposalRecord,
    payload: &RefinementDraftJobPayload,
    staged: &LoadedStagedWorkflowDraft,
    now_unix_ms: i64,
    recovered: bool,
) -> Result<WorkflowDraftCompletion, WorkflowLearningEngineError> {
    require_state(proposal, WorkflowProposalState::Drafting)?;
    verify_staged_identity(staged, &payload.group)?;
    let validation_id = format!("{}-validate", proposal.id);
    let validation_payload = bounded_json(&AnyValidationJobPayload::Refinement(
        RefinementValidationJobPayload {
            schema_version: PAYLOAD_SCHEMA_VERSION,
            refinement: payload.refinement.clone(),
            group: payload.group.clone(),
            draft_job_id: job.id.clone(),
            revision_sha256: staged.manifest.revision_sha256.clone(),
            artifact_sha256: staged.manifest.artifact_sha256.clone(),
        },
    ))?;
    let result_json = bounded_json(&serde_json::json!({
        "schema_version": PAYLOAD_SCHEMA_VERSION,
        "staged": true,
        "refinement_request_id": payload.refinement.request_id,
        "recovered_from_staging": recovered,
        "revision_sha256": staged.manifest.revision_sha256,
        "artifact_sha256": staged.manifest.artifact_sha256,
        "model": staged.manifest.model,
    }))?;
    Ok(WorkflowDraftCompletion {
        job_id: job.id.clone(),
        worker: worker_id.to_string(),
        result_json: Some(result_json),
        proposal_transition: WorkflowProposalTransition {
            proposal_id: proposal.id.clone(),
            expected_state: WorkflowProposalState::Drafting,
            expected_version: proposal.state_version,
            expected_revision_sha256: None,
            to_state: WorkflowProposalState::Validating,
            actor: worker_id.to_string(),
            reason: if recovered {
                "unique staged refinement recovered after interrupted model job".to_string()
            } else {
                "model refinement staged immutably for deterministic validation".to_string()
            },
            idempotency_key: format!("{}:validating", proposal.id),
            snoozed_until_unix_ms: None,
            occurred_at_unix_ms: now_unix_ms,
        },
        validation_job: NewWorkflowJob {
            id: validation_id.clone(),
            idempotency_key: format!("{validation_id}:enqueue"),
            proposal_id: proposal.id.clone(),
            revision_sha256: None,
            kind: WorkflowJobKind::Validate,
            payload_json: validation_payload,
            max_attempts: 3,
            run_after_unix_ms: now_unix_ms,
            created_at_unix_ms: now_unix_ms,
        },
        completed_at_unix_ms: now_unix_ms,
    })
}

pub(crate) fn transition(
    proposal: &WorkflowProposalRecord,
    to_state: WorkflowProposalState,
    suffix: &str,
    worker_id: &str,
    now_unix_ms: i64,
) -> WorkflowProposalTransition {
    WorkflowProposalTransition {
        proposal_id: proposal.id.clone(),
        expected_state: proposal.state,
        expected_version: proposal.state_version,
        expected_revision_sha256: proposal.revision_sha256.clone(),
        to_state,
        actor: worker_id.to_string(),
        reason: format!("workflow learning {suffix}"),
        idempotency_key: format!("{}:{suffix}", proposal.id),
        snoozed_until_unix_ms: None,
        occurred_at_unix_ms: now_unix_ms,
    }
}

pub(crate) fn required_proposal(
    store: &WorkflowLearningStore,
    proposal_id: &str,
) -> Result<WorkflowProposalRecord, WorkflowLearningEngineError> {
    store
        .get(proposal_id)?
        .ok_or_else(|| WorkflowLearningEngineError::InvalidPayload("proposal vanished".to_string()))
}

pub(crate) fn require_state(
    proposal: &WorkflowProposalRecord,
    expected: WorkflowProposalState,
) -> Result<(), WorkflowLearningEngineError> {
    if proposal.state == expected && proposal.revision_sha256.is_none() {
        Ok(())
    } else {
        Err(WorkflowLearningEngineError::InvalidPayload(format!(
            "proposal {} is {}, expected {} without revision",
            proposal.id,
            proposal.state.as_str(),
            expected.as_str()
        )))
    }
}

pub(crate) fn parse_group_payload(
    value: &str,
) -> Result<GroupJobPayload, WorkflowLearningEngineError> {
    match parse_draft_payload(value)? {
        DraftJobPayload::Discovery(payload) => Ok(payload),
        DraftJobPayload::Refinement(_) => Err(WorkflowLearningEngineError::InvalidPayload(
            "analysis jobs cannot consume refinement draft payloads".to_string(),
        )),
    }
}

pub(crate) fn parse_draft_payload(
    value: &str,
) -> Result<DraftJobPayload, WorkflowLearningEngineError> {
    let payload: DraftJobPayload = serde_json::from_str(value)?;
    match &payload {
        DraftJobPayload::Discovery(discovery) => validate_group_payload(discovery)?,
        DraftJobPayload::Refinement(refinement) => validate_refinement_draft_payload(refinement)?,
    }
    Ok(payload)
}

pub(crate) fn parse_any_validation_payload(
    value: &str,
) -> Result<AnyValidationJobPayload, WorkflowLearningEngineError> {
    let payload: AnyValidationJobPayload = serde_json::from_str(value)?;
    match &payload {
        AnyValidationJobPayload::Discovery(discovery) => validate_validation_payload(discovery)?,
        AnyValidationJobPayload::Refinement(refinement) => {
            validate_refinement_validation_payload(refinement)?
        }
    }
    Ok(payload)
}

pub(crate) fn analysis_catalog(
    proposals: &[WorkflowProposalRecord],
) -> (
    WorkflowAnalysisCatalog,
    BTreeMap<String, WorkflowProposalRecord>,
) {
    let mut catalog = WorkflowAnalysisCatalog::default();
    let mut by_signature = BTreeMap::new();
    for proposal in proposals {
        by_signature
            .entry(proposal.workflow_signature.clone())
            .or_insert_with(|| proposal.clone());
        if proposal.state == WorkflowProposalState::Active {
            if let Some(kind) = proposal.kind.and_then(existing_kind) {
                catalog
                    .existing_signatures
                    .insert(proposal.workflow_signature.clone(), kind);
            }
        } else {
            catalog
                .pending_signatures
                .insert(proposal.workflow_signature.clone());
        }
    }
    (catalog, by_signature)
}

pub(crate) fn first_group_source<'a>(
    group: &WorkflowGroupAnalysis,
    evidence: &'a HashMap<String, (String, Option<String>)>,
) -> Result<&'a (String, Option<String>), WorkflowLearningEngineError> {
    group
        .episode_ids
        .iter()
        .find_map(|id| evidence.get(id))
        .ok_or_else(|| {
            WorkflowLearningEngineError::InvalidPayload(
                "eligible group has no source episode".to_string(),
            )
        })
}

pub(crate) fn should_defer(group: &WorkflowGroupAnalysis) -> bool {
    !group.eligible
        && group.reasons.len() == 1
        && group.reasons[0] == WorkflowRejectionReason::InsufficientRecurrence
        && matches!(
            group.classification,
            WorkflowClassification::Skill
                | WorkflowClassification::Capspec
                | WorkflowClassification::Automation
                | WorkflowClassification::Refinement
        )
}

pub(crate) fn proposal_id_for(group: &WorkflowGroupAnalysis) -> String {
    let mut seed = group.signature.clone();
    for id in &group.episode_ids {
        seed.push(':');
        seed.push_str(id);
    }
    format!("wl-{}", hex::encode(Sha256::digest(seed.as_bytes())))
}

pub(crate) fn verify_staged_identity(
    staged: &LoadedStagedWorkflowDraft,
    group: &WorkflowGroupAnalysis,
) -> Result<(), WorkflowLearningEngineError> {
    if staged.manifest.workflow_signature == group.signature
        && WorkflowDraftKind::from_classification(group.classification)
            == Some(staged.manifest.kind)
    {
        Ok(())
    } else {
        Err(WorkflowLearningEngineError::InvalidPayload(
            "staged draft does not match deterministic analysis".to_string(),
        ))
    }
}

pub(crate) fn verify_validation_identity(
    staged: &LoadedStagedWorkflowDraft,
    payload: &ValidationJobPayload,
) -> Result<(), WorkflowLearningEngineError> {
    verify_staged_identity(staged, &payload.group)?;
    if staged.manifest.job_id == payload.draft_job_id
        && staged.manifest.revision_sha256 == payload.revision_sha256
        && staged.manifest.artifact_sha256 == payload.artifact_sha256
    {
        Ok(())
    } else {
        Err(WorkflowLearningEngineError::InvalidPayload(
            "validation payload does not match staged hashes".to_string(),
        ))
    }
}

pub(crate) fn verify_refinement_validation_identity(
    staged: &LoadedStagedWorkflowDraft,
    payload: &RefinementValidationJobPayload,
) -> Result<(), WorkflowLearningEngineError> {
    verify_staged_identity(staged, &payload.group)?;
    if staged.manifest.job_id == payload.draft_job_id
        && staged.manifest.revision_sha256 == payload.revision_sha256
        && staged.manifest.artifact_sha256 == payload.artifact_sha256
    {
        Ok(())
    } else {
        Err(WorkflowLearningEngineError::InvalidPayload(
            "refinement validation payload does not match staged hashes".to_string(),
        ))
    }
}

pub(crate) fn artifact_kind(kind: WorkflowDraftKind) -> WorkflowArtifactKind {
    match kind {
        WorkflowDraftKind::Skill => WorkflowArtifactKind::Skill,
        WorkflowDraftKind::Capspec => WorkflowArtifactKind::Capspec,
        WorkflowDraftKind::Automation => WorkflowArtifactKind::Automation,
        WorkflowDraftKind::Refinement => WorkflowArtifactKind::Refinement,
    }
}

pub(crate) fn bounded_json(value: &impl Serialize) -> Result<String, WorkflowLearningEngineError> {
    let encoded = serde_json::to_string(value)?;
    if encoded.len() <= RESULT_MAX_BYTES {
        Ok(encoded)
    } else {
        Err(WorkflowLearningEngineError::InvalidPayload(
            "serialized learning evidence exceeds 64 KiB".to_string(),
        ))
    }
}

pub(crate) fn bounded_error(message: &str) -> String {
    message.chars().take(2_048).collect()
}

pub(crate) fn safe_model_error_message(code: &str) -> &'static str {
    match code {
        "model_timeout" => "active model completion timed out",
        "model_completion_failed" => "active model completion failed",
        "response_too_large" => "active model response exceeded the size limit",
        "invalid_structured_output" => "active model returned invalid structured output",
        "invalid_draft" => "active model returned a draft that failed validation",
        "unsafe_refinement_instruction" => {
            "operator refinement instruction failed the secret and content policy"
        }
        _ => "active model could not produce this workflow draft",
    }
}

fn existing_kind(kind: WorkflowArtifactKind) -> Option<ExistingCapabilityKind> {
    match kind {
        WorkflowArtifactKind::Skill | WorkflowArtifactKind::Refinement => {
            Some(ExistingCapabilityKind::Skill)
        }
        WorkflowArtifactKind::Capspec => Some(ExistingCapabilityKind::Capspec),
        WorkflowArtifactKind::Automation => Some(ExistingCapabilityKind::Automation),
    }
}

fn is_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_group_payload(payload: &GroupJobPayload) -> Result<(), WorkflowLearningEngineError> {
    if payload.schema_version == PAYLOAD_SCHEMA_VERSION && payload.group.eligible {
        Ok(())
    } else {
        Err(WorkflowLearningEngineError::InvalidPayload(
            "group payload schema or eligibility is invalid".to_string(),
        ))
    }
}

fn validate_refinement_draft_payload(
    payload: &RefinementDraftJobPayload,
) -> Result<(), WorkflowLearningEngineError> {
    if payload.schema_version != PAYLOAD_SCHEMA_VERSION
        || !payload.group.eligible
        || !is_hash(&payload.group.signature)
        || validate_refinement_link(&payload.refinement).is_err()
    {
        return Err(WorkflowLearningEngineError::InvalidPayload(
            "refinement draft payload identity is invalid".to_string(),
        ));
    }
    Ok(())
}

fn validate_validation_payload(
    payload: &ValidationJobPayload,
) -> Result<(), WorkflowLearningEngineError> {
    if payload.schema_version == PAYLOAD_SCHEMA_VERSION
        && payload.group.eligible
        && safe_job_id(&payload.draft_job_id)
        && is_hash(&payload.revision_sha256)
        && is_hash(&payload.artifact_sha256)
    {
        Ok(())
    } else {
        Err(WorkflowLearningEngineError::InvalidPayload(
            "validation payload identity is invalid".to_string(),
        ))
    }
}

fn validate_refinement_validation_payload(
    payload: &RefinementValidationJobPayload,
) -> Result<(), WorkflowLearningEngineError> {
    if payload.schema_version == PAYLOAD_SCHEMA_VERSION
        && payload.group.eligible
        && is_hash(&payload.group.signature)
        && safe_job_id(&payload.draft_job_id)
        && is_hash(&payload.revision_sha256)
        && is_hash(&payload.artifact_sha256)
        && validate_refinement_link(&payload.refinement).is_ok()
    {
        Ok(())
    } else {
        Err(WorkflowLearningEngineError::InvalidPayload(
            "refinement validation payload identity is invalid".to_string(),
        ))
    }
}

fn validate_refinement_link(
    link: &WorkflowRefinementJobLink,
) -> Result<(), WorkflowLearningEngineError> {
    if safe_job_id(&link.request_id)
        && link.expected_request_version > 0
        && safe_job_id(&link.parent_proposal_id)
        && link.parent_proposal_version > 0
        && is_hash(&link.parent_revision_sha256)
        && is_hash(&link.parent_artifact_sha256)
        && safe_job_id(&link.parent_staging_job_id)
    {
        Ok(())
    } else {
        Err(WorkflowLearningEngineError::InvalidPayload(
            "refinement job link is invalid".to_string(),
        ))
    }
}

fn safe_job_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
