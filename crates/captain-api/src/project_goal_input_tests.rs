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

fn create_project_row(state: &AppState) -> project::Project {
    state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep project healthy".to_string(),
            deadline: None,
        })
        .unwrap()
}

#[tokio::test]
async fn create_project_goal_normalizes_text_before_storage() {
    let (_tmp, state) = test_state();
    let project = create_project_row(&state);
    let state = std::sync::Arc::new(state);

    let response = create_project_goal(
        axum::extract::State(state.clone()),
        axum::extract::Path(project.slug.clone()),
        axum::Json(CreateProjectGoalReq {
            id: Some(" goal-one ".to_string()),
            name: Some(" Keep Healthy ".to_string()),
            description: Some(" Watch status ".to_string()),
            check_command: " cargo test ".to_string(),
            recovery_command: Some(" echo recover ".to_string()),
            interval_secs: Some(60),
            escalation_threshold: Some(3),
            max_llm_calls_per_hour: Some(5),
            escalation_channel: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    let stored = state.kernel.goal_store.get("goal-one").unwrap();
    assert_eq!(stored.name, "Keep Healthy");
    assert_eq!(stored.description, "Watch status");
    assert_eq!(stored.check_command, "cargo test");
    assert_eq!(stored.recovery_command.as_deref(), Some("echo recover"));
}

#[tokio::test]
async fn create_project_goal_rejects_long_command_without_echoing_input() {
    let (_tmp, state) = test_state();
    let project = create_project_row(&state);
    let state = std::sync::Arc::new(state);

    let response = create_project_goal(
        axum::extract::State(state.clone()),
        axum::extract::Path(project.id.clone()),
        axum::Json(CreateProjectGoalReq {
            id: Some("goal-one".to_string()),
            name: Some("Keep Healthy".to_string()),
            description: None,
            check_command: format!("{}-/Users/example/private-ghp_secret", "x".repeat(2_001)),
            recovery_command: None,
            interval_secs: Some(60),
            escalation_threshold: Some(3),
            max_llm_calls_per_hour: Some(5),
            escalation_channel: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("check_command is too long"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
    assert!(state.kernel.goal_store.get("goal-one").is_none());
}

#[tokio::test]
async fn update_project_goal_rejects_long_recovery_without_mutating_goal() {
    let (_tmp, state) = test_state();
    let project = create_project_row(&state);
    let mut goal = build_project_goal(
        &state,
        &project,
        Some("goal-one".to_string()),
        Some("Keep Healthy".to_string()),
        Some("Watch status".to_string()),
        "cargo test".to_string(),
        Some("echo recover".to_string()),
        Some(60),
        Some(3),
        Some(5),
        None,
    );
    goal.status = GoalStatus::Active;
    state.kernel.goal_store.add(goal).unwrap();
    let state = std::sync::Arc::new(state);

    let response = update_project_goal(
        axum::extract::State(state.clone()),
        axum::extract::Path((project.id.clone(), "goal-one".to_string())),
        axum::Json(UpdateProjectGoalReq {
            name: Some("Renamed".to_string()),
            description: None,
            check_command: None,
            recovery_command: Some(format!(
                "{}-/Users/example/private-ghp_secret",
                "x".repeat(2_001)
            )),
            interval_secs: None,
            escalation_threshold: None,
            max_llm_calls_per_hour: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("recovery_command is too long"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
    let stored = state.kernel.goal_store.get("goal-one").unwrap();
    assert_eq!(stored.name, "Keep Healthy");
    assert_eq!(stored.recovery_command.as_deref(), Some("echo recover"));
}

#[tokio::test]
async fn update_project_goal_rejects_invalid_goal_id_without_echoing_input() {
    let (_tmp, state) = test_state();
    let project = create_project_row(&state);
    let mut goal = build_project_goal(
        &state,
        &project,
        Some("goal-one".to_string()),
        Some("Keep Healthy".to_string()),
        None,
        "cargo test".to_string(),
        None,
        Some(60),
        Some(3),
        Some(5),
        None,
    );
    goal.status = GoalStatus::Active;
    state.kernel.goal_store.add(goal).unwrap();
    let state = std::sync::Arc::new(state);

    let response = update_project_goal(
        axum::extract::State(state.clone()),
        axum::extract::Path((
            project.slug.clone(),
            "bad-/Users/example/private-ghp_secret".to_string(),
        )),
        axum::Json(UpdateProjectGoalReq {
            name: Some("Renamed".to_string()),
            description: None,
            check_command: None,
            recovery_command: None,
            interval_secs: None,
            escalation_threshold: None,
            max_llm_calls_per_hour: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("goal id must be 3..=64 chars"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
    let stored = state.kernel.goal_store.get("goal-one").unwrap();
    assert_eq!(stored.name, "Keep Healthy");
}

#[tokio::test]
async fn pause_project_goal_missing_goal_does_not_echo_goal_id() {
    let (_tmp, state) = test_state();
    let project = create_project_row(&state);
    let state = std::sync::Arc::new(state);

    let response = pause_project_goal(
        axum::extract::State(state),
        axum::extract::Path((project.slug.clone(), "missing-goal".to_string())),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("project goal not found"));
    assert!(!body.contains("missing-goal"));
}

#[tokio::test]
async fn delete_project_goal_trims_goal_id_before_lookup() {
    let (_tmp, state) = test_state();
    let project = create_project_row(&state);
    let goal = build_project_goal(
        &state,
        &project,
        Some("goal-one".to_string()),
        Some("Keep Healthy".to_string()),
        None,
        "cargo test".to_string(),
        None,
        Some(60),
        Some(3),
        Some(5),
        None,
    );
    state.kernel.goal_store.add(goal).unwrap();
    let state = std::sync::Arc::new(state);

    let response = delete_project_goal(
        axum::extract::State(state.clone()),
        axum::extract::Path((project.slug.clone(), " goal-one ".to_string())),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert!(state.kernel.goal_store.get("goal-one").is_none());
}
