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

#[tokio::test]
async fn create_project_normalizes_text_before_storage() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);

    let response = create_project(
        axum::extract::State(state.clone()),
        axum::Json(CreateProjectReq {
            name: "  Demo Project  ".to_string(),
            slug: " demo-project ".to_string(),
            goal: "  Ship safely  ".to_string(),
            deadline: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    let projects = state.kernel.memory.project_list(true).unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].name, "Demo Project");
    assert_eq!(projects[0].slug, "demo-project");
    assert_eq!(projects[0].goal, "Ship safely");
}

#[tokio::test]
async fn create_project_rejects_long_goal_without_echoing_input() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);

    let response = create_project(
        axum::extract::State(state.clone()),
        axum::Json(CreateProjectReq {
            name: "Demo Project".to_string(),
            slug: "demo-project".to_string(),
            goal: format!("{}-/Users/example/private-ghp_secret", "x".repeat(2_001)),
            deadline: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("goal is too long"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
    assert!(state.kernel.memory.project_list(true).unwrap().is_empty());
}
