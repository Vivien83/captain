//! Reconcile workflow-learning rows left in-flight by an abrupt stop.

use super::CaptainKernel;
use captain_memory::workflow_learning::WorkflowEpisodeStore;
use captain_memory::workflow_learning_control::WorkflowLearningStore;
use tracing::{info, warn};

pub(super) fn reconcile_workflow_learning(kernel: &CaptainKernel) {
    let now_unix_ms = chrono::Utc::now().timestamp_millis();
    let store = WorkflowEpisodeStore::new(kernel.memory.usage_conn());
    match store.reconcile_incomplete() {
        Ok(summary)
            if summary.episodes_reconciled > 0
                || summary.steps_interrupted > 0
                || summary.analysis_claims_released > 0 =>
        {
            info!(
                episodes = summary.episodes_reconciled,
                steps = summary.steps_interrupted,
                analysis_claims = summary.analysis_claims_released,
                "Reconciled workflow learning after interrupted Captain process"
            );
        }
        Ok(_) => {}
        Err(error) => warn!(error = %error, "Failed to reconcile workflow learning at boot"),
    }

    let control = WorkflowLearningStore::new(kernel.memory.usage_conn());
    match control.reconcile_jobs_after_restart(now_unix_ms) {
        Ok(summary) if summary.uncertain_effects > 0 => warn!(
            retried_without_effect = summary.retried_without_effect,
            uncertain_effects = summary.uncertain_effects,
            dead = summary.dead,
            "Workflow learning blocked automatic replay of effects interrupted by restart"
        ),
        Ok(summary) if summary.retried_without_effect > 0 || summary.dead > 0 => {
            info!(
                retried_without_effect = summary.retried_without_effect,
                dead = summary.dead,
                "Reconciled workflow-learning jobs owned by the previous process"
            );
        }
        Ok(_) => {}
        Err(error) => warn!(error = %error, "Failed to reconcile workflow-learning jobs at boot"),
    }
    match control.reconcile_outbox_after_restart(now_unix_ms) {
        Ok(summary) if summary.retried > 0 || summary.dead > 0 => info!(
            retried = summary.retried,
            dead = summary.dead,
            "Reconciled workflow-learning notification deliveries after restart"
        ),
        Ok(_) => {}
        Err(error) => {
            warn!(error = %error, "Failed to reconcile workflow-learning outbox at boot")
        }
    }
}
