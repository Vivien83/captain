use captain_memory::workflow_learning_control::{
    NewWorkflowProposal, PublishValidatedDraft, WorkflowArtifactKind, WorkflowLearningStore,
    WorkflowProposalState, WorkflowProposalTransition,
};
use captain_memory::workflow_learning_outbox::WorkflowOutboxStatus;
use captain_memory::MemorySubstrate;

use crate::workflow_learning_delivery::WORKFLOW_PROPOSAL_OUTBOX_TOPIC;
use crate::workflow_learning_snooze::WorkflowSnoozeScheduler;

const PROPOSAL_ID: &str = "runtime-snooze";
const REVISION: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn scheduler_wakes_only_due_proposals_with_an_exact_notification() {
    let store = snoozed_store(1_000);
    let scheduler =
        WorkflowSnoozeScheduler::new(store.clone(), "captain:workflow-snooze:test").unwrap();

    assert!(scheduler.wake_next_due(999).unwrap().is_none());
    let woke = scheduler.wake_next_due(1_000).unwrap().unwrap();
    assert_eq!(woke.proposal.state, WorkflowProposalState::Proposed);
    assert_eq!(woke.proposal.state_version, 6);
    assert_eq!(woke.notification.topic, WORKFLOW_PROPOSAL_OUTBOX_TOPIC);
    assert_eq!(woke.notification.status, WorkflowOutboxStatus::Pending);
    assert_eq!(woke.notification.attempt_count, 0);
    assert_eq!(woke.notification.revision_sha256.as_deref(), Some(REVISION));
    let payload: serde_json::Value = serde_json::from_str(&woke.notification.payload_json).unwrap();
    assert_eq!(payload["proposal_id"], PROPOSAL_ID);
    assert_eq!(payload["revision_sha256"], REVISION);
    assert_eq!(payload["state"], "proposed");
    assert!(scheduler.wake_next_due(2_000).unwrap().is_none());
}

#[test]
fn scheduler_rejects_an_unsafe_worker_identity() {
    let store = snoozed_store(1_000);
    assert!(WorkflowSnoozeScheduler::new(store, "bad actor with spaces").is_err());
}

fn snoozed_store(deadline: i64) -> WorkflowLearningStore {
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
            staging_job_id: "runtime-snooze-draft".to_string(),
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
