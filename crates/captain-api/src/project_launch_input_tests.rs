use super::*;
use axum::body::to_bytes;
use axum::response::IntoResponse;
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

fn launch_req(local_path: std::path::PathBuf) -> LaunchProjectReq {
    LaunchProjectReq {
        name: Some(" Demo Launch ".to_string()),
        slug: Some(" demo-launch ".to_string()),
        goal: " Ship safely ".to_string(),
        repo_path: None,
        local_path: Some(local_path.display().to_string()),
        source_type: Some(" local ".to_string()),
        github_full_name: None,
        github_clone_url: None,
        github_branch: None,
        github_repo_id: None,
        branch: Some(" main ".to_string()),
        create_worktree: None,
        create_folder: Some(true),
        autonomy_level: Some(" supervised ".to_string()),
        acceptance_criteria: vec![" First criterion ".to_string(), " ".to_string()],
        deadline: None,
        goal_check_command: Some(" true ".to_string()),
        goal_recovery_command: Some(" echo recover ".to_string()),
        goal_interval_secs: Some(60),
    }
}

#[tokio::test]
async fn launch_project_normalizes_input_before_storage() {
    let (tmp, state) = test_state();
    let workspace = tmp.path().join("launch-workspace");
    let state = std::sync::Arc::new(state);

    let response = launch_project(
        axum::extract::State(state.clone()),
        axum::Json(launch_req(workspace)),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    let projects = state.kernel.memory.project_list(true).unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].name, "Demo Launch");
    assert_eq!(projects[0].slug, "demo-launch");
    assert_eq!(projects[0].goal, "Ship safely");
    assert_eq!(
        projects[0].metadata.pointer("/launch/autonomy_level"),
        Some(&serde_json::json!("supervised"))
    );
    assert_eq!(
        projects[0]
            .metadata
            .pointer("/launch/acceptance_criteria/0"),
        Some(&serde_json::json!("First criterion"))
    );
    let goals = state
        .kernel
        .goal_store
        .list_for_project(&projects[0].id, &projects[0].slug);
    assert_eq!(goals.len(), 1);
    assert_eq!(goals[0].check_command, "true");
    assert_eq!(goals[0].recovery_command.as_deref(), Some("echo recover"));
}

#[tokio::test]
async fn launch_project_rejects_long_criteria_without_partial_workspace_or_project() {
    let (tmp, state) = test_state();
    let workspace = tmp.path().join("should-not-create");
    let state = std::sync::Arc::new(state);
    let mut req = launch_req(workspace.clone());
    req.acceptance_criteria = vec![format!(
        "{}-/Users/example/private-ghp_secret",
        "x".repeat(401)
    )];

    let response = launch_project(axum::extract::State(state.clone()), axum::Json(req))
        .await
        .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("acceptance criterion is too long"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
    assert!(state.kernel.memory.project_list(true).unwrap().is_empty());
    assert!(!workspace.exists());
}

#[tokio::test]
async fn launch_project_rejects_goal_guard_without_partial_project() {
    let (tmp, state) = test_state();
    let workspace = tmp.path().join("guard-reject");
    let state = std::sync::Arc::new(state);
    let mut req = launch_req(workspace);
    req.goal_check_command = None;
    req.goal_recovery_command = Some("echo recover".to_string());

    let response = launch_project(axum::extract::State(state.clone()), axum::Json(req))
        .await
        .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("goal_recovery_command requires goal_check_command"));
    assert!(state.kernel.memory.project_list(true).unwrap().is_empty());
}
