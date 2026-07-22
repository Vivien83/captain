use std::fs;
use std::sync::RwLock;

use captain_memory::workflow_learning_control::{
    NewWorkflowProposal, PublishValidatedDraft, WorkflowArtifactKind, WorkflowLearningStore,
    WorkflowProposalState, WorkflowProposalTransition,
};
use captain_memory::workflow_learning_installation::WorkflowInstallationPhase;
use captain_memory::workflow_learning_queue::{NewWorkflowJob, WorkflowJobKind, WorkflowJobStatus};
use captain_memory::MemorySubstrate;
use captain_runtime::workflow_learning_analysis::{
    CanonicalWorkflow, CanonicalWorkflowNode, WorkflowClassification, WorkflowGroupAnalysis,
    WorkflowScope,
};
use captain_runtime::workflow_learning_delivery::{
    WorkflowDeliveryDisposition, WorkflowDeliveryEvent, WorkflowDeliveryPlanner,
};
use captain_runtime::workflow_learning_operator::WorkflowInstallRequestPayload;
use captain_runtime::workflow_learning_promotion::WorkflowPromotionRoot;
use captain_runtime::workflow_learning_promotion_types::PromoteWorkflowDraftRequest;
use captain_runtime::workflow_learning_proposer::{
    ActiveModelIdentity, AutomationScheduleDraft, WorkflowDraft, WorkflowDraftArtifact,
    WorkflowDraftKind,
};
use captain_runtime::workflow_learning_staging::{StageWorkflowDraftRequest, WorkflowStagingRoot};
use captain_skills::registry::SkillRegistry;
use captain_types::agent::AgentId;
use captain_types::workflow_learning::{
    ProposalCardState, ProposalInstallMode, WorkflowLifecycleCard, WorkflowLifecycleEvent,
};
use serde_json::json;

use super::kernel_workflow_learning_activation::{
    settle_activation_failure, WorkflowActivationExecutor,
};
use crate::cron::CronScheduler;

const WORKER: &str = "activation-test-worker";

struct ApprovedFixture {
    proposal_id: String,
    revision: String,
    artifact_sha256: String,
    staging_job_id: String,
}

fn store() -> WorkflowLearningStore {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    WorkflowLearningStore::new(memory.usage_conn())
}

fn model() -> ActiveModelIdentity {
    ActiveModelIdentity {
        provider: "codex".to_string(),
        model: "gpt-5.6-sol".to_string(),
    }
}

fn approve(
    home: &std::path::Path,
    control: &WorkflowLearningStore,
    proposal_id: &str,
    draft: &WorkflowDraft,
) -> ApprovedFixture {
    let staging_job_id = format!("{proposal_id}-draft");
    let signature = "a".repeat(64);
    let staged = WorkflowStagingRoot::new(home)
        .unwrap()
        .stage(StageWorkflowDraftRequest {
            job_id: &staging_job_id,
            workflow_signature: &signature,
            draft,
            active_model: &model(),
        })
        .unwrap();
    control
        .create_observed(&NewWorkflowProposal {
            id: proposal_id.to_string(),
            idempotency_key: format!("{proposal_id}:observed"),
            workflow_signature: signature.clone(),
            source_agent_id: "captain".to_string(),
            origin_channel: Some("telegram".to_string()),
            evidence_json: serde_json::to_string(&workflow_group(&signature)).unwrap(),
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
        control
            .transition(&proposal_transition(
                proposal_id,
                from,
                version,
                None,
                to,
                suffix,
                1_100 + version as i64,
            ))
            .unwrap();
    }
    let kind = match draft.kind {
        WorkflowDraftKind::Skill => WorkflowArtifactKind::Skill,
        WorkflowDraftKind::Capspec => WorkflowArtifactKind::Capspec,
        WorkflowDraftKind::Automation => WorkflowArtifactKind::Automation,
        WorkflowDraftKind::Refinement => WorkflowArtifactKind::Refinement,
    };
    control
        .publish_validated_draft(&PublishValidatedDraft {
            proposal_id: proposal_id.to_string(),
            expected_version: 3,
            staging_job_id: staging_job_id.clone(),
            revision_sha256: staged.revision_sha256.clone(),
            artifact_sha256: staged.artifact_sha256.clone(),
            kind,
            name: draft.name.clone(),
            validation_json: json!({
                "schema_version": 1,
                "checks": [
                    "whole_response_schema",
                    "native_artifact_parser",
                    "secret_scan",
                    "path_and_identifier_policy",
                    "immutable_staging_hashes"
                ],
                "model": model(),
                "limitations": draft.limitations,
            })
            .to_string(),
            actor: "test".to_string(),
            reason: "validated exact draft".to_string(),
            idempotency_key: format!("{proposal_id}:published"),
            occurred_at_unix_ms: 2_000,
        })
        .unwrap();
    let install_job_id = format!("{proposal_id}-install");
    let payload = WorkflowInstallRequestPayload {
        schema_version: 1,
        requested_mode: ProposalInstallMode::Activate,
        proposal_id: proposal_id.to_string(),
        revision_sha256: staged.revision_sha256.clone(),
        operator_actor: "telegram:42".to_string(),
    };
    control
        .approve_and_enqueue_install(
            &proposal_transition(
                proposal_id,
                WorkflowProposalState::Proposed,
                4,
                Some(&staged.revision_sha256),
                WorkflowProposalState::ApprovedPendingInstall,
                "approved",
                2_100,
            ),
            &NewWorkflowJob {
                id: install_job_id.clone(),
                idempotency_key: format!("{install_job_id}:enqueue"),
                proposal_id: proposal_id.to_string(),
                revision_sha256: Some(staged.revision_sha256.clone()),
                kind: WorkflowJobKind::Install,
                payload_json: serde_json::to_string(&payload).unwrap(),
                max_attempts: 3,
                run_after_unix_ms: 2_100,
                created_at_unix_ms: 2_100,
            },
            None,
        )
        .unwrap();
    ApprovedFixture {
        proposal_id: proposal_id.to_string(),
        revision: staged.revision_sha256,
        artifact_sha256: staged.artifact_sha256,
        staging_job_id,
    }
}

fn proposal_transition(
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
        actor: "test".to_string(),
        reason: suffix.to_string(),
        idempotency_key: format!("{proposal_id}:{suffix}"),
        snoozed_until_unix_ms: None,
        occurred_at_unix_ms: at,
    }
}

fn registries(
    home: &std::path::Path,
) -> (
    RwLock<SkillRegistry>,
    captain_capspec::CapabilityRegistry,
    CronScheduler,
) {
    fs::create_dir_all(home.join("skills/learned")).unwrap();
    fs::create_dir_all(home.join("capabilities")).unwrap();
    let mut skills = SkillRegistry::new(home.join("skills"));
    skills.load_all().unwrap();
    skills.freeze();
    let capspecs = captain_capspec::CapabilityRegistry::open(
        &home.join("capabilities"),
        &home.join("data/capabilities.db"),
    )
    .unwrap();
    (RwLock::new(skills), capspecs, CronScheduler::new(home, 100))
}

fn lifecycle_notice(control: &WorkflowLearningStore, job_id: &str) -> WorkflowLifecycleCard {
    let notice = control
        .get_outbox(&format!("{job_id}:notice"))
        .unwrap()
        .unwrap();
    serde_json::from_str(&notice.payload_json).unwrap()
}

fn delivered_lifecycle_events(
    control: &WorkflowLearningStore,
    home: &std::path::Path,
    count: usize,
) -> Vec<WorkflowLifecycleEvent> {
    let planner = WorkflowDeliveryPlanner::new(
        control.clone(),
        WorkflowStagingRoot::new(home).unwrap(),
        "activation-delivery-test",
        60_000,
    )
    .unwrap();
    (0..count)
        .map(|index| {
            let disposition = planner.claim_next(1_000_000 + index as i64).unwrap();
            let WorkflowDeliveryDisposition::Ready(delivery) = disposition else {
                panic!("expected exact activation lifecycle delivery, got {disposition:?}");
            };
            let WorkflowDeliveryEvent::Lifecycle(lifecycle) = &delivery.event else {
                panic!("expected an activation lifecycle card");
            };
            let event = lifecycle.event;
            planner
                .complete(
                    &delivery,
                    r#"{"schema_version":1}"#,
                    1_000_500 + index as i64,
                )
                .unwrap();
            event
        })
        .collect()
}

#[test]
fn workflow_activation_recovers_a_skill_promoted_before_a_power_loss() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let control = store();
    let draft = skill_draft("power-loss-skill");
    let fixture = approve(&home, &control, "power-loss-proposal", &draft);
    let claimed = control
        .claim_due_activation_job("worker-before-crash", 3_000, 60_000)
        .unwrap()
        .unwrap();
    control
        .mark_job_effect_started(&claimed.id, "worker-before-crash", 3_001)
        .unwrap();
    let promotions = WorkflowPromotionRoot::new(&home).unwrap();
    promotions
        .prepare(PromoteWorkflowDraftRequest {
            proposal_id: &fixture.proposal_id,
            staging_job_id: &fixture.staging_job_id,
            revision_sha256: &fixture.revision,
            artifact_sha256: &fixture.artifact_sha256,
        })
        .unwrap();
    promotions
        .promote(&fixture.proposal_id, &fixture.revision)
        .unwrap();

    assert_eq!(
        control
            .reconcile_jobs_after_restart(3_100)
            .unwrap()
            .uncertain_effects,
        1
    );
    let recovered = control
        .claim_uncertain_activation_job(WORKER, 3_100, 60_000)
        .unwrap()
        .unwrap();
    let (skills, capspecs, scheduler) = registries(&home);
    let executor = WorkflowActivationExecutor::new(
        control.clone(),
        WorkflowStagingRoot::new(&home).unwrap(),
        WorkflowPromotionRoot::new(&home).unwrap(),
        &skills,
        &capspecs,
        &scheduler,
    );
    assert_eq!(
        executor.execute(WORKER, &recovered, 3_100).unwrap(),
        WorkflowProposalState::ActiveCanary
    );
    let installed = lifecycle_notice(&control, &recovered.id);
    assert_eq!(
        installed.event,
        WorkflowLifecycleEvent::InstallationVerified
    );
    assert_eq!(installed.state, ProposalCardState::ActiveCanary);
    let canary = control
        .claim_due_activation_job(WORKER, 3_200, 60_000)
        .unwrap()
        .unwrap();
    assert_eq!(
        executor.execute(WORKER, &canary, 3_200).unwrap(),
        WorkflowProposalState::Active
    );
    assert_eq!(
        lifecycle_notice(&control, &canary.id).event,
        WorkflowLifecycleEvent::ActivationCompleted
    );
    assert_eq!(
        skills.read().unwrap().get("power-loss-skill").unwrap().path,
        home.join("skills/learned/power-loss-skill.md")
    );
}

#[test]
fn workflow_activation_runs_capspec_install_and_canary_in_native_registry() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let control = store();
    let draft = capspec_draft();
    let fixture = approve(&home, &control, "capspec-live-proposal", &draft);
    let (skills, capspecs, scheduler) = registries(&home);
    let executor = WorkflowActivationExecutor::new(
        control.clone(),
        WorkflowStagingRoot::new(&home).unwrap(),
        WorkflowPromotionRoot::new(&home).unwrap(),
        &skills,
        &capspecs,
        &scheduler,
    );
    let install = control
        .claim_due_activation_job(WORKER, 3_000, 60_000)
        .unwrap()
        .unwrap();
    assert_eq!(
        executor.execute(WORKER, &install, 3_000).unwrap(),
        WorkflowProposalState::ActiveCanary
    );
    let canary = control
        .claim_due_activation_job(WORKER, 3_100, 60_000)
        .unwrap()
        .unwrap();
    assert_eq!(
        executor.execute(WORKER, &canary, 3_100).unwrap(),
        WorkflowProposalState::Active
    );
    let view = capspecs
        .capability(
            &captain_capspec::CapabilityScope::Global,
            "learned-project-summary",
        )
        .unwrap();
    assert_eq!(view.status, captain_capspec::CapabilityStatus::Operational);
    assert!(view.active_hash.is_some());
    assert_eq!(
        control
            .get_installation(&fixture.proposal_id, &fixture.revision)
            .unwrap()
            .unwrap()
            .phase,
        WorkflowInstallationPhase::Active
    );
    assert_eq!(
        delivered_lifecycle_events(&control, &home, 2),
        vec![
            WorkflowLifecycleEvent::InstallationVerified,
            WorkflowLifecycleEvent::ActivationCompleted,
        ]
    );
}

#[test]
fn workflow_activation_keeps_automation_disabled_until_canary() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let control = store();
    let fixture = approve(
        &home,
        &control,
        "automation-live-proposal",
        &automation_draft("automation-live"),
    );
    let (skills, capspecs, scheduler) = registries(&home);
    let executor = WorkflowActivationExecutor::new(
        control.clone(),
        WorkflowStagingRoot::new(&home).unwrap(),
        WorkflowPromotionRoot::new(&home).unwrap(),
        &skills,
        &capspecs,
        &scheduler,
    );
    let install = control
        .claim_due_activation_job(WORKER, 3_000, 60_000)
        .unwrap()
        .unwrap();
    executor.execute(WORKER, &install, 3_000).unwrap();
    let scheduled = scheduler.list_jobs(AgentId::from_string("captain"));
    assert_eq!(scheduled.len(), 1);
    assert!(!scheduled[0].enabled);

    let canary = control
        .claim_due_activation_job(WORKER, 3_100, 60_000)
        .unwrap()
        .unwrap();
    assert_eq!(
        executor.execute(WORKER, &canary, 3_100).unwrap(),
        WorkflowProposalState::Active
    );
    assert!(scheduler.list_jobs(AgentId::from_string("captain"))[0].enabled);
    assert_eq!(
        control
            .get_installation(&fixture.proposal_id, &fixture.revision)
            .unwrap()
            .unwrap()
            .phase,
        WorkflowInstallationPhase::Active
    );
}

#[test]
fn workflow_activation_rolls_back_a_failed_automation_canary() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let control = store();
    let fixture = approve(
        &home,
        &control,
        "automation-rollback-proposal",
        &automation_draft("automation-rollback"),
    );
    let (skills, capspecs, scheduler) = registries(&home);
    let executor = WorkflowActivationExecutor::new(
        control.clone(),
        WorkflowStagingRoot::new(&home).unwrap(),
        WorkflowPromotionRoot::new(&home).unwrap(),
        &skills,
        &capspecs,
        &scheduler,
    );
    let install = control
        .claim_due_activation_job(WORKER, 3_000, 60_000)
        .unwrap()
        .unwrap();
    executor.execute(WORKER, &install, 3_000).unwrap();
    let canary = control
        .claim_due_activation_job(WORKER, 3_100, 60_000)
        .unwrap()
        .unwrap();
    control
        .mark_job_effect_started(&canary.id, WORKER, 3_101)
        .unwrap();
    assert_eq!(
        settle_activation_failure(
            &control,
            WORKER,
            &canary,
            "simulated canary verification failure",
            3_102,
        )
        .unwrap(),
        WorkflowJobStatus::Dead
    );
    let failure = lifecycle_notice(&control, &canary.id);
    assert_eq!(failure.event, WorkflowLifecycleEvent::ActivationFailed);
    assert!(failure.rollback_job_id.is_some());
    let rollback = control
        .claim_due_activation_job(WORKER, 3_200, 60_000)
        .unwrap()
        .unwrap();
    assert_eq!(rollback.kind, WorkflowJobKind::Rollback);
    assert_eq!(
        executor.execute(WORKER, &rollback, 3_200).unwrap(),
        WorkflowProposalState::RolledBack
    );
    assert_eq!(
        lifecycle_notice(&control, &rollback.id).event,
        WorkflowLifecycleEvent::RollbackCompleted
    );
    assert!(scheduler
        .list_jobs(AgentId::from_string("captain"))
        .is_empty());
    assert_eq!(
        control
            .get_installation(&fixture.proposal_id, &fixture.revision)
            .unwrap()
            .unwrap()
            .phase,
        WorkflowInstallationPhase::RolledBack
    );
    assert_eq!(
        delivered_lifecycle_events(&control, &home, 3),
        vec![
            WorkflowLifecycleEvent::InstallationVerified,
            WorkflowLifecycleEvent::ActivationFailed,
            WorkflowLifecycleEvent::RollbackCompleted,
        ]
    );
}

#[test]
fn workflow_activation_notifies_only_a_terminal_rollback_failure() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let control = store();
    approve(
        &home,
        &control,
        "rollback-dead-proposal",
        &automation_draft("rollback-dead"),
    );
    let (skills, capspecs, scheduler) = registries(&home);
    let executor = WorkflowActivationExecutor::new(
        control.clone(),
        WorkflowStagingRoot::new(&home).unwrap(),
        WorkflowPromotionRoot::new(&home).unwrap(),
        &skills,
        &capspecs,
        &scheduler,
    );
    let install = control
        .claim_due_activation_job(WORKER, 3_000, 60_000)
        .unwrap()
        .unwrap();
    executor.execute(WORKER, &install, 3_000).unwrap();
    let canary = control
        .claim_due_activation_job(WORKER, 3_100, 60_000)
        .unwrap()
        .unwrap();
    control
        .mark_job_effect_started(&canary.id, WORKER, 3_101)
        .unwrap();
    settle_activation_failure(&control, WORKER, &canary, "canary failed", 3_102).unwrap();

    for (index, at) in [10_000_i64, 20_000, 40_000].into_iter().enumerate() {
        let attempt = index + 1;
        let rollback = control
            .claim_due_activation_job(WORKER, at, 60_000)
            .unwrap()
            .unwrap();
        assert_eq!(rollback.kind, WorkflowJobKind::Rollback);
        control
            .mark_job_effect_started(&rollback.id, WORKER, at + 1)
            .unwrap();
        let status = settle_activation_failure(
            &control,
            WORKER,
            &rollback,
            "simulated rollback failure",
            at + 2,
        )
        .unwrap();
        if attempt < 3 {
            assert_eq!(status, WorkflowJobStatus::RetryWait);
            assert!(control
                .get_outbox(&format!("{}:notice", rollback.id))
                .unwrap()
                .is_none());
        } else {
            assert_eq!(status, WorkflowJobStatus::Dead);
            let failure = lifecycle_notice(&control, &rollback.id);
            assert_eq!(failure.event, WorkflowLifecycleEvent::RollbackFailed);
            assert_eq!(failure.state, ProposalCardState::InstallFailed);
            assert_eq!(
                failure.failure_message.as_deref(),
                Some("simulated rollback failure")
            );
        }
    }
    assert_eq!(
        delivered_lifecycle_events(&control, &home, 3),
        vec![
            WorkflowLifecycleEvent::InstallationVerified,
            WorkflowLifecycleEvent::ActivationFailed,
            WorkflowLifecycleEvent::RollbackFailed,
        ]
    );
}

fn skill_draft(name: &str) -> WorkflowDraft {
    WorkflowDraft {
        schema_version: 1,
        kind: WorkflowDraftKind::Skill,
        name: name.to_string(),
        purpose: "Reuse a verified operational workflow.".to_string(),
        trigger: "Use when the same verified workflow is requested.".to_string(),
        artifact: WorkflowDraftArtifact::SkillMarkdown {
            source: format!(
                "---\nname: {name}\ndescription: Learned power-loss-safe workflow\n---\n# Workflow\nRun the exact verified sequence."
            ),
        },
        required_capabilities: vec!["file_read".to_string()],
        expected_benefit: "Reliable reuse after restart.".to_string(),
        limitations: vec![],
    }
}

fn capspec_draft() -> WorkflowDraft {
    WorkflowDraft {
        schema_version: 1,
        kind: WorkflowDraftKind::Capspec,
        name: "learned-project-summary".to_string(),
        purpose: "Read one project summary with typed input.".to_string(),
        trigger: "Use when a project summary must be inspected.".to_string(),
        artifact: WorkflowDraftArtifact::CapspecToml {
            source: r#"format = 1
name = "learned-project-summary"
description = "Read a learned project summary."
version = "1.0.0"

[permissions]
tools = ["file_read"]
read_paths = ["{{input.path}}"]

[inputs.path]
type = "string"
description = "Summary path"

[[steps]]
id = "read"
tool = "file_read"
needs = []
with = { path = "{{input.path}}" }
"#
            .to_string(),
        },
        required_capabilities: vec!["file_read".to_string()],
        expected_benefit: "Typed and auditable summary reads.".to_string(),
        limitations: vec![],
    }
}

fn automation_draft(name: &str) -> WorkflowDraft {
    WorkflowDraft {
        schema_version: 1,
        kind: WorkflowDraftKind::Automation,
        name: name.to_string(),
        purpose: "Run a verified periodic status check.".to_string(),
        trigger: "Use when periodic status checks are requested.".to_string(),
        artifact: WorkflowDraftArtifact::Automation {
            schedule: AutomationScheduleDraft::Every { every_secs: 3_600 },
            instruction: "Inspect configured services and report verified health.".to_string(),
        },
        required_capabilities: vec!["ssh_health_check".to_string()],
        expected_benefit: "Reliable recurring health visibility.".to_string(),
        limitations: vec![],
    }
}

fn workflow_group(signature: &str) -> WorkflowGroupAnalysis {
    WorkflowGroupAnalysis {
        signature: signature.to_string(),
        classification: WorkflowClassification::Skill,
        eligible: true,
        reasons: vec![],
        occurrence_count: 3,
        distinct_turn_count: 3,
        distinct_session_count: 2,
        explicit_reuse_request: false,
        scope: WorkflowScope::Global,
        episode_ids: vec!["episode-1".into(), "episode-2".into(), "episode-3".into()],
        intent_samples: vec!["reuse this workflow".into()],
        canonical: CanonicalWorkflow {
            version: 1,
            nodes: vec![CanonicalWorkflowNode {
                index: 0,
                tool_name: "file_read".to_string(),
                role: "verify".to_string(),
                input_shape: json!({"path": "<path>"}),
                effect_class: "read".to_string(),
                verification_shape: "exact result".to_string(),
                dependencies: vec![],
            }],
        },
    }
}
