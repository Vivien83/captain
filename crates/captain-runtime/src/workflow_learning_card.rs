//! Exact, channel-neutral projection of one durable workflow proposal.

use std::collections::BTreeSet;

use captain_memory::workflow_learning_control::{
    workflow_operator_token, WorkflowArtifactKind, WorkflowIsolatedTestStatus,
    WorkflowProposalRecord, WorkflowProposalState,
};
use captain_types::workflow_learning::{
    ProposalCard, ProposalCardAction, ProposalCardEvidence, ProposalCardKind, ProposalCardModel,
    ProposalCardRisk, ProposalCardState, ProposalCardStep, ProposalCardValidationFact,
    ProposalIsolatedTest, ProposalIsolatedTestStatus, WorkflowIsolatedTestReport,
    PROPOSAL_CARD_SCHEMA_VERSION,
};
use serde::Deserialize;

use crate::workflow_learning_analysis::WorkflowGroupAnalysis;
use crate::workflow_learning_proposer::{ActiveModelIdentity, WorkflowDraftKind};
use crate::workflow_learning_staging::{WorkflowStagingError, WorkflowStagingRoot};

const VALIDATION_SCHEMA_VERSION: u16 = 1;
const REQUIRED_VALIDATION_CHECKS: [&str; 5] = [
    "whole_response_schema",
    "native_artifact_parser",
    "secret_scan",
    "path_and_identifier_policy",
    "immutable_staging_hashes",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidationEvidence {
    schema_version: u16,
    checks: Vec<String>,
    model: ActiveModelIdentity,
    limitations: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkflowProposalCardError {
    #[error("proposal is not ready for an operator card: {0}")]
    Incomplete(String),
    #[error("proposal card identity mismatch: {0}")]
    IdentityMismatch(String),
    #[error("proposal card evidence is invalid: {0}")]
    InvalidEvidence(String),
    #[error(transparent)]
    Staging(#[from] WorkflowStagingError),
}

pub fn project_workflow_proposal_card(
    proposal: &WorkflowProposalRecord,
    staging: &WorkflowStagingRoot,
) -> Result<ProposalCard, WorkflowProposalCardError> {
    let revision_sha256 = required(proposal.revision_sha256.as_deref(), "revision_sha256")?;
    let persisted_operator_token = required(proposal.operator_token.as_deref(), "operator_token")?;
    let expected_operator_token = workflow_operator_token(revision_sha256)
        .map_err(|error| WorkflowProposalCardError::InvalidEvidence(error.to_string()))?;
    if persisted_operator_token != expected_operator_token {
        return Err(WorkflowProposalCardError::IdentityMismatch(
            "persisted operator token does not match the immutable revision".to_string(),
        ));
    }
    let staging_job_id = required(proposal.staging_job_id.as_deref(), "staging_job_id")?;
    let artifact_sha256 = required(proposal.artifact_sha256.as_deref(), "artifact_sha256")?;
    let proposal_kind = proposal
        .kind
        .ok_or_else(|| WorkflowProposalCardError::Incomplete("kind is missing".to_string()))?;
    let proposal_name = required(proposal.name.as_deref(), "name")?;
    let validation_json = required(proposal.validation_json.as_deref(), "validation_json")?;

    let staged = staging.load_exact(staging_job_id, revision_sha256)?;
    if staged.manifest.workflow_signature != proposal.workflow_signature
        || staged.manifest.revision_sha256 != revision_sha256
        || staged.manifest.artifact_sha256 != artifact_sha256
        || staged.manifest.name != proposal_name
        || map_draft_kind(staged.manifest.kind) != map_artifact_kind(proposal_kind)
    {
        return Err(WorkflowProposalCardError::IdentityMismatch(
            "SQLite metadata does not identify the exact staged draft".to_string(),
        ));
    }

    let evidence: WorkflowGroupAnalysis = serde_json::from_str(&proposal.evidence_json)
        .map_err(|error| WorkflowProposalCardError::InvalidEvidence(error.to_string()))?;
    if evidence.signature != proposal.workflow_signature {
        return Err(WorkflowProposalCardError::IdentityMismatch(
            "evidence signature differs from the proposal".to_string(),
        ));
    }
    let validation: ValidationEvidence = serde_json::from_str(validation_json)
        .map_err(|error| WorkflowProposalCardError::InvalidEvidence(error.to_string()))?;
    validate_objective_evidence(
        &validation,
        &staged.manifest.model,
        &staged.manifest.draft.limitations,
    )?;

    let risk = classify_workflow_risk(&staged.manifest.draft.required_capabilities);
    let isolated_test =
        project_isolated_test(proposal, proposal_kind, proposal_name, artifact_sha256)?;
    let test_passed = isolated_test
        .as_ref()
        .is_some_and(|test| test.status == ProposalIsolatedTestStatus::Passed);
    let recommended_action = if risk == ProposalCardRisk::ReadOnly || test_passed {
        ProposalCardAction::Activate
    } else {
        ProposalCardAction::Test
    };
    let available_actions = proposal_actions(proposal.state, recommended_action, test_passed);

    Ok(ProposalCard {
        schema_version: PROPOSAL_CARD_SCHEMA_VERSION,
        proposal_id: proposal.id.clone(),
        lookup_token: persisted_operator_token.to_string(),
        decision_version: proposal.state_version,
        revision_sha256: revision_sha256.to_string(),
        state: map_state(proposal.state),
        kind: map_artifact_kind(proposal_kind),
        name: staged.manifest.draft.name,
        purpose: staged.manifest.draft.purpose,
        trigger: staged.manifest.draft.trigger,
        evidence: ProposalCardEvidence {
            occurrences: bounded_u32(evidence.occurrence_count),
            distinct_turns: bounded_u32(evidence.distinct_turn_count),
            distinct_sessions: bounded_u32(evidence.distinct_session_count),
            explicit_reuse_request: evidence.explicit_reuse_request,
        },
        steps: evidence
            .canonical
            .nodes
            .into_iter()
            .map(|node| ProposalCardStep {
                index: node.index,
                tool_name: node.tool_name,
                role: node.role,
                dependencies: node.dependencies,
            })
            .collect(),
        validation: validation
            .checks
            .into_iter()
            .map(|code| ProposalCardValidationFact { code, passed: true })
            .collect(),
        validation_limitations: validation.limitations,
        isolated_test,
        validated_by: ProposalCardModel {
            provider: validation.model.provider,
            model: validation.model.model,
        },
        required_authority: staged.manifest.draft.required_capabilities,
        expected_benefit: staged.manifest.draft.expected_benefit,
        risk,
        recommended_action,
        available_actions,
    })
}

pub fn classify_workflow_risk(required_authority: &[String]) -> ProposalCardRisk {
    if required_authority.is_empty() {
        return ProposalCardRisk::Unknown;
    }
    if required_authority
        .iter()
        .any(|authority| authority_is_mutating(authority))
    {
        return ProposalCardRisk::Mutation;
    }
    if required_authority
        .iter()
        .all(|authority| authority_is_read_only(authority))
    {
        ProposalCardRisk::ReadOnly
    } else {
        ProposalCardRisk::Unknown
    }
}

fn validate_objective_evidence(
    validation: &ValidationEvidence,
    staged_model: &ActiveModelIdentity,
    staged_limitations: &[String],
) -> Result<(), WorkflowProposalCardError> {
    if validation.schema_version != VALIDATION_SCHEMA_VERSION {
        return Err(WorkflowProposalCardError::InvalidEvidence(format!(
            "unsupported validation schema {}",
            validation.schema_version
        )));
    }
    if &validation.model != staged_model || validation.limitations != staged_limitations {
        return Err(WorkflowProposalCardError::IdentityMismatch(
            "validation facts do not match the staged draft".to_string(),
        ));
    }
    let actual = validation
        .checks
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = REQUIRED_VALIDATION_CHECKS
        .into_iter()
        .collect::<BTreeSet<_>>();
    if actual != expected || validation.checks.len() != expected.len() {
        return Err(WorkflowProposalCardError::InvalidEvidence(
            "objective validation checks are incomplete, duplicated, or unknown".to_string(),
        ));
    }
    Ok(())
}

fn required<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str, WorkflowProposalCardError> {
    value
        .filter(|value| !value.is_empty())
        .ok_or_else(|| WorkflowProposalCardError::Incomplete(format!("{field} is missing")))
}

fn bounded_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

pub(crate) fn map_artifact_kind(kind: WorkflowArtifactKind) -> ProposalCardKind {
    match kind {
        WorkflowArtifactKind::Skill => ProposalCardKind::Skill,
        WorkflowArtifactKind::Capspec => ProposalCardKind::Capspec,
        WorkflowArtifactKind::Automation => ProposalCardKind::Automation,
        WorkflowArtifactKind::Refinement => ProposalCardKind::Refinement,
    }
}

fn map_draft_kind(kind: WorkflowDraftKind) -> ProposalCardKind {
    match kind {
        WorkflowDraftKind::Skill => ProposalCardKind::Skill,
        WorkflowDraftKind::Capspec => ProposalCardKind::Capspec,
        WorkflowDraftKind::Automation => ProposalCardKind::Automation,
        WorkflowDraftKind::Refinement => ProposalCardKind::Refinement,
    }
}

pub(crate) fn map_state(state: WorkflowProposalState) -> ProposalCardState {
    match state {
        WorkflowProposalState::Observed => ProposalCardState::Observed,
        WorkflowProposalState::Eligible => ProposalCardState::Eligible,
        WorkflowProposalState::Drafting => ProposalCardState::Drafting,
        WorkflowProposalState::Validating => ProposalCardState::Validating,
        WorkflowProposalState::Proposed => ProposalCardState::Proposed,
        WorkflowProposalState::Dismissed => ProposalCardState::Dismissed,
        WorkflowProposalState::Snoozed => ProposalCardState::Snoozed,
        WorkflowProposalState::Superseded => ProposalCardState::Superseded,
        WorkflowProposalState::ApprovedPendingInstall => ProposalCardState::ApprovedPendingInstall,
        WorkflowProposalState::ActiveCanary => ProposalCardState::ActiveCanary,
        WorkflowProposalState::Active => ProposalCardState::Active,
        WorkflowProposalState::Rejected => ProposalCardState::Rejected,
        WorkflowProposalState::InstallFailed => ProposalCardState::InstallFailed,
        WorkflowProposalState::RolledBack => ProposalCardState::RolledBack,
    }
}

fn proposal_actions(
    state: WorkflowProposalState,
    recommended: ProposalCardAction,
    test_passed: bool,
) -> Vec<ProposalCardAction> {
    if state != WorkflowProposalState::Proposed {
        return vec![ProposalCardAction::Details];
    }
    let mut actions = vec![recommended];
    if test_passed && recommended == ProposalCardAction::Activate {
        actions.push(ProposalCardAction::Test);
    }
    actions.extend([
        ProposalCardAction::Details,
        ProposalCardAction::Edit,
        ProposalCardAction::Later,
        ProposalCardAction::Ignore,
    ]);
    actions
}

fn project_isolated_test(
    proposal: &WorkflowProposalRecord,
    kind: WorkflowArtifactKind,
    name: &str,
    artifact_sha256: &str,
) -> Result<Option<ProposalIsolatedTest>, WorkflowProposalCardError> {
    let Some(test) = &proposal.isolated_test else {
        return Ok(None);
    };
    let revision = proposal.revision_sha256.as_deref().ok_or_else(|| {
        WorkflowProposalCardError::Incomplete("revision_sha256 is missing".to_string())
    })?;
    if test.proposal_id != proposal.id || test.revision_sha256 != revision {
        return Err(WorkflowProposalCardError::IdentityMismatch(
            "isolated test identifies another proposal revision".to_string(),
        ));
    }
    let status = match test.status {
        WorkflowIsolatedTestStatus::Queued => ProposalIsolatedTestStatus::Queued,
        WorkflowIsolatedTestStatus::Passed => ProposalIsolatedTestStatus::Passed,
        WorkflowIsolatedTestStatus::Failed => ProposalIsolatedTestStatus::Failed,
    };
    let (checks, completed_at_unix_ms) = match test.status {
        WorkflowIsolatedTestStatus::Queued => {
            if test.result_json.is_some() || test.completed_at_unix_ms.is_some() {
                return Err(WorkflowProposalCardError::InvalidEvidence(
                    "queued isolated test already contains completion evidence".to_string(),
                ));
            }
            (Vec::new(), None)
        }
        WorkflowIsolatedTestStatus::Passed | WorkflowIsolatedTestStatus::Failed => {
            let report: WorkflowIsolatedTestReport =
                serde_json::from_str(test.result_json.as_deref().ok_or_else(|| {
                    WorkflowProposalCardError::InvalidEvidence(
                        "completed isolated test has no report".to_string(),
                    )
                })?)
                .map_err(|error| WorkflowProposalCardError::InvalidEvidence(error.to_string()))?;
            let completed_at = test.completed_at_unix_ms.ok_or_else(|| {
                WorkflowProposalCardError::InvalidEvidence(
                    "completed isolated test has no timestamp".to_string(),
                )
            })?;
            if report.schema_version != 1
                || report.proposal_id != proposal.id
                || report.revision_sha256 != revision
                || report.artifact_sha256 != artifact_sha256
                || report.kind != map_artifact_kind(kind)
                || report.name != name
                || report.passed != (test.status == WorkflowIsolatedTestStatus::Passed)
                || report.completed_at_unix_ms != completed_at
                || report.checks.is_empty()
                || report
                    .checks
                    .iter()
                    .any(|check| check.code.trim().is_empty())
            {
                return Err(WorkflowProposalCardError::IdentityMismatch(
                    "isolated test report does not match its durable revision".to_string(),
                ));
            }
            (report.checks, Some(completed_at))
        }
    };
    Ok(Some(ProposalIsolatedTest {
        status,
        revision_sha256: test.revision_sha256.clone(),
        job_id: test.job_id.clone(),
        checks,
        completed_at_unix_ms,
    }))
}

fn authority_is_mutating(authority: &str) -> bool {
    let value = authority.trim().to_ascii_lowercase();
    [
        "write",
        "create",
        "delete",
        "remove",
        "update",
        "install",
        "execute",
        "exec",
        "shell",
        "send",
        "publish",
        "post",
        "put",
        "patch",
        "commit",
        "push",
        "deploy",
        "restart",
        "kill",
        "secret",
        "config",
        "browser_interact",
    ]
    .iter()
    .any(|marker| token_contains(&value, marker))
}

fn authority_is_read_only(authority: &str) -> bool {
    let value = authority.trim().to_ascii_lowercase();
    [
        "read", "list", "get", "search", "fetch", "inspect", "status", "health", "query", "recall",
        "docs", "find", "view",
    ]
    .iter()
    .any(|marker| token_contains(&value, marker))
}

fn token_contains(value: &str, marker: &str) -> bool {
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .any(|part| part == marker)
}
