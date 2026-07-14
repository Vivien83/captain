use crate::project_goal_runtime::{add_project_goal, build_project_goal};
use crate::project_launch_input::ProjectLaunchGoalGuard;
use crate::project_launch_state::project_created_event_payload;
use crate::project_launch_tasks::project_launch_tasks;
use crate::project_lifecycle::lifecycle_json;
use crate::routes::AppState;
use captain_kernel::goals::Goal;
use captain_memory::{milestone, project, project_checkpoint, project_task};
use captain_types::agent::AgentId;
use captain_types::event::{Event, EventPayload, EventTarget};
use serde_json::Value;

pub(crate) struct ProjectLaunchRecords {
    pub(crate) goals: Vec<Goal>,
    pub(crate) tasks: Vec<project_task::ProjectTask>,
    pub(crate) milestone: milestone::Milestone,
    pub(crate) checkpoint: project_checkpoint::Checkpoint,
}

pub(crate) struct ProjectLaunchRecordsInput<'a> {
    pub(crate) project: &'a project::Project,
    pub(crate) goal: &'a str,
    pub(crate) criteria: &'a [String],
    pub(crate) goal_guard: &'a ProjectLaunchGoalGuard,
    pub(crate) deadline: Option<i64>,
    pub(crate) launch_state: &'a Value,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProjectLaunchRecordsError {
    BadRequest(String),
    Storage(String),
}

pub(crate) fn create_project_launch_records(
    state: &AppState,
    input: ProjectLaunchRecordsInput<'_>,
) -> Result<ProjectLaunchRecords, ProjectLaunchRecordsError> {
    let goals = create_launch_project_goals(state, input.project, input.goal, input.goal_guard)?;
    let tasks = create_launch_project_tasks(state, input.project, input.goal, input.criteria)?;
    let milestone = create_launch_project_milestone(state, input.project, input.deadline)?;
    let checkpoint = append_launch_project_checkpoint(
        state,
        input.project,
        input.goal,
        input.launch_state,
        goals.len(),
        tasks.len(),
        &milestone.id,
    )?;

    Ok(ProjectLaunchRecords {
        goals,
        tasks,
        milestone,
        checkpoint,
    })
}

pub(crate) async fn publish_project_launch_created_event(
    state: &AppState,
    project: &project::Project,
    launch_state: &Value,
    records: &ProjectLaunchRecords,
    rules_file_created: bool,
) {
    let event_payload = project_created_event_payload(
        project,
        launch_state,
        records.tasks.len(),
        records.goals.len(),
        &records.milestone.id,
        &records.checkpoint.id,
        rules_file_created,
    );
    if let Ok(bytes) = serde_json::to_vec(&event_payload) {
        state
            .kernel
            .publish_event(Event::new(
                AgentId::new(),
                EventTarget::Broadcast,
                EventPayload::Custom(bytes),
            ))
            .await;
    }
}

fn create_launch_project_goals(
    state: &AppState,
    project: &project::Project,
    goal: &str,
    goal_guard: &ProjectLaunchGoalGuard,
) -> Result<Vec<Goal>, ProjectLaunchRecordsError> {
    let Some(goal_command) = goal_guard.check_command.as_deref() else {
        return Ok(Vec::new());
    };

    let goal = build_project_goal(
        state,
        project,
        None,
        Some(format!("{} guard", project.name)),
        Some(format!("Maintain project goal: {goal}")),
        goal_command.to_string(),
        goal_guard.recovery_command.clone(),
        goal_guard.interval_secs,
        None,
        None,
        None,
    );
    add_project_goal(state, goal)
        .map(|goal| vec![goal])
        .map_err(ProjectLaunchRecordsError::BadRequest)
}

fn create_launch_project_tasks(
    state: &AppState,
    project: &project::Project,
    goal: &str,
    criteria: &[String],
) -> Result<Vec<project_task::ProjectTask>, ProjectLaunchRecordsError> {
    let task_specs = project_launch_tasks(&project.id, goal, criteria);
    let mut tasks = Vec::with_capacity(task_specs.len());
    for spec in task_specs {
        let task = state
            .kernel
            .memory
            .task_create(spec)
            .map_err(|error| ProjectLaunchRecordsError::Storage(error.to_string()))?;
        tasks.push(task);
    }
    Ok(tasks)
}

fn create_launch_project_milestone(
    state: &AppState,
    project: &project::Project,
    deadline: Option<i64>,
) -> Result<milestone::Milestone, ProjectLaunchRecordsError> {
    state
        .kernel
        .memory
        .milestone_create(milestone::NewMilestone {
            project_id: project.id.clone(),
            name: "Verified first delivery".to_string(),
            due_date: deadline,
            deliverables: launch_milestone_deliverables(),
        })
        .map_err(|error| ProjectLaunchRecordsError::Storage(error.to_string()))
}

fn append_launch_project_checkpoint(
    state: &AppState,
    project: &project::Project,
    goal: &str,
    launch_state: &Value,
    project_goal_count: usize,
    created_task_count: usize,
    milestone_id: &str,
) -> Result<project_checkpoint::Checkpoint, ProjectLaunchRecordsError> {
    state
        .kernel
        .memory
        .checkpoint_append(project_checkpoint::NewCheckpoint {
            project_id: project.id.clone(),
            session_id: None,
            summary: format!("Project launched: {}. Goal: {}", project.name, goal),
            state: launch_checkpoint_state(
                launch_state,
                project_goal_count,
                created_task_count,
                milestone_id,
            ),
        })
        .map_err(|error| ProjectLaunchRecordsError::Storage(error.to_string()))
}

fn launch_milestone_deliverables() -> Vec<String> {
    vec![
        "Scope and acceptance criteria are explicit".to_string(),
        "Implementation is completed in an isolated workspace when needed".to_string(),
        "Verification commands are recorded and passing or blockers are visible".to_string(),
        "Handoff includes decisions, risks, and learning candidates".to_string(),
    ]
}

fn launch_checkpoint_state(
    launch_state: &Value,
    project_goal_count: usize,
    created_task_count: usize,
    milestone_id: &str,
) -> Value {
    serde_json::json!({
        "launch": launch_state,
        "lifecycle": lifecycle_json("observe"),
        "project_goal_count": project_goal_count,
        "created_task_count": created_task_count,
        "milestone_id": milestone_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_kernel::CaptainKernel;
    use captain_types::config::{DefaultModelConfig, KernelConfig};
    use std::sync::Arc;
    use std::time::Instant;

    fn test_state() -> (tempfile::TempDir, AppState) {
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
        (tmp, state)
    }

    fn goal_guard() -> ProjectLaunchGoalGuard {
        ProjectLaunchGoalGuard {
            check_command: None,
            recovery_command: None,
            interval_secs: None,
        }
    }

    #[test]
    fn launch_checkpoint_state_keeps_counts_and_lifecycle() {
        let launch_state = serde_json::json!({"protocol": "captain.project_launch.v1"});

        let state = launch_checkpoint_state(&launch_state, 1, 7, "milestone-1");

        assert_eq!(state["launch"]["protocol"], "captain.project_launch.v1");
        assert_eq!(state["lifecycle"]["current_phase"], "observe");
        assert_eq!(state["project_goal_count"], 1);
        assert_eq!(state["created_task_count"], 7);
        assert_eq!(state["milestone_id"], "milestone-1");
    }

    #[test]
    fn create_project_launch_records_persists_backbone_without_goal_guard() {
        let (_tmp, state) = test_state();
        let project = state
            .kernel
            .memory
            .project_create(project::NewProject {
                name: "Demo Project".to_string(),
                slug: "demo-project".to_string(),
                goal: "Ship safely".to_string(),
                deadline: None,
            })
            .unwrap();
        let launch_state = serde_json::json!({"protocol": "captain.project_launch.v1"});

        let records = create_project_launch_records(
            &state,
            ProjectLaunchRecordsInput {
                project: &project,
                goal: "Ship safely",
                criteria: &["Smoke passes".to_string()],
                goal_guard: &goal_guard(),
                deadline: Some(123),
                launch_state: &launch_state,
            },
        )
        .unwrap();

        assert!(records.goals.is_empty());
        assert_eq!(records.tasks.len(), 7);
        assert_eq!(records.tasks[0].project_id, project.id);
        assert_eq!(records.milestone.name, "Verified first delivery");
        assert_eq!(records.milestone.due_date, Some(123));
        assert_eq!(
            records.milestone.deliverables,
            launch_milestone_deliverables()
        );
        assert_eq!(records.checkpoint.project_id, project.id);
        assert_eq!(
            records.checkpoint.summary,
            "Project launched: Demo Project. Goal: Ship safely"
        );
        assert_eq!(records.checkpoint.state["created_task_count"], 7);
        assert_eq!(records.checkpoint.state["project_goal_count"], 0);
        assert_eq!(
            records.checkpoint.state["milestone_id"],
            records.milestone.id
        );
    }
}
