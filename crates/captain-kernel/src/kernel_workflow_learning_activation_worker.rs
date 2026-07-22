//! Lease supervisor for the model-independent workflow activation lifecycle.

use std::sync::Arc;
use std::time::Duration;

use captain_memory::workflow_learning_control::WorkflowLearningStore;
use captain_runtime::workflow_learning_promotion::WorkflowPromotionRoot;
use captain_runtime::workflow_learning_staging::WorkflowStagingRoot;
use chrono::Utc;
use tracing::{info, warn};

use super::kernel_workflow_learning_activation::{
    bounded_error, retry_transient_activation_failure, settle_activation_failure,
    WorkflowActivationExecutor,
};
use super::CaptainKernel;

const WORKER_PREFIX: &str = "captain:workflow-activation-worker";
const LEASE_MS: i64 = 120_000;
const IDLE_DELAY: Duration = Duration::from_secs(2);
const ACTIVE_DELAY: Duration = Duration::from_millis(25);
const ERROR_DELAY: Duration = Duration::from_secs(10);

pub(super) fn spawn_workflow_learning_activation_worker(kernel: Arc<CaptainKernel>) {
    if !super::kernel_workflow_learning_worker::workflow_learning_enabled(
        kernel.config.skills.enabled,
        kernel.config.skills.mode,
    ) {
        return;
    }
    tokio::spawn(run_workflow_learning_activation_worker(kernel));
}

async fn run_workflow_learning_activation_worker(kernel: Arc<CaptainKernel>) {
    let control = WorkflowLearningStore::new(kernel.memory.usage_conn());
    let staging = match WorkflowStagingRoot::new(kernel.config.home_dir.clone()) {
        Ok(staging) => staging,
        Err(error) => {
            warn!(error = %error, "workflow activation staging is unavailable");
            return;
        }
    };
    let promotions = match WorkflowPromotionRoot::new(kernel.config.home_dir.clone()) {
        Ok(promotions) => promotions,
        Err(error) => {
            warn!(error = %error, "workflow promotion journal is unavailable");
            return;
        }
    };
    let executor = WorkflowActivationExecutor::new(
        control.clone(),
        staging,
        promotions,
        &kernel.skill_registry,
        &kernel.capspec_registry,
        &kernel.cron_scheduler,
    );
    let worker = format!("{WORKER_PREFIX}:{}", std::process::id());
    let mut last_error = None::<String>;

    loop {
        if kernel.supervisor.is_shutting_down() {
            break;
        }
        let now_unix_ms = Utc::now().timestamp_millis();
        let claimed = control
            .claim_uncertain_activation_job(&worker, now_unix_ms, LEASE_MS)
            .and_then(|job| match job {
                Some(job) => Ok(Some(job)),
                None => control.claim_due_activation_job(&worker, now_unix_ms, LEASE_MS),
            });
        let delay = match claimed {
            Ok(Some(job)) => match executor.execute(&worker, &job, now_unix_ms) {
                Ok(state) => {
                    if last_error.take().is_some() {
                        info!("workflow activation worker recovered");
                    }
                    info!(
                        job_id = job.id,
                        proposal_id = job.proposal_id,
                        kind = job.kind.as_str(),
                        state = state.as_str(),
                        "workflow activation lifecycle committed"
                    );
                    ACTIVE_DELAY
                }
                Err(error) => {
                    let message = bounded_error(&error.to_string());
                    let settled = retry_transient_activation_failure(
                        &control,
                        &worker,
                        &job,
                        error.as_ref(),
                        &message,
                        now_unix_ms,
                    )
                    .and_then(|retry| match retry {
                        Some(status) => Ok(status),
                        None => settle_activation_failure(
                            &control,
                            &worker,
                            &job,
                            &message,
                            now_unix_ms,
                        ),
                    });
                    if last_error.as_deref() != Some(message.as_str()) {
                        warn!(
                            job_id = job.id,
                            proposal_id = job.proposal_id,
                            kind = job.kind.as_str(),
                            error = %message,
                            settle_error = settled.as_ref().err().map(ToString::to_string),
                            "workflow activation job failed"
                        );
                        last_error = Some(message);
                    }
                    ERROR_DELAY
                }
            },
            Ok(None) => IDLE_DELAY,
            Err(error) => {
                let message = error.to_string();
                if last_error.as_deref() != Some(message.as_str()) {
                    warn!(error = %message, "workflow activation claim failed");
                    last_error = Some(message);
                }
                ERROR_DELAY
            }
        };
        tokio::time::sleep(delay).await;
    }
}
