use captain_memory::workflow_learning_control::{
    NewWorkflowProposal, PublishValidatedDraft, WorkflowArtifactKind, WorkflowLearningStore,
    WorkflowProposalState, WorkflowProposalTransition,
};
use captain_memory::workflow_learning_refinement::{
    NewWorkflowRefinementRequest, WorkflowRefinementState,
};
use captain_memory::MemorySubstrate;
use serde_json::json;
use tempfile::TempDir;

use crate::workflow_learning_analysis::{
    CanonicalWorkflow, CanonicalWorkflowNode, WorkflowClassification, WorkflowGroupAnalysis,
    WorkflowScope,
};
use crate::workflow_learning_engine_support::{parse_draft_payload, DraftJobPayload};
use crate::workflow_learning_proposer::{
    ActiveModelIdentity, WorkflowDraft, WorkflowDraftArtifact, WorkflowDraftKind,
};
use crate::workflow_learning_refinement::{
    WorkflowRefinementCaptureInput, WorkflowRefinementCoordinator,
    WorkflowRefinementCoordinatorError,
};
use crate::workflow_learning_staging::{StageWorkflowDraftRequest, WorkflowStagingRoot};

const PARENT_ID: &str = "capture-parent";
const REQUEST_ID: &str = "capture-refinement";
const PARENT_JOB_ID: &str = "capture-parent-draft";
const ACTOR: &str = "telegram:42";
const CONVERSATION: &str = "telegram:chat:42:root";

#[test]
fn pending_message_creates_one_exact_refinement_job_and_replays() {
    let (_home, _store, coordinator, input) = fixture();

    let first = coordinator
        .capture_pending_with_status(&input)
        .unwrap()
        .unwrap();
    let replay = coordinator
        .capture_pending_with_status(&input)
        .unwrap()
        .unwrap();

    assert!(!first.replayed);
    assert!(replay.replayed);
    assert_eq!(first.capture, replay.capture);
    let first = first.capture;
    assert_eq!(first.request.state, WorkflowRefinementState::Queued);
    assert_eq!(first.request.state_version, 1);
    assert_eq!(
        first.request.instruction.as_deref(),
        Some("Ajoute un tableau final concis avec les sources.")
    );
    assert_eq!(first.child_proposal.state, WorkflowProposalState::Drafting);
    assert!(!first
        .draft_job
        .payload_json
        .contains("Ajoute un tableau final"));
    let DraftJobPayload::Refinement(payload) =
        parse_draft_payload(&first.draft_job.payload_json).unwrap()
    else {
        panic!("refinement capture produced a discovery payload");
    };
    assert_eq!(payload.refinement.request_id, REQUEST_ID);
    assert_eq!(payload.refinement.expected_request_version, 1);
    assert_eq!(payload.refinement.parent_proposal_id, PARENT_ID);
    assert_eq!(payload.refinement.parent_staging_job_id, PARENT_JOB_ID);
    assert_eq!(
        payload.group.signature,
        first.child_proposal.workflow_signature
    );
}

#[test]
fn secret_instruction_is_rejected_before_persistence() {
    let (_home, store, coordinator, mut input) = fixture();
    input.instruction =
        "Utilise token: sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string();

    assert!(matches!(
        coordinator.capture_pending(&input),
        Err(WorkflowRefinementCoordinatorError::InvalidInput(_))
    ));
    let request = store.get_refinement_request(REQUEST_ID).unwrap().unwrap();
    assert_eq!(request.state, WorkflowRefinementState::AwaitingInput);
    assert!(request.instruction.is_none());
}

#[test]
fn ordinary_message_without_matching_binding_is_not_intercepted() {
    let (_home, _store, coordinator, mut input) = fixture();
    input.conversation_key = "telegram:another-chat".to_string();
    input.instruction =
        "Utilise token: sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string();

    assert!(coordinator.capture_pending(&input).unwrap().is_none());
}

fn fixture() -> (
    TempDir,
    WorkflowLearningStore,
    WorkflowRefinementCoordinator,
    WorkflowRefinementCaptureInput,
) {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let store = WorkflowLearningStore::new(memory.usage_conn());
    let home = tempfile::tempdir().unwrap();
    let staging = WorkflowStagingRoot::new(home.path().to_path_buf()).unwrap();
    let group = group();
    let draft = draft();
    let receipt = staging
        .stage(StageWorkflowDraftRequest {
            job_id: PARENT_JOB_ID,
            workflow_signature: &group.signature,
            draft: &draft,
            active_model: &ActiveModelIdentity {
                provider: "codex".to_string(),
                model: "gpt-5.6-sol".to_string(),
            },
        })
        .unwrap();
    store
        .create_observed(&NewWorkflowProposal {
            id: PARENT_ID.to_string(),
            idempotency_key: format!("{PARENT_ID}:observed"),
            workflow_signature: group.signature.clone(),
            source_agent_id: "captain".to_string(),
            origin_channel: Some("telegram".to_string()),
            evidence_json: serde_json::to_string(&group).unwrap(),
            created_at_unix_ms: 100,
        })
        .unwrap();
    for (from, version, to, suffix) in [
        (
            WorkflowProposalState::Observed,
            0,
            WorkflowProposalState::Eligible,
            "eligible",
        ),
        (
            WorkflowProposalState::Eligible,
            1,
            WorkflowProposalState::Drafting,
            "drafting",
        ),
        (
            WorkflowProposalState::Drafting,
            2,
            WorkflowProposalState::Validating,
            "validating",
        ),
    ] {
        store
            .transition(&transition(from, version, to, suffix, 200 + version as i64))
            .unwrap();
    }
    let parent = store
        .publish_validated_draft(&PublishValidatedDraft {
            proposal_id: PARENT_ID.to_string(),
            expected_version: 3,
            staging_job_id: PARENT_JOB_ID.to_string(),
            revision_sha256: receipt.revision_sha256.clone(),
            artifact_sha256: receipt.artifact_sha256,
            kind: WorkflowArtifactKind::Skill,
            name: draft.name,
            validation_json: "{}".to_string(),
            actor: "captain:validator".to_string(),
            reason: "validated fixture".to_string(),
            idempotency_key: format!("{PARENT_ID}:proposed"),
            occurred_at_unix_ms: 300,
        })
        .unwrap();
    store
        .begin_refinement_request(&NewWorkflowRefinementRequest {
            id: REQUEST_ID.to_string(),
            idempotency_key: format!("{REQUEST_ID}:begin"),
            proposal_id: PARENT_ID.to_string(),
            revision_sha256: receipt.revision_sha256,
            expected_proposal_version: parent.state_version,
            actor: ACTOR.to_string(),
            surface: "telegram".to_string(),
            conversation_key: CONVERSATION.to_string(),
            source_message_id: Some("100".to_string()),
            language: "fr".to_string(),
            expires_at_unix_ms: 100_000,
            created_at_unix_ms: 600,
        })
        .unwrap();
    let coordinator = WorkflowRefinementCoordinator::new(store.clone(), staging);
    (
        home,
        store,
        coordinator,
        WorkflowRefinementCaptureInput {
            actor: ACTOR.to_string(),
            surface: "telegram".to_string(),
            conversation_key: CONVERSATION.to_string(),
            captured_message_id: "101".to_string(),
            instruction: "  Ajoute un tableau final concis avec les sources.  ".to_string(),
            captured_at_unix_ms: 700,
        },
    )
}

fn group() -> WorkflowGroupAnalysis {
    WorkflowGroupAnalysis {
        signature: "a".repeat(64),
        classification: WorkflowClassification::Skill,
        eligible: true,
        reasons: Vec::new(),
        occurrence_count: 3,
        distinct_turn_count: 3,
        distinct_session_count: 2,
        explicit_reuse_request: false,
        scope: WorkflowScope::Global,
        episode_ids: vec!["episode-1".to_string()],
        intent_samples: vec!["research with sources".to_string()],
        canonical: CanonicalWorkflow {
            version: 1,
            nodes: vec![CanonicalWorkflowNode {
                index: 0,
                tool_name: "web_search".to_string(),
                role: "research".to_string(),
                input_shape: json!({"query": "text"}),
                effect_class: "read".to_string(),
                verification_shape: "result_received".to_string(),
                dependencies: Vec::new(),
            }],
        },
    }
}

fn draft() -> WorkflowDraft {
    WorkflowDraft {
        schema_version: 1,
        kind: WorkflowDraftKind::Skill,
        name: "sourced-research".to_string(),
        purpose: "Research a subject and keep source-backed conclusions.".to_string(),
        trigger: "Use when a question requires current sourced research.".to_string(),
        artifact: WorkflowDraftArtifact::SkillMarkdown {
            source: "---\nname: sourced-research\ndescription: Produce source-backed research\n---\n# Workflow\nSearch authoritative sources and cite the evidence."
                .to_string(),
        },
        required_capabilities: vec!["web_search".to_string()],
        expected_benefit: "Produces repeatable research with explicit evidence.".to_string(),
        limitations: vec!["Review high-stakes conclusions.".to_string()],
    }
}

fn transition(
    from: WorkflowProposalState,
    version: u64,
    to: WorkflowProposalState,
    suffix: &str,
    occurred_at_unix_ms: i64,
) -> WorkflowProposalTransition {
    WorkflowProposalTransition {
        proposal_id: PARENT_ID.to_string(),
        expected_state: from,
        expected_version: version,
        expected_revision_sha256: None,
        to_state: to,
        actor: "captain:validator".to_string(),
        reason: suffix.to_string(),
        idempotency_key: format!("{PARENT_ID}:{suffix}"),
        snoozed_until_unix_ms: None,
        occurred_at_unix_ms,
    }
}
