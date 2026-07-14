use crate::project_metadata::{project_source_from_metadata, project_workspace_from_metadata};
use crate::project_runtime_prompt_context::{
    format_project_goals_for_prompt, runtime_worker_prompt_for_project,
};
use crate::project_runtime_workers::RuntimeWorkerSpec;
use crate::project_workspace::default_project_path;
use crate::routes::AppState;
use captain_memory::{project, project_task};

pub(crate) fn runtime_worker_prompt(
    state: &AppState,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    runtime: &serde_json::Value,
) -> String {
    let workspace_path = project_workspace_path_for_runtime(state, project).unwrap_or_else(|| {
        default_project_path(state, &project.slug)
            .display()
            .to_string()
    });
    let project_goals = runtime_project_goals_context(state, project);
    runtime_worker_prompt_for_project(project, spec, runtime, &workspace_path, &project_goals)
}

fn runtime_project_goals_context(state: &AppState, project: &project::Project) -> String {
    format_project_goals_for_prompt(
        &state
            .kernel
            .goal_store
            .list_for_project(&project.id, &project.slug),
    )
}

pub(crate) fn project_workspace_path_for_runtime(
    state: &AppState,
    project: &project::Project,
) -> Option<String> {
    let workspace = project_workspace_from_metadata(&project.metadata);
    let source = project_source_from_metadata(&project.metadata);
    workspace
        .get("path")
        .and_then(|v| v.as_str())
        .or_else(|| source.get("path").and_then(|v| v.as_str()))
        .or_else(|| source.get("local_path").and_then(|v| v.as_str()))
        .map(str::to_string)
        .or_else(|| {
            let default = default_project_path(state, &project.slug);
            default.exists().then(|| default.display().to_string())
        })
}

pub(crate) fn update_project_task_for_phase(
    state: &AppState,
    project_id: &str,
    phase: &str,
    status: project_task::TaskStatus,
    assignee_agent_id: Option<String>,
) {
    let prefix = format!("{}:", phase.to_ascii_uppercase());
    let Ok(tasks) = state.kernel.memory.task_list_for_project(project_id) else {
        return;
    };
    let Some(task) = tasks
        .into_iter()
        .find(|task| task.title.to_ascii_uppercase().starts_with(&prefix))
    else {
        return;
    };
    let _ = state.kernel.memory.task_update(
        &task.id,
        project_task::TaskPatch {
            status: Some(status),
            assignee_agent_id: Some(assignee_agent_id),
            ..Default::default()
        },
    );
}

#[cfg(test)]
#[path = "project_runtime_worker_support_tests.rs"]
mod project_runtime_worker_support_tests;
