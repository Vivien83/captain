use crate::project_launch_input::{LaunchProjectReq, NormalizedProjectLaunch};
use crate::project_launch_project::{
    create_active_launch_project, ProjectLaunchProjectError, ProjectLaunchProjectInput,
};
use crate::project_launch_records::{
    create_project_launch_records, ProjectLaunchRecords, ProjectLaunchRecordsError,
    ProjectLaunchRecordsInput,
};
use crate::project_launch_state::{project_launch_state, ProjectLaunchStateInput};
use crate::project_runtime_mutation::captain_manager_json;
use crate::project_workspace::{prepare_project_workspace, project_workspace_root};
use crate::routes::AppState;
use captain_memory::project;
use serde_json::Value;
use std::sync::Arc;

pub(crate) struct ProjectLaunchFlow {
    pub(crate) project: project::Project,
    pub(crate) records: ProjectLaunchRecords,
    pub(crate) launch_state: Value,
    pub(crate) rules_file_created: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProjectLaunchFlowError {
    Workspace(String),
    Conflict,
    BadRequest(String),
    Storage(String),
}

pub(crate) async fn prepare_project_launch_flow(
    state: &Arc<AppState>,
    req: &LaunchProjectReq,
    launch: &NormalizedProjectLaunch,
) -> Result<ProjectLaunchFlow, ProjectLaunchFlowError> {
    let launch_artifacts = prepare_project_launch_artifacts(state, req, launch).await?;
    let project = create_active_launch_project(
        state,
        ProjectLaunchProjectInput {
            name: &launch.name,
            slug: &launch.slug,
            goal: &launch.goal,
            deadline: req.deadline,
            launch_state: launch_artifacts.launch_state.clone(),
            manager: captain_manager_json(state),
        },
    )
    .map_err(project_launch_project_error)?;

    let records = create_project_launch_records(
        state,
        ProjectLaunchRecordsInput {
            project: &project,
            goal: &launch.goal,
            criteria: &launch.criteria,
            goal_guard: &launch.goal_guard,
            deadline: req.deadline,
            launch_state: &launch_artifacts.launch_state,
        },
    )
    .map_err(project_launch_records_error)?;

    Ok(ProjectLaunchFlow {
        project,
        records,
        launch_state: launch_artifacts.launch_state,
        rules_file_created: launch_artifacts.rules_file_created,
    })
}

struct ProjectLaunchArtifacts {
    launch_state: Value,
    rules_file_created: bool,
}

async fn prepare_project_launch_artifacts(
    state: &Arc<AppState>,
    req: &LaunchProjectReq,
    launch: &NormalizedProjectLaunch,
) -> Result<ProjectLaunchArtifacts, ProjectLaunchFlowError> {
    let create_worktree = req.create_worktree.unwrap_or(true);
    let workspace = prepare_project_workspace(state, req, &launch.slug)
        .await
        .map_err(ProjectLaunchFlowError::Workspace)?;
    let rules_file = workspace.workspace_path.as_deref().map(|path| {
        captain_runtime::project_rules::seed_captain_project_rules_file(
            path,
            &launch.name,
            &launch.slug,
            &launch.goal,
        )
    });
    let default_root = project_workspace_root(state);
    let launch_state = project_launch_state(ProjectLaunchStateInput {
        repo_path: workspace.repo_path.as_deref(),
        branch: workspace.branch.as_deref(),
        source: &workspace.source,
        workspace_path: workspace.workspace_path.as_deref(),
        default_root: &default_root,
        authorized: workspace.authorized,
        authorization_error: workspace.authorization_error.as_deref(),
        rules_file: rules_file.as_ref(),
        create_worktree,
        autonomy_level: &launch.autonomy_level,
        acceptance_criteria: &launch.criteria,
    });

    Ok(ProjectLaunchArtifacts {
        launch_state,
        rules_file_created: rules_file.is_some(),
    })
}

fn project_launch_project_error(error: ProjectLaunchProjectError) -> ProjectLaunchFlowError {
    match error {
        ProjectLaunchProjectError::Conflict => ProjectLaunchFlowError::Conflict,
        ProjectLaunchProjectError::Storage(error) => ProjectLaunchFlowError::Storage(error),
    }
}

fn project_launch_records_error(error: ProjectLaunchRecordsError) -> ProjectLaunchFlowError {
    match error {
        ProjectLaunchRecordsError::BadRequest(error) => ProjectLaunchFlowError::BadRequest(error),
        ProjectLaunchRecordsError::Storage(error) => ProjectLaunchFlowError::Storage(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_launch_input::normalize_project_launch_request;
    use captain_kernel::CaptainKernel;
    use captain_types::config::{DefaultModelConfig, KernelConfig};
    use std::time::Instant;

    fn test_state() -> (tempfile::TempDir, Arc<AppState>) {
        let tmp = tempfile::tempdir().unwrap();
        let config = KernelConfig {
            home_dir: tmp.path().to_path_buf(),
            data_dir: tmp.path().join("data"),
            default_model: DefaultModelConfig {
                provider: "ollama".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OLLAMA_API_KEY".to_string(),
                base_url: None,
            },
            ..KernelConfig::default()
        };
        let kernel = Arc::new(CaptainKernel::boot_with_config(config).unwrap());
        kernel.set_self_handle();
        let state = AppState {
            kernel,
            started_at: Instant::now(),
            peer_registry: None,
            bridge_manager: tokio::sync::Mutex::new(None),
            channels_config: tokio::sync::RwLock::new(Default::default()),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            clawhub_cache: dashmap::DashMap::new(),
            ask_user_channels: dashmap::DashMap::new(),
            provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
        };
        (tmp, Arc::new(state))
    }

    fn launch_req(path: std::path::PathBuf) -> LaunchProjectReq {
        LaunchProjectReq {
            name: Some(" Demo ".to_string()),
            slug: Some(" demo ".to_string()),
            goal: " Ship safely ".to_string(),
            repo_path: None,
            local_path: Some(path.display().to_string()),
            source_type: Some(" local ".to_string()),
            github_full_name: None,
            github_clone_url: None,
            github_branch: None,
            github_repo_id: None,
            branch: Some(" main ".to_string()),
            create_worktree: None,
            create_folder: Some(true),
            autonomy_level: Some(" supervised ".to_string()),
            acceptance_criteria: vec![" Smoke passes ".to_string()],
            deadline: Some(123),
            goal_check_command: None,
            goal_recovery_command: None,
            goal_interval_secs: None,
        }
    }

    #[tokio::test]
    async fn prepare_project_launch_flow_persists_project_and_backbone() {
        let (tmp, state) = test_state();
        let mut req = launch_req(tmp.path().join("workspace"));
        let launch = normalize_project_launch_request(&mut req).unwrap();

        let flow = prepare_project_launch_flow(&state, &req, &launch)
            .await
            .unwrap();

        assert_eq!(flow.project.name, "Demo");
        assert_eq!(flow.project.slug, "demo");
        assert_eq!(flow.project.status, project::ProjectStatus::Active);
        assert_eq!(flow.project.deadline, Some(123));
        assert_eq!(flow.records.tasks.len(), 7);
        assert_eq!(flow.records.goals.len(), 0);
        assert_eq!(flow.records.milestone.name, "Verified first delivery");
        assert_eq!(flow.launch_state["protocol"], "captain.project_launch.v1");
        assert_eq!(flow.launch_state["source"]["type"], "local");
        assert_eq!(flow.launch_state["branch"], "main");
        assert_eq!(flow.launch_state["autonomy_level"], "supervised");
        assert_eq!(flow.rules_file_created, true);
    }

    #[tokio::test]
    async fn prepare_project_launch_flow_reports_slug_conflict() {
        let (tmp, state) = test_state();
        let mut req = launch_req(tmp.path().join("workspace"));
        let launch = normalize_project_launch_request(&mut req).unwrap();
        prepare_project_launch_flow(&state, &req, &launch)
            .await
            .unwrap();

        let error = match prepare_project_launch_flow(&state, &req, &launch).await {
            Err(error) => error,
            Ok(_) => panic!("duplicate slug should fail"),
        };

        assert_eq!(error, ProjectLaunchFlowError::Conflict);
    }
}
