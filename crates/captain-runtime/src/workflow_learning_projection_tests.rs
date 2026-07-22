use captain_memory::workflow_learning_control::{
    NewWorkflowProposal, PublishValidatedDraft, WorkflowArtifactKind, WorkflowLearningStore,
    WorkflowProposalState, WorkflowProposalTransition,
};
use captain_memory::MemorySubstrate;
use captain_types::workflow_learning::{
    ProposalCardState, WorkflowProjectionStatus, WorkflowTimelineEntry,
};
use serde_json::json;

use crate::workflow_learning_analysis::{
    CanonicalWorkflow, CanonicalWorkflowNode, WorkflowClassification, WorkflowGroupAnalysis,
    WorkflowScope,
};
use crate::workflow_learning_projection::project_workflow_learning_list;
use crate::workflow_learning_proposer::{
    ActiveModelIdentity, WorkflowDraft, WorkflowDraftArtifact, WorkflowDraftKind,
};
use crate::workflow_learning_staging::{StageWorkflowDraftRequest, WorkflowStagingRoot};

fn store() -> WorkflowLearningStore {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    WorkflowLearningStore::new(memory.usage_conn())
}

fn create_observed(store: &WorkflowLearningStore, id: &str, signature: &str) {
    store
        .create_observed(&NewWorkflowProposal {
            id: id.to_string(),
            idempotency_key: format!("{id}:observed"),
            workflow_signature: signature.to_string(),
            source_agent_id: "captain".to_string(),
            origin_channel: Some("telegram".to_string()),
            evidence_json: serde_json::to_string(&group(signature)).unwrap(),
            created_at_unix_ms: 1_000,
        })
        .unwrap();
}

#[test]
fn observed_workflow_is_visible_as_building_without_operator_actions() {
    let temp = tempfile::tempdir().unwrap();
    let staging = WorkflowStagingRoot::new(temp.path().join("captain-home")).unwrap();
    let store = store();
    create_observed(&store, "proposal-building", &"a".repeat(64));

    let list = project_workflow_learning_list(&store, &staging, 10).unwrap();

    assert_eq!(list.returned, 1);
    let view = &list.workflows[0];
    assert_eq!(view.state, ProposalCardState::Observed);
    assert_eq!(view.projection_status, WorkflowProjectionStatus::Building);
    assert!(view.card.is_none());
    assert!(view.projection_error.is_none());
    assert_eq!(view.timeline.len(), 1);
}

#[test]
fn validated_workflow_exposes_one_verified_card_and_ordered_history() {
    let temp = tempfile::tempdir().unwrap();
    let staging = WorkflowStagingRoot::new(temp.path().join("captain-home")).unwrap();
    let store = store();
    let signature = "b".repeat(64);
    create_observed(&store, "proposal-ready", &signature);
    for (expected, version, next, key, at) in [
        (
            WorkflowProposalState::Observed,
            0,
            WorkflowProposalState::Eligible,
            "eligible",
            1_100,
        ),
        (
            WorkflowProposalState::Eligible,
            1,
            WorkflowProposalState::Drafting,
            "drafting",
            1_200,
        ),
        (
            WorkflowProposalState::Drafting,
            2,
            WorkflowProposalState::Validating,
            "validating",
            1_300,
        ),
    ] {
        store
            .transition(&WorkflowProposalTransition {
                proposal_id: "proposal-ready".to_string(),
                expected_state: expected,
                expected_version: version,
                expected_revision_sha256: None,
                to_state: next,
                actor: "captain:test".to_string(),
                reason: "projection fixture".to_string(),
                idempotency_key: format!("proposal-ready:{key}"),
                snoozed_until_unix_ms: None,
                occurred_at_unix_ms: at,
            })
            .unwrap();
    }
    let draft = draft();
    let model = model();
    let receipt = staging
        .stage(StageWorkflowDraftRequest {
            job_id: "draft-job-view",
            workflow_signature: &signature,
            draft: &draft,
            active_model: &model,
        })
        .unwrap();
    store
        .publish_validated_draft(&PublishValidatedDraft {
            proposal_id: "proposal-ready".to_string(),
            expected_version: 3,
            staging_job_id: "draft-job-view".to_string(),
            revision_sha256: receipt.revision_sha256,
            artifact_sha256: receipt.artifact_sha256,
            kind: WorkflowArtifactKind::Skill,
            name: draft.name.clone(),
            validation_json: validation_json(&model, &draft),
            actor: "captain:validator".to_string(),
            reason: "projection validated".to_string(),
            idempotency_key: "proposal-ready:published".to_string(),
            occurred_at_unix_ms: 2_000,
        })
        .unwrap();

    let list = project_workflow_learning_list(&store, &staging, 10).unwrap();

    let view = &list.workflows[0];
    assert_eq!(view.projection_status, WorkflowProjectionStatus::Verified);
    let card = view.card.as_ref().unwrap();
    assert_eq!(card.proposal_id, "proposal-ready");
    assert_eq!(card.decision_version, 4);
    assert_eq!(card.name, "source-watch");
    assert_eq!(view.timeline.len(), 5);
    assert!(view
        .timeline
        .windows(2)
        .all(|pair| { pair[0].occurred_at_unix_ms() <= pair[1].occurred_at_unix_ms() }));
    assert!(matches!(
        view.timeline.last(),
        Some(WorkflowTimelineEntry::Proposal {
            to_state: ProposalCardState::Proposed,
            ..
        })
    ));
}

fn model() -> ActiveModelIdentity {
    ActiveModelIdentity {
        provider: "codex".to_string(),
        model: "gpt-5.6-sol".to_string(),
    }
}

fn draft() -> WorkflowDraft {
    WorkflowDraft {
        schema_version: 1,
        kind: WorkflowDraftKind::Skill,
        name: "source-watch".to_string(),
        purpose: "Research current changes from authoritative sources.".to_string(),
        trigger: "Use when a current answer needs verified sources.".to_string(),
        artifact: WorkflowDraftArtifact::SkillMarkdown {
            source: "---\nname: source-watch\ndescription: Verified research\n---\n# Workflow\nSearch and cite sources.".to_string(),
        },
        required_capabilities: vec!["web_search".to_string()],
        expected_benefit: "Repeatable source-backed research.".to_string(),
        limitations: vec!["Human review remains required.".to_string()],
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

fn validation_json(model: &ActiveModelIdentity, draft: &WorkflowDraft) -> String {
    json!({
        "schema_version": 1,
        "checks": [
            "whole_response_schema",
            "native_artifact_parser",
            "secret_scan",
            "path_and_identifier_policy",
            "immutable_staging_hashes"
        ],
        "model": model,
        "limitations": draft.limitations,
    })
    .to_string()
}
