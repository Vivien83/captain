use super::*;
use axum::body::to_bytes;
use axum::response::IntoResponse;
use captain_kernel::CaptainKernel;
use captain_memory::project;
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

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn delete_project_removes_project_and_project_goals_by_slug() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Delete me".to_string(),
            slug: "delete-me".to_string(),
            goal: "Clean project rows".to_string(),
            deadline: None,
        })
        .unwrap();
    let goal = crate::project_goal_runtime::build_project_goal(
        &state,
        &project,
        Some("delete-goal".to_string()),
        Some("Delete Goal".to_string()),
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

    let response = delete_project(
        axum::extract::State(state.clone()),
        axum::extract::Path(format!(" {} ", project.slug)),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["status"], "deleted");
    assert_eq!(body["project_id"], project.id);
    assert_eq!(body["removed_goals"], 1);
    assert!(state
        .kernel
        .memory
        .project_get(&project.id)
        .unwrap()
        .is_none());
    assert!(state.kernel.goal_store.get("delete-goal").is_none());
}

#[tokio::test]
async fn delete_project_rejects_invalid_lookup_without_echoing_input() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);

    let response = delete_project(
        axum::extract::State(state),
        axum::extract::Path("bad-/Users/example/private-ghp_secret".to_string()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = response_json(response).await.to_string();
    assert!(body.contains("project identifier is invalid"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
}
