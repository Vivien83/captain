use super::*;
use axum::body::to_bytes;
use axum::http::StatusCode;
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

async fn response_text(response: axum::response::Response) -> String {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_str(&response_text(response).await).unwrap()
}

#[tokio::test]
async fn pause_project_runtime_marks_runtime_paused() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Pause runtime".to_string(),
            slug: "pause-runtime".to_string(),
            goal: "Pause workers".to_string(),
            deadline: None,
        })
        .unwrap();
    let state = std::sync::Arc::new(state);

    let response = pause_project_runtime(
        axum::extract::State(state.clone()),
        axum::extract::Path(format!(" {} ", project.slug)),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["project"]["slug"], "pause-runtime");
    assert_eq!(body["runtime"]["status"], "paused");
    assert_eq!(body["runtime"]["control"]["paused"], true);
    assert_eq!(body["runtime"]["control"]["takeover"], false);
    assert!(body["runtime"]["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["kind"] == "project.paused"));

    let stored = state
        .kernel
        .memory
        .project_get(&project.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.metadata["runtime"]["status"], "paused");
    assert_eq!(stored.metadata["runtime"]["orchestrator"]["active"], false);
}

#[tokio::test]
async fn pause_project_runtime_rejects_invalid_identifier_without_echoing_input() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);

    let response = pause_project_runtime(
        axum::extract::State(state),
        axum::extract::Path("bad-/Users/example/private-ghp_secret".to_string()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response_text(response).await;
    assert!(body.contains("project identifier is invalid"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
}
