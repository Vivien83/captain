use crate::project_runtime_mutation::{captain_agent_id, update_project_runtime_state};
use crate::project_runtime_worker_manifest::runtime_worker_manifest_for_state;
use crate::project_runtime_worker_support::{
    project_workspace_path_for_runtime, runtime_worker_prompt, update_project_task_for_phase,
};
use crate::project_runtime_workers::{mark_runtime_worker_started, RuntimeWorkerSpec};
use crate::routes::AppState;
use captain_memory::{project, project_task};
use captain_types::agent::AgentId;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

pub(crate) struct StartedProjectRuntimeWorker {
    pub(crate) agent_id: AgentId,
    pub(crate) prompt: String,
}

pub(crate) async fn start_project_runtime_worker_agent(
    state: &Arc<AppState>,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    run_id: &str,
    phase: &str,
    runtime_snapshot: &serde_json::Value,
) -> Result<StartedProjectRuntimeWorker, String> {
    let project_workspace = project_workspace_path_for_runtime(state, project).map(PathBuf::from);
    let prepared_manifest = runtime_worker_manifest_for_state(
        state,
        project,
        spec,
        runtime_snapshot,
        project_workspace,
    );
    let parent = runtime_worker_parent_agent_id(captain_agent_id(state));
    let agent_id = state
        .kernel
        .spawn_agent_with_parent(prepared_manifest.manifest, parent, None)
        .map_err(|error| spawn_worker_error(phase, &error.to_string()))?;
    update_project_task_for_phase(
        state,
        &project.id,
        phase,
        project_task::TaskStatus::Doing,
        Some(agent_id.to_string()),
    );
    update_project_runtime_state(state, &project.id, |runtime, project| {
        mark_runtime_worker_started(
            runtime,
            project,
            spec,
            run_id,
            &agent_id.to_string(),
            &prepared_manifest.authorized_tools,
        );
    })
    .await?;

    Ok(StartedProjectRuntimeWorker {
        agent_id,
        prompt: runtime_worker_prompt(state, project, spec, runtime_snapshot),
    })
}

fn runtime_worker_parent_agent_id(captain_id: Option<String>) -> Option<AgentId> {
    captain_id.and_then(|id| AgentId::from_str(&id).ok())
}

fn spawn_worker_error(phase: &str, error: &str) -> String {
    format!("spawn {phase} worker failed: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_worker_parent_agent_id_ignores_invalid_manager_id() {
        assert!(runtime_worker_parent_agent_id(Some("not-a-uuid".to_string())).is_none());

        let parent = runtime_worker_parent_agent_id(Some(
            "00000000-0000-0000-0000-000000000001".to_string(),
        ));
        assert_eq!(
            parent.map(|agent_id| agent_id.to_string()).as_deref(),
            Some("00000000-0000-0000-0000-000000000001")
        );
    }

    #[test]
    fn spawn_worker_error_keeps_existing_operator_text() {
        assert_eq!(
            spawn_worker_error("build", "model unavailable"),
            "spawn build worker failed: model unavailable"
        );
    }
}
