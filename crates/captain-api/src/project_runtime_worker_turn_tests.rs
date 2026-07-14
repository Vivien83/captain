use super::*;
use captain_kernel::CaptainKernel;
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

#[tokio::test]
async fn record_project_stream_event_appends_project_timeline_event() {
    let (_tmp, state) = test_state();
    let project = create_worker_turn_project(&state);
    seed_worker_turn_runtime(&state, &project);
    let state = std::sync::Arc::new(state);
    let project = load_project(&state, &project.id);
    let agent_id = AgentId::new();

    record_intermediate_message_event(&state, &project, agent_id)
        .await
        .unwrap();

    let event = stored_first_runtime_event(&state, &project.id);
    assert_worker_note_event(&event, agent_id);
}

fn create_worker_turn_project(state: &AppState) -> project::Project {
    state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Worker turn".to_string(),
            slug: "worker-turn".to_string(),
            goal: "Record stream events".to_string(),
            deadline: None,
        })
        .unwrap()
}

fn seed_worker_turn_runtime(state: &AppState, project: &project::Project) {
    state
        .kernel
        .memory
        .project_update(
            &project.id,
            project::ProjectPatch {
                metadata: Some(serde_json::json!({
                    "runtime": {
                        "status": "running",
                        "current_phase": "build",
                        "timeline": []
                    }
                })),
                ..Default::default()
            },
        )
        .unwrap();
}

fn load_project(state: &AppState, project_id: &str) -> project::Project {
    state
        .kernel
        .memory
        .project_get(project_id)
        .unwrap()
        .unwrap()
}

async fn record_intermediate_message_event(
    state: &std::sync::Arc<AppState>,
    project: &project::Project,
    agent_id: AgentId,
) -> Result<(), String> {
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    record_project_stream_event(
        state,
        project,
        &crate::project_runtime_workers::RUNTIME_WORKER_SPECS[3],
        "run-1",
        "build",
        agent_id,
        StreamEvent::IntermediateMessage {
            content: "inspected files".to_string(),
        },
        tx,
    )
    .await
}

fn stored_first_runtime_event(state: &AppState, project_id: &str) -> serde_json::Value {
    let stored = state
        .kernel
        .memory
        .project_get(project_id)
        .unwrap()
        .unwrap();
    stored.metadata["runtime"]["timeline"][0].clone()
}

fn assert_worker_note_event(event: &serde_json::Value, agent_id: AgentId) {
    assert_eq!(event["kind"], "worker.note");
    assert_eq!(event["phase"], "build");
    assert_eq!(event["status"], "running");
    assert_eq!(event["data"]["run_id"], "run-1");
    assert_eq!(event["data"]["agent_id"], agent_id.to_string());
    assert!(event["detail"]
        .as_str()
        .unwrap()
        .contains("inspected files"));
}
