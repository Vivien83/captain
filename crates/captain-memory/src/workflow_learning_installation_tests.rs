use std::sync::{Arc, Barrier};

use rusqlite::TransactionBehavior;

use crate::workflow_learning_control::{
    NewWorkflowProposal, PublishValidatedDraft, WorkflowArtifactKind, WorkflowLearningStore,
    WorkflowProposalState, WorkflowProposalTransition,
};
use crate::workflow_learning_installation::{
    transition_installation_in_tx, NewWorkflowInstallation, WorkflowInstallationPhase,
    WorkflowInstallationTransition,
};
use crate::workflow_learning_queue::{NewWorkflowJob, WorkflowJobKind};
use crate::MemorySubstrate;

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
) -> WorkflowProposalTransition {
    WorkflowProposalTransition {
        proposal_id: id.to_string(),
        expected_state: from,
        expected_version: version,
        expected_revision_sha256: revision.map(str::to_string),
        to_state: to,
        actor: "operator:test".to_string(),
        reason: "test transition".to_string(),
        idempotency_key: format!("{id}:{suffix}"),
        snoozed_until_unix_ms: None,
        occurred_at_unix_ms: 2_000 + version as i64,
    }
}

fn create_approved(store: &WorkflowLearningStore, id: &str) -> String {
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
            .transition(&proposal_transition(id, from, version, None, to, suffix))
            .unwrap();
    }
    let revision = "b".repeat(64);
    store
        .publish_validated_draft(&PublishValidatedDraft {
            proposal_id: id.to_string(),
            expected_version: 3,
            staging_job_id: "draft-job".to_string(),
            revision_sha256: revision.clone(),
            artifact_sha256: "c".repeat(64),
            kind: WorkflowArtifactKind::Skill,
            name: "verified-research".to_string(),
            validation_json: r#"{"checks":"green"}"#.to_string(),
            actor: "captain:validator".to_string(),
            reason: "validated".to_string(),
            idempotency_key: format!("{id}:published"),
            occurred_at_unix_ms: 2_500,
        })
        .unwrap();
    let approval = proposal_transition(
        id,
        WorkflowProposalState::Proposed,
        4,
        Some(&revision),
        WorkflowProposalState::ApprovedPendingInstall,
        "approved",
    );
    let job = NewWorkflowJob {
        id: format!("{id}-install"),
        idempotency_key: format!("{id}:install-job"),
        proposal_id: id.to_string(),
        revision_sha256: Some(revision.clone()),
        kind: WorkflowJobKind::Install,
        payload_json: "{}".to_string(),
        max_attempts: 3,
        run_after_unix_ms: 3_000,
        created_at_unix_ms: 3_000,
    };
    store
        .approve_and_enqueue_install(&approval, &job, None)
        .unwrap();
    revision
}

fn prepared_input(id: &str, revision: &str) -> NewWorkflowInstallation {
    NewWorkflowInstallation {
        proposal_id: id.to_string(),
        revision_sha256: revision.to_string(),
        kind: WorkflowArtifactKind::Skill,
        target_locator: "skills/learned/verified-research.md".to_string(),
        backup_locator: Some(format!("learning/rollback/{id}/{revision}/previous.bin")),
        backup_sha256: Some("d".repeat(64)),
        installed_sha256: "c".repeat(64),
        actor: "captain:installer".to_string(),
        reason: "filesystem journal prepared".to_string(),
        idempotency_key: format!("{id}:installation-prepared"),
        occurred_at_unix_ms: 3_100,
    }
}

fn installation_transition(
    id: &str,
    revision: &str,
    from: WorkflowInstallationPhase,
    version: u64,
    to: WorkflowInstallationPhase,
    suffix: &str,
) -> WorkflowInstallationTransition {
    WorkflowInstallationTransition {
        proposal_id: id.to_string(),
        revision_sha256: revision.to_string(),
        expected_phase: from,
        expected_version: version,
        to_phase: to,
        last_error: None,
        actor: "captain:installer".to_string(),
        reason: "verified installation effect".to_string(),
        idempotency_key: format!("{id}:{suffix}"),
        occurred_at_unix_ms: 3_200 + version as i64,
    }
}

#[test]
fn prepared_promoted_verified_are_exact_idempotent_and_audited() {
    let store = store();
    let revision = create_approved(&store, "proposal-install");
    let input = prepared_input("proposal-install", &revision);

    let prepared = store.record_installation_prepared(&input).unwrap();
    let duplicate = store.record_installation_prepared(&input).unwrap();
    assert_eq!(prepared, duplicate);
    assert_eq!(prepared.phase, WorkflowInstallationPhase::Prepared);
    assert_eq!(prepared.phase_version, 0);

    let promoted_request = installation_transition(
        "proposal-install",
        &revision,
        WorkflowInstallationPhase::Prepared,
        0,
        WorkflowInstallationPhase::Promoted,
        "promoted",
    );
    let promoted = store
        .record_installation_promoted(&promoted_request)
        .unwrap();
    assert_eq!(promoted.phase, WorkflowInstallationPhase::Promoted);
    assert!(promoted.promoted_at_unix_ms.is_some());
    assert_eq!(
        store
            .record_installation_promoted(&promoted_request)
            .unwrap(),
        promoted
    );

    let verified = store
        .record_installation_verified(&installation_transition(
            "proposal-install",
            &revision,
            WorkflowInstallationPhase::Promoted,
            1,
            WorkflowInstallationPhase::Verified,
            "verified",
        ))
        .unwrap();
    assert_eq!(verified.phase, WorkflowInstallationPhase::Verified);
    assert_eq!(verified.phase_version, 2);
    assert!(verified.verified_at_unix_ms.is_some());
    assert_eq!(
        store.record_installation_prepared(&input).unwrap(),
        verified
    );
    assert_eq!(
        store
            .record_installation_promoted(&promoted_request)
            .unwrap(),
        verified
    );
    let events = store
        .installation_events("proposal-install", &revision)
        .unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].from_phase, None);
    assert_eq!(events[2].to_phase, WorkflowInstallationPhase::Verified);
}

#[test]
fn stale_concurrent_phase_change_has_one_winner() {
    let store = store();
    let revision = create_approved(&store, "proposal-race");
    store
        .record_installation_prepared(&prepared_input("proposal-race", &revision))
        .unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let handles = ["winner-a", "winner-b"]
        .into_iter()
        .map(|suffix| {
            let store = store.clone();
            let revision = revision.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let request = installation_transition(
                    "proposal-race",
                    &revision,
                    WorkflowInstallationPhase::Prepared,
                    0,
                    WorkflowInstallationPhase::Promoted,
                    suffix,
                );
                barrier.wait();
                store.record_installation_promoted(&request)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
    assert_eq!(
        store
            .get_installation("proposal-race", &revision)
            .unwrap()
            .unwrap()
            .phase,
        WorkflowInstallationPhase::Promoted
    );
}

#[test]
fn prepared_installation_rejects_wrong_hash_state_and_escaping_locator() {
    let store = store();
    let revision = create_approved(&store, "proposal-invalid");
    let mut wrong_hash = prepared_input("proposal-invalid", &revision);
    wrong_hash.installed_sha256 = "e".repeat(64);
    assert!(store.record_installation_prepared(&wrong_hash).is_err());

    let mut escaping = prepared_input("proposal-invalid", &revision);
    escaping.idempotency_key = "proposal-invalid:escaping".to_string();
    escaping.target_locator = "../outside.md".to_string();
    assert!(store.record_installation_prepared(&escaping).is_err());

    let mut current_directory = prepared_input("proposal-invalid", &revision);
    current_directory.idempotency_key = "proposal-invalid:current-directory".to_string();
    current_directory.target_locator = ".".to_string();
    assert!(store
        .record_installation_prepared(&current_directory)
        .is_err());
    assert!(store
        .get_installation("proposal-invalid", &revision)
        .unwrap()
        .is_none());
}

#[test]
fn rollback_and_quarantine_phases_preserve_error_evidence() {
    let store = store();
    let revision = create_approved(&store, "proposal-rollback");
    store
        .record_installation_prepared(&prepared_input("proposal-rollback", &revision))
        .unwrap();
    store
        .record_installation_promoted(&installation_transition(
            "proposal-rollback",
            &revision,
            WorkflowInstallationPhase::Prepared,
            0,
            WorkflowInstallationPhase::Promoted,
            "promoted",
        ))
        .unwrap();
    let mut pending = installation_transition(
        "proposal-rollback",
        &revision,
        WorkflowInstallationPhase::Promoted,
        1,
        WorkflowInstallationPhase::RollbackPending,
        "rollback-pending",
    );
    pending.last_error = Some("registry verification failed".to_string());
    store
        .record_installation_rollback_pending(&pending)
        .unwrap();

    let rolled_back_request = installation_transition(
        "proposal-rollback",
        &revision,
        WorkflowInstallationPhase::RollbackPending,
        2,
        WorkflowInstallationPhase::RolledBack,
        "rolled-back",
    );
    {
        let mut conn = store.lock_conn().unwrap();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        transition_installation_in_tx(&tx, &rolled_back_request).unwrap();
        tx.commit().unwrap();
    }
    let quarantined = store
        .record_installation_quarantined(&installation_transition(
            "proposal-rollback",
            &revision,
            WorkflowInstallationPhase::RolledBack,
            3,
            WorkflowInstallationPhase::Quarantined,
            "quarantined",
        ))
        .unwrap();
    assert_eq!(quarantined.phase, WorkflowInstallationPhase::Quarantined);
    assert!(quarantined.rolled_back_at_unix_ms.is_some());
}

#[test]
fn file_backed_restart_recovers_exact_installation_phase() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("memory.db");
    let revision;
    {
        let memory = MemorySubstrate::open(&database, 0.01).unwrap();
        let store = WorkflowLearningStore::new(memory.usage_conn());
        revision = create_approved(&store, "proposal-restart-install");
        store
            .record_installation_prepared(&prepared_input("proposal-restart-install", &revision))
            .unwrap();
        store
            .record_installation_promoted(&installation_transition(
                "proposal-restart-install",
                &revision,
                WorkflowInstallationPhase::Prepared,
                0,
                WorkflowInstallationPhase::Promoted,
                "promoted",
            ))
            .unwrap();
    }

    let memory = MemorySubstrate::open(&database, 0.01).unwrap();
    let store = WorkflowLearningStore::new(memory.usage_conn());
    let recovered = store
        .get_installation("proposal-restart-install", &revision)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.phase, WorkflowInstallationPhase::Promoted);
    assert_eq!(recovered.phase_version, 1);
    assert_eq!(
        store
            .installation_events("proposal-restart-install", &revision)
            .unwrap()
            .len(),
        2
    );
}
