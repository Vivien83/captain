//! Model-independent worker for isolated Skill Learning V2 tests.

use std::sync::Arc;
use std::time::Duration;

use captain_memory::workflow_learning_control::{
    WorkflowLearningStore, WorkflowProposalState, WorkflowProposalTransition,
};
use captain_memory::workflow_learning_outbox::NewWorkflowOutboxItem;
use captain_memory::workflow_learning_queue::{WorkflowJobRecord, WorkflowJobStatus};
use captain_memory::workflow_learning_test::{
    WorkflowIsolatedTestCompletion, WorkflowIsolatedTestStatus,
};
use captain_runtime::workflow_learning_delivery::WORKFLOW_LIFECYCLE_OUTBOX_TOPIC;
use captain_runtime::workflow_learning_isolated_test::WorkflowIsolatedTestRunner;
use captain_runtime::workflow_learning_operator::WorkflowInstallRequestPayload;
use captain_runtime::workflow_learning_staging::WorkflowStagingRoot;
use captain_types::workflow_learning::ProposalInstallMode;
use tracing::{info, warn};

use super::CaptainKernel;

const WORKER_PREFIX: &str = "captain:workflow-isolated-test";
const LEASE_MS: i64 = 120_000;
const IDLE_DELAY: Duration = Duration::from_secs(2);
const ACTIVE_DELAY: Duration = Duration::from_millis(25);
const ERROR_DELAY: Duration = Duration::from_secs(10);

pub(super) fn spawn_workflow_learning_test_worker(kernel: Arc<CaptainKernel>) {
    if !super::kernel_workflow_learning_worker::workflow_learning_enabled(
        kernel.config.skills.enabled,
        kernel.config.skills.mode,
    ) {
        return;
    }
    tokio::spawn(run_workflow_learning_test_worker(kernel));
}

async fn run_workflow_learning_test_worker(kernel: Arc<CaptainKernel>) {
    let control = WorkflowLearningStore::new(kernel.memory.usage_conn());
    let staging = match WorkflowStagingRoot::new(kernel.config.home_dir.clone()) {
        Ok(staging) => staging,
        Err(error) => {
            warn!(error = %error, "workflow isolated-test staging is unavailable");
            return;
        }
    };
    let runner = WorkflowIsolatedTestRunner::new(staging);
    let worker = format!("{WORKER_PREFIX}:{}", std::process::id());
    let mut last_error = None::<String>;

    loop {
        if kernel.supervisor.is_shutting_down() {
            break;
        }
        let now_unix_ms = chrono::Utc::now().timestamp_millis();
        let delay = match control.claim_due_isolated_test_job(&worker, now_unix_ms, LEASE_MS) {
            Ok(Some(job)) => {
                match execute_claimed_test(&control, &runner, &worker, &job, now_unix_ms) {
                    Ok(passed) => {
                        if last_error.take().is_some() {
                            info!("workflow isolated-test worker recovered");
                        }
                        info!(
                            job_id = job.id,
                            proposal_id = job.proposal_id,
                            passed,
                            "workflow isolated test committed"
                        );
                        ACTIVE_DELAY
                    }
                    Err(error) => {
                        let message = error.to_string();
                        let retry_at =
                            now_unix_ms.saturating_add(retry_backoff_ms(job.attempt_count));
                        match control.fail_job(
                            &job.id,
                            &worker,
                            "isolated_test_execution",
                            &bounded_error(&message),
                            true,
                            retry_at,
                            now_unix_ms,
                        ) {
                            Ok(failed) => {
                                if last_error.as_ref() != Some(&message) {
                                    warn!(
                                        job_id = failed.id,
                                        status = failed.status.as_str(),
                                        error = %message,
                                        "workflow isolated test could not commit"
                                    );
                                    last_error = Some(message);
                                }
                                if failed.status == WorkflowJobStatus::Dead {
                                    ERROR_DELAY
                                } else {
                                    IDLE_DELAY
                                }
                            }
                            Err(settle_error) => {
                                warn!(
                                    job_id = job.id,
                                    error = %message,
                                    settle_error = %settle_error,
                                    "workflow isolated-test failure could not be settled"
                                );
                                ERROR_DELAY
                            }
                        }
                    }
                }
            }
            Ok(None) => IDLE_DELAY,
            Err(error) => {
                let message = error.to_string();
                if last_error.as_ref() != Some(&message) {
                    warn!(error = %message, "workflow isolated-test claim failed");
                    last_error = Some(message);
                }
                ERROR_DELAY
            }
        };
        tokio::time::sleep(delay).await;
    }
}

fn execute_claimed_test(
    control: &WorkflowLearningStore,
    runner: &WorkflowIsolatedTestRunner,
    worker: &str,
    job: &WorkflowJobRecord,
    completed_at_unix_ms: i64,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let payload: WorkflowInstallRequestPayload = serde_json::from_str(&job.payload_json)?;
    if payload.requested_mode != ProposalInstallMode::Test
        || payload.proposal_id != job.proposal_id
        || job.revision_sha256.as_deref() != Some(payload.revision_sha256.as_str())
    {
        return Err(invalid_data("claimed job is not an exact isolated-test request").into());
    }
    let proposal = control
        .get(&job.proposal_id)?
        .ok_or_else(|| invalid_data("isolated-test proposal no longer exists"))?;
    let test = proposal
        .isolated_test
        .as_ref()
        .ok_or_else(|| invalid_data("isolated-test record is missing"))?;
    if proposal.state != WorkflowProposalState::ApprovedPendingInstall
        || proposal.revision_sha256.as_deref() != Some(payload.revision_sha256.as_str())
        || test.job_id != job.id
        || test.status != WorkflowIsolatedTestStatus::Queued
    {
        return Err(invalid_data("isolated-test durable identities disagree").into());
    }

    let report = runner.run(&proposal, completed_at_unix_ms)?;
    let result_json = serde_json::to_string(&report)?;
    let transition = WorkflowProposalTransition {
        proposal_id: proposal.id.clone(),
        expected_state: WorkflowProposalState::ApprovedPendingInstall,
        expected_version: proposal.state_version,
        expected_revision_sha256: Some(payload.revision_sha256.clone()),
        to_state: WorkflowProposalState::Proposed,
        actor: worker.to_string(),
        reason: if report.passed {
            "isolated native test passed"
        } else {
            "isolated native test failed"
        }
        .to_string(),
        idempotency_key: format!("{}:complete", job.id),
        snoozed_until_unix_ms: None,
        occurred_at_unix_ms: completed_at_unix_ms,
    };
    let notification = NewWorkflowOutboxItem {
        id: format!("{}:notice", job.id),
        idempotency_key: format!("{}:notice", job.id),
        proposal_id: proposal.id,
        revision_sha256: Some(payload.revision_sha256.clone()),
        topic: WORKFLOW_LIFECYCLE_OUTBOX_TOPIC.to_string(),
        payload_json: serde_json::to_string(&serde_json::json!({
            "schema_version": 1,
            "event": "isolated_test_completed",
            "proposal_id": payload.proposal_id,
            "revision_sha256": payload.revision_sha256,
            "test_job_id": job.id,
            "state": "proposed",
            "passed": report.passed,
        }))?,
        max_attempts: 8,
        run_after_unix_ms: completed_at_unix_ms,
        created_at_unix_ms: completed_at_unix_ms,
    };
    control.complete_isolated_test(&WorkflowIsolatedTestCompletion {
        job_id: job.id.clone(),
        worker: worker.to_string(),
        passed: report.passed,
        result_json,
        proposal_transition: transition,
        notification: Some(notification),
        completed_at_unix_ms,
    })?;
    Ok(report.passed)
}

fn retry_backoff_ms(attempt_count: u32) -> i64 {
    let exponent = attempt_count.saturating_sub(1).min(6);
    (5_000_i64.saturating_mul(1_i64 << exponent)).min(5 * 60 * 1_000)
}

fn bounded_error(error: &str) -> String {
    let bounded = captain_types::truncate_str(error.trim(), 2_048).trim();
    if bounded.is_empty() {
        "isolated test failed".to_string()
    } else {
        bounded.to_string()
    }
}

fn invalid_data(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use captain_memory::workflow_learning_control::{
        NewWorkflowProposal, PublishValidatedDraft, WorkflowArtifactKind, WorkflowLearningStore,
        WorkflowProposalState, WorkflowProposalTransition,
    };
    use captain_memory::workflow_learning_test::WorkflowIsolatedTestStatus;
    use captain_memory::MemorySubstrate;
    use captain_runtime::workflow_learning_analysis::{
        CanonicalWorkflow, CanonicalWorkflowNode, WorkflowClassification, WorkflowGroupAnalysis,
        WorkflowScope,
    };
    use captain_runtime::workflow_learning_isolated_test::WorkflowIsolatedTestRunner;
    use captain_runtime::workflow_learning_operator::WorkflowLearningOperator;
    use captain_runtime::workflow_learning_proposer::{
        ActiveModelIdentity, WorkflowDraft, WorkflowDraftArtifact, WorkflowDraftKind,
    };
    use captain_runtime::workflow_learning_staging::{
        StageWorkflowDraftRequest, WorkflowStagingRoot,
    };
    use captain_types::workflow_learning::ProposalCardAction;
    use serde_json::json;

    use super::{bounded_error, execute_claimed_test, retry_backoff_ms};

    #[test]
    fn isolated_test_retries_are_bounded_and_errors_are_safe() {
        assert_eq!(retry_backoff_ms(1), 5_000);
        assert_eq!(retry_backoff_ms(20), 300_000);
        assert_eq!(bounded_error("  "), "isolated test failed");
        assert_eq!(bounded_error(&"x".repeat(3_000)).len(), 2_048);
    }

    #[test]
    fn claimed_test_runs_native_registry_and_never_writes_active_state() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let store = WorkflowLearningStore::new(memory.usage_conn());
        let temp = tempfile::tempdir().unwrap();
        let captain_home = temp.path().join("captain-home");
        let staging = WorkflowStagingRoot::new(captain_home.clone()).unwrap();
        let signature = "b".repeat(64);
        let draft = WorkflowDraft {
            schema_version: 1,
            kind: WorkflowDraftKind::Skill,
            name: "private-native-test".to_string(),
            purpose: "Prove isolated native loading.".to_string(),
            trigger: "Use only in this test.".to_string(),
            artifact: WorkflowDraftArtifact::SkillMarkdown {
                source: "---\nname: private-native-test\ndescription: Private native test\n---\n# Workflow\nRun the exact private test.".to_string(),
            },
            required_capabilities: vec!["file_write".to_string()],
            expected_benefit: "No active-state mutation.".to_string(),
            limitations: vec!["Test fixture only.".to_string()],
        };
        let model = ActiveModelIdentity {
            provider: "codex".to_string(),
            model: "gpt-5.6-sol".to_string(),
        };
        let staged = staging
            .stage(StageWorkflowDraftRequest {
                job_id: "worker-test-draft",
                workflow_signature: &signature,
                draft: &draft,
                active_model: &model,
            })
            .unwrap();
        let evidence = WorkflowGroupAnalysis {
            signature: signature.clone(),
            classification: WorkflowClassification::Skill,
            eligible: true,
            reasons: vec![],
            occurrence_count: 3,
            distinct_turn_count: 3,
            distinct_session_count: 2,
            explicit_reuse_request: false,
            scope: WorkflowScope::Global,
            episode_ids: vec!["episode-1".into(), "episode-2".into(), "episode-3".into()],
            intent_samples: vec!["test this privately".into()],
            canonical: CanonicalWorkflow {
                version: 1,
                nodes: vec![CanonicalWorkflowNode {
                    index: 0,
                    tool_name: "file_write".to_string(),
                    role: "write".to_string(),
                    input_shape: json!({"path":"<private>"}),
                    effect_class: "write".to_string(),
                    verification_shape: "native_registry".to_string(),
                    dependencies: vec![],
                }],
            },
        };
        store
            .create_observed(&NewWorkflowProposal {
                id: "worker-test-proposal".to_string(),
                idempotency_key: "worker-test-proposal:observed".to_string(),
                workflow_signature: signature,
                source_agent_id: "captain".to_string(),
                origin_channel: Some("telegram".to_string()),
                evidence_json: serde_json::to_string(&evidence).unwrap(),
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
                .transition(&WorkflowProposalTransition {
                    proposal_id: "worker-test-proposal".to_string(),
                    expected_state: from,
                    expected_version: version,
                    expected_revision_sha256: None,
                    to_state: to,
                    actor: "test".to_string(),
                    reason: suffix.to_string(),
                    idempotency_key: format!("worker-test-proposal:{suffix}"),
                    snoozed_until_unix_ms: None,
                    occurred_at_unix_ms: 200 + version as i64,
                })
                .unwrap();
        }
        store
            .publish_validated_draft(&PublishValidatedDraft {
                proposal_id: "worker-test-proposal".to_string(),
                expected_version: 3,
                staging_job_id: "worker-test-draft".to_string(),
                revision_sha256: staged.revision_sha256,
                artifact_sha256: staged.artifact_sha256,
                kind: WorkflowArtifactKind::Skill,
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
                    "model": model,
                    "limitations": draft.limitations,
                })
                .to_string(),
                actor: "test".to_string(),
                reason: "validated".to_string(),
                idempotency_key: "worker-test-proposal:published".to_string(),
                occurred_at_unix_ms: 300,
            })
            .unwrap();
        let proposed = store.get("worker-test-proposal").unwrap().unwrap();
        let token = proposed.operator_token.clone().unwrap();
        WorkflowLearningOperator::new(store.clone(), staging.clone())
            .resolve_at_version(
                &token,
                proposed.state_version,
                ProposalCardAction::Test,
                "telegram:42",
                400,
            )
            .unwrap();
        let job = store
            .claim_due_isolated_test_job("isolated-worker", 400, 30_000)
            .unwrap()
            .unwrap();

        assert!(execute_claimed_test(
            &store,
            &WorkflowIsolatedTestRunner::new(staging),
            "isolated-worker",
            &job,
            500,
        )
        .unwrap());

        let completed = store.get("worker-test-proposal").unwrap().unwrap();
        assert_eq!(completed.state, WorkflowProposalState::Proposed);
        assert_eq!(
            completed.isolated_test.unwrap().status,
            WorkflowIsolatedTestStatus::Passed
        );
        assert!(store
            .get_outbox(&format!("{}:notice", job.id))
            .unwrap()
            .is_some());
        assert!(!captain_home
            .join("skills/learned/private-native-test.md")
            .exists());
    }
}
