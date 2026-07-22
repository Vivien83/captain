use crate::workflow_learning_control::{
    NewWorkflowProposal, PublishValidatedDraft, WorkflowArtifactKind, WorkflowLearningStore,
    WorkflowProposalState, WorkflowProposalTransition,
};
use crate::workflow_learning_outbox::{NewWorkflowOutboxItem, WorkflowOutboxStatus};
use crate::workflow_learning_snooze::WorkflowSnoozeWake;
use crate::MemorySubstrate;

const PROPOSAL_ID: &str = "proposal-snooze";
const REVISION: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn store_with_snoozed_proposal(deadline: i64) -> WorkflowLearningStore {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let store = WorkflowLearningStore::new(memory.usage_conn());
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
            .transition(&transition(
                from,
                version,
                None,
                to,
                suffix,
                None,
                200 + version as i64,
            ))
            .unwrap();
    }
    store
        .publish_validated_draft(&PublishValidatedDraft {
            proposal_id: PROPOSAL_ID.to_string(),
            expected_version: 3,
            staging_job_id: "draft-job-snooze".to_string(),
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
    store
        .transition(&transition(
            WorkflowProposalState::Proposed,
            4,
            Some(REVISION),
            WorkflowProposalState::Snoozed,
            "snoozed",
            Some(deadline),
            400,
        ))
        .unwrap();
    store
}

#[test]
fn due_listing_and_wake_are_atomic_idempotent_and_exact() {
    let store = store_with_snoozed_proposal(1_000);
    assert!(store.list_due_snoozed(999, 10).unwrap().is_empty());
    assert_eq!(store.list_due_snoozed(1_000, 10).unwrap().len(), 1);
    let request = wake_request("wake-notification", "wake:key", 1_000);

    let first = store.wake_snoozed_and_notify(&request).unwrap();
    let replay = store.wake_snoozed_and_notify(&request).unwrap();

    assert_eq!(first, replay);
    assert_eq!(first.proposal.state, WorkflowProposalState::Proposed);
    assert_eq!(first.proposal.state_version, 6);
    assert!(first.proposal.snoozed_until_unix_ms.is_none());
    assert_eq!(first.notification.status, WorkflowOutboxStatus::Pending);
    assert_eq!(
        first.notification.revision_sha256.as_deref(),
        Some(REVISION)
    );
    assert!(store.list_due_snoozed(2_000, 10).unwrap().is_empty());
    assert_eq!(store.events(PROPOSAL_ID).unwrap().len(), 7);
}

#[test]
fn early_wake_is_rejected_without_mutating_the_snooze() {
    let store = store_with_snoozed_proposal(2_000);
    let request = wake_request("wake-early", "wake:early", 1_999);

    assert!(store.wake_snoozed_and_notify(&request).is_err());
    let proposal = store.get(PROPOSAL_ID).unwrap().unwrap();
    assert_eq!(proposal.state, WorkflowProposalState::Snoozed);
    assert_eq!(proposal.state_version, 5);
    assert!(store.get_outbox("wake-early").unwrap().is_none());
}

#[test]
fn notification_conflict_rolls_back_the_state_transition() {
    let store = store_with_snoozed_proposal(1_000);
    let conflicting = NewWorkflowOutboxItem {
        id: "existing-notification".to_string(),
        idempotency_key: "wake:conflict".to_string(),
        proposal_id: PROPOSAL_ID.to_string(),
        revision_sha256: Some(REVISION.to_string()),
        topic: "workflow_learning.lifecycle".to_string(),
        payload_json: "{}".to_string(),
        max_attempts: 8,
        run_after_unix_ms: 1_000,
        created_at_unix_ms: 1_000,
    };
    store.enqueue_outbox(&conflicting).unwrap();
    let request = wake_request("wake-conflict", "wake:conflict", 1_000);

    assert!(store.wake_snoozed_and_notify(&request).is_err());
    let proposal = store.get(PROPOSAL_ID).unwrap().unwrap();
    assert_eq!(proposal.state, WorkflowProposalState::Snoozed);
    assert_eq!(proposal.state_version, 5);
    assert_eq!(store.events(PROPOSAL_ID).unwrap().len(), 6);
}

fn wake_request(id: &str, key: &str, at: i64) -> WorkflowSnoozeWake {
    WorkflowSnoozeWake {
        proposal_transition: transition(
            WorkflowProposalState::Snoozed,
            5,
            Some(REVISION),
            WorkflowProposalState::Proposed,
            "wake",
            None,
            at,
        ),
        notification: NewWorkflowOutboxItem {
            id: id.to_string(),
            idempotency_key: key.to_string(),
            proposal_id: PROPOSAL_ID.to_string(),
            revision_sha256: Some(REVISION.to_string()),
            topic: "workflow_learning.proposed".to_string(),
            payload_json: format!(
                r#"{{"schema_version":1,"proposal_id":"{PROPOSAL_ID}","revision_sha256":"{REVISION}","state":"proposed"}}"#
            ),
            max_attempts: 8,
            run_after_unix_ms: at,
            created_at_unix_ms: at,
        },
    }
}

fn transition(
    from: WorkflowProposalState,
    version: u64,
    revision: Option<&str>,
    to: WorkflowProposalState,
    suffix: &str,
    deadline: Option<i64>,
    at: i64,
) -> WorkflowProposalTransition {
    WorkflowProposalTransition {
        proposal_id: PROPOSAL_ID.to_string(),
        expected_state: from,
        expected_version: version,
        expected_revision_sha256: revision.map(str::to_string),
        to_state: to,
        actor: "captain:workflow-snooze".to_string(),
        reason: suffix.to_string(),
        idempotency_key: format!("{PROPOSAL_ID}:{suffix}:v{version}"),
        snoozed_until_unix_ms: deadline,
        occurred_at_unix_ms: at,
    }
}
