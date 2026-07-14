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
async fn get_project_by_slug_returns_public_enriched_view() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Detail".to_string(),
            slug: "detail".to_string(),
            goal: "Inspect safely".to_string(),
            deadline: None,
        })
        .unwrap();
    state
        .kernel
        .memory
        .project_update(
            &project.id,
            project::ProjectPatch {
                metadata: Some(serde_json::json!({
                    "workspace": {"path": "/Users/example/private"},
                    "secret": "ghp_secret"
                })),
                ..Default::default()
            },
        )
        .unwrap();
    let state = std::sync::Arc::new(state);

    let response = get_project_by_slug(
        axum::extract::State(state),
        axum::extract::Path("detail".to_string()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["slug"], "detail");
    assert!(body.get("metadata").is_none());
    let encoded = serde_json::to_string(&body).unwrap();
    assert!(!encoded.contains("/Users/example/private"));
    assert!(!encoded.contains("ghp_secret"));
}
