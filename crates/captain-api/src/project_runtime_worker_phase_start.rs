use crate::project_runtime_mutation::{project_runtime_json, update_project_runtime_state};
use crate::project_runtime_orchestrator::{
    mark_runtime_waiting, runtime_orchestrator_allows_continue,
};
use crate::project_runtime_worker_decision::{
    runtime_worker_existing_decision, RuntimeWorkerExistingDecision,
};
use crate::project_runtime_worker_recovery::{
    clear_stale_runtime_worker_agent, mark_runtime_worker_recovered,
};
use crate::project_runtime_workers::{
    mark_runtime_worker_skipped, runtime_existing_worker_status, RuntimeWorkerSpec,
    RUNTIME_WORKER_SPECS,
};
use crate::routes::AppState;
use captain_memory::project;
use std::sync::Arc;

#[allow(clippy::large_enum_variant)]
pub(crate) enum ProjectWorkerPhaseStart {
    Waiting,
    SkippedDone,
    Ready(ProjectWorkerPhaseReady),
}

pub(crate) struct ProjectWorkerPhaseReady {
    pub(crate) project: project::Project,
    pub(crate) spec: &'static RuntimeWorkerSpec,
    pub(crate) runtime_snapshot: serde_json::Value,
}

enum ExistingWorkerDisposition {
    SkipDone,
    Launch,
}

pub(crate) async fn prepare_project_worker_phase_start(
    state: &Arc<AppState>,
    project_id: &str,
    run_id: &str,
    phase: &'static str,
) -> Result<ProjectWorkerPhaseStart, String> {
    if !runtime_allows_continue(state, project_id).await? {
        update_project_runtime_state(state, project_id, |runtime, _project| {
            mark_runtime_waiting(runtime, phase, run_id);
        })
        .await?;
        return Ok(ProjectWorkerPhaseStart::Waiting);
    }

    let spec = runtime_worker_spec_for_phase(phase)?;
    let project = load_project_for_phase(state, project_id)?;
    let existing_runtime = project_runtime_json(state, &project, None);
    let existing_status = runtime_existing_worker_status(&existing_runtime, phase);
    let disposition = reconcile_existing_worker(
        state,
        &project,
        spec,
        run_id,
        phase,
        &existing_runtime,
        existing_status,
    )
    .await?;
    if matches!(disposition, ExistingWorkerDisposition::SkipDone) {
        return Ok(ProjectWorkerPhaseStart::SkippedDone);
    }

    let runtime_snapshot = project_runtime_json(state, &project, None);
    Ok(ProjectWorkerPhaseStart::Ready(ProjectWorkerPhaseReady {
        project,
        spec,
        runtime_snapshot,
    }))
}

async fn runtime_allows_continue(state: &Arc<AppState>, project_id: &str) -> Result<bool, String> {
    let project = load_project_for_phase(state, project_id)?;
    let runtime = project_runtime_json(state, &project, None);
    Ok(runtime_orchestrator_allows_continue(&runtime))
}

fn load_project_for_phase(
    state: &Arc<AppState>,
    project_id: &str,
) -> Result<project::Project, String> {
    state
        .kernel
        .memory
        .project_get(project_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| project_not_found_error(project_id))
}

fn runtime_worker_spec_for_phase(
    phase: &'static str,
) -> Result<&'static RuntimeWorkerSpec, String> {
    RUNTIME_WORKER_SPECS
        .iter()
        .find(|spec| spec.phase == phase)
        .ok_or_else(|| unknown_phase_error(phase))
}

async fn reconcile_existing_worker(
    state: &Arc<AppState>,
    project: &project::Project,
    spec: &'static RuntimeWorkerSpec,
    run_id: &str,
    phase: &'static str,
    runtime: &serde_json::Value,
    status: Option<String>,
) -> Result<ExistingWorkerDisposition, String> {
    match runtime_worker_existing_decision(runtime, status.as_deref()) {
        RuntimeWorkerExistingDecision::SkipDone => {
            update_project_runtime_state(state, &project.id, |runtime, project| {
                mark_runtime_worker_skipped(runtime, project, spec, run_id);
            })
            .await?;
            Ok(ExistingWorkerDisposition::SkipDone)
        }
        RuntimeWorkerExistingDecision::Blocked { status } => {
            Err(existing_worker_blocked_error(phase, &status))
        }
        RuntimeWorkerExistingDecision::RecoverRunning => {
            let cleared_agent_id = clear_stale_runtime_worker_agent(state, project, spec)?;
            update_project_runtime_state(state, &project.id, |runtime, project| {
                mark_runtime_worker_recovered(
                    runtime,
                    project,
                    spec,
                    run_id,
                    cleared_agent_id.as_deref(),
                );
            })
            .await?;
            Ok(ExistingWorkerDisposition::Launch)
        }
        RuntimeWorkerExistingDecision::AlreadyRunning => Err(existing_worker_running_error(phase)),
        RuntimeWorkerExistingDecision::Launch => Ok(ExistingWorkerDisposition::Launch),
    }
}

fn project_not_found_error(project_id: &str) -> String {
    format!("project '{project_id}' not found")
}

fn unknown_phase_error(phase: &str) -> String {
    format!("unknown project runtime phase: {phase}")
}

fn existing_worker_blocked_error(phase: &str, status: &str) -> String {
    format!("{phase} worker is already {status}; manual review or a fresh start is required")
}

fn existing_worker_running_error(phase: &str) -> String {
    format!("{phase} worker already running")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_worker_spec_for_phase_keeps_hermes_worker_order() {
        let observe = runtime_worker_spec_for_phase("observe").unwrap();
        let build = runtime_worker_spec_for_phase("build").unwrap();
        let learn = runtime_worker_spec_for_phase("learn").unwrap();

        assert_eq!(observe.role, "observer");
        assert_eq!(build.dependencies, &["plan"]);
        assert_eq!(learn.role, "librarian");
    }

    #[test]
    fn runtime_worker_spec_for_phase_rejects_unknown_phase_with_existing_text() {
        assert_eq!(
            runtime_worker_spec_for_phase("ship").unwrap_err(),
            "unknown project runtime phase: ship"
        );
    }

    #[test]
    fn existing_worker_errors_keep_operator_contract() {
        assert_eq!(
            existing_worker_blocked_error("build", "failed"),
            "build worker is already failed; manual review or a fresh start is required"
        );
        assert_eq!(
            existing_worker_running_error("verify"),
            "verify worker already running"
        );
        assert_eq!(
            project_not_found_error("project-1"),
            "project 'project-1' not found"
        );
    }
}
