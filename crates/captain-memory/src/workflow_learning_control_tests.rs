use crate::workflow_learning_control::{
    NewWorkflowProposal, PublishValidatedDraft, WorkflowArtifactKind, WorkflowLearningControlError,
    WorkflowLearningStore, WorkflowProposalState, WorkflowProposalTransition,
};
use crate::MemorySubstrate;

fn store() -> WorkflowLearningStore {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    WorkflowLearningStore::new(memory.usage_conn())
}

fn observed(id: &str) -> NewWorkflowProposal {
    NewWorkflowProposal {
        id: id.to_string(),
        idempotency_key: format!("observe:{id}"),
        workflow_signature: "a".repeat(64),
        source_agent_id: "captain".to_string(),
        origin_channel: Some("telegram".to_string()),
        evidence_json: r#"{"episodes":3,"sessions":2}"#.to_string(),
        created_at_unix_ms: 1_000,
    }
}

fn transition(
    proposal_id: &str,
    expected_state: WorkflowProposalState,
    expected_version: u64,
    revision: Option<&str>,
    to_state: WorkflowProposalState,
    key: &str,
) -> WorkflowProposalTransition {
    WorkflowProposalTransition {
        proposal_id: proposal_id.to_string(),
        expected_state,
        expected_version,
        expected_revision_sha256: revision.map(str::to_string),
        to_state,
        actor: "operator:test".to_string(),
        reason: "test transition".to_string(),
        idempotency_key: key.to_string(),
        snoozed_until_unix_ms: (to_state == WorkflowProposalState::Snoozed).then_some(50_000),
        occurred_at_unix_ms: 2_000 + expected_version as i64,
    }
}

fn advance_to_validating(store: &WorkflowLearningStore, id: &str) {
    store.create_observed(&observed(id)).unwrap();
    store
        .transition(&transition(
            id,
            WorkflowProposalState::Observed,
            0,
            None,
            WorkflowProposalState::Eligible,
            &format!("{id}:eligible"),
        ))
        .unwrap();
    store
        .transition(&transition(
            id,
            WorkflowProposalState::Eligible,
            1,
            None,
            WorkflowProposalState::Drafting,
            &format!("{id}:drafting"),
        ))
        .unwrap();
    store
        .transition(&transition(
            id,
            WorkflowProposalState::Drafting,
            2,
            None,
            WorkflowProposalState::Validating,
            &format!("{id}:validating"),
        ))
        .unwrap();
}

fn publish(store: &WorkflowLearningStore, id: &str) -> String {
    let revision = "b".repeat(64);
    publish_revision(store, id, &revision);
    revision
}

fn publish_revision(store: &WorkflowLearningStore, id: &str, revision: &str) {
    store
        .publish_validated_draft(&PublishValidatedDraft {
            proposal_id: id.to_string(),
            expected_version: 3,
            staging_job_id: "draft-job".to_string(),
            revision_sha256: revision.to_string(),
            artifact_sha256: "c".repeat(64),
            kind: WorkflowArtifactKind::Skill,
            name: "verified-research".to_string(),
            validation_json: r#"{"schema":true,"secrets":false}"#.to_string(),
            actor: "captain:validator".to_string(),
            reason: "all objective checks passed".to_string(),
            idempotency_key: format!("{id}:published"),
            occurred_at_unix_ms: 3_000,
        })
        .unwrap();
}

#[test]
fn observed_creation_is_idempotent_and_audited() {
    let store = store();
    let first = store.create_observed(&observed("proposal-1")).unwrap();
    let second = store.create_observed(&observed("proposal-1")).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.state, WorkflowProposalState::Observed);
    assert_eq!(first.state_version, 0);
    let events = store.events("proposal-1").unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].from_state, None);
    assert_eq!(events[0].to_state, WorkflowProposalState::Observed);

    let mut changed = observed("proposal-other");
    changed.idempotency_key = "observe:proposal-1".to_string();
    assert!(store.create_observed(&changed).is_err());
}

#[test]
fn state_machine_rejects_illegal_and_stale_transitions() {
    let store = store();
    store.create_observed(&observed("proposal-2")).unwrap();

    assert!(store
        .transition(&transition(
            "proposal-2",
            WorkflowProposalState::Observed,
            0,
            None,
            WorkflowProposalState::Active,
            "proposal-2:illegal",
        ))
        .is_err());

    let eligible = store
        .transition(&transition(
            "proposal-2",
            WorkflowProposalState::Observed,
            0,
            None,
            WorkflowProposalState::Eligible,
            "proposal-2:eligible",
        ))
        .unwrap();
    assert_eq!(eligible.state_version, 1);
    assert!(store
        .transition(&transition(
            "proposal-2",
            WorkflowProposalState::Observed,
            0,
            None,
            WorkflowProposalState::Dismissed,
            "proposal-2:stale",
        ))
        .is_err());
}

#[test]
fn published_revision_is_immutable_and_duplicate_callback_is_idempotent() {
    let store = store();
    advance_to_validating(&store, "proposal-3");
    let revision = publish(&store, "proposal-3");

    let published = store.get("proposal-3").unwrap().unwrap();
    assert_eq!(published.state, WorkflowProposalState::Proposed);
    assert_eq!(published.state_version, 4);
    assert_eq!(
        published.revision_sha256.as_deref(),
        Some(revision.as_str())
    );
    assert_eq!(published.operator_token.as_deref(), Some(&revision[..20]));
    assert_eq!(
        store
            .get_by_operator_token(&revision[..20])
            .unwrap()
            .unwrap()
            .id,
        "proposal-3"
    );

    let duplicate = publish(&store, "proposal-3");
    assert_eq!(duplicate, revision);
    assert_eq!(store.events("proposal-3").unwrap().len(), 5);

    let wrong_revision = "d".repeat(64);
    assert!(store
        .transition(&transition(
            "proposal-3",
            WorkflowProposalState::Proposed,
            4,
            Some(&wrong_revision),
            WorkflowProposalState::Rejected,
            "proposal-3:wrong-revision",
        ))
        .is_err());
}

#[test]
fn operator_token_collision_cannot_publish_an_ambiguous_callback() {
    let store = store();
    advance_to_validating(&store, "proposal-token-a");
    advance_to_validating(&store, "proposal-token-b");
    let prefix = "0123456789abcdefabcd";
    let first_revision = format!("{prefix}{}", "a".repeat(44));
    let second_revision = format!("{prefix}{}", "b".repeat(44));

    publish_revision(&store, "proposal-token-a", &first_revision);
    let result = store.publish_validated_draft(&PublishValidatedDraft {
        proposal_id: "proposal-token-b".to_string(),
        expected_version: 3,
        staging_job_id: "draft-job-b".to_string(),
        revision_sha256: second_revision,
        artifact_sha256: "d".repeat(64),
        kind: WorkflowArtifactKind::Skill,
        name: "collision-safe".to_string(),
        validation_json: r#"{"schema":true,"secrets":false}"#.to_string(),
        actor: "captain:validator".to_string(),
        reason: "all objective checks passed".to_string(),
        idempotency_key: "proposal-token-b:published".to_string(),
        occurred_at_unix_ms: 3_000,
    });

    assert!(matches!(
        result,
        Err(WorkflowLearningControlError::Conflict(_))
    ));
    let unchanged = store.get("proposal-token-b").unwrap().unwrap();
    assert_eq!(unchanged.state, WorkflowProposalState::Validating);
    assert_eq!(unchanged.state_version, 3);
    assert_eq!(unchanged.revision_sha256, None);
    assert_eq!(unchanged.operator_token, None);
    assert!(store.get_by_operator_token("not-hex").is_err());
}

#[test]
fn concurrent_operator_decisions_have_one_cas_winner() {
    use std::sync::{Arc, Barrier};

    let store = Arc::new(store());
    advance_to_validating(&store, "proposal-4");
    let revision = publish(&store, "proposal-4");
    let barrier = Arc::new(Barrier::new(3));

    let dismiss_store = Arc::clone(&store);
    let dismiss_barrier = Arc::clone(&barrier);
    let dismiss_revision = revision.clone();
    let dismiss = std::thread::spawn(move || {
        dismiss_barrier.wait();
        dismiss_store.transition(&transition(
            "proposal-4",
            WorkflowProposalState::Proposed,
            4,
            Some(&dismiss_revision),
            WorkflowProposalState::Dismissed,
            "proposal-4:dismiss",
        ))
    });

    let reject_store = Arc::clone(&store);
    let reject_barrier = Arc::clone(&barrier);
    let reject_revision = revision.clone();
    let reject = std::thread::spawn(move || {
        reject_barrier.wait();
        reject_store.transition(&transition(
            "proposal-4",
            WorkflowProposalState::Proposed,
            4,
            Some(&reject_revision),
            WorkflowProposalState::Rejected,
            "proposal-4:reject",
        ))
    });

    barrier.wait();
    let results = [dismiss.join().unwrap(), reject.join().unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
    let current = store.get("proposal-4").unwrap().unwrap();
    assert!(matches!(
        current.state,
        WorkflowProposalState::Dismissed | WorkflowProposalState::Rejected
    ));
}

#[test]
fn generic_transition_cannot_approve_without_atomic_install_job() {
    let store = store();
    advance_to_validating(&store, "proposal-effect");
    let revision = publish(&store, "proposal-effect");
    let result = store.transition(&transition(
        "proposal-effect",
        WorkflowProposalState::Proposed,
        4,
        Some(&revision),
        WorkflowProposalState::ApprovedPendingInstall,
        "proposal-effect:unsafe-approve",
    ));
    assert!(result.is_err());
    assert_eq!(
        store.get("proposal-effect").unwrap().unwrap().state,
        WorkflowProposalState::Proposed
    );
}

#[test]
fn snooze_requires_a_deadline_and_can_return_to_proposed() {
    let store = store();
    advance_to_validating(&store, "proposal-5");
    let revision = publish(&store, "proposal-5");
    let snoozed = store
        .transition(&transition(
            "proposal-5",
            WorkflowProposalState::Proposed,
            4,
            Some(&revision),
            WorkflowProposalState::Snoozed,
            "proposal-5:snooze",
        ))
        .unwrap();
    assert_eq!(snoozed.snoozed_until_unix_ms, Some(50_000));
    let restored = store
        .transition(&transition(
            "proposal-5",
            WorkflowProposalState::Snoozed,
            5,
            Some(&revision),
            WorkflowProposalState::Proposed,
            "proposal-5:restore",
        ))
        .unwrap();
    assert_eq!(restored.snoozed_until_unix_ms, None);
}

#[test]
fn evidence_and_identifiers_are_bounded_before_sqlite() {
    let store = store();
    let mut invalid = observed("proposal-6");
    invalid.evidence_json = "not-json".to_string();
    assert!(store.create_observed(&invalid).is_err());

    let mut oversized = observed("proposal-7");
    oversized.evidence_json = format!(r#"{{"data":"{}"}}"#, "x".repeat(70_000));
    assert!(store.create_observed(&oversized).is_err());
}
