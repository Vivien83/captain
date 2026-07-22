use crate::workflow_learning_control::{
    NewWorkflowProposal, PublishValidatedDraft, WorkflowArtifactKind, WorkflowLearningStore,
    WorkflowProposalState, WorkflowProposalTransition,
};
use crate::workflow_learning_outbox::{NewWorkflowOutboxItem, WorkflowOutboxStatus};
use crate::workflow_learning_queue::{
    NewWorkflowJob, WorkflowJobEffectState, WorkflowJobKind, WorkflowJobStatus,
};
use crate::workflow_learning_test::{
    NewWorkflowIsolatedTest, WorkflowIsolatedTestCompletion, WorkflowIsolatedTestStatus,
};
use crate::MemorySubstrate;

const WORKER: &str = "captain:isolated-test";

fn store() -> WorkflowLearningStore {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    WorkflowLearningStore::new(memory.usage_conn())
}

fn transition(
    id: &str,
    from: WorkflowProposalState,
    version: u64,
    revision: Option<&str>,
    to: WorkflowProposalState,
    key: &str,
    at: i64,
) -> WorkflowProposalTransition {
    WorkflowProposalTransition {
        proposal_id: id.to_string(),
        expected_state: from,
        expected_version: version,
        expected_revision_sha256: revision.map(str::to_string),
        to_state: to,
        actor: WORKER.to_string(),
        reason: key.to_string(),
        idempotency_key: format!("{id}:{key}"),
        snoozed_until_unix_ms: None,
        occurred_at_unix_ms: at,
    }
}

fn create_proposed(store: &WorkflowLearningStore, id: &str) -> String {
    store
        .create_observed(&NewWorkflowProposal {
            id: id.to_string(),
            idempotency_key: format!("{id}:observed"),
            workflow_signature: "a".repeat(64),
            source_agent_id: "captain".to_string(),
            origin_channel: Some("telegram".to_string()),
            evidence_json: "{}".to_string(),
            created_at_unix_ms: 1_000,
        })
        .unwrap();
    for (from, version, to, key) in [
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
                id,
                from,
                version,
                None,
                to,
                key,
                1_100 + version as i64,
            ))
            .unwrap();
    }
    let seed = id.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        hash.wrapping_mul(0x100000001b3)
            .wrapping_add(u64::from(byte))
    });
    let revision = format!("{seed:016x}").repeat(4);
    store
        .publish_validated_draft(&PublishValidatedDraft {
            proposal_id: id.to_string(),
            expected_version: 3,
            staging_job_id: format!("{id}-draft"),
            revision_sha256: revision.clone(),
            artifact_sha256: "c".repeat(64),
            kind: WorkflowArtifactKind::Skill,
            name: format!("{id}-skill"),
            validation_json: "{}".to_string(),
            actor: WORKER.to_string(),
            reason: "published".to_string(),
            idempotency_key: format!("{id}:published"),
            occurred_at_unix_ms: 2_000,
        })
        .unwrap();
    revision
}

fn request_test(store: &WorkflowLearningStore, id: &str, revision: &str, at: i64) -> String {
    let job_id = format!("{id}-test-job");
    store
        .approve_and_enqueue_isolated_test(
            &transition(
                id,
                WorkflowProposalState::Proposed,
                4,
                Some(revision),
                WorkflowProposalState::ApprovedPendingInstall,
                "test-requested",
                at,
            ),
            &NewWorkflowJob {
                id: job_id.clone(),
                idempotency_key: format!("{id}:test-job"),
                proposal_id: id.to_string(),
                revision_sha256: Some(revision.to_string()),
                kind: WorkflowJobKind::Install,
                payload_json: r#"{"requested_mode":"test"}"#.to_string(),
                max_attempts: 3,
                run_after_unix_ms: at,
                created_at_unix_ms: at,
            },
            &NewWorkflowIsolatedTest {
                id: format!("{id}-test"),
                idempotency_key: format!("{id}:isolated-test"),
                proposal_id: id.to_string(),
                revision_sha256: revision.to_string(),
                job_id: job_id.clone(),
                requested_by: WORKER.to_string(),
                requested_at_unix_ms: at,
            },
        )
        .unwrap();
    job_id
}

fn completion(
    id: &str,
    revision: &str,
    job_id: &str,
    passed: bool,
    at: i64,
) -> WorkflowIsolatedTestCompletion {
    WorkflowIsolatedTestCompletion {
        job_id: job_id.to_string(),
        worker: WORKER.to_string(),
        passed,
        result_json: format!(r#"{{"passed":{passed}}}"#),
        proposal_transition: transition(
            id,
            WorkflowProposalState::ApprovedPendingInstall,
            5,
            Some(revision),
            WorkflowProposalState::Proposed,
            "test-completed",
            at,
        ),
        notification: Some(NewWorkflowOutboxItem {
            id: format!("{id}-tested-notice"),
            idempotency_key: format!("{id}:tested-notice"),
            proposal_id: id.to_string(),
            revision_sha256: Some(revision.to_string()),
            topic: "workflow_learning.lifecycle".to_string(),
            payload_json: format!(r#"{{"passed":{passed}}}"#),
            max_attempts: 8,
            run_after_unix_ms: at,
            created_at_unix_ms: at,
        }),
        completed_at_unix_ms: at,
    }
}

#[test]
fn isolated_test_completion_atomically_unlocks_review_and_preserves_evidence() {
    let store = store();
    let id = "isolated-pass";
    let revision = create_proposed(&store, id);
    let job_id = request_test(&store, id, &revision, 2_100);

    let claimed = store.claim_due_job(WORKER, 2_100, 60_000).unwrap().unwrap();
    assert_eq!(claimed.id, job_id);
    assert_eq!(claimed.effect_state, WorkflowJobEffectState::None);
    let result = store
        .complete_isolated_test(&completion(id, &revision, &job_id, true, 2_200))
        .unwrap();

    assert_eq!(result.proposal.state, WorkflowProposalState::Proposed);
    assert_eq!(result.proposal.state_version, 6);
    assert_eq!(result.job.status, WorkflowJobStatus::Succeeded);
    assert_eq!(result.job.effect_state, WorkflowJobEffectState::Completed);
    assert_eq!(
        result.isolated_test.status,
        WorkflowIsolatedTestStatus::Passed
    );
    assert_eq!(
        result.isolated_test.result_json.as_deref(),
        Some(r#"{"passed":true}"#)
    );
    assert_eq!(
        result.notification.unwrap().status,
        WorkflowOutboxStatus::Pending
    );
    assert_eq!(
        store.get(id).unwrap().unwrap().isolated_test.unwrap(),
        result.isolated_test
    );
}

#[test]
fn failed_isolated_test_is_not_an_install_failure_and_can_be_retried() {
    let store = store();
    let id = "isolated-fail";
    let revision = create_proposed(&store, id);
    let job_id = request_test(&store, id, &revision, 2_100);
    store.claim_due_job(WORKER, 2_100, 60_000).unwrap();

    let result = store
        .complete_isolated_test(&completion(id, &revision, &job_id, false, 2_200))
        .unwrap();
    assert_eq!(result.proposal.state, WorkflowProposalState::Proposed);
    assert_eq!(
        result.isolated_test.status,
        WorkflowIsolatedTestStatus::Failed
    );
    assert!(store.get_installation(id, &revision).unwrap().is_none());
}

#[test]
fn crash_before_isolated_test_commit_is_replayed_without_uncertain_effect() {
    let store = store();
    let id = "isolated-restart";
    let revision = create_proposed(&store, id);
    let job_id = request_test(&store, id, &revision, 2_100);
    store
        .claim_due_job("worker-before-crash", 2_100, 60_000)
        .unwrap();

    let recovery = store.reconcile_jobs_after_restart(2_150).unwrap();
    assert_eq!(recovery.retried_without_effect, 1);
    assert_eq!(recovery.uncertain_effects, 0);
    let reclaimed = store.claim_due_job(WORKER, 2_150, 60_000).unwrap().unwrap();
    assert_eq!(reclaimed.id, job_id);
    let result = store
        .complete_isolated_test(&completion(id, &revision, &job_id, true, 2_200))
        .unwrap();
    assert_eq!(
        result.isolated_test.status,
        WorkflowIsolatedTestStatus::Passed
    );
}

#[test]
fn completion_rolls_back_when_lifecycle_notification_is_invalid() {
    let store = store();
    let id = "isolated-atomic";
    let revision = create_proposed(&store, id);
    let job_id = request_test(&store, id, &revision, 2_100);
    store.claim_due_job(WORKER, 2_100, 60_000).unwrap();
    let mut request = completion(id, &revision, &job_id, true, 2_200);
    request.notification.as_mut().unwrap().payload_json = "not-json".to_string();

    assert!(store.complete_isolated_test(&request).is_err());
    let proposal = store.get(id).unwrap().unwrap();
    assert_eq!(
        proposal.state,
        WorkflowProposalState::ApprovedPendingInstall
    );
    assert_eq!(
        proposal.isolated_test.unwrap().status,
        WorkflowIsolatedTestStatus::Queued
    );
    assert_eq!(
        store.get_job(&job_id).unwrap().unwrap().status,
        WorkflowJobStatus::Running
    );
}

#[test]
fn direct_unlock_cannot_bypass_the_atomic_test_completion() {
    let store = store();
    let id = "isolated-guard";
    let revision = create_proposed(&store, id);
    request_test(&store, id, &revision, 2_100);

    let direct = store.transition(&transition(
        id,
        WorkflowProposalState::ApprovedPendingInstall,
        5,
        Some(&revision),
        WorkflowProposalState::Proposed,
        "unsafe-unlock",
        2_200,
    ));
    assert!(direct.is_err());
    assert_eq!(
        store.get(id).unwrap().unwrap().state,
        WorkflowProposalState::ApprovedPendingInstall
    );
}

#[test]
fn isolated_test_claim_never_steals_an_activation_install() {
    let store = store();
    let activation_id = "activation-install";
    let activation_revision = create_proposed(&store, activation_id);
    store
        .approve_and_enqueue_install(
            &transition(
                activation_id,
                WorkflowProposalState::Proposed,
                4,
                Some(&activation_revision),
                WorkflowProposalState::ApprovedPendingInstall,
                "activate",
                2_050,
            ),
            &NewWorkflowJob {
                id: "activation-job".to_string(),
                idempotency_key: "activation-job:enqueue".to_string(),
                proposal_id: activation_id.to_string(),
                revision_sha256: Some(activation_revision),
                kind: WorkflowJobKind::Install,
                payload_json: r#"{"requested_mode":"activate"}"#.to_string(),
                max_attempts: 3,
                run_after_unix_ms: 2_050,
                created_at_unix_ms: 2_050,
            },
            None,
        )
        .unwrap();

    let test_id = "isolated-claim";
    let test_revision = create_proposed(&store, test_id);
    let test_job = request_test(&store, test_id, &test_revision, 2_100);
    let claimed = store
        .claim_due_isolated_test_job(WORKER, 2_100, 60_000)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, test_job);
    assert_eq!(
        store.get_job("activation-job").unwrap().unwrap().status,
        WorkflowJobStatus::Pending
    );
}

#[test]
fn activation_claim_never_steals_an_isolated_test_install() {
    let store = store();
    let test_id = "isolated-not-activation";
    let test_revision = create_proposed(&store, test_id);
    let test_job = request_test(&store, test_id, &test_revision, 2_000);

    let activation_id = "activation-only";
    let activation_revision = create_proposed(&store, activation_id);
    store
        .approve_and_enqueue_install(
            &transition(
                activation_id,
                WorkflowProposalState::Proposed,
                4,
                Some(&activation_revision),
                WorkflowProposalState::ApprovedPendingInstall,
                "activate",
                2_100,
            ),
            &NewWorkflowJob {
                id: "activation-only-job".to_string(),
                idempotency_key: "activation-only-job:enqueue".to_string(),
                proposal_id: activation_id.to_string(),
                revision_sha256: Some(activation_revision),
                kind: WorkflowJobKind::Install,
                payload_json: r#"{"requested_mode":"activate"}"#.to_string(),
                max_attempts: 3,
                run_after_unix_ms: 2_100,
                created_at_unix_ms: 2_100,
            },
            None,
        )
        .unwrap();

    let claimed = store
        .claim_due_activation_job(WORKER, 2_100, 60_000)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, "activation-only-job");
    assert_eq!(
        store.get_job(&test_job).unwrap().unwrap().status,
        WorkflowJobStatus::Pending
    );
}
