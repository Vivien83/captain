use crate::project_runtime_checkpoints::{append_runtime_phase_checkpoint, trim_runtime_text};
use crate::project_runtime_defaults::project_session_id;
use crate::project_runtime_mutation::{project_runtime_json, update_project_runtime_state};
use crate::project_runtime_worker_cleanup::{
    mark_runtime_worker_cleaned, runtime_worker_cleanup_failure, runtime_worker_cleanup_success,
    RuntimeWorkerCleanup,
};
use crate::project_runtime_worker_failure::mark_runtime_worker_failed;
use crate::project_runtime_worker_result::{
    mark_runtime_worker_turn_result, runtime_worker_turn_outcome, RuntimeWorkerTurnOutcome,
};
use crate::project_runtime_worker_support::update_project_task_for_phase;
use crate::project_runtime_workers::RuntimeWorkerSpec;
use crate::routes::AppState;
use captain_memory::{project, project_task};
use captain_runtime::agent_loop::AgentLoopResult;
use captain_types::agent::AgentId;
use chrono::Utc;
use std::sync::Arc;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn finish_successful_project_worker_phase(
    state: &Arc<AppState>,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    run_id: &str,
    phase: &str,
    agent_id: AgentId,
    result: AgentLoopResult,
    runtime_snapshot: &serde_json::Value,
) -> Result<bool, String> {
    let (updated, outcome) = record_successful_worker_turn(
        state,
        project,
        spec,
        run_id,
        phase,
        agent_id,
        result,
        runtime_snapshot,
    )
    .await?;

    if outcome.blocked {
        append_worker_phase_checkpoint(state, &updated, run_id, phase, "blocked");
        return Err(worker_blocker_error(phase, &outcome.summary));
    }

    let cleanup = stop_completed_project_worker(state, project, phase, agent_id);
    let updated = record_worker_cleanup(state, project, spec, run_id, agent_id, cleanup).await?;
    append_worker_phase_checkpoint(state, &updated, run_id, phase, "done");
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
async fn record_successful_worker_turn(
    state: &Arc<AppState>,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    run_id: &str,
    phase: &str,
    agent_id: AgentId,
    result: AgentLoopResult,
    runtime_snapshot: &serde_json::Value,
) -> Result<(project::Project, RuntimeWorkerTurnOutcome), String> {
    let outcome = runtime_worker_turn_outcome(spec, &result, runtime_snapshot);
    update_project_task_for_phase(
        state,
        &project.id,
        phase,
        task_status_for_worker_outcome(outcome.blocked),
        Some(agent_id.to_string()),
    );
    let updated = update_project_runtime_state(state, &project.id, |runtime, project| {
        mark_runtime_worker_turn_result(
            runtime,
            project,
            spec,
            run_id,
            &agent_id.to_string(),
            &result,
            &outcome,
        );
    })
    .await?;

    Ok((updated, outcome))
}

fn append_worker_phase_checkpoint(
    state: &Arc<AppState>,
    project: &project::Project,
    run_id: &str,
    phase: &str,
    status: &str,
) {
    let runtime = project_runtime_json(state, project, None);
    append_runtime_phase_checkpoint(
        state,
        project,
        &runtime,
        run_id,
        phase,
        status,
        project_session_id(project),
    );
}

fn stop_completed_project_worker(
    state: &Arc<AppState>,
    project: &project::Project,
    phase: &str,
    agent_id: AgentId,
) -> RuntimeWorkerCleanup {
    let stopped_at = Utc::now().to_rfc3339();
    match state.kernel.kill_agent(agent_id) {
        Ok(()) => cleanup_from_stop_result(stopped_at, Ok(())),
        Err(error) => {
            let error = error.to_string();
            tracing::warn!(
                agent_id = %agent_id,
                phase = phase,
                project_id = %project.id,
                "failed to stop completed project runtime worker: {error}"
            );
            cleanup_from_stop_result(stopped_at, Err(error))
        }
    }
}

fn cleanup_from_stop_result(
    stopped_at: String,
    stop_result: Result<(), String>,
) -> RuntimeWorkerCleanup {
    match stop_result {
        Ok(()) => runtime_worker_cleanup_success(stopped_at),
        Err(error) => runtime_worker_cleanup_failure(stopped_at, error),
    }
}

async fn record_worker_cleanup(
    state: &Arc<AppState>,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    run_id: &str,
    agent_id: AgentId,
    cleanup: RuntimeWorkerCleanup,
) -> Result<project::Project, String> {
    let updated = update_project_runtime_state(state, &project.id, |runtime, project| {
        mark_runtime_worker_cleaned(
            runtime,
            project,
            spec,
            run_id,
            &agent_id.to_string(),
            &cleanup,
        );
    })
    .await?;

    Ok(updated)
}

pub(crate) async fn finish_failed_project_worker_phase(
    state: &Arc<AppState>,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    run_id: &str,
    phase: &str,
    agent_id: AgentId,
    error: &str,
) -> Result<bool, String> {
    let error = trim_runtime_text(error, 1200);
    update_project_task_for_phase(
        state,
        &project.id,
        phase,
        project_task::TaskStatus::Blocked,
        Some(agent_id.to_string()),
    );
    let updated = update_project_runtime_state(state, &project.id, |runtime, project| {
        mark_runtime_worker_failed(
            runtime,
            project,
            spec,
            run_id,
            &agent_id.to_string(),
            &error,
        );
    })
    .await?;
    let runtime = project_runtime_json(state, &updated, None);
    append_runtime_phase_checkpoint(
        state,
        &updated,
        &runtime,
        run_id,
        phase,
        "failed",
        project_session_id(&updated),
    );
    Err(worker_failed_error(phase, &error))
}

fn task_status_for_worker_outcome(blocked: bool) -> project_task::TaskStatus {
    if blocked {
        project_task::TaskStatus::Blocked
    } else {
        project_task::TaskStatus::Done
    }
}

fn worker_blocker_error(phase: &str, summary: &str) -> String {
    format!("{phase} worker reported a blocker: {summary}")
}

fn worker_failed_error(phase: &str, error: &str) -> String {
    format!("{phase} worker failed: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_status_for_worker_outcome_maps_blocked_and_done() {
        assert_eq!(
            task_status_for_worker_outcome(true),
            project_task::TaskStatus::Blocked
        );
        assert_eq!(
            task_status_for_worker_outcome(false),
            project_task::TaskStatus::Done
        );
    }

    #[test]
    fn worker_phase_errors_keep_existing_operator_text() {
        assert_eq!(
            worker_blocker_error("build", "missing deploy key"),
            "build worker reported a blocker: missing deploy key"
        );
        assert_eq!(
            worker_failed_error("verify", "cargo test failed"),
            "verify worker failed: cargo test failed"
        );
    }

    #[test]
    fn cleanup_from_stop_result_preserves_cleanup_contract() {
        let stopped = cleanup_from_stop_result("2026-05-25T00:00:00Z".to_string(), Ok(()));
        assert_eq!(stopped.status, "stopped");
        assert_eq!(stopped.stopped_at, "2026-05-25T00:00:00Z");
        assert!(stopped.error.is_none());

        let failed = cleanup_from_stop_result(
            "2026-05-25T00:01:00Z".to_string(),
            Err("agent still running".to_string()),
        );
        assert_eq!(failed.status, "cleanup_failed");
        assert_eq!(failed.stopped_at, "2026-05-25T00:01:00Z");
        assert_eq!(failed.error.as_deref(), Some("agent still running"));
    }
}
