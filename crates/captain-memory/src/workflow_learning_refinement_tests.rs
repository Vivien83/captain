use crate::workflow_learning_control::{
    NewWorkflowProposal, PublishValidatedDraft, WorkflowArtifactKind, WorkflowLearningStore,
    WorkflowProposalState, WorkflowProposalTransition,
};
use crate::workflow_learning_refinement::{NewWorkflowRefinementRequest, WorkflowRefinementState};
use crate::MemorySubstrate;

const PROPOSAL_ID: &str = "proposal-refinement";
const REVISION: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn exact_refinement_binding_is_durable_idempotent_and_audited() {
    let store = proposed_store();
    let request = edit_request("edit-1", "edit:key:1", "telegram:chat:root", 1_000);

    let first = store.begin_refinement_request(&request).unwrap();
    let replay = store.begin_refinement_request(&request).unwrap();

    assert_eq!(first, replay);
    assert_eq!(first.state, WorkflowRefinementState::AwaitingInput);
    assert_eq!(first.state_version, 0);
    assert_eq!(
        store
            .pending_refinement_for_binding("telegram", "telegram:chat:root", "telegram:42", 1_100,)
            .unwrap(),
        Some(first.clone())
    );
    let events = store.refinement_events(&first.id).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].from_state, None);
    assert_eq!(events[0].to_state, WorkflowRefinementState::AwaitingInput);
}

#[test]
fn repeated_click_replays_one_binding_but_another_conversation_cannot_steal_it() {
    let store = proposed_store();
    store
        .begin_refinement_request(&edit_request(
            "edit-1",
            "edit:key:1",
            "telegram:chat:root",
            1_000,
        ))
        .unwrap();

    let same_binding = edit_request("edit-2", "edit:key:2", "telegram:chat:root", 1_001);
    let replay = store.begin_refinement_request(&same_binding).unwrap();
    assert_eq!(replay.id, "edit-1");
    let other_binding_same_revision =
        edit_request("edit-3", "edit:key:3", "telegram:another:root", 1_002);
    assert!(store
        .begin_refinement_request(&other_binding_same_revision)
        .is_err());
    assert!(store.get_refinement_request("edit-2").unwrap().is_none());
    assert!(store.get_refinement_request("edit-3").unwrap().is_none());
}

#[test]
fn pending_binding_survives_database_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("memory.db");
    {
        let memory = MemorySubstrate::open(&database, 0.01).unwrap();
        let store = WorkflowLearningStore::new(memory.usage_conn());
        populate_proposed(&store);
        store
            .begin_refinement_request(&edit_request(
                "edit-restart",
                "edit:key:restart",
                "telegram:chat:root",
                1_000,
            ))
            .unwrap();
    }

    let memory = MemorySubstrate::open(&database, 0.01).unwrap();
    let store = WorkflowLearningStore::new(memory.usage_conn());
    let restored = store
        .pending_refinement_for_binding("telegram", "telegram:chat:root", "telegram:42", 1_100)
        .unwrap()
        .unwrap();
    assert_eq!(restored.id, "edit-restart");
    assert_eq!(restored.state, WorkflowRefinementState::AwaitingInput);
}

#[test]
fn expired_binding_is_audited_and_releases_both_uniqueness_guards() {
    let store = proposed_store();
    let mut first = edit_request("edit-1", "edit:key:1", "telegram:chat:root", 1_000);
    first.expires_at_unix_ms = 61_000;
    store.begin_refinement_request(&first).unwrap();

    assert!(store
        .pending_refinement_for_binding("telegram", "telegram:chat:root", "telegram:42", 61_000,)
        .unwrap()
        .is_none());
    let expired = store.get_refinement_request("edit-1").unwrap().unwrap();
    assert_eq!(expired.state, WorkflowRefinementState::Expired);
    assert_eq!(expired.state_version, 1);
    assert_eq!(store.refinement_events("edit-1").unwrap().len(), 2);

    let second = edit_request("edit-2", "edit:key:2", "telegram:chat:root", 61_001);
    assert_eq!(
        store.begin_refinement_request(&second).unwrap().state,
        WorkflowRefinementState::AwaitingInput
    );
}

#[test]
fn stale_proposal_identity_and_changed_replay_are_rejected_without_rows() {
    let store = proposed_store();
    let mut stale = edit_request("edit-stale", "edit:key:stale", "telegram:chat:root", 1_000);
    stale.expected_proposal_version = 3;
    assert!(store.begin_refinement_request(&stale).is_err());
    assert!(store
        .get_refinement_request("edit-stale")
        .unwrap()
        .is_none());

    let exact = edit_request("edit-1", "edit:key:1", "telegram:chat:root", 1_000);
    store.begin_refinement_request(&exact).unwrap();
    let mut changed = exact;
    changed.language = "en".to_string();
    assert!(store.begin_refinement_request(&changed).is_err());
}

fn edit_request(
    id: &str,
    key: &str,
    conversation_key: &str,
    at: i64,
) -> NewWorkflowRefinementRequest {
    NewWorkflowRefinementRequest {
        id: id.to_string(),
        idempotency_key: key.to_string(),
        proposal_id: PROPOSAL_ID.to_string(),
        revision_sha256: REVISION.to_string(),
        expected_proposal_version: 4,
        actor: "telegram:42".to_string(),
        surface: "telegram".to_string(),
        conversation_key: conversation_key.to_string(),
        source_message_id: Some("100".to_string()),
        language: "fr".to_string(),
        expires_at_unix_ms: at + 30 * 60 * 1_000,
        created_at_unix_ms: at,
    }
}

fn proposed_store() -> WorkflowLearningStore {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let store = WorkflowLearningStore::new(memory.usage_conn());
    populate_proposed(&store);
    store
}

fn populate_proposed(store: &WorkflowLearningStore) {
    store
        .create_observed(&NewWorkflowProposal {
            id: PROPOSAL_ID.to_string(),
            idempotency_key: format!("{PROPOSAL_ID}:observed"),
            workflow_signature: "a".repeat(64),
            source_agent_id: "captain".to_string(),
            origin_channel: Some("telegram".to_string()),
            evidence_json: "{}".to_string(),
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
            .transition(&WorkflowProposalTransition {
                proposal_id: PROPOSAL_ID.to_string(),
                expected_state: from,
                expected_version: version,
                expected_revision_sha256: None,
                to_state: to,
                actor: "captain:test".to_string(),
                reason: suffix.to_string(),
                idempotency_key: format!("{PROPOSAL_ID}:{suffix}"),
                snoozed_until_unix_ms: None,
                occurred_at_unix_ms: 200 + version as i64,
            })
            .unwrap();
    }
    store
        .publish_validated_draft(&PublishValidatedDraft {
            proposal_id: PROPOSAL_ID.to_string(),
            expected_version: 3,
            staging_job_id: "draft-job-refinement".to_string(),
            revision_sha256: REVISION.to_string(),
            artifact_sha256: "c".repeat(64),
            kind: WorkflowArtifactKind::Skill,
            name: "sourced-research".to_string(),
            validation_json: "{}".to_string(),
            actor: "captain:validator".to_string(),
            reason: "validated".to_string(),
            idempotency_key: format!("{PROPOSAL_ID}:published"),
            occurred_at_unix_ms: 300,
        })
        .unwrap();
}
