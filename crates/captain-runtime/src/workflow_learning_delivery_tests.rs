use captain_memory::workflow_learning_control::{
    NewWorkflowProposal, PublishValidatedDraft, WorkflowArtifactKind, WorkflowLearningStore,
    WorkflowProposalState, WorkflowProposalTransition,
};
use captain_memory::workflow_learning_outbox::{NewWorkflowOutboxItem, WorkflowOutboxStatus};
use captain_memory::workflow_learning_test::WorkflowIsolatedTestCompletion;
use captain_memory::MemorySubstrate;
use captain_types::workflow_learning::{
    ProposalCardAction, ProposalCardKind, ProposalCardState, ProposalInstallMode,
    ProposalIsolatedTestCheck, ProposalIsolatedTestStatus, WorkflowIsolatedTestReport,
};
use serde_json::json;

use crate::workflow_learning_analysis::{
    CanonicalWorkflow, CanonicalWorkflowNode, WorkflowClassification, WorkflowGroupAnalysis,
    WorkflowScope,
};
use crate::workflow_learning_delivery::{
    WorkflowDeliveryDisposition, WorkflowDeliveryEvent, WorkflowDeliveryPlanner,
    WORKFLOW_LIFECYCLE_OUTBOX_TOPIC, WORKFLOW_PROPOSAL_OUTBOX_TOPIC,
};
use crate::workflow_learning_operator::WorkflowLearningOperator;
use crate::workflow_learning_proposer::{
    ActiveModelIdentity, WorkflowDraft, WorkflowDraftArtifact, WorkflowDraftKind,
};
use crate::workflow_learning_staging::{StageWorkflowDraftRequest, WorkflowStagingRoot};

const PROPOSAL_ID: &str = "proposal-delivery";
const WORKER: &str = "captain:workflow-delivery-test";

struct Fixture {
    _temp: tempfile::TempDir,
    store: WorkflowLearningStore,
    planner: WorkflowDeliveryPlanner,
    staging: WorkflowStagingRoot,
    revision: String,
}

fn fixture(payload_transform: impl FnOnce(serde_json::Value) -> serde_json::Value) -> Fixture {
    fixture_with_capabilities(payload_transform, vec!["web_search".to_string()])
}

fn fixture_with_capabilities(
    payload_transform: impl FnOnce(serde_json::Value) -> serde_json::Value,
    required_capabilities: Vec<String>,
) -> Fixture {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let store = WorkflowLearningStore::new(memory.usage_conn());
    let temp = tempfile::tempdir().unwrap();
    let staging = WorkflowStagingRoot::new(temp.path().join("captain-home")).unwrap();
    let signature = "a".repeat(64);
    let mut draft = workflow_draft();
    draft.required_capabilities = required_capabilities;
    let active_model = active_model();
    let staged = staging
        .stage(StageWorkflowDraftRequest {
            job_id: "draft-job-delivery",
            workflow_signature: &signature,
            draft: &draft,
            active_model: &active_model,
        })
        .unwrap();

    store
        .create_observed(&NewWorkflowProposal {
            id: PROPOSAL_ID.to_string(),
            idempotency_key: format!("{PROPOSAL_ID}:observed"),
            workflow_signature: signature.clone(),
            source_agent_id: "captain".to_string(),
            origin_channel: Some("telegram".to_string()),
            evidence_json: serde_json::to_string(&workflow_group(&signature)).unwrap(),
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
                200 + version as i64,
            ))
            .unwrap();
    }
    let validation_json = json!({
        "schema_version": 1,
        "checks": [
            "whole_response_schema",
            "native_artifact_parser",
            "secret_scan",
            "path_and_identifier_policy",
            "immutable_staging_hashes"
        ],
        "model": active_model,
        "limitations": draft.limitations,
    })
    .to_string();
    store
        .publish_validated_draft(&PublishValidatedDraft {
            proposal_id: PROPOSAL_ID.to_string(),
            expected_version: 3,
            staging_job_id: "draft-job-delivery".to_string(),
            revision_sha256: staged.revision_sha256.clone(),
            artifact_sha256: staged.artifact_sha256,
            kind: WorkflowArtifactKind::Skill,
            name: draft.name,
            validation_json,
            actor: WORKER.to_string(),
            reason: "exact staged revision validated".to_string(),
            idempotency_key: format!("{PROPOSAL_ID}:published"),
            occurred_at_unix_ms: 300,
        })
        .unwrap();
    let payload = payload_transform(json!({
        "schema_version": 1,
        "proposal_id": PROPOSAL_ID,
        "revision_sha256": staged.revision_sha256,
        "state": "proposed",
    }));
    let revision = payload["revision_sha256"].as_str().unwrap().to_string();
    store
        .enqueue_outbox(&NewWorkflowOutboxItem {
            id: format!("{PROPOSAL_ID}-proposed"),
            idempotency_key: format!("{PROPOSAL_ID}:proposed-notification"),
            proposal_id: PROPOSAL_ID.to_string(),
            revision_sha256: Some(revision.clone()),
            topic: WORKFLOW_PROPOSAL_OUTBOX_TOPIC.to_string(),
            payload_json: payload.to_string(),
            max_attempts: 8,
            run_after_unix_ms: 1_000,
            created_at_unix_ms: 300,
        })
        .unwrap();
    let planner =
        WorkflowDeliveryPlanner::new(store.clone(), staging.clone(), WORKER, 60_000).unwrap();
    Fixture {
        _temp: temp,
        store,
        planner,
        staging,
        revision,
    }
}

#[test]
fn exact_proposed_outbox_projects_and_completes_one_shared_card() {
    let fixture = fixture(|payload| payload);

    let WorkflowDeliveryDisposition::Ready(delivery) = fixture.planner.claim_next(1_000).unwrap()
    else {
        panic!("expected a ready workflow proposal delivery");
    };
    assert_eq!(delivery.card.state, ProposalCardState::Proposed);
    assert_eq!(delivery.card.revision_sha256, fixture.revision);
    assert_eq!(delivery.outbox.attempt_count, 1);

    let completed = fixture
        .planner
        .complete(
            &delivery,
            r#"{"schema_version":1,"channel":"telegram","external_message_id":"42"}"#,
            1_100,
        )
        .unwrap();
    assert_eq!(completed.status, WorkflowOutboxStatus::Delivered);
    assert!(matches!(
        fixture.planner.claim_next(2_000).unwrap(),
        WorkflowDeliveryDisposition::Idle
    ));
}

#[test]
fn exact_isolated_test_completion_projects_a_rich_activation_card() {
    let fixture = fixture_with_capabilities(|payload| payload, vec!["file_write".to_string()]);
    let WorkflowDeliveryDisposition::Ready(proposed) = fixture.planner.claim_next(1_000).unwrap()
    else {
        panic!("expected the initial proposal delivery");
    };
    fixture
        .planner
        .complete(&proposed, r#"{"schema_version":1}"#, 1_010)
        .unwrap();

    let decision_version = fixture
        .store
        .get(PROPOSAL_ID)
        .unwrap()
        .unwrap()
        .state_version;
    let operator = WorkflowLearningOperator::new(fixture.store.clone(), fixture.staging.clone());
    let queued = operator
        .resolve_at_version(
            &proposed.card.lookup_token,
            decision_version,
            ProposalCardAction::Test,
            "telegram:42",
            1_100,
        )
        .unwrap();
    assert_eq!(
        queued.outcome,
        captain_types::workflow_learning::ProposalOperatorOutcome::InstallQueued {
            mode: ProposalInstallMode::Test,
        }
    );
    let job = fixture
        .store
        .claim_due_isolated_test_job("isolated-worker", 1_100, 30_000)
        .unwrap()
        .unwrap();
    let proposal = fixture.store.get(PROPOSAL_ID).unwrap().unwrap();
    let report = WorkflowIsolatedTestReport {
        schema_version: 1,
        proposal_id: proposal.id.clone(),
        revision_sha256: proposal.revision_sha256.clone().unwrap(),
        artifact_sha256: proposal.artifact_sha256.clone().unwrap(),
        kind: ProposalCardKind::Skill,
        name: proposal.name.clone().unwrap(),
        passed: true,
        checks: vec![ProposalIsolatedTestCheck {
            code: "native_skill_registry".to_string(),
            passed: true,
            detail: "exact staged bytes loaded from a private registry".to_string(),
        }],
        completed_at_unix_ms: 1_200,
    };
    let notification_payload = json!({
        "schema_version": 1,
        "event": "isolated_test_completed",
        "proposal_id": proposal.id,
        "revision_sha256": report.revision_sha256,
        "test_job_id": job.id,
        "state": "proposed",
        "passed": true,
    });
    fixture
        .store
        .complete_isolated_test(&WorkflowIsolatedTestCompletion {
            job_id: job.id.clone(),
            worker: "isolated-worker".to_string(),
            passed: true,
            result_json: serde_json::to_string(&report).unwrap(),
            proposal_transition: WorkflowProposalTransition {
                proposal_id: proposal.id.clone(),
                expected_state: WorkflowProposalState::ApprovedPendingInstall,
                expected_version: proposal.state_version,
                expected_revision_sha256: proposal.revision_sha256.clone(),
                to_state: WorkflowProposalState::Proposed,
                actor: "isolated-worker".to_string(),
                reason: "isolated test passed".to_string(),
                idempotency_key: format!("{}:test-completed", proposal.id),
                snoozed_until_unix_ms: None,
                occurred_at_unix_ms: 1_200,
            },
            notification: Some(NewWorkflowOutboxItem {
                id: format!("{}-test-completed", proposal.id),
                idempotency_key: format!("{}:test-completed-notification", proposal.id),
                proposal_id: proposal.id,
                revision_sha256: Some(report.revision_sha256.clone()),
                topic: WORKFLOW_LIFECYCLE_OUTBOX_TOPIC.to_string(),
                payload_json: notification_payload.to_string(),
                max_attempts: 8,
                run_after_unix_ms: 1_200,
                created_at_unix_ms: 1_200,
            }),
            completed_at_unix_ms: 1_200,
        })
        .unwrap();

    let WorkflowDeliveryDisposition::Ready(delivery) = fixture.planner.claim_next(1_200).unwrap()
    else {
        panic!("expected the isolated-test lifecycle delivery");
    };
    assert_eq!(
        delivery.event,
        WorkflowDeliveryEvent::IsolatedTestCompleted { passed: true }
    );
    assert_eq!(delivery.card.state, ProposalCardState::Proposed);
    assert!(delivery
        .card
        .available_actions
        .contains(&ProposalCardAction::Activate));
    assert!(delivery
        .card
        .available_actions
        .contains(&ProposalCardAction::Test));
    assert_eq!(
        delivery.card.isolated_test.as_ref().unwrap().status,
        ProposalIsolatedTestStatus::Passed
    );
    fixture
        .planner
        .complete(&delivery, r#"{"schema_version":1}"#, 1_250)
        .unwrap();
    operator
        .resolve_at_version(
            &delivery.card.lookup_token,
            delivery.card.decision_version,
            ProposalCardAction::Activate,
            "telegram:42",
            1_300,
        )
        .unwrap();
    fixture
        .store
        .enqueue_outbox(&NewWorkflowOutboxItem {
            id: format!("{PROPOSAL_ID}-late-test-notice"),
            idempotency_key: format!("{PROPOSAL_ID}:late-test-notice"),
            proposal_id: PROPOSAL_ID.to_string(),
            revision_sha256: Some(report.revision_sha256),
            topic: WORKFLOW_LIFECYCLE_OUTBOX_TOPIC.to_string(),
            payload_json: notification_payload.to_string(),
            max_attempts: 8,
            run_after_unix_ms: 1_400,
            created_at_unix_ms: 1_400,
        })
        .unwrap();
    let WorkflowDeliveryDisposition::Suppressed { proposal_state, .. } =
        fixture.planner.claim_next(1_400).unwrap()
    else {
        panic!("expected a late lifecycle notice to be suppressed");
    };
    assert_eq!(proposal_state, "approved_pending_install");
}

#[test]
fn lifecycle_notice_without_exact_test_evidence_is_dead_lettered() {
    let fixture = fixture(|payload| payload);
    let WorkflowDeliveryDisposition::Ready(proposed) = fixture.planner.claim_next(1_000).unwrap()
    else {
        panic!("expected the initial proposal delivery");
    };
    fixture
        .planner
        .complete(&proposed, r#"{"schema_version":1}"#, 1_010)
        .unwrap();
    fixture
        .store
        .enqueue_outbox(&NewWorkflowOutboxItem {
            id: format!("{PROPOSAL_ID}-invalid-lifecycle"),
            idempotency_key: format!("{PROPOSAL_ID}:invalid-lifecycle"),
            proposal_id: PROPOSAL_ID.to_string(),
            revision_sha256: Some(fixture.revision.clone()),
            topic: WORKFLOW_LIFECYCLE_OUTBOX_TOPIC.to_string(),
            payload_json: json!({
                "schema_version": 1,
                "event": "isolated_test_completed",
                "proposal_id": PROPOSAL_ID,
                "revision_sha256": fixture.revision,
                "test_job_id": "missing-test-job",
                "state": "proposed",
                "passed": true,
            })
            .to_string(),
            max_attempts: 8,
            run_after_unix_ms: 1_100,
            created_at_unix_ms: 1_100,
        })
        .unwrap();

    let WorkflowDeliveryDisposition::DeadLettered { reason, .. } =
        fixture.planner.claim_next(1_100).unwrap()
    else {
        panic!("expected invalid lifecycle evidence to be dead-lettered");
    };
    assert!(reason.contains("no durable isolated-test evidence"));
}

#[test]
fn malformed_notification_is_dead_lettered_without_retries() {
    let fixture = fixture(|mut payload| {
        payload["unexpected"] = json!(true);
        payload
    });

    let WorkflowDeliveryDisposition::DeadLettered { outbox_id, reason } =
        fixture.planner.claim_next(1_000).unwrap()
    else {
        panic!("expected malformed outbox to be dead-lettered");
    };
    assert_eq!(outbox_id, format!("{PROPOSAL_ID}-proposed"));
    assert!(reason.contains("unknown field"));
    let stored = fixture.store.get_outbox(&outbox_id).unwrap().unwrap();
    assert_eq!(stored.status, WorkflowOutboxStatus::Dead);
    assert_eq!(stored.attempt_count, 1);
}

#[test]
fn transport_failure_uses_bounded_backoff_and_preserves_the_outbox_identity() {
    let fixture = fixture(|payload| payload);
    let WorkflowDeliveryDisposition::Ready(delivery) = fixture.planner.claim_next(1_000).unwrap()
    else {
        panic!("expected a ready workflow proposal delivery");
    };

    let retry = fixture
        .planner
        .retry(&delivery, "telegram temporarily unavailable", 1_100)
        .unwrap();

    assert_eq!(retry.status, WorkflowOutboxStatus::RetryWait);
    assert_eq!(retry.run_after_unix_ms, 6_100);
    assert_eq!(retry.idempotency_key, delivery.outbox.idempotency_key);
    assert_eq!(retry.attempt_count, 1);
}

#[test]
fn proposal_worker_never_claims_an_older_unrelated_outbox_topic() {
    let fixture = fixture(|payload| payload);
    fixture
        .store
        .enqueue_outbox(&NewWorkflowOutboxItem {
            id: "proposal-delivery-lifecycle".to_string(),
            idempotency_key: "proposal-delivery:lifecycle-notification".to_string(),
            proposal_id: PROPOSAL_ID.to_string(),
            revision_sha256: Some(fixture.revision.clone()),
            topic: "workflow_learning.unrelated".to_string(),
            payload_json: "{}".to_string(),
            max_attempts: 8,
            run_after_unix_ms: 500,
            created_at_unix_ms: 100,
        })
        .unwrap();

    let WorkflowDeliveryDisposition::Ready(delivery) = fixture.planner.claim_next(1_000).unwrap()
    else {
        panic!("expected the proposed notification topic");
    };

    assert_eq!(delivery.outbox.topic, WORKFLOW_PROPOSAL_OUTBOX_TOPIC);
    let foreign = fixture
        .store
        .get_outbox("proposal-delivery-lifecycle")
        .unwrap()
        .unwrap();
    assert_eq!(foreign.status, WorkflowOutboxStatus::Pending);
    assert_eq!(foreign.attempt_count, 0);
}

#[test]
fn stale_proposal_notification_is_suppressed_as_delivered() {
    let fixture = fixture(|payload| payload);
    fixture
        .store
        .transition(&transition(
            WorkflowProposalState::Proposed,
            4,
            Some(&fixture.revision),
            WorkflowProposalState::Dismissed,
            "dismissed",
            900,
        ))
        .unwrap();

    let WorkflowDeliveryDisposition::Suppressed { proposal_state, .. } =
        fixture.planner.claim_next(1_000).unwrap()
    else {
        panic!("expected stale notification to be suppressed");
    };
    assert_eq!(proposal_state, "dismissed");
    let stored = fixture
        .store
        .get_outbox(&format!("{PROPOSAL_ID}-proposed"))
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, WorkflowOutboxStatus::Delivered);
    assert!(stored
        .delivery_result_json
        .as_deref()
        .unwrap()
        .contains("proposal_not_actionable"));
}

fn transition(
    from: WorkflowProposalState,
    version: u64,
    revision: Option<&str>,
    to: WorkflowProposalState,
    suffix: &str,
    at: i64,
) -> WorkflowProposalTransition {
    WorkflowProposalTransition {
        proposal_id: PROPOSAL_ID.to_string(),
        expected_state: from,
        expected_version: version,
        expected_revision_sha256: revision.map(str::to_string),
        to_state: to,
        actor: WORKER.to_string(),
        reason: format!("test transition {suffix}"),
        idempotency_key: format!("{PROPOSAL_ID}:{suffix}"),
        snoozed_until_unix_ms: None,
        occurred_at_unix_ms: at,
    }
}

fn active_model() -> ActiveModelIdentity {
    ActiveModelIdentity {
        provider: "codex".to_string(),
        model: "gpt-5.6-sol".to_string(),
    }
}

fn workflow_draft() -> WorkflowDraft {
    WorkflowDraft {
        schema_version: 1,
        kind: WorkflowDraftKind::Skill,
        name: "sourced-research".to_string(),
        purpose: "Research a subject using authoritative sources.".to_string(),
        trigger: "Use when a current answer requires sources.".to_string(),
        artifact: WorkflowDraftArtifact::SkillMarkdown {
            source: "---\nname: sourced-research\ndescription: Source-backed research\n---\n# Workflow\nSearch, compare, and cite authoritative sources.".to_string(),
        },
        required_capabilities: vec!["web_search".to_string()],
        expected_benefit: "Repeatable research with explicit evidence.".to_string(),
        limitations: vec!["Human review remains required for high-stakes claims.".to_string()],
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
        intent_samples: vec!["research this".into()],
        canonical: CanonicalWorkflow {
            version: 1,
            nodes: vec![CanonicalWorkflowNode {
                index: 0,
                tool_name: "web_search".to_string(),
                role: "research".to_string(),
                input_shape: json!({"query":"<text>"}),
                effect_class: "read".to_string(),
                verification_shape: "sources".to_string(),
                dependencies: vec![],
            }],
        },
    }
}
