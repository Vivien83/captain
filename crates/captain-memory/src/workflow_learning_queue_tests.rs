use crate::workflow_learning_control::{
    NewWorkflowProposal, PublishValidatedDraft, WorkflowArtifactKind, WorkflowLearningStore,
    WorkflowProposalState, WorkflowProposalTransition,
};
use crate::workflow_learning_outbox::{NewWorkflowOutboxItem, WorkflowOutboxStatus};
use crate::workflow_learning_queue::{
    NewWorkflowJob, WorkflowJobEffectState, WorkflowJobKind, WorkflowJobStatus,
};
use crate::MemorySubstrate;

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
) -> WorkflowProposalTransition {
    WorkflowProposalTransition {
        proposal_id: id.to_string(),
        expected_state: from,
        expected_version: version,
        expected_revision_sha256: revision.map(str::to_string),
        to_state: to,
        actor: "operator:test".to_string(),
        reason: "test decision".to_string(),
        idempotency_key: key.to_string(),
        snoozed_until_unix_ms: None,
        occurred_at_unix_ms: 2_000 + version as i64,
    }
}

fn proposed(store: &WorkflowLearningStore, id: &str) -> String {
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
            .transition(&transition(
                id,
                from,
                version,
                None,
                to,
                &format!("{id}:{suffix}"),
            ))
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
    revision
}

fn job(
    id: &str,
    proposal_id: &str,
    revision: Option<&str>,
    kind: WorkflowJobKind,
) -> NewWorkflowJob {
    NewWorkflowJob {
        id: id.to_string(),
        idempotency_key: format!("job:{id}"),
        proposal_id: proposal_id.to_string(),
        revision_sha256: revision.map(str::to_string),
        kind,
        payload_json: "{}".to_string(),
        max_attempts: 3,
        run_after_unix_ms: 3_000,
        created_at_unix_ms: 2_900,
    }
}

#[test]
fn job_without_started_effect_is_retried_after_crash() {
    let store = store();
    store
        .create_observed(&NewWorkflowProposal {
            id: "proposal-1".to_string(),
            idempotency_key: "proposal-1:observed".to_string(),
            workflow_signature: "a".repeat(64),
            source_agent_id: "captain".to_string(),
            origin_channel: None,
            evidence_json: "{}".to_string(),
            created_at_unix_ms: 1,
        })
        .unwrap();
    store
        .enqueue_job(&job(
            "analyze-1",
            "proposal-1",
            None,
            WorkflowJobKind::Analyze,
        ))
        .unwrap();
    store
        .claim_due_job("worker-dead", 3_000, 1_000)
        .unwrap()
        .unwrap();

    let recovery = store.reconcile_expired_jobs(4_001).unwrap();
    assert_eq!(recovery.retried_without_effect, 1);
    let reclaimed = store
        .claim_due_job("worker-new", 4_001, 1_000)
        .unwrap()
        .unwrap();
    assert_eq!(reclaimed.id, "analyze-1");
    assert_eq!(reclaimed.attempt_count, 2);
}

#[test]
fn started_model_or_install_effect_is_never_replayed_automatically() {
    let store = store();
    store
        .create_observed(&NewWorkflowProposal {
            id: "proposal-2".to_string(),
            idempotency_key: "proposal-2:observed".to_string(),
            workflow_signature: "a".repeat(64),
            source_agent_id: "captain".to_string(),
            origin_channel: None,
            evidence_json: "{}".to_string(),
            created_at_unix_ms: 1,
        })
        .unwrap();
    store
        .transition(&transition(
            "proposal-2",
            WorkflowProposalState::Observed,
            0,
            None,
            WorkflowProposalState::Eligible,
            "proposal-2:eligible",
        ))
        .unwrap();
    store
        .transition(&transition(
            "proposal-2",
            WorkflowProposalState::Eligible,
            1,
            None,
            WorkflowProposalState::Drafting,
            "proposal-2:drafting",
        ))
        .unwrap();
    store
        .enqueue_job(&job("draft-1", "proposal-2", None, WorkflowJobKind::Draft))
        .unwrap();
    store.claim_due_job("worker-dead", 3_000, 1_000).unwrap();
    let started = store
        .mark_job_effect_started("draft-1", "worker-dead", 3_100)
        .unwrap();
    assert_eq!(started.effect_state, WorkflowJobEffectState::Started);

    let recovery = store.reconcile_expired_jobs(4_001).unwrap();
    assert_eq!(recovery.uncertain_effects, 1);
    assert!(store
        .claim_due_job("worker-new", 4_001, 1_000)
        .unwrap()
        .is_none());
    assert_eq!(
        store.get_job("draft-1").unwrap().unwrap().status,
        WorkflowJobStatus::Uncertain
    );
}

#[test]
fn failed_job_retries_only_before_effect_start() {
    let store = store();
    store
        .create_observed(&NewWorkflowProposal {
            id: "proposal-3".to_string(),
            idempotency_key: "proposal-3:observed".to_string(),
            workflow_signature: "a".repeat(64),
            source_agent_id: "captain".to_string(),
            origin_channel: None,
            evidence_json: "{}".to_string(),
            created_at_unix_ms: 1,
        })
        .unwrap();
    store
        .enqueue_job(&job(
            "analyze-2",
            "proposal-3",
            None,
            WorkflowJobKind::Analyze,
        ))
        .unwrap();
    store.claim_due_job("worker", 3_000, 1_000).unwrap();
    let retry = store
        .fail_job(
            "analyze-2",
            "worker",
            "provider_unavailable",
            "temporary outage",
            true,
            5_000,
            3_100,
        )
        .unwrap();
    assert_eq!(retry.status, WorkflowJobStatus::RetryWait);
}

#[test]
fn observed_model_failure_can_retry_but_an_interrupted_call_cannot() {
    let store = store();
    store
        .create_observed(&NewWorkflowProposal {
            id: "proposal-known-model-error".to_string(),
            idempotency_key: "proposal-known-model-error:observed".to_string(),
            workflow_signature: "a".repeat(64),
            source_agent_id: "captain".to_string(),
            origin_channel: None,
            evidence_json: "{}".to_string(),
            created_at_unix_ms: 1,
        })
        .unwrap();
    store
        .transition(&transition(
            "proposal-known-model-error",
            WorkflowProposalState::Observed,
            0,
            None,
            WorkflowProposalState::Eligible,
            "proposal-known-model-error:eligible",
        ))
        .unwrap();
    store
        .transition(&transition(
            "proposal-known-model-error",
            WorkflowProposalState::Eligible,
            1,
            None,
            WorkflowProposalState::Drafting,
            "proposal-known-model-error:drafting",
        ))
        .unwrap();
    store
        .enqueue_job(&job(
            "draft-known-model-error",
            "proposal-known-model-error",
            None,
            WorkflowJobKind::Draft,
        ))
        .unwrap();
    store.claim_due_job("worker", 3_000, 10_000).unwrap();
    store
        .mark_job_effect_started("draft-known-model-error", "worker", 3_001)
        .unwrap();

    let retry = store
        .fail_job_after_known_effect(
            "draft-known-model-error",
            "worker",
            "invalid_structured_output",
            "model returned invalid JSON",
            true,
            5_000,
            3_100,
            None,
        )
        .unwrap();
    assert_eq!(retry.status, WorkflowJobStatus::RetryWait);
    assert_eq!(retry.effect_state, WorkflowJobEffectState::None);
    assert_eq!(
        store.list_uncertain_jobs(10).unwrap(),
        Vec::<crate::workflow_learning_queue::WorkflowJobRecord>::new()
    );
    assert_eq!(
        store
            .fail_job_after_known_effect(
                "draft-known-model-error",
                "worker",
                "invalid_structured_output",
                "model returned invalid JSON",
                true,
                5_000,
                3_100,
                None,
            )
            .unwrap(),
        retry
    );
}

#[test]
fn expired_job_lease_cannot_complete_before_reconciliation() {
    let store = store();
    store
        .create_observed(&NewWorkflowProposal {
            id: "proposal-expired".to_string(),
            idempotency_key: "proposal-expired:observed".to_string(),
            workflow_signature: "a".repeat(64),
            source_agent_id: "captain".to_string(),
            origin_channel: None,
            evidence_json: "{}".to_string(),
            created_at_unix_ms: 1,
        })
        .unwrap();
    store
        .enqueue_job(&job(
            "analyze-expired",
            "proposal-expired",
            None,
            WorkflowJobKind::Analyze,
        ))
        .unwrap();
    store.claim_due_job("worker", 3_000, 1_000).unwrap();
    assert!(store
        .complete_job("analyze-expired", "worker", Some("{}"), 4_001)
        .is_err());
    assert_eq!(
        store
            .reconcile_expired_jobs(4_001)
            .unwrap()
            .retried_without_effect,
        1
    );
}

#[test]
fn approval_transition_install_job_and_notification_commit_together() {
    let store = store();
    let revision = proposed(&store, "proposal-4");
    let approval = transition(
        "proposal-4",
        WorkflowProposalState::Proposed,
        4,
        Some(&revision),
        WorkflowProposalState::ApprovedPendingInstall,
        "proposal-4:approve",
    );
    let install = job(
        "install-1",
        "proposal-4",
        Some(&revision),
        WorkflowJobKind::Install,
    );
    let notification = NewWorkflowOutboxItem {
        id: "approval-message".to_string(),
        idempotency_key: "proposal-4:approval-message".to_string(),
        proposal_id: "proposal-4".to_string(),
        revision_sha256: Some(revision.clone()),
        topic: "workflow.approved".to_string(),
        payload_json: r#"{"state":"approved_pending_install"}"#.to_string(),
        max_attempts: 8,
        run_after_unix_ms: 3_000,
        created_at_unix_ms: 3_000,
    };

    let approved = store
        .approve_and_enqueue_install(&approval, &install, Some(&notification))
        .unwrap();
    assert_eq!(
        approved.state,
        WorkflowProposalState::ApprovedPendingInstall
    );
    assert_eq!(
        store.get_job("install-1").unwrap().unwrap().kind,
        WorkflowJobKind::Install
    );
    assert_eq!(
        store
            .get_outbox("approval-message")
            .unwrap()
            .unwrap()
            .status,
        WorkflowOutboxStatus::Pending
    );

    let duplicate = store
        .approve_and_enqueue_install(&approval, &install, Some(&notification))
        .unwrap();
    assert_eq!(duplicate.id, approved.id);
}

#[test]
fn preapproval_claim_never_takes_installation_work() {
    let store = store();
    let revision = proposed(&store, "proposal-install-scope");
    let approval = transition(
        "proposal-install-scope",
        WorkflowProposalState::Proposed,
        4,
        Some(&revision),
        WorkflowProposalState::ApprovedPendingInstall,
        "proposal-install-scope:approve",
    );
    let install = job(
        "a-install-scope",
        "proposal-install-scope",
        Some(&revision),
        WorkflowJobKind::Install,
    );
    store
        .approve_and_enqueue_install(&approval, &install, None)
        .unwrap();

    store
        .create_observed(&NewWorkflowProposal {
            id: "proposal-analysis-scope".to_string(),
            idempotency_key: "proposal-analysis-scope:observed".to_string(),
            workflow_signature: "d".repeat(64),
            source_agent_id: "captain".to_string(),
            origin_channel: None,
            evidence_json: "{}".to_string(),
            created_at_unix_ms: 1_000,
        })
        .unwrap();
    store
        .enqueue_job(&job(
            "z-analyze-scope",
            "proposal-analysis-scope",
            None,
            WorkflowJobKind::Analyze,
        ))
        .unwrap();

    let preapproval = store
        .claim_due_preapproval_job("preapproval-worker", 3_000, 1_000)
        .unwrap()
        .unwrap();
    assert_eq!(preapproval.id, "z-analyze-scope");
    assert_eq!(preapproval.kind, WorkflowJobKind::Analyze);

    let installation = store
        .claim_due_job("installation-worker", 3_000, 1_000)
        .unwrap()
        .unwrap();
    assert_eq!(installation.id, "a-install-scope");
    assert_eq!(installation.kind, WorkflowJobKind::Install);
}

#[test]
fn observed_proposal_rolls_back_when_analysis_job_is_invalid() {
    let store = store();
    let proposal = NewWorkflowProposal {
        id: "proposal-atomic-observe".to_string(),
        idempotency_key: "proposal-atomic-observe:observed".to_string(),
        workflow_signature: "e".repeat(64),
        source_agent_id: "captain".to_string(),
        origin_channel: None,
        evidence_json: "{}".to_string(),
        created_at_unix_ms: 1_000,
    };
    let mut analysis = job(
        "analyze-atomic-observe",
        &proposal.id,
        None,
        WorkflowJobKind::Analyze,
    );
    analysis.payload_json = "not-json".to_string();

    assert!(store
        .observe_and_enqueue_analysis(&proposal, &analysis)
        .is_err());
    assert!(store.get(&proposal.id).unwrap().is_none());
    assert!(store.get_job(&analysis.id).unwrap().is_none());
}

#[test]
fn invalid_atomic_approval_rolls_back_the_state_transition() {
    let store = store();
    let revision = proposed(&store, "proposal-5");
    let approval = transition(
        "proposal-5",
        WorkflowProposalState::Proposed,
        4,
        Some(&revision),
        WorkflowProposalState::ApprovedPendingInstall,
        "proposal-5:approve",
    );
    let install = job(
        "install-2",
        "proposal-5",
        Some(&revision),
        WorkflowJobKind::Install,
    );
    let invalid_notification = NewWorkflowOutboxItem {
        id: "broken-message".to_string(),
        idempotency_key: "proposal-5:broken-message".to_string(),
        proposal_id: "proposal-5".to_string(),
        revision_sha256: Some(revision),
        topic: "workflow.approved".to_string(),
        payload_json: "not-json".to_string(),
        max_attempts: 8,
        run_after_unix_ms: 3_000,
        created_at_unix_ms: 3_000,
    };

    assert!(store
        .approve_and_enqueue_install(&approval, &install, Some(&invalid_notification))
        .is_err());
    assert_eq!(
        store.get("proposal-5").unwrap().unwrap().state,
        WorkflowProposalState::Proposed
    );
    assert!(store.get_job("install-2").unwrap().is_none());
}

#[test]
fn file_backed_restart_immediately_recovers_an_unstarted_job_lease() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("memory.db");
    {
        let memory = MemorySubstrate::open(&database, 0.01).unwrap();
        let store = WorkflowLearningStore::new(memory.usage_conn());
        store
            .create_observed(&NewWorkflowProposal {
                id: "proposal-restart".to_string(),
                idempotency_key: "proposal-restart:observed".to_string(),
                workflow_signature: "a".repeat(64),
                source_agent_id: "captain".to_string(),
                origin_channel: None,
                evidence_json: "{}".to_string(),
                created_at_unix_ms: 1,
            })
            .unwrap();
        store
            .enqueue_job(&job(
                "analyze-restart",
                "proposal-restart",
                None,
                WorkflowJobKind::Analyze,
            ))
            .unwrap();
        store
            .claim_due_job("worker-before-crash", 3_000, 10_000)
            .unwrap();
    }

    let memory = MemorySubstrate::open(&database, 0.01).unwrap();
    let store = WorkflowLearningStore::new(memory.usage_conn());
    let summary = store.reconcile_jobs_after_restart(3_100).unwrap();
    assert_eq!(summary.retried_without_effect, 1);
    let recovered = store
        .claim_due_job("worker-after-restart", 3_100, 1_000)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.id, "analyze-restart");
    assert_eq!(recovered.attempt_count, 2);
}

#[test]
fn restart_routes_a_started_install_only_to_exact_activation_recovery() {
    let store = store();
    let revision = proposed(&store, "proposal-restart-effect");
    let approval = transition(
        "proposal-restart-effect",
        WorkflowProposalState::Proposed,
        4,
        Some(&revision),
        WorkflowProposalState::ApprovedPendingInstall,
        "proposal-restart-effect:approve",
    );
    let install = job(
        "install-restart-effect",
        "proposal-restart-effect",
        Some(&revision),
        WorkflowJobKind::Install,
    );
    store
        .approve_and_enqueue_install(&approval, &install, None)
        .unwrap();
    store
        .claim_due_job("worker-before-restart", 3_000, 10_000)
        .unwrap();
    store
        .mark_job_effect_started("install-restart-effect", "worker-before-restart", 3_001)
        .unwrap();

    let summary = store.reconcile_jobs_after_restart(3_100).unwrap();
    assert_eq!(summary.uncertain_effects, 1);
    let interrupted = store.get_job("install-restart-effect").unwrap().unwrap();
    assert_eq!(interrupted.status, WorkflowJobStatus::Uncertain);
    assert_eq!(
        interrupted.error_code.as_deref(),
        Some("effect_interrupted")
    );
    assert!(store
        .claim_due_job("worker-after-restart", 3_100, 1_000)
        .unwrap()
        .is_none());
    let recovered = store
        .claim_uncertain_activation_job("activation-recovery", 3_100, 1_000)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.id, "install-restart-effect");
    assert_eq!(recovered.status, WorkflowJobStatus::Running);
    assert_eq!(recovered.effect_state, WorkflowJobEffectState::Started);
    assert_eq!(recovered.attempt_count, 1);
    assert_eq!(
        recovered.lease_owner.as_deref(),
        Some("activation-recovery")
    );
}
