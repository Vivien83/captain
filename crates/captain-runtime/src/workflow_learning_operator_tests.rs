use std::sync::{Arc, Barrier};

use captain_memory::workflow_learning_control::{
    NewWorkflowProposal, PublishValidatedDraft, WorkflowArtifactKind, WorkflowLearningStore,
    WorkflowProposalState, WorkflowProposalTransition,
};
use captain_memory::workflow_learning_queue::WorkflowJobKind;
use captain_memory::workflow_learning_test::WorkflowIsolatedTestCompletion;
use captain_memory::MemorySubstrate;
use captain_types::workflow_learning::{
    ProposalCardAction, ProposalCardKind, ProposalCardState, ProposalInstallMode,
    ProposalIsolatedTestCheck, ProposalOperatorContext, ProposalOperatorOutcome,
    WorkflowIsolatedTestReport,
};
use serde_json::json;

use crate::workflow_learning_analysis::{
    CanonicalWorkflow, CanonicalWorkflowNode, WorkflowClassification, WorkflowGroupAnalysis,
    WorkflowScope,
};
use crate::workflow_learning_operator::{
    WorkflowInstallRequestPayload, WorkflowLearningOperator, WorkflowOperatorError,
    WORKFLOW_OPERATOR_SNOOZE_MS,
};
use crate::workflow_learning_proposer::{
    ActiveModelIdentity, WorkflowDraft, WorkflowDraftArtifact, WorkflowDraftKind,
};
use crate::workflow_learning_staging::{StageWorkflowDraftRequest, WorkflowStagingRoot};

struct Fixture {
    _temp: tempfile::TempDir,
    operator: WorkflowLearningOperator,
    store: WorkflowLearningStore,
    token: String,
}

fn fixture(required_capabilities: Vec<String>) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let staging = WorkflowStagingRoot::new(temp.path().join("captain-home")).unwrap();
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let store = WorkflowLearningStore::new(memory.usage_conn());
    let signature = "a".repeat(64);
    let draft = draft(required_capabilities);
    let model = model();
    let receipt = staging
        .stage(StageWorkflowDraftRequest {
            job_id: "draft-job-operator",
            workflow_signature: &signature,
            draft: &draft,
            active_model: &model,
        })
        .unwrap();
    store
        .create_observed(&NewWorkflowProposal {
            id: "proposal-operator".to_string(),
            idempotency_key: "proposal-operator:observed".to_string(),
            workflow_signature: signature.clone(),
            source_agent_id: "captain".to_string(),
            origin_channel: Some("telegram".to_string()),
            evidence_json: serde_json::to_string(&group(&signature)).unwrap(),
            created_at_unix_ms: 1_000,
        })
        .unwrap();
    for (expected, version, next, key) in [
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
                proposal_id: "proposal-operator".to_string(),
                expected_state: expected,
                expected_version: version,
                expected_revision_sha256: None,
                to_state: next,
                actor: "captain:test".to_string(),
                reason: "test fixture".to_string(),
                idempotency_key: format!("proposal-operator:{key}"),
                snoozed_until_unix_ms: None,
                occurred_at_unix_ms: 1_100 + version as i64,
            })
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
        "model": model,
        "limitations": draft.limitations,
    })
    .to_string();
    let published = store
        .publish_validated_draft(&PublishValidatedDraft {
            proposal_id: "proposal-operator".to_string(),
            expected_version: 3,
            staging_job_id: "draft-job-operator".to_string(),
            revision_sha256: receipt.revision_sha256,
            artifact_sha256: receipt.artifact_sha256,
            kind: WorkflowArtifactKind::Skill,
            name: draft.name,
            validation_json,
            actor: "captain:validator".to_string(),
            reason: "fixture validated".to_string(),
            idempotency_key: "proposal-operator:published".to_string(),
            occurred_at_unix_ms: 2_000,
        })
        .unwrap();
    let token = published.operator_token.unwrap();
    Fixture {
        _temp: temp,
        operator: WorkflowLearningOperator::new(store.clone(), staging),
        store,
        token,
    }
}

fn model() -> ActiveModelIdentity {
    ActiveModelIdentity {
        provider: "codex".to_string(),
        model: "gpt-5.6-sol".to_string(),
    }
}

fn draft(required_capabilities: Vec<String>) -> WorkflowDraft {
    WorkflowDraft {
        schema_version: 1,
        kind: WorkflowDraftKind::Skill,
        name: "operator-research".to_string(),
        purpose: "Research with authoritative sources.".to_string(),
        trigger: "Use for sourced research.".to_string(),
        artifact: WorkflowDraftArtifact::SkillMarkdown {
            source: "---\nname: operator-research\ndescription: Sourced research\n---\n# Workflow\nSearch and cite.".to_string(),
        },
        required_capabilities,
        expected_benefit: "Reliable repeatable research.".to_string(),
        limitations: vec!["Review high-stakes claims.".to_string()],
    }
}

fn group(signature: &str) -> WorkflowGroupAnalysis {
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

#[test]
fn read_only_activation_queues_one_exact_install_and_replays() {
    let fixture = fixture(vec!["web_search".to_string()]);
    let decision_version = fixture
        .store
        .get("proposal-operator")
        .unwrap()
        .unwrap()
        .state_version;
    let first = fixture
        .operator
        .resolve_at_version(
            &fixture.token,
            decision_version,
            ProposalCardAction::Activate,
            "telegram:42",
            10_000,
        )
        .unwrap();
    assert_eq!(
        first.outcome,
        ProposalOperatorOutcome::InstallQueued {
            mode: ProposalInstallMode::Activate
        }
    );
    assert_eq!(first.card.state, ProposalCardState::ApprovedPendingInstall);
    assert!(first.retire_keyboard);
    assert!(!first.replayed);

    let job = fixture
        .store
        .claim_due_job("installer", 10_000, 30_000)
        .unwrap()
        .unwrap();
    assert_eq!(job.kind, WorkflowJobKind::Install);
    assert_eq!(job.revision_sha256, first.card.revision_sha256.into());
    let payload: WorkflowInstallRequestPayload = serde_json::from_str(&job.payload_json).unwrap();
    assert_eq!(payload.requested_mode, ProposalInstallMode::Activate);
    assert_eq!(payload.operator_actor, "telegram:42");

    let replay = fixture
        .operator
        .resolve_at_version(
            &fixture.token,
            decision_version,
            ProposalCardAction::Activate,
            "telegram:42",
            10_001,
        )
        .unwrap();
    assert!(replay.replayed);
    assert!(fixture
        .store
        .claim_due_job("other", 10_001, 30_000)
        .unwrap()
        .is_none());
}

#[test]
fn mutation_requires_test_mode_and_never_silently_activates() {
    let fixture = fixture(vec!["file_write".to_string()]);
    let test_decision_version = fixture
        .store
        .get("proposal-operator")
        .unwrap()
        .unwrap()
        .state_version;
    assert!(matches!(
        fixture.operator.resolve_at_version(
            &fixture.token,
            test_decision_version,
            ProposalCardAction::Activate,
            "tui:operator",
            20_000
        ),
        Err(WorkflowOperatorError::ActionUnavailable { .. })
    ));
    assert_eq!(
        fixture
            .store
            .get("proposal-operator")
            .unwrap()
            .unwrap()
            .state,
        WorkflowProposalState::Proposed
    );

    let tested = fixture
        .operator
        .resolve_at_version(
            &fixture.token,
            test_decision_version,
            ProposalCardAction::Test,
            "tui:operator",
            20_001,
        )
        .unwrap();
    assert_eq!(
        tested.outcome,
        ProposalOperatorOutcome::InstallQueued {
            mode: ProposalInstallMode::Test
        }
    );
    let test_job = fixture
        .store
        .claim_due_isolated_test_job("isolated-worker", 20_001, 30_000)
        .unwrap()
        .unwrap();
    let proposal = fixture.store.get("proposal-operator").unwrap().unwrap();
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
            detail: "private registry loaded exact bytes".to_string(),
        }],
        completed_at_unix_ms: 20_002,
    };
    fixture
        .store
        .complete_isolated_test(&WorkflowIsolatedTestCompletion {
            job_id: test_job.id,
            worker: "isolated-worker".to_string(),
            passed: true,
            result_json: serde_json::to_string(&report).unwrap(),
            proposal_transition: WorkflowProposalTransition {
                proposal_id: proposal.id,
                expected_state: WorkflowProposalState::ApprovedPendingInstall,
                expected_version: proposal.state_version,
                expected_revision_sha256: proposal.revision_sha256,
                to_state: WorkflowProposalState::Proposed,
                actor: "isolated-worker".to_string(),
                reason: "isolated test passed".to_string(),
                idempotency_key: "proposal-operator:test-complete".to_string(),
                snoozed_until_unix_ms: None,
                occurred_at_unix_ms: 20_002,
            },
            notification: None,
            completed_at_unix_ms: 20_002,
        })
        .unwrap();

    assert!(matches!(
        fixture.operator.resolve_at_version(
            &fixture.token,
            test_decision_version,
            ProposalCardAction::Test,
            "tui:operator",
            20_003,
        ),
        Err(WorkflowOperatorError::Stale(_))
    ));
    let activation_decision_version = fixture
        .store
        .get("proposal-operator")
        .unwrap()
        .unwrap()
        .state_version;

    let activated = fixture
        .operator
        .resolve_at_version(
            &fixture.token,
            activation_decision_version,
            ProposalCardAction::Activate,
            "tui:operator",
            20_004,
        )
        .unwrap();
    assert_eq!(
        activated.outcome,
        ProposalOperatorOutcome::InstallQueued {
            mode: ProposalInstallMode::Activate
        }
    );
}

#[test]
fn later_is_durable_for_twenty_four_hours_and_duplicate_safe() {
    let fixture = fixture(vec!["web_search".to_string()]);
    let decision_version = 4;
    let now = 30_000;
    let first = fixture
        .operator
        .resolve_at_version(
            &fixture.token,
            decision_version,
            ProposalCardAction::Later,
            "telegram:42",
            now,
        )
        .unwrap();
    assert_eq!(
        first.outcome,
        ProposalOperatorOutcome::Snoozed {
            until_unix_ms: now + WORKFLOW_OPERATOR_SNOOZE_MS
        }
    );
    assert_eq!(first.card.state, ProposalCardState::Snoozed);

    let replay = fixture
        .operator
        .resolve_at_version(
            &fixture.token,
            decision_version,
            ProposalCardAction::Later,
            "telegram:42",
            now + 500,
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, first.outcome);
    assert_eq!(fixture.store.events("proposal-operator").unwrap().len(), 6);
}

#[test]
fn details_and_edit_request_do_not_mutate_the_proposal() {
    let fixture = fixture(vec!["web_search".to_string()]);
    let decision_version = 4;
    let details = fixture
        .operator
        .resolve_at_version(
            &fixture.token,
            decision_version,
            ProposalCardAction::Details,
            "web:operator",
            40_000,
        )
        .unwrap();
    let missing_context = fixture.operator.resolve_at_version(
        &fixture.token,
        decision_version,
        ProposalCardAction::Edit,
        "web:operator",
        40_001,
    );
    assert!(matches!(
        missing_context,
        Err(WorkflowOperatorError::MissingRefinementContext)
    ));
    let context = ProposalOperatorContext {
        surface: "telegram".to_string(),
        conversation_key: "telegram:chat:42:thread:root".to_string(),
        source_message_id: Some("100".to_string()),
        language: "fr".to_string(),
    };
    let edit = fixture
        .operator
        .resolve_with_context_at_version(
            &fixture.token,
            decision_version,
            ProposalCardAction::Edit,
            "telegram:42",
            &context,
            40_001,
        )
        .unwrap();
    assert_eq!(details.outcome, ProposalOperatorOutcome::Details);
    let ProposalOperatorOutcome::EditRequested {
        request_id,
        expires_at_unix_ms,
    } = &edit.outcome
    else {
        panic!("edit did not create a durable refinement request");
    };
    assert!(request_id.starts_with("wr-"));
    assert_eq!(*expires_at_unix_ms, 40_001 + 15 * 60 * 1_000);
    assert!(!details.retire_keyboard);
    assert!(!edit.retire_keyboard);
    let replay = fixture
        .operator
        .resolve_with_context_at_version(
            &fixture.token,
            decision_version,
            ProposalCardAction::Edit,
            "telegram:42",
            &context,
            40_002,
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, edit.outcome);

    let blocked = fixture.operator.resolve_at_version(
        &fixture.token,
        decision_version,
        ProposalCardAction::Activate,
        "telegram:42",
        40_003,
    );
    assert!(matches!(blocked, Err(WorkflowOperatorError::Control(_))));
    let proposal = fixture.store.get("proposal-operator").unwrap().unwrap();
    assert_eq!(proposal.state, WorkflowProposalState::Proposed);
    assert_eq!(proposal.state_version, 4);
}

#[test]
fn concurrent_activate_and_ignore_have_one_atomic_winner() {
    let fixture = fixture(vec!["web_search".to_string()]);
    let decision_version = 4;
    let operator = Arc::new(fixture.operator);
    let barrier = Arc::new(Barrier::new(3));
    let token = fixture.token;

    let activate_operator = Arc::clone(&operator);
    let activate_barrier = Arc::clone(&barrier);
    let activate_token = token.clone();
    let activate = std::thread::spawn(move || {
        activate_barrier.wait();
        activate_operator.resolve_at_version(
            &activate_token,
            decision_version,
            ProposalCardAction::Activate,
            "telegram:42",
            50_000,
        )
    });
    let ignore_operator = Arc::clone(&operator);
    let ignore_barrier = Arc::clone(&barrier);
    let ignore = std::thread::spawn(move || {
        ignore_barrier.wait();
        ignore_operator.resolve_at_version(
            &token,
            decision_version,
            ProposalCardAction::Ignore,
            "telegram:42",
            50_000,
        )
    });

    barrier.wait();
    let results = [activate.join().unwrap(), ignore.join().unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
    let proposal = fixture.store.get("proposal-operator").unwrap().unwrap();
    assert!(matches!(
        proposal.state,
        WorkflowProposalState::ApprovedPendingInstall | WorkflowProposalState::Dismissed
    ));
    let install = fixture
        .store
        .claim_due_job("installer", 50_000, 30_000)
        .unwrap();
    assert_eq!(
        install.is_some(),
        proposal.state == WorkflowProposalState::ApprovedPendingInstall
    );
}

#[test]
fn invalid_actor_and_unknown_token_fail_before_any_decision() {
    let fixture = fixture(vec!["web_search".to_string()]);
    assert!(matches!(
        fixture.operator.resolve_at_version(
            &fixture.token,
            4,
            ProposalCardAction::Ignore,
            "telegram user",
            60_000
        ),
        Err(WorkflowOperatorError::InvalidActor)
    ));
    assert!(matches!(
        fixture.operator.resolve_at_version(
            "ffffffffffffffffffff",
            4,
            ProposalCardAction::Ignore,
            "telegram:42",
            60_000
        ),
        Err(WorkflowOperatorError::UnknownToken)
    ));
    assert_eq!(
        fixture
            .store
            .get("proposal-operator")
            .unwrap()
            .unwrap()
            .state,
        WorkflowProposalState::Proposed
    );
}
