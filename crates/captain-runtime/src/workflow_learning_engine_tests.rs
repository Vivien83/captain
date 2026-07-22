use std::collections::VecDeque;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use captain_memory::workflow_learning::{
    NewWorkflowEpisode, NewWorkflowEpisodeStep, WorkflowEpisodeStatus, WorkflowEpisodeStore,
    WorkflowStepOutcome, WorkflowStepStatus,
};
use captain_memory::workflow_learning_control::{WorkflowLearningStore, WorkflowProposalState};
use captain_memory::workflow_learning_outbox::WorkflowOutboxStatus;
use captain_memory::workflow_learning_queue::WorkflowJobKind;
use captain_memory::workflow_learning_refinement::{
    NewWorkflowRefinementRequest, WorkflowRefinementState,
};
use captain_memory::MemorySubstrate;
use serde_json::json;

use crate::reflection_job::ReflectionCompleter;
use crate::workflow_learning_engine::{
    WorkflowJobRunOutcome, WorkflowLearningEngine, WorkflowLearningEngineConfig,
};
use crate::workflow_learning_engine_support::{parse_draft_payload, DraftJobPayload};
use crate::workflow_learning_proposer::{
    parse_workflow_draft, ActiveModelIdentity, WorkflowDraftKind, WorkflowDraftProposer,
    WorkflowProposerOutcome,
};
use crate::workflow_learning_refinement::{
    WorkflowRefinementCaptureInput, WorkflowRefinementCoordinator,
};
use crate::workflow_learning_staging::{StageWorkflowDraftRequest, WorkflowStagingRoot};

struct ScriptedCompleter {
    responses: Mutex<VecDeque<Result<String, String>>>,
    calls: Mutex<usize>,
}

impl ScriptedCompleter {
    fn new(responses: Vec<Result<String, String>>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            calls: Mutex::new(0),
        }
    }

    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

#[async_trait]
impl ReflectionCompleter for ScriptedCompleter {
    async fn complete(&self, _model: &str, _system: &str, _user: &str) -> Result<String, String> {
        *self.calls.lock().unwrap() += 1;
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted response")
    }
}

struct Harness {
    _home: tempfile::TempDir,
    _memory: MemorySubstrate,
    episodes: WorkflowEpisodeStore,
    control: WorkflowLearningStore,
    engine: WorkflowLearningEngine,
    completer: Arc<ScriptedCompleter>,
}

fn harness(responses: Vec<Result<String, String>>) -> Harness {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let episodes = WorkflowEpisodeStore::new(memory.usage_conn());
    let control = WorkflowLearningStore::new(memory.usage_conn());
    let completer = Arc::new(ScriptedCompleter::new(responses));
    let proposer = WorkflowDraftProposer::new(
        completer.clone(),
        ActiveModelIdentity {
            provider: "codex".to_string(),
            model: "gpt-5.6-sol".to_string(),
        },
        Duration::from_secs(5),
        "en",
    );
    let home = tempfile::tempdir().unwrap();
    let engine = WorkflowLearningEngine::new(
        episodes.clone(),
        control.clone(),
        proposer,
        WorkflowStagingRoot::new(home.path().join("captain-home")).unwrap(),
        WorkflowLearningEngineConfig::default(),
    )
    .unwrap();
    Harness {
        _home: home,
        _memory: memory,
        episodes,
        control,
        engine,
        completer,
    }
}

fn valid_skill_response() -> String {
    json!({
        "decision": "draft",
        "schema_version": 1,
        "kind": "skill",
        "name": "source-backed-brief",
        "purpose": "Research a subject and produce a concise source-backed brief.",
        "trigger": "Use when current facts must be compared across reliable sources.",
        "artifact": {
            "format": "skill_markdown",
            "source": "---\nname: source-backed-brief\ndescription: Produce a concise source-backed research brief\n---\n# Workflow\nSearch authoritative sources, inspect the strongest results, compare claims, and cite the evidence."
        },
        "required_capabilities": ["web_search", "web_fetch"],
        "expected_benefit": "Produces repeatable current research with explicit evidence.",
        "limitations": ["A human should review high-stakes conclusions."]
    })
    .to_string()
}

fn valid_capspec_response() -> String {
    let source = r#"format = 1
name = "extract-couple-documents"
description = "Read and extract a document through a typed path."
version = "1.0.0"

[permissions]
tools = ["file_read", "document_extract"]
read_paths = ["{{input.path}}"]

[inputs.path]
type = "string"
description = "Document path"

[[steps]]
id = "read"
tool = "file_read"
needs = []
with = { path = "{{input.path}}" }

[[steps]]
id = "extract"
tool = "document_extract"
needs = ["read"]
with = { path = "{{input.path}}", format = "markdown" }
"#;
    json!({
        "decision": "draft",
        "schema_version": 1,
        "kind": "capspec",
        "name": "extract-couple-documents",
        "purpose": "Read and extract a household document through a typed path.",
        "trigger": "Use when a household document must be indexed reliably.",
        "artifact": {"format": "capspec_toml", "source": source},
        "required_capabilities": ["file_read", "document_extract"],
        "expected_benefit": "Makes document extraction deterministic and auditable.",
        "limitations": ["Scanned documents may still require OCR before extraction."]
    })
    .to_string()
}

fn valid_automation_response() -> String {
    json!({
        "decision": "draft",
        "schema_version": 1,
        "kind": "automation",
        "name": "twice-daily-vps-status",
        "purpose": "Send a concise VPS status report twice per day.",
        "trigger": "Use when recurring VPS status delivery is requested.",
        "artifact": {
            "format": "automation",
            "schedule": {"kind": "every", "every_secs": 43200},
            "instruction": "Send the verified VPS status summary to the configured channel."
        },
        "required_capabilities": ["channel_send"],
        "expected_benefit": "Keeps operational status visible on a stable cadence.",
        "limitations": ["The underlying health data must already be verified."]
    })
    .to_string()
}

fn refined_skill_response() -> String {
    valid_skill_response().replace(
        "Search authoritative sources, inspect the strongest results, compare claims, and cite the evidence.",
        "Search authoritative sources, inspect the strongest results, compare claims, cite the evidence, and finish with a concise comparison table.",
    )
}

fn assert_advanced(outcome: WorkflowJobRunOutcome, expected_kind: WorkflowJobKind) {
    let WorkflowJobRunOutcome::Advanced {
        kind,
        job_id,
        proposal_id,
    } = outcome
    else {
        panic!("expected an advanced workflow job");
    };
    assert_eq!(kind, expected_kind);
    assert!(!job_id.is_empty());
    assert!(!proposal_id.is_empty());
}

fn assert_retrying(outcome: WorkflowJobRunOutcome, expected_kind: WorkflowJobKind) {
    let WorkflowJobRunOutcome::Retrying {
        kind,
        job_id,
        proposal_id,
    } = outcome
    else {
        panic!("expected a retrying workflow job");
    };
    assert_eq!(kind, expected_kind);
    assert!(!job_id.is_empty());
    assert!(!proposal_id.is_empty());
}

fn assert_rejected(outcome: WorkflowJobRunOutcome, expected_kind: WorkflowJobKind) {
    let WorkflowJobRunOutcome::Rejected {
        kind,
        job_id,
        proposal_id,
    } = outcome
    else {
        panic!("expected a rejected workflow job");
    };
    assert_eq!(kind, expected_kind);
    assert!(!job_id.is_empty());
    assert!(!proposal_id.is_empty());
}

fn add_research_episode(
    store: &WorkflowEpisodeStore,
    id: &str,
    session: &str,
    turn: &str,
    at: i64,
) {
    store
        .begin_episode(&NewWorkflowEpisode {
            id: id.to_string(),
            session_id: session.to_string(),
            turn_id: turn.to_string(),
            agent_id: "captain".to_string(),
            origin_channel: Some("telegram".to_string()),
            project_id: None,
            workspace_scope: None,
            intent_redacted: "Research the current subject with reliable sources".to_string(),
            intent_fingerprint: "intent-research".to_string(),
            secret_detected: false,
            explicit_reuse_request: false,
            started_at_unix_ms: at,
        })
        .unwrap();
    for (ordinal, tool, dependencies, shape) in [
        (0, "web_search", "[]", r#"{"query":"<text>"}"#),
        (1, "web_fetch", "[\"search\"]", r#"{"url":"<url>"}"#),
    ] {
        let tool_use_id = if ordinal == 0 { "search" } else { "fetch" };
        store
            .begin_step(&NewWorkflowEpisodeStep {
                episode_id: id.to_string(),
                tool_use_id: tool_use_id.to_string(),
                ordinal,
                tool_name: tool.to_string(),
                dependency_ids_json: dependencies.to_string(),
                input_shape_json: shape.to_string(),
                input_fingerprint: format!("fingerprint-{ordinal}"),
                effect_class: "read".to_string(),
                secret_detected: false,
                started_at_unix_ms: at + 10 + i64::from(ordinal),
            })
            .unwrap();
        store
            .finish_step(
                id,
                tool_use_id,
                &WorkflowStepOutcome {
                    status: WorkflowStepStatus::Succeeded,
                    output_class: Some("result_received".to_string()),
                    verification_marker: Some("result_received".to_string()),
                    retry_count: 0,
                    completed_at_unix_ms: at + 20 + i64::from(ordinal),
                },
            )
            .unwrap();
    }
    store
        .finish_episode(id, WorkflowEpisodeStatus::Succeeded, None, at + 30)
        .unwrap();
}

fn add_linear_episode(
    store: &WorkflowEpisodeStore,
    id: &str,
    session: &str,
    turn: &str,
    at: i64,
    intent: &str,
    steps: &[(&str, serde_json::Value, &str, &str)],
) {
    store
        .begin_episode(&NewWorkflowEpisode {
            id: id.to_string(),
            session_id: session.to_string(),
            turn_id: turn.to_string(),
            agent_id: "captain".to_string(),
            origin_channel: Some("telegram".to_string()),
            project_id: None,
            workspace_scope: None,
            intent_redacted: intent.to_string(),
            intent_fingerprint: format!("intent-{}", steps[0].0),
            secret_detected: false,
            explicit_reuse_request: false,
            started_at_unix_ms: at,
        })
        .unwrap();

    let mut previous_id = None;
    for (ordinal, (tool, shape, effect_class, verification_marker)) in steps.iter().enumerate() {
        let tool_use_id = format!("{id}-step-{ordinal}");
        let dependency_ids = previous_id.iter().cloned().collect::<Vec<String>>();
        store
            .begin_step(&NewWorkflowEpisodeStep {
                episode_id: id.to_string(),
                tool_use_id: tool_use_id.clone(),
                ordinal: ordinal as u32,
                tool_name: (*tool).to_string(),
                dependency_ids_json: serde_json::to_string(&dependency_ids).unwrap(),
                input_shape_json: shape.to_string(),
                input_fingerprint: format!("{tool}-shape-{ordinal}"),
                effect_class: (*effect_class).to_string(),
                secret_detected: false,
                started_at_unix_ms: at + 10 + ordinal as i64,
            })
            .unwrap();
        store
            .finish_step(
                id,
                &tool_use_id,
                &WorkflowStepOutcome {
                    status: WorkflowStepStatus::Succeeded,
                    output_class: Some("tool_success".to_string()),
                    verification_marker: Some((*verification_marker).to_string()),
                    retry_count: 0,
                    completed_at_unix_ms: at + 20 + ordinal as i64,
                },
            )
            .unwrap();
        previous_id = Some(tool_use_id);
    }
    store
        .finish_episode(id, WorkflowEpisodeStatus::Succeeded, None, at + 40)
        .unwrap();
}

fn add_document_episode(
    store: &WorkflowEpisodeStore,
    id: &str,
    session: &str,
    turn: &str,
    at: i64,
) {
    add_linear_episode(
        store,
        id,
        session,
        turn,
        at,
        "Index the household document and extract its verified content",
        &[
            (
                "file_read",
                json!({"path": "<path>"}),
                "read",
                "result_received",
            ),
            (
                "document_extract",
                json!({"format": "enum:markdown", "path": "<path>"}),
                "read",
                "result_received",
            ),
        ],
    );
}

fn add_automation_episode(
    store: &WorkflowEpisodeStore,
    id: &str,
    session: &str,
    turn: &str,
    at: i64,
) {
    add_linear_episode(
        store,
        id,
        session,
        turn,
        at,
        "Schedule and send the recurring VPS status report",
        &[
            (
                "cron_create",
                json!({"schedule": "<string>"}),
                "external",
                "operation_confirmed",
            ),
            (
                "channel_send",
                json!({"channel": "<string>", "message": "<text>"}),
                "external",
                "operation_confirmed",
            ),
        ],
    );
}

fn add_memory_episode(store: &WorkflowEpisodeStore, id: &str, session: &str, turn: &str, at: i64) {
    add_linear_episode(
        store,
        id,
        session,
        turn,
        at,
        "Recall the user's preferred deployment policy",
        &[(
            "memory_recall",
            json!({"query": "<text>"}),
            "read",
            "result_received",
        )],
    );
}

type EpisodeAdder = fn(&WorkflowEpisodeStore, &str, &str, &str, i64);

async fn assert_real_domain_pipeline(
    adder: EpisodeAdder,
    response: String,
    expected_kind: &str,
    expected_name: &str,
) {
    let harness = harness(vec![Ok(response)]);
    for (suffix, session, turn, at) in [
        ("a", "session-a", "turn-a", 1_000),
        ("b", "session-a", "turn-b", 1_100),
        ("c", "session-b", "turn-c", 1_200),
    ] {
        adder(
            &harness.episodes,
            &format!("{expected_kind}-{suffix}"),
            session,
            turn,
            at,
        );
    }

    let scan = harness.engine.scan_once(2_000).unwrap();
    assert_eq!(scan.proposals_created, 1, "{expected_kind}");
    assert_eq!(scan.rejected, 0, "{expected_kind}");
    assert_advanced(
        harness.engine.run_next_job(2_100).await.unwrap(),
        WorkflowJobKind::Analyze,
    );
    assert_advanced(
        harness.engine.run_next_job(2_200).await.unwrap(),
        WorkflowJobKind::Draft,
    );
    assert_advanced(
        harness.engine.run_next_job(2_300).await.unwrap(),
        WorkflowJobKind::Validate,
    );

    let proposals = harness
        .control
        .list(Some(WorkflowProposalState::Proposed), 10)
        .unwrap();
    assert_eq!(proposals.len(), 1, "{expected_kind}");
    let proposal = &proposals[0];
    assert_eq!(
        proposal.kind.as_ref().map(|kind| kind.as_str()),
        Some(expected_kind)
    );
    assert_eq!(proposal.name.as_deref(), Some(expected_name));
    assert_eq!(harness.completer.calls(), 1);
    assert_eq!(proposal.revision_sha256.as_ref().unwrap().len(), 64);
    assert_eq!(proposal.artifact_sha256.as_ref().unwrap().len(), 64);
}

#[tokio::test]
async fn recurrence_stays_pending_then_reaches_one_proposed_revision_end_to_end() {
    let harness = harness(vec![Ok(valid_skill_response())]);
    add_research_episode(&harness.episodes, "episode-a", "session-a", "turn-a", 1_000);
    let first = harness.engine.scan_once(2_000).unwrap();
    assert_eq!(first.deferred, 1);
    assert_eq!(
        harness
            .episodes
            .get_episode("episode-a")
            .unwrap()
            .unwrap()
            .analysis_status,
        "pending"
    );

    add_research_episode(&harness.episodes, "episode-b", "session-b", "turn-b", 2_100);
    assert_eq!(harness.engine.scan_once(3_000).unwrap().deferred, 2);
    add_research_episode(&harness.episodes, "episode-c", "session-b", "turn-c", 3_100);
    let eligible = harness.engine.scan_once(4_000).unwrap();
    assert_eq!(eligible.proposals_created, 1);
    for id in ["episode-a", "episode-b", "episode-c"] {
        assert_eq!(
            harness
                .episodes
                .get_episode(id)
                .unwrap()
                .unwrap()
                .analysis_status,
            "processed"
        );
    }

    assert_advanced(
        harness.engine.run_next_job(4_100).await.unwrap(),
        WorkflowJobKind::Analyze,
    );
    assert_advanced(
        harness.engine.run_next_job(4_200).await.unwrap(),
        WorkflowJobKind::Draft,
    );
    assert_eq!(harness.completer.calls(), 1);
    assert_advanced(
        harness.engine.run_next_job(4_300).await.unwrap(),
        WorkflowJobKind::Validate,
    );

    let proposals = harness
        .control
        .list(Some(WorkflowProposalState::Proposed), 10)
        .unwrap();
    assert_eq!(proposals.len(), 1);
    let proposal = &proposals[0];
    assert_eq!(proposal.revision_sha256.as_ref().unwrap().len(), 64);
    assert_eq!(proposal.kind.unwrap().as_str(), "skill");
    let outbox = harness
        .control
        .get_outbox(&format!("{}-proposed", proposal.id))
        .unwrap()
        .unwrap();
    assert_eq!(outbox.status, WorkflowOutboxStatus::Pending);
}

#[tokio::test]
async fn real_domain_matrix_proposes_skill_capspec_and_automation_but_not_memory() {
    assert_real_domain_pipeline(
        add_research_episode,
        valid_skill_response(),
        "skill",
        "source-backed-brief",
    )
    .await;
    assert_real_domain_pipeline(
        add_document_episode,
        valid_capspec_response(),
        "capspec",
        "extract-couple-documents",
    )
    .await;
    assert_real_domain_pipeline(
        add_automation_episode,
        valid_automation_response(),
        "automation",
        "twice-daily-vps-status",
    )
    .await;

    let memory = harness(Vec::new());
    for (suffix, session, turn, at) in [
        ("a", "session-a", "turn-a", 1_000),
        ("b", "session-a", "turn-b", 1_100),
        ("c", "session-b", "turn-c", 1_200),
    ] {
        add_memory_episode(
            &memory.episodes,
            &format!("memory-{suffix}"),
            session,
            turn,
            at,
        );
    }
    let scan = memory.engine.scan_once(2_000).unwrap();
    assert_eq!(scan.proposals_created, 0);
    assert_eq!(scan.rejected, 3);
    assert!(memory.control.list(None, 10).unwrap().is_empty());
    assert_eq!(memory.completer.calls(), 0);
    assert_eq!(
        memory.engine.run_next_job(2_100).await.unwrap(),
        WorkflowJobRunOutcome::Idle
    );
}

#[tokio::test]
async fn invalid_structured_model_output_retries_with_backoff_then_succeeds() {
    let harness = harness(vec![Ok("not-json".to_string()), Ok(valid_skill_response())]);
    for (id, session, turn, at) in [
        ("retry-a", "session-a", "turn-a", 1_000),
        ("retry-b", "session-b", "turn-b", 1_100),
        ("retry-c", "session-b", "turn-c", 1_200),
    ] {
        add_research_episode(&harness.episodes, id, session, turn, at);
    }
    harness.engine.scan_once(2_000).unwrap();
    harness.engine.run_next_job(2_100).await.unwrap();
    assert_retrying(
        harness.engine.run_next_job(2_200).await.unwrap(),
        WorkflowJobKind::Draft,
    );
    assert_eq!(harness.completer.calls(), 1);
    assert_eq!(
        harness.engine.run_next_job(2_201).await.unwrap(),
        WorkflowJobRunOutcome::Idle
    );
    assert_advanced(
        harness.engine.run_next_job(32_200).await.unwrap(),
        WorkflowJobKind::Draft,
    );
    assert_eq!(harness.completer.calls(), 2);
}

#[tokio::test]
async fn staging_failure_rejects_without_replaying_the_model() {
    let harness = harness(vec![Ok(valid_skill_response())]);
    for (id, session, turn, at) in [
        ("stage-a", "session-a", "turn-a", 1_000),
        ("stage-b", "session-b", "turn-b", 1_100),
        ("stage-c", "session-b", "turn-c", 1_200),
    ] {
        add_research_episode(&harness.episodes, id, session, turn, at);
    }
    harness.engine.scan_once(2_000).unwrap();
    harness.engine.run_next_job(2_100).await.unwrap();

    let learning_root = harness._home.path().join("captain-home").join("learning");
    fs::create_dir_all(&learning_root).unwrap();
    fs::write(learning_root.join("staging"), b"not-a-directory").unwrap();

    assert_rejected(
        harness.engine.run_next_job(2_200).await.unwrap(),
        WorkflowJobKind::Draft,
    );
    assert_eq!(harness.completer.calls(), 1);
    assert_eq!(
        harness.engine.run_next_job(100_000).await.unwrap(),
        WorkflowJobRunOutcome::Idle
    );
    assert_eq!(
        harness
            .control
            .list(Some(WorkflowProposalState::Rejected), 10)
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn captured_refinement_replaces_parent_end_to_end_with_one_model_call() {
    let harness = harness(vec![
        Ok(valid_skill_response()),
        Ok(refined_skill_response()),
    ]);
    let parent = drive_parent_to_proposed(&harness, "refine").await;
    let captured = begin_and_capture_refinement(&harness, &parent, 5_000);

    assert_advanced(
        harness.engine.run_next_job(5_100).await.unwrap(),
        WorkflowJobKind::Draft,
    );
    assert_eq!(harness.completer.calls(), 2);
    assert_advanced(
        harness.engine.run_next_job(5_200).await.unwrap(),
        WorkflowJobKind::Validate,
    );

    assert_eq!(
        harness.control.get(&parent.id).unwrap().unwrap().state,
        WorkflowProposalState::Superseded
    );
    let child = harness
        .control
        .get(&captured.child_proposal.id)
        .unwrap()
        .unwrap();
    assert_eq!(child.state, WorkflowProposalState::Proposed);
    assert_eq!(child.name.as_deref(), Some("source-backed-brief"));
    assert_eq!(
        harness
            .control
            .get_refinement_request("engine-refinement")
            .unwrap()
            .unwrap()
            .state,
        WorkflowRefinementState::Completed
    );
    assert_eq!(
        harness
            .control
            .get_outbox(&format!("{}-proposed", child.id))
            .unwrap()
            .unwrap()
            .status,
        WorkflowOutboxStatus::Pending
    );
}

#[tokio::test]
async fn staged_refinement_recovers_after_crash_without_replaying_model() {
    let harness = harness(vec![Ok(valid_skill_response())]);
    let parent = drive_parent_to_proposed(&harness, "recover").await;
    let captured = begin_and_capture_refinement(&harness, &parent, 5_000);
    let job = harness
        .control
        .claim_due_preapproval_job("captain:workflow-learning-v2", 5_100, 120_000)
        .unwrap()
        .unwrap();
    let DraftJobPayload::Refinement(payload) = parse_draft_payload(&job.payload_json).unwrap()
    else {
        panic!("captured refinement produced a discovery job");
    };
    harness
        .control
        .mark_job_effect_started(&job.id, "captain:workflow-learning-v2", 5_101)
        .unwrap();
    let WorkflowProposerOutcome::Draft(refined) =
        parse_workflow_draft(&refined_skill_response(), WorkflowDraftKind::Skill).unwrap()
    else {
        panic!("refined fixture declined");
    };
    harness
        .engine
        .staging
        .stage(StageWorkflowDraftRequest {
            job_id: &job.id,
            workflow_signature: &payload.group.signature,
            draft: &refined,
            active_model: harness.engine.proposer.active_model(),
        })
        .unwrap();

    harness.control.reconcile_expired_jobs(130_000).unwrap();
    let recovery = harness.engine.recover_staged_drafts(130_100).unwrap();
    assert_eq!(recovery.recovered, 1);
    assert_eq!(harness.completer.calls(), 1);
    assert_advanced(
        harness.engine.run_next_job(130_200).await.unwrap(),
        WorkflowJobKind::Validate,
    );
    assert_eq!(
        harness
            .control
            .get(&captured.child_proposal.id)
            .unwrap()
            .unwrap()
            .state,
        WorkflowProposalState::Proposed
    );
    assert_eq!(harness.completer.calls(), 1);
}

#[tokio::test]
async fn staged_discovery_reopens_after_power_loss_without_replaying_model() {
    let root = tempfile::tempdir().unwrap();
    let db_path = root.path().join("memory.sqlite3");
    let captain_home = root.path().join("captain-home");

    {
        let memory = MemorySubstrate::open(&db_path, 0.01).unwrap();
        let episodes = WorkflowEpisodeStore::new(memory.usage_conn());
        let control = WorkflowLearningStore::new(memory.usage_conn());
        let completer = Arc::new(ScriptedCompleter::new(vec![Ok(valid_skill_response())]));
        let proposer = WorkflowDraftProposer::new(
            completer.clone(),
            ActiveModelIdentity {
                provider: "codex".to_string(),
                model: "gpt-5.6-sol".to_string(),
            },
            Duration::from_secs(5),
            "en",
        );
        let engine = WorkflowLearningEngine::new(
            episodes.clone(),
            control.clone(),
            proposer,
            WorkflowStagingRoot::new(&captain_home).unwrap(),
            WorkflowLearningEngineConfig::default(),
        )
        .unwrap();
        for (suffix, session, turn, at) in [
            ("a", "session-a", "turn-a", 1_000),
            ("b", "session-a", "turn-b", 1_100),
            ("c", "session-b", "turn-c", 1_200),
        ] {
            add_research_episode(
                &episodes,
                &format!("power-loss-{suffix}"),
                session,
                turn,
                at,
            );
        }
        assert_eq!(engine.scan_once(2_000).unwrap().proposals_created, 1);
        assert_advanced(
            engine.run_next_job(2_100).await.unwrap(),
            WorkflowJobKind::Analyze,
        );

        let job = control
            .claim_due_preapproval_job("captain:workflow-learning-v2", 2_200, 120_000)
            .unwrap()
            .unwrap();
        assert_eq!(job.kind, WorkflowJobKind::Draft);
        let DraftJobPayload::Discovery(payload) = parse_draft_payload(&job.payload_json).unwrap()
        else {
            panic!("discovery produced a refinement job");
        };
        control
            .mark_job_effect_started(&job.id, "captain:workflow-learning-v2", 2_201)
            .unwrap();
        let WorkflowProposerOutcome::Draft(draft) =
            engine.proposer.draft(&payload.group).await.unwrap()
        else {
            panic!("fixture unexpectedly declined");
        };
        engine
            .staging
            .stage(StageWorkflowDraftRequest {
                job_id: &job.id,
                workflow_signature: &payload.group.signature,
                draft: &draft,
                active_model: engine.proposer.active_model(),
            })
            .unwrap();
        assert_eq!(completer.calls(), 1);
    }

    let memory = MemorySubstrate::open(&db_path, 0.01).unwrap();
    let episodes = WorkflowEpisodeStore::new(memory.usage_conn());
    let control = WorkflowLearningStore::new(memory.usage_conn());
    let completer = Arc::new(ScriptedCompleter::new(Vec::new()));
    let proposer = WorkflowDraftProposer::new(
        completer.clone(),
        ActiveModelIdentity {
            provider: "codex".to_string(),
            model: "gpt-5.6-sol".to_string(),
        },
        Duration::from_secs(5),
        "en",
    );
    let engine = WorkflowLearningEngine::new(
        episodes,
        control.clone(),
        proposer,
        WorkflowStagingRoot::new(&captain_home).unwrap(),
        WorkflowLearningEngineConfig::default(),
    )
    .unwrap();

    control.reconcile_expired_jobs(130_000).unwrap();
    let recovery = engine.recover_staged_drafts(130_100).unwrap();
    assert_eq!(recovery.recovered, 1);
    assert_eq!(completer.calls(), 0);
    assert_advanced(
        engine.run_next_job(130_200).await.unwrap(),
        WorkflowJobKind::Validate,
    );
    assert_eq!(completer.calls(), 0);
    assert_eq!(
        control
            .list(Some(WorkflowProposalState::Proposed), 10)
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn declined_refinement_leaves_parent_proposed_and_fails_request() {
    let harness = harness(vec![
        Ok(valid_skill_response()),
        Ok(json!({
            "decision": "decline",
            "schema_version": 1,
            "reason": "The requested change would remove the source verification contract."
        })
        .to_string()),
    ]);
    let parent = drive_parent_to_proposed(&harness, "decline").await;
    let captured = begin_and_capture_refinement(&harness, &parent, 5_000);

    assert_rejected(
        harness.engine.run_next_job(5_100).await.unwrap(),
        WorkflowJobKind::Draft,
    );
    assert_eq!(
        harness.control.get(&parent.id).unwrap().unwrap().state,
        WorkflowProposalState::Proposed
    );
    assert_eq!(
        harness
            .control
            .get(&captured.child_proposal.id)
            .unwrap()
            .unwrap()
            .state,
        WorkflowProposalState::Rejected
    );
    assert_eq!(
        harness
            .control
            .get_refinement_request("engine-refinement")
            .unwrap()
            .unwrap()
            .state,
        WorkflowRefinementState::Failed
    );
}

async fn drive_parent_to_proposed(
    harness: &Harness,
    prefix: &str,
) -> captain_memory::workflow_learning_control::WorkflowProposalRecord {
    for (suffix, session, turn, at) in [
        ("a", "session-a", "turn-a", 1_000),
        ("b", "session-b", "turn-b", 1_100),
        ("c", "session-b", "turn-c", 1_200),
    ] {
        add_research_episode(
            &harness.episodes,
            &format!("{prefix}-{suffix}"),
            session,
            turn,
            at,
        );
    }
    harness.engine.scan_once(2_000).unwrap();
    assert_advanced(
        harness.engine.run_next_job(2_100).await.unwrap(),
        WorkflowJobKind::Analyze,
    );
    assert_advanced(
        harness.engine.run_next_job(2_200).await.unwrap(),
        WorkflowJobKind::Draft,
    );
    assert_advanced(
        harness.engine.run_next_job(2_300).await.unwrap(),
        WorkflowJobKind::Validate,
    );
    harness
        .control
        .list(Some(WorkflowProposalState::Proposed), 10)
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
}

fn begin_and_capture_refinement(
    harness: &Harness,
    parent: &captain_memory::workflow_learning_control::WorkflowProposalRecord,
    now_unix_ms: i64,
) -> captain_memory::workflow_learning_refinement_capture::WorkflowRefinementCaptureResult {
    harness
        .control
        .begin_refinement_request(&NewWorkflowRefinementRequest {
            id: "engine-refinement".to_string(),
            idempotency_key: "engine-refinement:begin".to_string(),
            proposal_id: parent.id.clone(),
            revision_sha256: parent.revision_sha256.clone().unwrap(),
            expected_proposal_version: parent.state_version,
            actor: "telegram:42".to_string(),
            surface: "telegram".to_string(),
            conversation_key: "telegram:chat:42:root".to_string(),
            source_message_id: Some("100".to_string()),
            language: "fr".to_string(),
            expires_at_unix_ms: now_unix_ms + 60_000,
            created_at_unix_ms: now_unix_ms,
        })
        .unwrap();
    WorkflowRefinementCoordinator::new(harness.control.clone(), harness.engine.staging.clone())
        .capture_pending(&WorkflowRefinementCaptureInput {
            actor: "telegram:42".to_string(),
            surface: "telegram".to_string(),
            conversation_key: "telegram:chat:42:root".to_string(),
            captured_message_id: "101".to_string(),
            instruction: "Ajoute un tableau final concis sans retirer les sources.".to_string(),
            captured_at_unix_ms: now_unix_ms + 10,
        })
        .unwrap()
        .unwrap()
}
