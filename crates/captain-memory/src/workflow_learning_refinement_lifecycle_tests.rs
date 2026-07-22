use crate::workflow_learning_control::{
    NewWorkflowProposal, PublishValidatedDraft, WorkflowArtifactKind, WorkflowLearningStore,
    WorkflowProposalState, WorkflowProposalTransition,
};
use crate::workflow_learning_outbox::{NewWorkflowOutboxItem, WorkflowOutboxStatus};
use crate::workflow_learning_pipeline::{WorkflowDraftCompletion, WorkflowPipelineRejection};
use crate::workflow_learning_pipeline_types::WorkflowValidationCompletion;
use crate::workflow_learning_queue::{NewWorkflowJob, WorkflowJobKind, WorkflowJobStatus};
use crate::workflow_learning_refinement::{NewWorkflowRefinementRequest, WorkflowRefinementState};
use crate::workflow_learning_refinement_capture::CaptureWorkflowRefinement;
use crate::workflow_learning_refinement_lifecycle::{
    WorkflowRefinementRejection, WorkflowRefinementValidationCompletion,
};
use crate::MemorySubstrate;
use rusqlite::params;

const PARENT_ID: &str = "lifecycle-parent";
const CHILD_ID: &str = "lifecycle-child";
const REQUEST_ID: &str = "lifecycle-request";
const DRAFT_JOB_ID: &str = "lifecycle-child-draft";
const VALIDATE_JOB_ID: &str = "lifecycle-child-validate";
const PARENT_REVISION: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const CHILD_REVISION: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const WORKER: &str = "captain:refinement-worker";

#[test]
fn validated_child_parent_supersession_request_job_and_outbox_commit_once() {
    let (store, completion) = store_with_running_validation();

    let first = store.complete_refinement_validation(&completion).unwrap();
    let replay = store.complete_refinement_validation(&completion).unwrap();

    assert_eq!(first, replay);
    assert_eq!(first.request.state, WorkflowRefinementState::Completed);
    assert_eq!(first.request.state_version, 2);
    assert_eq!(
        first.parent_proposal.state,
        WorkflowProposalState::Superseded
    );
    assert_eq!(first.child_proposal.state, WorkflowProposalState::Proposed);
    assert_eq!(
        first.child_proposal.revision_sha256.as_deref(),
        Some(CHILD_REVISION)
    );
    assert_eq!(first.job.status, WorkflowJobStatus::Succeeded);
    assert_eq!(
        first.notification.as_ref().unwrap().status,
        WorkflowOutboxStatus::Pending
    );
    assert_eq!(store.refinement_events(REQUEST_ID).unwrap().len(), 3);
}

#[test]
fn notification_conflict_rolls_back_publish_supersession_job_and_request() {
    let (store, completion) = store_with_running_validation();
    store
        .enqueue_outbox(&NewWorkflowOutboxItem {
            id: "existing-lifecycle-notification".to_string(),
            idempotency_key: format!("{CHILD_ID}:proposed-notification"),
            proposal_id: PARENT_ID.to_string(),
            revision_sha256: Some(PARENT_REVISION.to_string()),
            topic: "workflow_learning.lifecycle".to_string(),
            payload_json: "{}".to_string(),
            max_attempts: 8,
            run_after_unix_ms: 729,
            created_at_unix_ms: 729,
        })
        .unwrap();

    assert!(store.complete_refinement_validation(&completion).is_err());
    assert_eq!(
        store.get(PARENT_ID).unwrap().unwrap().state,
        WorkflowProposalState::Proposed
    );
    assert_eq!(
        store.get(CHILD_ID).unwrap().unwrap().state,
        WorkflowProposalState::Validating
    );
    assert_eq!(
        store.get_job(VALIDATE_JOB_ID).unwrap().unwrap().status,
        WorkflowJobStatus::Running
    );
    assert_eq!(
        store
            .get_refinement_request(REQUEST_ID)
            .unwrap()
            .unwrap()
            .state,
        WorkflowRefinementState::Queued
    );
}

#[test]
fn completion_rejects_split_identity_or_time_before_any_mutation() {
    let (store, mut completion) = store_with_running_validation();
    completion.validation.worker = "captain:other-worker".to_string();

    assert!(store.complete_refinement_validation(&completion).is_err());
    assert_eq!(
        store.get(PARENT_ID).unwrap().unwrap().state,
        WorkflowProposalState::Proposed
    );
    assert_eq!(
        store.get(CHILD_ID).unwrap().unwrap().state,
        WorkflowProposalState::Validating
    );
    assert_eq!(
        store.get_job(VALIDATE_JOB_ID).unwrap().unwrap().status,
        WorkflowJobStatus::Running
    );

    let (store, mut completion) = store_with_running_validation();
    completion.parent_transition.occurred_at_unix_ms += 1;
    assert!(store.complete_refinement_validation(&completion).is_err());
    assert_eq!(
        store
            .get_refinement_request(REQUEST_ID)
            .unwrap()
            .unwrap()
            .state,
        WorkflowRefinementState::Queued
    );
}

#[test]
fn rejected_child_marks_request_failed_without_touching_parent() {
    let store = store_with_captured_child();
    let draft = store
        .claim_due_preapproval_job(WORKER, 710, 60_000)
        .unwrap()
        .unwrap();
    store
        .mark_job_effect_started(&draft.id, WORKER, 711)
        .unwrap();
    let rejection = draft_rejection(draft.id, 720);

    let first = store.reject_refinement(&rejection).unwrap();
    let replay = store.reject_refinement(&rejection).unwrap();
    assert_eq!(first, replay);
    assert_eq!(first.request.state, WorkflowRefinementState::Failed);
    assert_eq!(
        first.request.last_error.as_deref(),
        Some("operator refinement was declined by the active model")
    );
    assert_eq!(first.child_proposal.state, WorkflowProposalState::Rejected);
    assert_eq!(first.parent_proposal.state, WorkflowProposalState::Proposed);
}

#[test]
fn rejection_refuses_a_parent_that_changed_after_capture() {
    let store = store_with_captured_child();
    let draft = store
        .claim_due_preapproval_job(WORKER, 710, 60_000)
        .unwrap()
        .unwrap();
    store
        .mark_job_effect_started(&draft.id, WORKER, 711)
        .unwrap();
    // Public transitions are blocked while refinement is queued. Corrupt the
    // row directly to verify that lifecycle completion still fails closed.
    store
        .lock_conn()
        .unwrap()
        .execute(
            "UPDATE workflow_learning_proposals
             SET state = 'dismissed', state_version = state_version + 1, updated_at = ?1
             WHERE id = ?2",
            params![715, PARENT_ID],
        )
        .unwrap();

    assert!(store
        .reject_refinement(&draft_rejection(draft.id, 720))
        .is_err());
    assert_eq!(
        store.get(CHILD_ID).unwrap().unwrap().state,
        WorkflowProposalState::Drafting
    );
    assert_eq!(
        store
            .get_refinement_request(REQUEST_ID)
            .unwrap()
            .unwrap()
            .state,
        WorkflowRefinementState::Queued
    );
}

fn draft_rejection(job_id: String, completed_at_unix_ms: i64) -> WorkflowRefinementRejection {
    WorkflowRefinementRejection {
        request_id: REQUEST_ID.to_string(),
        expected_request_version: 1,
        rejection: WorkflowPipelineRejection {
            job_id,
            worker: WORKER.to_string(),
            job_kind: WorkflowJobKind::Draft,
            result_json: Some(r#"{"decision":"decline"}"#.to_string()),
            proposal_transition: transition(
                CHILD_ID,
                WorkflowProposalState::Drafting,
                2,
                None,
                WorkflowProposalState::Rejected,
                "rejected",
                completed_at_unix_ms,
            ),
            notification: None,
            completed_at_unix_ms,
        },
        actor: WORKER.to_string(),
        reason: "operator refinement was declined by the active model".to_string(),
        idempotency_key: format!("{REQUEST_ID}:failed"),
        completed_at_unix_ms,
    }
}

fn store_with_running_validation() -> (
    WorkflowLearningStore,
    WorkflowRefinementValidationCompletion,
) {
    let store = store_with_captured_child();
    let draft = store
        .claim_due_preapproval_job(WORKER, 710, 60_000)
        .unwrap()
        .unwrap();
    store
        .mark_job_effect_started(&draft.id, WORKER, 711)
        .unwrap();
    store
        .complete_draft_and_enqueue_validation(&WorkflowDraftCompletion {
            job_id: draft.id,
            worker: WORKER.to_string(),
            result_json: Some(r#"{"staged":true}"#.to_string()),
            proposal_transition: transition(
                CHILD_ID,
                WorkflowProposalState::Drafting,
                2,
                None,
                WorkflowProposalState::Validating,
                "validating",
                720,
            ),
            validation_job: NewWorkflowJob {
                id: VALIDATE_JOB_ID.to_string(),
                idempotency_key: format!("{VALIDATE_JOB_ID}:enqueue"),
                proposal_id: CHILD_ID.to_string(),
                revision_sha256: None,
                kind: WorkflowJobKind::Validate,
                payload_json: "{}".to_string(),
                max_attempts: 3,
                run_after_unix_ms: 720,
                created_at_unix_ms: 720,
            },
            completed_at_unix_ms: 720,
        })
        .unwrap();
    let validation = store
        .claim_due_preapproval_job(WORKER, 721, 60_000)
        .unwrap()
        .unwrap();
    assert_eq!(validation.kind, WorkflowJobKind::Validate);
    (
        store,
        WorkflowRefinementValidationCompletion {
            request_id: REQUEST_ID.to_string(),
            expected_request_version: 1,
            parent_transition: WorkflowProposalTransition {
                proposal_id: PARENT_ID.to_string(),
                expected_state: WorkflowProposalState::Proposed,
                expected_version: 4,
                expected_revision_sha256: Some(PARENT_REVISION.to_string()),
                to_state: WorkflowProposalState::Superseded,
                actor: WORKER.to_string(),
                reason: "validated refinement superseded parent".to_string(),
                idempotency_key: format!("{PARENT_ID}:superseded:{CHILD_ID}"),
                snoozed_until_unix_ms: None,
                occurred_at_unix_ms: 730,
            },
            validation: WorkflowValidationCompletion {
                job_id: validation.id,
                worker: WORKER.to_string(),
                result_json: Some(r#"{"validated":true}"#.to_string()),
                publish: PublishValidatedDraft {
                    proposal_id: CHILD_ID.to_string(),
                    expected_version: 3,
                    staging_job_id: DRAFT_JOB_ID.to_string(),
                    revision_sha256: CHILD_REVISION.to_string(),
                    artifact_sha256: "e".repeat(64),
                    kind: WorkflowArtifactKind::Skill,
                    name: "refined-research".to_string(),
                    validation_json: "{}".to_string(),
                    actor: WORKER.to_string(),
                    reason: "refinement validated".to_string(),
                    idempotency_key: format!("{CHILD_ID}:proposed"),
                    occurred_at_unix_ms: 730,
                },
                notification: Some(NewWorkflowOutboxItem {
                    id: format!("{CHILD_ID}-proposed"),
                    idempotency_key: format!("{CHILD_ID}:proposed-notification"),
                    proposal_id: CHILD_ID.to_string(),
                    revision_sha256: Some(CHILD_REVISION.to_string()),
                    topic: "workflow_learning.proposed".to_string(),
                    payload_json: "{}".to_string(),
                    max_attempts: 8,
                    run_after_unix_ms: 730,
                    created_at_unix_ms: 730,
                }),
                completed_at_unix_ms: 730,
            },
            actor: WORKER.to_string(),
            idempotency_key: format!("{REQUEST_ID}:completed"),
            completed_at_unix_ms: 730,
        },
    )
}

fn store_with_captured_child() -> WorkflowLearningStore {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let store = WorkflowLearningStore::new(memory.usage_conn());
    create_proposed(&store);
    store
        .begin_refinement_request(&NewWorkflowRefinementRequest {
            id: REQUEST_ID.to_string(),
            idempotency_key: format!("{REQUEST_ID}:begin"),
            proposal_id: PARENT_ID.to_string(),
            revision_sha256: PARENT_REVISION.to_string(),
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
        .capture_refinement_and_enqueue_draft(&CaptureWorkflowRefinement {
            request_id: REQUEST_ID.to_string(),
            expected_request_version: 0,
            actor: "telegram:42".to_string(),
            instruction: "keep sources and improve the summary".to_string(),
            captured_message_id: "101".to_string(),
            child_proposal: NewWorkflowProposal {
                id: CHILD_ID.to_string(),
                idempotency_key: format!("{CHILD_ID}:observed"),
                workflow_signature: "f".repeat(64),
                source_agent_id: "captain".to_string(),
                origin_channel: Some("telegram".to_string()),
                evidence_json: "{}".to_string(),
                created_at_unix_ms: 700,
            },
            eligible_transition: transition(
                CHILD_ID,
                WorkflowProposalState::Observed,
                0,
                None,
                WorkflowProposalState::Eligible,
                "eligible",
                700,
            ),
            drafting_transition: transition(
                CHILD_ID,
                WorkflowProposalState::Eligible,
                1,
                None,
                WorkflowProposalState::Drafting,
                "drafting",
                700,
            ),
            draft_job: NewWorkflowJob {
                id: DRAFT_JOB_ID.to_string(),
                idempotency_key: format!("{DRAFT_JOB_ID}:enqueue"),
                proposal_id: CHILD_ID.to_string(),
                revision_sha256: None,
                kind: WorkflowJobKind::Draft,
                payload_json: "{}".to_string(),
                max_attempts: 3,
                run_after_unix_ms: 700,
                created_at_unix_ms: 700,
            },
            idempotency_key: format!("{REQUEST_ID}:capture:101"),
            captured_at_unix_ms: 700,
        })
        .unwrap();
    store
}

fn create_proposed(store: &WorkflowLearningStore) {
    store
        .create_observed(&NewWorkflowProposal {
            id: PARENT_ID.to_string(),
            idempotency_key: format!("{PARENT_ID}:observed"),
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
                PARENT_ID,
                from,
                version,
                None,
                to,
                suffix,
                200 + version as i64,
            ))
            .unwrap();
    }
    store
        .publish_validated_draft(&PublishValidatedDraft {
            proposal_id: PARENT_ID.to_string(),
            expected_version: 3,
            staging_job_id: format!("{PARENT_ID}-draft"),
            revision_sha256: PARENT_REVISION.to_string(),
            artifact_sha256: "c".repeat(64),
            kind: WorkflowArtifactKind::Skill,
            name: "sourced-research".to_string(),
            validation_json: "{}".to_string(),
            actor: "captain:validator".to_string(),
            reason: "validated".to_string(),
            idempotency_key: format!("{PARENT_ID}:published"),
            occurred_at_unix_ms: 300,
        })
        .unwrap();
}

fn transition(
    proposal_id: &str,
    from: WorkflowProposalState,
    version: u64,
    revision: Option<&str>,
    to: WorkflowProposalState,
    suffix: &str,
    at: i64,
) -> WorkflowProposalTransition {
    WorkflowProposalTransition {
        proposal_id: proposal_id.to_string(),
        expected_state: from,
        expected_version: version,
        expected_revision_sha256: revision.map(str::to_string),
        to_state: to,
        actor: if proposal_id == CHILD_ID && version <= 1 {
            "telegram:42".to_string()
        } else {
            WORKER.to_string()
        },
        reason: suffix.to_string(),
        idempotency_key: format!("{proposal_id}:{suffix}:v{version}"),
        snoozed_until_unix_ms: None,
        occurred_at_unix_ms: at,
    }
}
