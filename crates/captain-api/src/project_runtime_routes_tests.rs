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
async fn project_runtime_resolves_slug_and_applies_event_window() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Runtime".to_string(),
            slug: "runtime-demo".to_string(),
            goal: "Show current runtime".to_string(),
            deadline: None,
        })
        .unwrap();
    let session_id = crate::project_runtime_defaults::project_session_id(&project);
    for idx in 0..2 {
        state
            .kernel
            .memory
            .append_session_event(
                &session_id,
                "project_runtime_event",
                &serde_json::json!({
                    "event": {
                        "id": format!("event-{idx}"),
                        "ts": format!("2026-05-24T10:00:0{idx}Z"),
                        "kind": "worker.note",
                        "title": format!("Event {idx}"),
                        "detail": "safe",
                        "phase": "build",
                        "status": "running",
                        "data": {"secret": "raw-event-secret"}
                    }
                }),
            )
            .unwrap();
    }
    let state = std::sync::Arc::new(state);

    let response = project_runtime(
        axum::extract::State(state),
        axum::extract::Path(format!(" {} ", project.slug)),
        axum::extract::Query(ProjectRuntimeQuery { events: Some(1) }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["project"]["slug"], "runtime-demo");
    assert!(body.get("chat").is_none());
    assert_eq!(body["transcript"]["stored_count"], 2);
    assert_eq!(body["transcript"]["limit"], 1);
    assert_eq!(body["transcript"]["count"], 1);
    assert_eq!(body["transcript"]["events"].as_array().unwrap().len(), 1);
    assert!(!body.to_string().contains("raw-event-secret"));
}
