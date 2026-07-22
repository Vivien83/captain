use crate::workflow_learning_control::{
    NewWorkflowProposal, PublishValidatedDraft, WorkflowArtifactKind, WorkflowLearningStore,
    WorkflowProposalState, WorkflowProposalTransition,
};
use crate::workflow_learning_queue::{NewWorkflowJob, WorkflowJobKind, WorkflowJobStatus};
use crate::workflow_learning_refinement::{NewWorkflowRefinementRequest, WorkflowRefinementState};
use crate::workflow_learning_refinement_capture::CaptureWorkflowRefinement;
use crate::MemorySubstrate;
use rusqlite::params;

const PARENT_ID: &str = "refinement-parent";
const REVISION: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn capture_creates_one_drafting_child_and_job_atomically_and_idempotently() {
    let store = store_with_pending_refinement();
    let capture = capture_request("message-101", "change the title and keep sources");

    let first = store
        .capture_refinement_and_enqueue_draft(&capture)
        .unwrap();
    let replay = store
        .capture_refinement_and_enqueue_draft(&capture)
        .unwrap();

    assert_eq!(first, replay);
    assert_eq!(first.request.state, WorkflowRefinementState::Queued);
    assert_eq!(first.request.state_version, 1);
    assert_eq!(first.child_proposal.state, WorkflowProposalState::Drafting);
    assert_eq!(first.child_proposal.state_version, 2);
    assert_eq!(first.draft_job.kind, WorkflowJobKind::Draft);
    assert_eq!(first.draft_job.status, WorkflowJobStatus::Pending);
    assert_eq!(
        store.get(PARENT_ID).unwrap().unwrap().state,
        WorkflowProposalState::Proposed
    );
    assert_eq!(
        store.refinement_events("refinement-request").unwrap().len(),
        2
    );

    store
        .transition(&WorkflowProposalTransition {
            proposal_id: "refinement-child".to_string(),
            expected_state: WorkflowProposalState::Drafting,
            expected_version: 2,
            expected_revision_sha256: None,
            to_state: WorkflowProposalState::Validating,
            actor: "captain:test-worker".to_string(),
            reason: "draft completed".to_string(),
            idempotency_key: "refinement-child:late-validating".to_string(),
            snoozed_until_unix_ms: None,
            occurred_at_unix_ms: 750,
        })
        .unwrap();
    let late_replay = store
        .capture_refinement_and_enqueue_draft(&capture)
        .unwrap();
    assert_eq!(
        late_replay.child_proposal.state,
        WorkflowProposalState::Validating
    );
}

#[test]
fn different_second_message_cannot_replace_the_captured_instruction() {
    let store = store_with_pending_refinement();
    let capture = capture_request("message-101", "change the title and keep sources");
    store
        .capture_refinement_and_enqueue_draft(&capture)
        .unwrap();
    let changed = capture_request("message-102", "remove all source checks");

    assert!(store
        .capture_refinement_and_enqueue_draft(&changed)
        .is_err());
    let request = store
        .get_refinement_request("refinement-request")
        .unwrap()
        .unwrap();
    assert_eq!(
        request.instruction.as_deref(),
        Some("change the title and keep sources")
    );
    assert_eq!(request.captured_message_id.as_deref(), Some("message-101"));
}

#[test]
fn changed_parent_or_expired_binding_produces_no_child_and_no_job() {
    let store = store_with_pending_refinement();
    // Bypass the public transition guard to emulate an externally corrupted
    // database and retain defense-in-depth at the atomic capture boundary.
    store
        .lock_conn()
        .unwrap()
        .execute(
            "UPDATE workflow_learning_proposals
             SET state = 'snoozed', state_version = state_version + 1,
                 snoozed_until = ?1, updated_at = ?2
             WHERE id = ?3",
            params![10_000, 800, PARENT_ID],
        )
        .unwrap();
    let capture = capture_request("message-101", "change the title");

    assert!(store
        .capture_refinement_and_enqueue_draft(&capture)
        .is_err());
    assert!(store.get("refinement-child").unwrap().is_none());
    assert!(store.get_job("refinement-draft").unwrap().is_none());

    let store = store_with_pending_refinement();
    let mut expired = capture_request("message-101", "change the title");
    expired.captured_at_unix_ms = 2_000_001;
    assert!(store
        .capture_refinement_and_enqueue_draft(&expired)
        .is_err());
    assert!(store.get("refinement-child").unwrap().is_none());
}

fn capture_request(message_id: &str, instruction: &str) -> CaptureWorkflowRefinement {
    let child = "refinement-child";
    CaptureWorkflowRefinement {
        request_id: "refinement-request".to_string(),
        expected_request_version: 0,
        actor: "telegram:42".to_string(),
        instruction: instruction.to_string(),
        captured_message_id: message_id.to_string(),
        child_proposal: NewWorkflowProposal {
            id: child.to_string(),
            idempotency_key: "refinement-child:observed".to_string(),
            workflow_signature: "d".repeat(64),
            source_agent_id: "captain".to_string(),
            origin_channel: Some("telegram".to_string()),
            evidence_json: r#"{"eligible":true}"#.to_string(),
            created_at_unix_ms: 700,
        },
        eligible_transition: WorkflowProposalTransition {
            proposal_id: child.to_string(),
            expected_state: WorkflowProposalState::Observed,
            expected_version: 0,
            expected_revision_sha256: None,
            to_state: WorkflowProposalState::Eligible,
            actor: "telegram:42".to_string(),
            reason: "explicit operator refinement".to_string(),
            idempotency_key: "refinement-child:eligible".to_string(),
            snoozed_until_unix_ms: None,
            occurred_at_unix_ms: 700,
        },
        drafting_transition: WorkflowProposalTransition {
            proposal_id: child.to_string(),
            expected_state: WorkflowProposalState::Eligible,
            expected_version: 1,
            expected_revision_sha256: None,
            to_state: WorkflowProposalState::Drafting,
            actor: "telegram:42".to_string(),
            reason: "operator refinement queued".to_string(),
            idempotency_key: "refinement-child:drafting".to_string(),
            snoozed_until_unix_ms: None,
            occurred_at_unix_ms: 700,
        },
        draft_job: NewWorkflowJob {
            id: "refinement-draft".to_string(),
            idempotency_key: "refinement-draft:enqueue".to_string(),
            proposal_id: child.to_string(),
            revision_sha256: None,
            kind: WorkflowJobKind::Draft,
            payload_json: r#"{"schema_version":1}"#.to_string(),
            max_attempts: 3,
            run_after_unix_ms: 700,
            created_at_unix_ms: 700,
        },
        idempotency_key: "refinement-request:capture:message-101".to_string(),
        captured_at_unix_ms: 700,
    }
}

fn store_with_pending_refinement() -> WorkflowLearningStore {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let store = WorkflowLearningStore::new(memory.usage_conn());
    create_proposed(&store, PARENT_ID, "a", REVISION);
    store
        .begin_refinement_request(&NewWorkflowRefinementRequest {
            id: "refinement-request".to_string(),
            idempotency_key: "refinement-request:begin".to_string(),
            proposal_id: PARENT_ID.to_string(),
            revision_sha256: REVISION.to_string(),
            expected_proposal_version: 4,
            actor: "telegram:42".to_string(),
            surface: "telegram".to_string(),
            conversation_key: "telegram:chat:root".to_string(),
            source_message_id: Some("100".to_string()),
            language: "fr".to_string(),
            expires_at_unix_ms: 2_000_000,
            created_at_unix_ms: 600,
        })
        .unwrap();
    store
}

fn create_proposed(store: &WorkflowLearningStore, id: &str, signature: &str, revision: &str) {
    store
        .create_observed(&NewWorkflowProposal {
            id: id.to_string(),
            idempotency_key: format!("{id}:observed"),
            workflow_signature: signature.repeat(64),
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
                proposal_id: id.to_string(),
                expected_state: from,
                expected_version: version,
                expected_revision_sha256: None,
                to_state: to,
                actor: "captain:test".to_string(),
                reason: suffix.to_string(),
                idempotency_key: format!("{id}:{suffix}"),
                snoozed_until_unix_ms: None,
                occurred_at_unix_ms: 200 + version as i64,
            })
            .unwrap();
    }
    store
        .publish_validated_draft(&PublishValidatedDraft {
            proposal_id: id.to_string(),
            expected_version: 3,
            staging_job_id: format!("{id}-draft"),
            revision_sha256: revision.to_string(),
            artifact_sha256: "c".repeat(64),
            kind: WorkflowArtifactKind::Skill,
            name: "sourced-research".to_string(),
            validation_json: "{}".to_string(),
            actor: "captain:validator".to_string(),
            reason: "validated".to_string(),
            idempotency_key: format!("{id}:published"),
            occurred_at_unix_ms: 300,
        })
        .unwrap();
}
