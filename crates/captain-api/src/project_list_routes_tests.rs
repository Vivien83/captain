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

async fn response_body(response: axum::response::Response) -> serde_json::Value {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn list_projects_hides_archived_until_requested() {
    let (_tmp, state) = test_state();
    let active = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Active".to_string(),
            slug: "active".to_string(),
            goal: "Keep visible".to_string(),
            deadline: None,
        })
        .unwrap();
    let archived = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Archived".to_string(),
            slug: "archived".to_string(),
            goal: "Hide by default".to_string(),
            deadline: None,
        })
        .unwrap();
    state.kernel.memory.project_archive(&archived.id).unwrap();
    let state = std::sync::Arc::new(state);

    let response = list_projects(
        axum::extract::State(state.clone()),
        axum::extract::Query(HashMap::new()),
    )
    .await
    .into_response();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = response_body(response).await;
    assert_eq!(body["projects"].as_array().unwrap().len(), 1);
    assert_eq!(body["projects"][0]["id"], active.id);

    let response = list_projects(
        axum::extract::State(state),
        axum::extract::Query(HashMap::from([(
            "include_archived".to_string(),
            "true".to_string(),
        )])),
    )
    .await
    .into_response();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = response_body(response).await;
    assert_eq!(body["projects"].as_array().unwrap().len(), 2);
}
