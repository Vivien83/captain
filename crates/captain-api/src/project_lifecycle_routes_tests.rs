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
async fn set_project_lifecycle_phase_rejects_invalid_phase_without_echoing_input() {
    let (_tmp, state) = test_state();
    let created = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep lifecycle bounded".to_string(),
            deadline: None,
        })
        .unwrap();
    state
        .kernel
        .memory
        .project_update(
            &created.id,
            project::ProjectPatch {
                metadata: Some(serde_json::json!({
                    "lifecycle": {
                        "protocol": "captain.project_lifecycle.v1",
                        "current_phase": "observe"
                    }
                })),
                ..Default::default()
            },
        )
        .unwrap();
    let state = std::sync::Arc::new(state);

    let response = set_project_lifecycle_phase(
        axum::extract::State(state.clone()),
        axum::extract::Path("demo".to_string()),
        axum::Json(SetLifecyclePhaseReq {
            phase: "invalid-/Users/example/private-ghp_secret".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("unknown project lifecycle phase"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
    let stored = state
        .kernel
        .memory
        .project_get(&created.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.metadata["lifecycle"]["current_phase"], "observe");
}
