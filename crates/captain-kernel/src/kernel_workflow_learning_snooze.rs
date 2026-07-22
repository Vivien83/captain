//! Durable wake-up loop for snoozed Skill Learning V2 proposals.

use std::sync::Arc;
use std::time::Duration;

use captain_memory::workflow_learning_control::WorkflowLearningStore;
use captain_runtime::workflow_learning_snooze::WorkflowSnoozeScheduler;
use tracing::{info, warn};

use super::CaptainKernel;

const SCAN_INTERVAL: Duration = Duration::from_secs(60);
const MAX_WAKE_PER_TICK: usize = 100;

pub(super) fn spawn_workflow_learning_snooze_worker(kernel: Arc<CaptainKernel>) {
    if !super::kernel_workflow_learning_worker::workflow_learning_enabled(
        kernel.config.skills.enabled,
        kernel.config.skills.mode,
    ) {
        return;
    }
    tokio::spawn(run_workflow_learning_snooze_worker(kernel));
}

async fn run_workflow_learning_snooze_worker(kernel: Arc<CaptainKernel>) {
    let scheduler = match WorkflowSnoozeScheduler::new(
        WorkflowLearningStore::new(kernel.memory.usage_conn()),
        snooze_worker_actor(std::process::id()),
    ) {
        Ok(scheduler) => scheduler,
        Err(error) => {
            warn!(error = %error, "workflow learning snooze worker cannot start");
            return;
        }
    };
    let mut interval = tokio::time::interval(SCAN_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_error = None::<String>;

    loop {
        interval.tick().await;
        if kernel.supervisor.is_shutting_down() {
            break;
        }
        let now_unix_ms = chrono::Utc::now().timestamp_millis();
        let mut woke = 0;
        let mut scan_failed = false;
        for _ in 0..MAX_WAKE_PER_TICK {
            match scheduler.wake_next_due(now_unix_ms) {
                Ok(Some(result)) => {
                    woke += 1;
                    info!(
                        proposal_id = result.proposal.id,
                        outbox_id = result.notification.id,
                        "workflow learning snooze elapsed"
                    );
                }
                Ok(None) => break,
                Err(error) => {
                    scan_failed = true;
                    let message = error.to_string();
                    if last_error.as_ref() != Some(&message) {
                        warn!(error = %message, "workflow learning snooze worker error");
                        last_error = Some(message);
                    }
                    break;
                }
            }
        }
        if !scan_failed && last_error.take().is_some() {
            info!("workflow learning snooze worker recovered");
        }
        if woke > 0 {
            info!(
                woke,
                "workflow learning proposals returned to operator review"
            );
        }
    }
}

fn snooze_worker_actor(process_id: u32) -> String {
    format!("captain:workflow-snooze:{process_id}")
}

#[cfg(test)]
mod tests {
    use super::snooze_worker_actor;

    #[test]
    fn worker_identity_is_process_scoped_and_control_plane_safe() {
        let actor = snooze_worker_actor(42);
        assert_eq!(actor, "captain:workflow-snooze:42");
        assert!(actor.len() <= 128);
        assert!(actor.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
        }));
    }
}
