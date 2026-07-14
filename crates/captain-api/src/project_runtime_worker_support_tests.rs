use super::*;
use crate::project_runtime_prompt_context::runtime_worker_system_prompt_for_tools;
use crate::project_runtime_worker_tools::runtime_worker_authorized_tools;
use crate::project_runtime_workers::RUNTIME_WORKER_SPECS;
use captain_kernel::CaptainKernel;
use captain_memory::project_task;
use captain_types::config::{DefaultModelConfig, KernelConfig};
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
    let kernel = std::sync::Arc::new(CaptainKernel::boot_with_config(config).unwrap());
    kernel.set_self_handle();
    let state = AppState {
        kernel,
        started_at: Instant::now(),
        peer_registry: None,
        bridge_manager: tokio::sync::Mutex::new(None),
        channels_config: tokio::sync::RwLock::new(Default::default()),
        shutdown_notify: std::sync::Arc::new(tokio::sync::Notify::new()),
        clawhub_cache: dashmap::DashMap::new(),
        ask_user_channels: dashmap::DashMap::new(),
        provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
    };
    (tmp, state)
}

fn runtime_worker_system_prompt(spec: &RuntimeWorkerSpec) -> String {
    runtime_worker_system_prompt_for_tools(
        spec.phase,
        &runtime_worker_authorized_tools(&spec.profile),
    )
}

#[test]
fn runtime_worker_prompt_names_authorized_tools_and_tool_request_path() {
    let prompt = runtime_worker_system_prompt(&RUNTIME_WORKER_SPECS[0]);

    assert!(prompt.contains("Authorized tools:"));
    assert!(prompt.contains("capability_search"));
    assert!(prompt.contains("TOOL_REQUEST"));
}

#[test]
fn project_workspace_path_prefers_workspace_then_source_then_existing_default() {
    let (tmp, state) = test_state();
    let mut project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo runtime".to_string(),
            slug: "demo-runtime".to_string(),
            goal: "Ship".to_string(),
            deadline: None,
        })
        .unwrap();
    project.metadata = serde_json::json!({
        "launch": {
            "source": {"local_path": "/tmp/source-path"},
            "workspace": {"path": "/tmp/workspace-path"}
        }
    });

    assert_eq!(
        project_workspace_path_for_runtime(&state, &project).as_deref(),
        Some("/tmp/workspace-path")
    );

    project.metadata = serde_json::json!({
        "launch": {"source": {"path": "/tmp/source-path"}}
    });
    assert_eq!(
        project_workspace_path_for_runtime(&state, &project).as_deref(),
        Some("/tmp/source-path")
    );

    project.metadata = serde_json::json!({});
    let default_path = tmp.path().join("workspaces/projects/demo-runtime");
    std::fs::create_dir_all(&default_path).unwrap();
    assert_eq!(
        project_workspace_path_for_runtime(&state, &project).as_deref(),
        Some(default_path.to_str().unwrap())
    );
}

#[test]
fn update_project_task_for_phase_updates_matching_phase_task_only() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Task project".to_string(),
            slug: "task-project".to_string(),
            goal: "Track phase".to_string(),
            deadline: None,
        })
        .unwrap();
    let build = state
        .kernel
        .memory
        .task_create(project_task::NewProjectTask {
            project_id: project.id.clone(),
            parent_id: None,
            title: "BUILD: implement runtime".to_string(),
            description: String::new(),
            priority: 0,
            deadline: None,
            assignee_agent_id: None,
        })
        .unwrap();
    let verify = state
        .kernel
        .memory
        .task_create(project_task::NewProjectTask {
            project_id: project.id.clone(),
            parent_id: None,
            title: "VERIFY: test runtime".to_string(),
            description: String::new(),
            priority: 0,
            deadline: None,
            assignee_agent_id: None,
        })
        .unwrap();

    update_project_task_for_phase(
        &state,
        &project.id,
        "build",
        project_task::TaskStatus::Doing,
        Some("agent-1".to_string()),
    );

    let build = state.kernel.memory.task_get(&build.id).unwrap().unwrap();
    let verify = state.kernel.memory.task_get(&verify.id).unwrap().unwrap();
    assert_eq!(build.status, project_task::TaskStatus::Doing);
    assert_eq!(build.assignee_agent_id.as_deref(), Some("agent-1"));
    assert_eq!(verify.status, project_task::TaskStatus::Todo);
    assert_eq!(verify.assignee_agent_id, None);
}
