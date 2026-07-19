//! Durable dispatch queue for exact operator-authorized CapSpec resumes.

use super::kernel_capspec_resume::KernelCapabilityResumeInvoker;
use super::CaptainKernel;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

const OPERATOR_RESUME_SWEEP_INTERVAL: Duration = Duration::from_secs(5);
const OPERATOR_RESUME_SWEEP_LIMIT: usize = 500;

/// Recover exact operator-authorized resumes without treating ordinary
/// interrupted runs as implicit work. The durable state is the source of
/// truth, so the first tick also closes the decision-to-spawn crash window.
impl CaptainKernel {
    /// Start the lifecycle-bound recovery loop. The task owns only a weak
    /// kernel reference and therefore exits with its API, Desktop, or TUI host.
    pub fn spawn_capspec_operator_resume_recovery(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let weak_kernel = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(OPERATOR_RESUME_SWEEP_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let Some(kernel) = weak_kernel.upgrade() else {
                    break;
                };
                let run_ids = match kernel
                    .capspec_executor
                    .list_operator_resume_run_ids(OPERATOR_RESUME_SWEEP_LIMIT)
                {
                    Ok(run_ids) => run_ids,
                    Err(error) => {
                        warn!(%error, "failed to scan durable CapSpec operator resumes");
                        continue;
                    }
                };
                for run_id in run_ids {
                    match kernel.capspec_executor.is_run_active(&run_id) {
                        Ok(true) => continue,
                        Ok(false) => {}
                        Err(error) => {
                            warn!(%error, %run_id, "failed to inspect active CapSpec run lease");
                            continue;
                        }
                    }
                    schedule_capspec_operator_resume(Arc::clone(&kernel), run_id, None);
                }
            }
        })
    }
}

pub(super) fn schedule_capspec_operator_resume(
    kernel: Arc<CaptainKernel>,
    run_id: String,
    prepared_invoker: Option<KernelCapabilityResumeInvoker>,
) {
    tokio::spawn(async move {
        drive_capspec_operator_resume(kernel, run_id, prepared_invoker).await;
    });
}

async fn drive_capspec_operator_resume(
    kernel: Arc<CaptainKernel>,
    run_id: String,
    prepared_invoker: Option<KernelCapabilityResumeInvoker>,
) {
    match kernel.capspec_executor.claim_operator_resume(&run_id) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            warn!(%error, %run_id, "failed to claim durable CapSpec operator resume");
            return;
        }
    }

    let invoker = match prepared_invoker {
        Some(invoker) => invoker,
        None => match prepare_invoker(&kernel, &run_id) {
            Ok(invoker) => invoker,
            Err(error) => {
                if let Err(release_error) = kernel.capspec_executor.release_operator_resume(&run_id)
                {
                    warn!(
                        error = %release_error,
                        %run_id,
                        "failed to release deferred CapSpec operator resume"
                    );
                }
                debug!(%error, %run_id, "CapSpec operator resume remains queued after preflight");
                return;
            }
        },
    };

    let result = kernel.capspec_executor.resume(&run_id, &invoker).await;
    if matches!(
        &result,
        Err(captain_capspec::ExecutorError::RunAlreadyExecuting(_))
    ) {
        return;
    }
    if let Err(error) = kernel.capspec_executor.finish_operator_resume(&run_id) {
        warn!(%error, %run_id, "failed to settle durable CapSpec operator resume");
    }
    let outcome = match &result {
        Ok(_) => "completed",
        Err(captain_capspec::ExecutorError::WaitingDecision { .. }) => "waiting-decision",
        Err(captain_capspec::ExecutorError::RunInterrupted { .. }) => "interrupted",
        Err(_) => "stopped",
    };
    kernel.record_capspec_management_audit(
        "capspec-resume",
        "operator-resume",
        &format!("run={run_id}"),
        outcome,
    );
}

fn prepare_invoker(
    kernel: &Arc<CaptainKernel>,
    run_id: &str,
) -> Result<KernelCapabilityResumeInvoker, String> {
    let context = kernel
        .capspec_executor
        .resume_context(run_id)
        .map_err(|error| error.to_string())?;
    KernelCapabilityResumeInvoker::prepare(Arc::clone(kernel), context)
}
