use crate::workflow_learning_control::{
    NewWorkflowProposal, PublishValidatedDraft, WorkflowArtifactKind, WorkflowLearningStore,
    WorkflowProposalState, WorkflowProposalTransition,
};
use crate::workflow_learning_outbox::{NewWorkflowOutboxItem, WorkflowOutboxStatus};
use crate::workflow_learning_pipeline::{
    WorkflowAnalysisCompletion, WorkflowDraftCompletion, WorkflowPipelineRejection,
    WorkflowValidationCompletion,
};
use crate::workflow_learning_queue::{
    NewWorkflowJob, WorkflowJobEffectState, WorkflowJobKind, WorkflowJobStatus,
};
use crate::MemorySubstrate;

const WORKER: &str = "captain:workflow-learning-worker";

fn store() -> WorkflowLearningStore {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    WorkflowLearningStore::new(memory.usage_conn())
}

fn transition(
    id: &str,
    from: WorkflowProposalState,
    version: u64,
    to: WorkflowProposalState,
    suffix: &str,
    at: i64,
) -> WorkflowProposalTransition {
    WorkflowProposalTransition {
        proposal_id: id.to_string(),
        expected_state: from,
        expected_version: version,
        expected_revision_sha256: None,
        to_state: to,
        actor: WORKER.to_string(),
        reason: format!("pipeline {suffix}"),
        idempotency_key: format!("{id}:{suffix}"),
        snoozed_until_unix_ms: None,
        occurred_at_unix_ms: at,
    }
}

fn job(id: &str, proposal_id: &str, kind: WorkflowJobKind, at: i64) -> NewWorkflowJob {
    NewWorkflowJob {
        id: id.to_string(),
        idempotency_key: format!("{id}:enqueue"),
        proposal_id: proposal_id.to_string(),
        revision_sha256: None,
        kind,
        payload_json: "{}".to_string(),
        max_attempts: 3,
        run_after_unix_ms: at,
        created_at_unix_ms: at,
    }
}

fn observed_with_analyze(store: &WorkflowLearningStore, id: &str) -> String {
    store
        .create_observed(&NewWorkflowProposal {
            id: id.to_string(),
            idempotency_key: format!("{id}:observed"),
            workflow_signature: "a".repeat(64),
            source_agent_id: "captain".to_string(),
            origin_channel: Some("telegram".to_string()),
            evidence_json: r#"{"episodes":3}"#.to_string(),
            created_at_unix_ms: 1_000,
        })
        .unwrap();
    let analyze_id = format!("{id}-analyze");
    store
        .enqueue_job(&job(&analyze_id, id, WorkflowJobKind::Analyze, 1_100))
        .unwrap();
    analyze_id
}

fn claim(store: &WorkflowLearningStore, id: &str, at: i64) {
    let claimed = store.claim_due_job(WORKER, at, 60_000).unwrap().unwrap();
    assert_eq!(claimed.id, id);
}

fn complete_analysis(
    store: &WorkflowLearningStore,
    id: &str,
    analyze_id: &str,
) -> WorkflowAnalysisCompletion {
    claim(store, analyze_id, 1_200);
    let request = WorkflowAnalysisCompletion {
        job_id: analyze_id.to_string(),
        worker: WORKER.to_string(),
        result_json: Some(r#"{"classification":"skill"}"#.to_string()),
        eligibility_transition: transition(
            id,
            WorkflowProposalState::Observed,
            0,
            WorkflowProposalState::Eligible,
            "eligible",
            1_300,
        ),
        drafting_transition: transition(
            id,
            WorkflowProposalState::Eligible,
            1,
            WorkflowProposalState::Drafting,
            "drafting",
            1_301,
        ),
        draft_job: job(&format!("{id}-draft"), id, WorkflowJobKind::Draft, 1_301),
        completed_at_unix_ms: 1_301,
    };
    store.complete_analysis_and_enqueue_draft(&request).unwrap();
    request
}

fn complete_draft(store: &WorkflowLearningStore, id: &str) -> WorkflowDraftCompletion {
    let draft_id = format!("{id}-draft");
    claim(store, &draft_id, 1_400);
    store
        .mark_job_effect_started(&draft_id, WORKER, 1_401)
        .unwrap();
    let request = WorkflowDraftCompletion {
        job_id: draft_id,
        worker: WORKER.to_string(),
        result_json: Some(r#"{"staged":true}"#.to_string()),
        proposal_transition: transition(
            id,
            WorkflowProposalState::Drafting,
            2,
            WorkflowProposalState::Validating,
            "validating",
            1_500,
        ),
        validation_job: job(
            &format!("{id}-validate"),
            id,
            WorkflowJobKind::Validate,
            1_500,
        ),
        completed_at_unix_ms: 1_500,
    };
    store
        .complete_draft_and_enqueue_validation(&request)
        .unwrap();
    request
}

fn validation_request(id: &str) -> WorkflowValidationCompletion {
    let revision = "b".repeat(64);
    WorkflowValidationCompletion {
        job_id: format!("{id}-validate"),
        worker: WORKER.to_string(),
        result_json: Some(r#"{"checks":"green"}"#.to_string()),
        publish: PublishValidatedDraft {
            proposal_id: id.to_string(),
            expected_version: 3,
            staging_job_id: format!("{id}-draft"),
            revision_sha256: revision.clone(),
            artifact_sha256: "c".repeat(64),
            kind: WorkflowArtifactKind::Skill,
            name: format!("{id}-skill"),
            validation_json: r#"{"checks":"green"}"#.to_string(),
            actor: WORKER.to_string(),
            reason: "validated exact staged draft".to_string(),
            idempotency_key: format!("{id}:published"),
            occurred_at_unix_ms: 1_700,
        },
        notification: Some(NewWorkflowOutboxItem {
            id: format!("{id}-proposed-message"),
            idempotency_key: format!("{id}:proposed-message"),
            proposal_id: id.to_string(),
            revision_sha256: Some(revision),
            topic: "workflow_learning.proposed".to_string(),
            payload_json: r#"{"state":"proposed"}"#.to_string(),
            max_attempts: 5,
            run_after_unix_ms: 1_700,
            created_at_unix_ms: 1_700,
        }),
        completed_at_unix_ms: 1_700,
    }
}

#[test]
fn preapproval_pipeline_commits_every_phase_and_replays_late() {
    let store = store();
    let id = "pipeline-happy";
    let analyze_id = observed_with_analyze(&store, id);
    let analysis = complete_analysis(&store, id, &analyze_id);
    let draft = complete_draft(&store, id);
    claim(&store, &format!("{id}-validate"), 1_600);
    let validation = validation_request(id);

    let published = store.complete_validation_and_publish(&validation).unwrap();
    assert_eq!(published.proposal.state, WorkflowProposalState::Proposed);
    assert_eq!(published.proposal.state_version, 4);
    assert_eq!(published.job.status, WorkflowJobStatus::Succeeded);
    assert_eq!(
        published.job.effect_state,
        WorkflowJobEffectState::Completed
    );
    assert_eq!(
        published.notification.unwrap().status,
        WorkflowOutboxStatus::Pending
    );

    assert_eq!(
        store
            .complete_analysis_and_enqueue_draft(&analysis)
            .unwrap()
            .proposal
            .state,
        WorkflowProposalState::Proposed
    );
    assert_eq!(
        store
            .complete_draft_and_enqueue_validation(&draft)
            .unwrap()
            .proposal
            .state,
        WorkflowProposalState::Proposed
    );
    assert_eq!(
        store
            .complete_validation_and_publish(&validation)
            .unwrap()
            .proposal
            .state_version,
        4
    );
    assert_eq!(store.events(id).unwrap().len(), 5);
}

#[test]
fn invalid_continuation_rolls_back_transition_and_job_completion() {
    let store = store();
    let id = "pipeline-rollback";
    let analyze_id = observed_with_analyze(&store, id);
    complete_analysis(&store, id, &analyze_id);
    let draft_id = format!("{id}-draft");
    claim(&store, &draft_id, 1_400);
    store
        .mark_job_effect_started(&draft_id, WORKER, 1_401)
        .unwrap();
    let request = WorkflowDraftCompletion {
        job_id: draft_id.clone(),
        worker: WORKER.to_string(),
        result_json: Some(r#"{"staged":true}"#.to_string()),
        proposal_transition: transition(
            id,
            WorkflowProposalState::Drafting,
            2,
            WorkflowProposalState::Validating,
            "validating",
            1_500,
        ),
        validation_job: NewWorkflowJob {
            payload_json: "not-json".to_string(),
            ..job(
                &format!("{id}-validate"),
                id,
                WorkflowJobKind::Validate,
                1_500,
            )
        },
        completed_at_unix_ms: 1_500,
    };

    assert!(store
        .complete_draft_and_enqueue_validation(&request)
        .is_err());
    assert_eq!(
        store.get(id).unwrap().unwrap().state,
        WorkflowProposalState::Drafting
    );
    let draft_job = store.get_job(&draft_id).unwrap().unwrap();
    assert_eq!(draft_job.status, WorkflowJobStatus::Running);
    assert_eq!(draft_job.effect_state, WorkflowJobEffectState::Started);
    assert!(store.get_job(&format!("{id}-validate")).unwrap().is_none());
}

#[test]
fn validation_rejection_is_terminal_audited_and_not_published() {
    let store = store();
    let id = "pipeline-rejected";
    let analyze_id = observed_with_analyze(&store, id);
    complete_analysis(&store, id, &analyze_id);
    complete_draft(&store, id);
    let validate_id = format!("{id}-validate");
    claim(&store, &validate_id, 1_600);
    let rejection = WorkflowPipelineRejection {
        job_id: validate_id,
        worker: WORKER.to_string(),
        job_kind: WorkflowJobKind::Validate,
        result_json: Some(r#"{"valid":false,"reason":"duplicate"}"#.to_string()),
        proposal_transition: transition(
            id,
            WorkflowProposalState::Validating,
            3,
            WorkflowProposalState::Rejected,
            "rejected",
            1_700,
        ),
        notification: Some(NewWorkflowOutboxItem {
            id: format!("{id}-rejected-message"),
            idempotency_key: format!("{id}:rejected-message"),
            proposal_id: id.to_string(),
            revision_sha256: None,
            topic: "workflow_learning.rejected".to_string(),
            payload_json: r#"{"state":"rejected"}"#.to_string(),
            max_attempts: 5,
            run_after_unix_ms: 1_700,
            created_at_unix_ms: 1_700,
        }),
        completed_at_unix_ms: 1_700,
    };

    let result = store.reject_pipeline_candidate(&rejection).unwrap();
    assert_eq!(result.proposal.state, WorkflowProposalState::Rejected);
    assert!(result.proposal.revision_sha256.is_none());
    assert_eq!(result.job.status, WorkflowJobStatus::Succeeded);
    assert_eq!(
        store
            .reject_pipeline_candidate(&rejection)
            .unwrap()
            .proposal
            .state_version,
        4
    );
}

#[test]
fn unique_staged_draft_can_settle_an_uncertain_model_job_without_replay() {
    let store = store();
    let id = "pipeline-recovered-draft";
    let analyze_id = observed_with_analyze(&store, id);
    complete_analysis(&store, id, &analyze_id);
    let draft_id = format!("{id}-draft");
    claim(&store, &draft_id, 1_400);
    store
        .mark_job_effect_started(&draft_id, WORKER, 1_401)
        .unwrap();
    assert_eq!(
        store
            .reconcile_expired_jobs(61_401)
            .unwrap()
            .uncertain_effects,
        1
    );
    assert_eq!(store.list_uncertain_jobs(10).unwrap()[0].id, draft_id);
    let recovery = WorkflowDraftCompletion {
        job_id: draft_id.clone(),
        worker: WORKER.to_string(),
        result_json: Some(
            r#"{"recovered_from_staging":true,"revision_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}"#
                .to_string(),
        ),
        proposal_transition: transition(
            id,
            WorkflowProposalState::Drafting,
            2,
            WorkflowProposalState::Validating,
            "validating-after-recovery",
            61_500,
        ),
        validation_job: job(
            &format!("{id}-validate"),
            id,
            WorkflowJobKind::Validate,
            61_500,
        ),
        completed_at_unix_ms: 61_500,
    };

    let result = store
        .recover_staged_draft_and_enqueue_validation(&recovery)
        .unwrap();
    assert_eq!(result.proposal.state, WorkflowProposalState::Validating);
    assert_eq!(result.job.status, WorkflowJobStatus::Succeeded);
    assert_eq!(result.job.effect_state, WorkflowJobEffectState::Completed);
    assert_eq!(result.next_job.unwrap().kind, WorkflowJobKind::Validate);
    assert_eq!(
        store
            .recover_staged_draft_and_enqueue_validation(&recovery)
            .unwrap()
            .proposal
            .state_version,
        3
    );
}
