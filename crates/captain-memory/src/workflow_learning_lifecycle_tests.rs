use crate::workflow_learning_control::{
    NewWorkflowProposal, PublishValidatedDraft, WorkflowArtifactKind, WorkflowLearningStore,
    WorkflowProposalState, WorkflowProposalTransition,
};
use crate::workflow_learning_installation::{
    NewWorkflowInstallation, WorkflowInstallationPhase, WorkflowInstallationTransition,
};
use crate::workflow_learning_lifecycle::{
    WorkflowCanaryCompletion, WorkflowEffectFailure, WorkflowInstallCompletion,
    WorkflowRollbackCompletion,
};
use crate::workflow_learning_outbox::{NewWorkflowOutboxItem, WorkflowOutboxStatus};
use crate::workflow_learning_queue::{
    NewWorkflowJob, WorkflowJobEffectState, WorkflowJobKind, WorkflowJobStatus,
};
use crate::MemorySubstrate;

const WORKER: &str = "captain:workflow-learning-worker";

fn store() -> WorkflowLearningStore {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    WorkflowLearningStore::new(memory.usage_conn())
}

fn proposal_transition(
    id: &str,
    from: WorkflowProposalState,
    version: u64,
    revision: Option<&str>,
    to: WorkflowProposalState,
    suffix: &str,
    at: i64,
) -> WorkflowProposalTransition {
    WorkflowProposalTransition {
        proposal_id: id.to_string(),
        expected_state: from,
        expected_version: version,
        expected_revision_sha256: revision.map(str::to_string),
        to_state: to,
        actor: WORKER.to_string(),
        reason: format!("lifecycle {suffix}"),
        idempotency_key: format!("{id}:{suffix}"),
        snoozed_until_unix_ms: None,
        occurred_at_unix_ms: at,
    }
}

fn create_approved(store: &WorkflowLearningStore, id: &str) -> (String, String) {
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
            .transition(&proposal_transition(
                id,
                from,
                version,
                None,
                to,
                suffix,
                1_100 + version as i64,
            ))
            .unwrap();
    }
    let revision = "b".repeat(64);
    store
        .publish_validated_draft(&PublishValidatedDraft {
            proposal_id: id.to_string(),
            expected_version: 3,
            staging_job_id: format!("{id}-draft"),
            revision_sha256: revision.clone(),
            artifact_sha256: "c".repeat(64),
            kind: WorkflowArtifactKind::Skill,
            name: format!("{id}-skill"),
            validation_json: r#"{"checks":"green"}"#.to_string(),
            actor: WORKER.to_string(),
            reason: "validated exact draft".to_string(),
            idempotency_key: format!("{id}:published"),
            occurred_at_unix_ms: 2_000,
        })
        .unwrap();
    let install_id = format!("{id}-install");
    store
        .approve_and_enqueue_install(
            &proposal_transition(
                id,
                WorkflowProposalState::Proposed,
                4,
                Some(&revision),
                WorkflowProposalState::ApprovedPendingInstall,
                "approved",
                2_100,
            ),
            &job(&install_id, id, &revision, WorkflowJobKind::Install, 2_100),
            None,
        )
        .unwrap();
    (revision, install_id)
}

fn job(
    id: &str,
    proposal_id: &str,
    revision: &str,
    kind: WorkflowJobKind,
    at: i64,
) -> NewWorkflowJob {
    NewWorkflowJob {
        id: id.to_string(),
        idempotency_key: format!("{id}:enqueue"),
        proposal_id: proposal_id.to_string(),
        revision_sha256: Some(revision.to_string()),
        kind,
        payload_json: "{}".to_string(),
        max_attempts: 3,
        run_after_unix_ms: at,
        created_at_unix_ms: at,
    }
}

fn notification(id: &str, proposal_id: &str, revision: &str, at: i64) -> NewWorkflowOutboxItem {
    NewWorkflowOutboxItem {
        id: id.to_string(),
        idempotency_key: format!("{id}:outbox"),
        proposal_id: proposal_id.to_string(),
        revision_sha256: Some(revision.to_string()),
        topic: "workflow_learning.lifecycle".to_string(),
        payload_json: "{}".to_string(),
        max_attempts: 5,
        run_after_unix_ms: at,
        created_at_unix_ms: at,
    }
}

fn claim_and_start(store: &WorkflowLearningStore, id: &str, at: i64) {
    let claimed = store.claim_due_job(WORKER, at, 60_000).unwrap().unwrap();
    assert_eq!(claimed.id, id);
    store.mark_job_effect_started(id, WORKER, at + 1).unwrap();
}

fn installation_transition(
    id: &str,
    revision: &str,
    from: WorkflowInstallationPhase,
    version: u64,
    to: WorkflowInstallationPhase,
    suffix: &str,
    at: i64,
    last_error: Option<&str>,
) -> WorkflowInstallationTransition {
    WorkflowInstallationTransition {
        proposal_id: id.to_string(),
        revision_sha256: revision.to_string(),
        expected_phase: from,
        expected_version: version,
        to_phase: to,
        last_error: last_error.map(str::to_string),
        actor: WORKER.to_string(),
        reason: format!("installation {suffix}"),
        idempotency_key: format!("{id}:installation:{suffix}"),
        occurred_at_unix_ms: at,
    }
}

fn prepare_installation(store: &WorkflowLearningStore, id: &str, revision: &str) {
    store
        .record_installation_prepared(&NewWorkflowInstallation {
            proposal_id: id.to_string(),
            revision_sha256: revision.to_string(),
            kind: WorkflowArtifactKind::Skill,
            target_locator: format!("skills/learned/{id}-skill.md"),
            backup_locator: None,
            backup_sha256: None,
            installed_sha256: "c".repeat(64),
            actor: WORKER.to_string(),
            reason: "filesystem journal prepared".to_string(),
            idempotency_key: format!("{id}:installation:prepared"),
            occurred_at_unix_ms: 3_100,
        })
        .unwrap();
}

fn promote_and_verify(store: &WorkflowLearningStore, id: &str, revision: &str) {
    store
        .record_installation_promoted(&installation_transition(
            id,
            revision,
            WorkflowInstallationPhase::Prepared,
            0,
            WorkflowInstallationPhase::Promoted,
            "promoted",
            3_200,
            None,
        ))
        .unwrap();
    store
        .record_installation_verified(&installation_transition(
            id,
            revision,
            WorkflowInstallationPhase::Promoted,
            1,
            WorkflowInstallationPhase::Verified,
            "verified",
            3_300,
            None,
        ))
        .unwrap();
}

#[test]
fn install_and_canary_success_commit_every_related_row_and_replay_late() {
    let store = store();
    let id = "lifecycle-happy";
    let (revision, install_id) = create_approved(&store, id);
    claim_and_start(&store, &install_id, 3_000);
    prepare_installation(&store, id, &revision);
    promote_and_verify(&store, id, &revision);

    let install = WorkflowInstallCompletion {
        job_id: install_id.clone(),
        worker: WORKER.to_string(),
        result_json: Some(r#"{"registry":"verified"}"#.to_string()),
        proposal_transition: proposal_transition(
            id,
            WorkflowProposalState::ApprovedPendingInstall,
            5,
            Some(&revision),
            WorkflowProposalState::ActiveCanary,
            "install-complete",
            3_400,
        ),
        canary_job: job(
            &format!("{id}-canary"),
            id,
            &revision,
            WorkflowJobKind::Canary,
            3_500,
        ),
        notification: Some(notification(
            &format!("{id}-installed"),
            id,
            &revision,
            3_400,
        )),
        completed_at_unix_ms: 3_400,
    };
    let installed = store.complete_install_and_enqueue_canary(&install).unwrap();
    assert_eq!(
        installed.proposal.state,
        WorkflowProposalState::ActiveCanary
    );
    assert_eq!(installed.job.status, WorkflowJobStatus::Succeeded);
    assert_eq!(
        installed.next_job.as_ref().unwrap().kind,
        WorkflowJobKind::Canary
    );
    assert_eq!(
        installed.notification.as_ref().unwrap().status,
        WorkflowOutboxStatus::Pending
    );

    let canary_id = format!("{id}-canary");
    claim_and_start(&store, &canary_id, 3_500);
    let canary = WorkflowCanaryCompletion {
        job_id: canary_id,
        worker: WORKER.to_string(),
        result_json: Some(r#"{"canary":"passed"}"#.to_string()),
        proposal_transition: proposal_transition(
            id,
            WorkflowProposalState::ActiveCanary,
            6,
            Some(&revision),
            WorkflowProposalState::Active,
            "canary-complete",
            3_600,
        ),
        installation_transition: installation_transition(
            id,
            &revision,
            WorkflowInstallationPhase::Verified,
            2,
            WorkflowInstallationPhase::Active,
            "active",
            3_600,
            None,
        ),
        notification: None,
        completed_at_unix_ms: 3_600,
    };
    let active = store.complete_canary_activation(&canary).unwrap();
    assert_eq!(active.proposal.state, WorkflowProposalState::Active);
    assert_eq!(
        active.installation.as_ref().unwrap().phase,
        WorkflowInstallationPhase::Active
    );

    let replay = store.complete_install_and_enqueue_canary(&install).unwrap();
    assert_eq!(replay.proposal.state, WorkflowProposalState::Active);
    assert_eq!(
        replay.installation.as_ref().unwrap().phase,
        WorkflowInstallationPhase::Active
    );
    let mut altered = install.clone();
    altered.proposal_transition.reason = "different intent".to_string();
    assert!(store.complete_install_and_enqueue_canary(&altered).is_err());
}

#[test]
fn known_install_failure_and_rollback_are_atomic_and_idempotent() {
    let store = store();
    let id = "lifecycle-rollback";
    let (revision, install_id) = create_approved(&store, id);
    claim_and_start(&store, &install_id, 3_000);
    prepare_installation(&store, id, &revision);
    store
        .record_installation_promoted(&installation_transition(
            id,
            &revision,
            WorkflowInstallationPhase::Prepared,
            0,
            WorkflowInstallationPhase::Promoted,
            "promoted",
            3_200,
            None,
        ))
        .unwrap();

    let error = "registry verification failed";
    let rollback_id = format!("{id}-rollback");
    let failure = WorkflowEffectFailure {
        job_id: install_id,
        worker: WORKER.to_string(),
        job_kind: WorkflowJobKind::Install,
        error_code: "registry_verification_failed".to_string(),
        error_message: error.to_string(),
        proposal_transition: proposal_transition(
            id,
            WorkflowProposalState::ApprovedPendingInstall,
            5,
            Some(&revision),
            WorkflowProposalState::InstallFailed,
            "install-failed",
            3_400,
        ),
        installation_transition: Some(installation_transition(
            id,
            &revision,
            WorkflowInstallationPhase::Promoted,
            1,
            WorkflowInstallationPhase::Failed,
            "failed",
            3_400,
            Some(error),
        )),
        rollback_job: Some(job(
            &rollback_id,
            id,
            &revision,
            WorkflowJobKind::Rollback,
            3_500,
        )),
        notification: None,
        failed_at_unix_ms: 3_400,
    };
    let failed = store
        .fail_known_effect_and_schedule_rollback(&failure)
        .unwrap();
    assert_eq!(failed.proposal.state, WorkflowProposalState::InstallFailed);
    assert_eq!(
        failed.proposal.last_error_code.as_deref(),
        Some("registry_verification_failed")
    );
    assert_eq!(failed.job.status, WorkflowJobStatus::Dead);
    assert_eq!(failed.job.effect_state, WorkflowJobEffectState::Completed);
    assert_eq!(
        failed.installation.as_ref().unwrap().phase,
        WorkflowInstallationPhase::Failed
    );

    claim_and_start(&store, &rollback_id, 3_500);
    store
        .record_installation_rollback_pending(&installation_transition(
            id,
            &revision,
            WorkflowInstallationPhase::Failed,
            2,
            WorkflowInstallationPhase::RollbackPending,
            "rollback-pending",
            3_501,
            Some(error),
        ))
        .unwrap();
    let rollback = WorkflowRollbackCompletion {
        job_id: rollback_id,
        worker: WORKER.to_string(),
        result_json: Some(r#"{"rollback":"verified"}"#.to_string()),
        proposal_transition: proposal_transition(
            id,
            WorkflowProposalState::InstallFailed,
            6,
            Some(&revision),
            WorkflowProposalState::RolledBack,
            "rollback-complete",
            3_600,
        ),
        installation_transition: installation_transition(
            id,
            &revision,
            WorkflowInstallationPhase::RollbackPending,
            3,
            WorkflowInstallationPhase::RolledBack,
            "rolled-back",
            3_600,
            None,
        ),
        notification: Some(notification(
            &format!("{id}-rolled-back"),
            id,
            &revision,
            3_600,
        )),
        completed_at_unix_ms: 3_600,
    };
    let rolled_back = store.complete_rollback(&rollback).unwrap();
    assert_eq!(
        rolled_back.proposal.state,
        WorkflowProposalState::RolledBack
    );
    assert_eq!(
        rolled_back.installation.as_ref().unwrap().phase,
        WorkflowInstallationPhase::RolledBack
    );
    let replay = store
        .fail_known_effect_and_schedule_rollback(&failure)
        .unwrap();
    assert_eq!(replay.proposal.state, WorkflowProposalState::RolledBack);
}

#[test]
fn failure_before_prepare_settles_without_fabricating_a_rollback() {
    let store = store();
    let id = "lifecycle-no-effect";
    let (revision, install_id) = create_approved(&store, id);
    claim_and_start(&store, &install_id, 3_000);
    let failure = WorkflowEffectFailure {
        job_id: install_id,
        worker: WORKER.to_string(),
        job_kind: WorkflowJobKind::Install,
        error_code: "unsafe_filesystem".to_string(),
        error_message: "promotion root is a symlink".to_string(),
        proposal_transition: proposal_transition(
            id,
            WorkflowProposalState::ApprovedPendingInstall,
            5,
            Some(&revision),
            WorkflowProposalState::InstallFailed,
            "prepare-failed",
            3_100,
        ),
        installation_transition: None,
        rollback_job: None,
        notification: None,
        failed_at_unix_ms: 3_100,
    };
    let result = store
        .fail_known_effect_and_schedule_rollback(&failure)
        .unwrap();
    assert_eq!(result.proposal.state, WorkflowProposalState::InstallFailed);
    assert!(result.installation.is_none());
    assert!(result.next_job.is_none());
    assert!(store.get_installation(id, &revision).unwrap().is_none());
}

#[test]
fn invalid_canary_relation_rolls_back_every_partial_mutation() {
    let store = store();
    let id = "lifecycle-invalid";
    let (revision, install_id) = create_approved(&store, id);
    claim_and_start(&store, &install_id, 3_000);
    prepare_installation(&store, id, &revision);
    promote_and_verify(&store, id, &revision);
    let request = WorkflowInstallCompletion {
        job_id: install_id.clone(),
        worker: WORKER.to_string(),
        result_json: None,
        proposal_transition: proposal_transition(
            id,
            WorkflowProposalState::ApprovedPendingInstall,
            5,
            Some(&revision),
            WorkflowProposalState::ActiveCanary,
            "install-complete",
            3_400,
        ),
        canary_job: job(
            "wrong-canary",
            id,
            &revision,
            WorkflowJobKind::Rollback,
            3_500,
        ),
        notification: None,
        completed_at_unix_ms: 3_400,
    };
    assert!(store.complete_install_and_enqueue_canary(&request).is_err());
    assert_eq!(
        store.get(id).unwrap().unwrap().state,
        WorkflowProposalState::ApprovedPendingInstall
    );
    assert_eq!(
        store.get_job(&install_id).unwrap().unwrap().status,
        WorkflowJobStatus::Running
    );
}
