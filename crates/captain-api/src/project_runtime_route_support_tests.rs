use super::*;
use captain_kernel::CaptainKernel;
use captain_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::atomic::{AtomicBool, Ordering};
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
async fn mutate_project_runtime_resolves_slug_and_returns_public_runtime() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Runtime support".to_string(),
            slug: "runtime-support".to_string(),
            goal: "Keep route support small".to_string(),
            deadline: None,
        })
        .unwrap();
    let state = Arc::new(state);

    let (status, Json(body)) =
        mutate_project_runtime(&state, "runtime-support", |runtime, runtime_project| {
            assert_eq!(runtime_project.id, project.id);
            runtime["status"] = serde_json::json!("running");
            runtime["current_phase"] = serde_json::json!("build");
        })
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["project"]["slug"], "runtime-support");
    assert_eq!(body["runtime"]["status"], "running");
    assert!(body["project"].get("metadata").is_none());
}

#[tokio::test]
async fn mutate_project_runtime_rejects_invalid_identifier_without_mutating() {
    let (_tmp, state) = test_state();
    let called = AtomicBool::new(false);
    let state = Arc::new(state);

    let (status, Json(body)) =
        mutate_project_runtime(&state, "bad-/Users/example/private", |_, _| {
            called.store(true, Ordering::SeqCst);
        })
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "project identifier is invalid");
    assert!(!called.load(Ordering::SeqCst));
    assert!(!serde_json::to_string(&body)
        .unwrap()
        .contains("/Users/example/private"));
}
