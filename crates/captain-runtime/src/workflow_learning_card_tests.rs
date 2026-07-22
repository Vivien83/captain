use captain_memory::workflow_learning_control::{
    WorkflowArtifactKind, WorkflowIsolatedTestRecord, WorkflowIsolatedTestStatus,
    WorkflowProposalRecord, WorkflowProposalState,
};
use captain_types::workflow_learning::{
    ProposalCardAction, ProposalCardKind, ProposalCardRisk, ProposalCardState,
    ProposalIsolatedTestCheck, ProposalIsolatedTestStatus, WorkflowIsolatedTestReport,
};
use serde_json::json;

use crate::workflow_learning_analysis::{
    CanonicalWorkflow, CanonicalWorkflowNode, WorkflowClassification, WorkflowGroupAnalysis,
    WorkflowScope,
};
use crate::workflow_learning_card::{classify_workflow_risk, project_workflow_proposal_card};
use crate::workflow_learning_proposer::{
    ActiveModelIdentity, WorkflowDraft, WorkflowDraftArtifact, WorkflowDraftKind,
};
use crate::workflow_learning_staging::{StageWorkflowDraftRequest, WorkflowStagingRoot};

fn model() -> ActiveModelIdentity {
    ActiveModelIdentity {
        provider: "codex".to_string(),
        model: "gpt-5.6-sol".to_string(),
    }
}

fn draft(required_capabilities: Vec<String>) -> WorkflowDraft {
    WorkflowDraft {
        schema_version: 1,
        kind: WorkflowDraftKind::Skill,
        name: "sourced-research".to_string(),
        purpose: "Research a subject using authoritative sources.".to_string(),
        trigger: "Use when a current answer requires sources.".to_string(),
        artifact: WorkflowDraftArtifact::SkillMarkdown {
            source: "---\nname: sourced-research\ndescription: Source-backed research\n---\n# Workflow\nSearch, compare, and cite authoritative sources.".to_string(),
        },
        required_capabilities,
        expected_benefit: "Repeatable research with explicit evidence.".to_string(),
        limitations: vec!["Human review remains required for high-stakes claims.".to_string()],
    }
}

fn group(signature: &str) -> WorkflowGroupAnalysis {
    WorkflowGroupAnalysis {
        signature: signature.to_string(),
        classification: WorkflowClassification::Skill,
        eligible: true,
        reasons: vec![],
        occurrence_count: 3,
        distinct_turn_count: 3,
        distinct_session_count: 2,
        explicit_reuse_request: false,
        scope: WorkflowScope::Global,
        episode_ids: vec!["episode-1".into(), "episode-2".into(), "episode-3".into()],
        intent_samples: vec!["research this".into()],
        canonical: CanonicalWorkflow {
            version: 1,
            nodes: vec![CanonicalWorkflowNode {
                index: 0,
                tool_name: "web_search".to_string(),
                role: "research".to_string(),
                input_shape: json!({"query":"<text>"}),
                effect_class: "read".to_string(),
                verification_shape: "sources".to_string(),
                dependencies: vec![],
            }],
        },
    }
}

fn staged_proposal(
    required_capabilities: Vec<String>,
) -> (
    tempfile::TempDir,
    WorkflowStagingRoot,
    WorkflowProposalRecord,
) {
    let temp = tempfile::tempdir().unwrap();
    let staging = WorkflowStagingRoot::new(temp.path().join("captain-home")).unwrap();
    let signature = "a".repeat(64);
    let draft = draft(required_capabilities);
    let receipt = staging
        .stage(StageWorkflowDraftRequest {
            job_id: "draft-job-1",
            workflow_signature: &signature,
            draft: &draft,
            active_model: &model(),
        })
        .unwrap();
    let validation_json = json!({
        "schema_version": 1,
        "checks": [
            "whole_response_schema",
            "native_artifact_parser",
            "secret_scan",
            "path_and_identifier_policy",
            "immutable_staging_hashes"
        ],
        "model": model(),
        "limitations": draft.limitations,
    })
    .to_string();
    let revision_sha256 = receipt.revision_sha256.clone();
    let proposal = WorkflowProposalRecord {
        id: "proposal-1".to_string(),
        idempotency_key: "proposal-1:observed".to_string(),
        workflow_signature: signature.clone(),
        state: WorkflowProposalState::Proposed,
        state_version: 4,
        revision_sha256: Some(revision_sha256.clone()),
        operator_token: Some(revision_sha256[..20].to_string()),
        artifact_sha256: Some(receipt.artifact_sha256),
        staging_job_id: Some("draft-job-1".to_string()),
        kind: Some(WorkflowArtifactKind::Skill),
        name: Some("sourced-research".to_string()),
        source_agent_id: "captain".to_string(),
        origin_channel: Some("telegram".to_string()),
        evidence_json: serde_json::to_string(&group(&signature)).unwrap(),
        validation_json: Some(validation_json),
        isolated_test: None,
        snoozed_until_unix_ms: None,
        last_error_code: None,
        last_error_message: None,
        created_at_unix_ms: 1_000,
        updated_at_unix_ms: 2_000,
    };
    (temp, staging, proposal)
}

#[test]
fn exact_read_only_revision_projects_one_shared_operator_card() {
    let (_temp, staging, proposal) = staged_proposal(vec!["web_search".to_string()]);

    let card = project_workflow_proposal_card(&proposal, &staging).unwrap();

    assert_eq!(card.state, ProposalCardState::Proposed);
    assert_eq!(card.risk, ProposalCardRisk::ReadOnly);
    assert_eq!(card.recommended_action, ProposalCardAction::Activate);
    assert_eq!(card.evidence.occurrences, 3);
    assert_eq!(card.evidence.distinct_sessions, 2);
    assert_eq!(card.steps[0].tool_name, "web_search");
    assert_eq!(card.validation.len(), 5);
    assert!(card.validation.iter().all(|fact| fact.passed));
    assert_eq!(card.lookup_token.len(), 20);
    assert_eq!(
        card.lookup_token,
        proposal.operator_token.as_deref().unwrap()
    );
    assert_eq!(card.available_actions[0], ProposalCardAction::Activate);
}

#[test]
fn mutation_or_unknown_authority_recommends_a_test() {
    assert_eq!(
        classify_workflow_risk(&["file_write".to_string()]),
        ProposalCardRisk::Mutation
    );
    assert_eq!(
        classify_workflow_risk(&["custom_provider".to_string()]),
        ProposalCardRisk::Unknown
    );
    assert_eq!(classify_workflow_risk(&[]), ProposalCardRisk::Unknown);

    let (_temp, staging, proposal) = staged_proposal(vec!["file_write".to_string()]);
    let card = project_workflow_proposal_card(&proposal, &staging).unwrap();
    assert_eq!(card.recommended_action, ProposalCardAction::Test);
    assert_eq!(card.available_actions[0], ProposalCardAction::Test);
}

#[test]
fn exact_passed_test_unlocks_activation_but_keeps_retest_available() {
    let (_temp, staging, mut proposal) = staged_proposal(vec!["file_write".to_string()]);
    let revision = proposal.revision_sha256.clone().unwrap();
    let artifact = proposal.artifact_sha256.clone().unwrap();
    let completed_at = 3_000;
    let report = WorkflowIsolatedTestReport {
        schema_version: 1,
        proposal_id: proposal.id.clone(),
        revision_sha256: revision.clone(),
        artifact_sha256: artifact,
        kind: ProposalCardKind::Skill,
        name: proposal.name.clone().unwrap(),
        passed: true,
        checks: vec![ProposalIsolatedTestCheck {
            code: "native_skill_registry".to_string(),
            passed: true,
            detail: "exact private artifact loaded".to_string(),
        }],
        completed_at_unix_ms: completed_at,
    };
    proposal.state_version = 6;
    proposal.isolated_test = Some(WorkflowIsolatedTestRecord {
        id: "test-1".to_string(),
        idempotency_key: "test-1:key".to_string(),
        proposal_id: proposal.id.clone(),
        revision_sha256: revision,
        job_id: "test-job-1".to_string(),
        status: WorkflowIsolatedTestStatus::Passed,
        requested_by: "telegram:42".to_string(),
        result_json: Some(serde_json::to_string(&report).unwrap()),
        requested_at_unix_ms: 2_500,
        completed_at_unix_ms: Some(completed_at),
        updated_at_unix_ms: completed_at,
    });

    let card = project_workflow_proposal_card(&proposal, &staging).unwrap();
    assert_eq!(card.decision_version, 6);
    assert_eq!(card.recommended_action, ProposalCardAction::Activate);
    assert!(card
        .available_actions
        .contains(&ProposalCardAction::Activate));
    assert!(card.available_actions.contains(&ProposalCardAction::Test));
    assert_eq!(
        card.isolated_test.unwrap().status,
        ProposalIsolatedTestStatus::Passed
    );
}

#[test]
fn stale_or_incomplete_identity_never_projects_an_actionable_card() {
    let (_temp, staging, mut proposal) = staged_proposal(vec!["web_search".to_string()]);
    let exact_artifact_sha256 = proposal.artifact_sha256.clone();
    proposal.artifact_sha256 = Some("b".repeat(64));
    assert!(project_workflow_proposal_card(&proposal, &staging).is_err());

    proposal.artifact_sha256 = None;
    assert!(project_workflow_proposal_card(&proposal, &staging).is_err());

    proposal.artifact_sha256 = exact_artifact_sha256;
    proposal.operator_token = Some("0".repeat(20));
    assert!(project_workflow_proposal_card(&proposal, &staging).is_err());
}
