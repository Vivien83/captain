use crate::project_metadata::{metadata_set_runtime, project_metadata};
use crate::project_runtime_state::project_runtime_state_for_project;
use crate::routes::AppState;
use captain_memory::project;
use serde_json::Value;

pub(crate) struct ProjectLaunchProjectInput<'a> {
    pub(crate) name: &'a str,
    pub(crate) slug: &'a str,
    pub(crate) goal: &'a str,
    pub(crate) deadline: Option<i64>,
    pub(crate) launch_state: Value,
    pub(crate) manager: Value,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProjectLaunchProjectError {
    Conflict,
    Storage(String),
}

pub(crate) fn create_active_launch_project(
    state: &AppState,
    input: ProjectLaunchProjectInput<'_>,
) -> Result<project::Project, ProjectLaunchProjectError> {
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: input.name.to_string(),
            slug: input.slug.to_string(),
            goal: input.goal.to_string(),
            deadline: input.deadline,
        })
        .map_err(|error| {
            let message = error.to_string();
            if message.to_lowercase().contains("unique") {
                ProjectLaunchProjectError::Conflict
            } else {
                ProjectLaunchProjectError::Storage(message)
            }
        })?;

    let mut metadata = project_metadata(Some(input.launch_state), "observe");
    metadata_set_runtime(
        &mut metadata,
        project_runtime_state_for_project(&project, Some("ready"), None, input.manager),
    );

    Ok(state
        .kernel
        .memory
        .project_update(
            &project.id,
            project::ProjectPatch {
                status: Some(project::ProjectStatus::Active),
                metadata: Some(metadata),
                ..Default::default()
            },
        )
        .ok()
        .flatten()
        .unwrap_or(project))
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

    fn launch_input(slug: &str) -> ProjectLaunchProjectInput<'_> {
        ProjectLaunchProjectInput {
            name: "Demo Project",
            slug,
            goal: "Ship safely",
            deadline: Some(123),
            launch_state: serde_json::json!({"protocol": "captain.project_launch.v1"}),
            manager: serde_json::json!({"id": null, "role": "project_manager"}),
        }
    }

    #[test]
    fn create_active_launch_project_sets_metadata_and_runtime_ready() {
        let (_tmp, state) = test_state();

        let project = create_active_launch_project(&state, launch_input("demo-project")).unwrap();

        assert_eq!(project.status, project::ProjectStatus::Active);
        assert_eq!(project.deadline, Some(123));
        assert_eq!(
            project.metadata["launch"]["protocol"],
            "captain.project_launch.v1"
        );
        assert_eq!(project.metadata["lifecycle"]["current_phase"], "observe");
        assert_eq!(project.metadata["runtime"]["status"], "ready");
        assert_eq!(
            project.metadata["runtime"]["manager_agent"]["role"],
            "project_manager"
        );
    }

    #[test]
    fn create_active_launch_project_reports_slug_conflict() {
        let (_tmp, state) = test_state();
        create_active_launch_project(&state, launch_input("demo-project")).unwrap();

        let error = create_active_launch_project(&state, launch_input("demo-project")).unwrap_err();

        assert_eq!(error, ProjectLaunchProjectError::Conflict);
    }
}
