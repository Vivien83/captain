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
async fn update_project_updates_typed_fields_by_slug() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Before".to_string(),
            slug: "demo".to_string(),
            goal: "Old goal".to_string(),
            deadline: None,
        })
        .unwrap();
    let state = std::sync::Arc::new(state);

    let response = update_project(
        axum::extract::State(state.clone()),
        axum::extract::Path(format!(" {} ", project.slug)),
        axum::Json(UpdateProjectReq {
            name: Some("  After  ".to_string()),
            goal: Some("  New goal  ".to_string()),
            status: Some("active".to_string()),
            deadline: Some(42),
            metadata: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["name"], "After");
    assert_eq!(body["goal"], "New goal");
    assert_eq!(body["status"], "active");
    assert_eq!(body["deadline"], 42);

    let stored = state
        .kernel
        .memory
        .project_get(&project.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.name, "After");
    assert_eq!(stored.goal, "New goal");
    assert_eq!(stored.status.as_str(), "active");
    assert_eq!(stored.deadline, Some(42));
}

#[tokio::test]
async fn update_project_rejects_external_metadata_patch() {
    let (_tmp, state) = test_state();
    let created = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep metadata internal".to_string(),
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
                    "runtime": {"status": "ready", "current_phase": "observe"}
                })),
                ..Default::default()
            },
        )
        .unwrap();
    let state = std::sync::Arc::new(state);

    let response = update_project(
        axum::extract::State(state.clone()),
        axum::extract::Path("demo".to_string()),
        axum::Json(UpdateProjectReq {
            name: Some("Renamed".to_string()),
            goal: None,
            status: None,
            deadline: None,
            metadata: Some(serde_json::json!({
                "runtime": {"status": "done"},
                "workspace": {"path": "/Users/example/private"},
                "token": "ghp_secret"
            })),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let stored = state
        .kernel
        .memory
        .project_get(&created.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.name, "Demo");
    assert_eq!(stored.metadata["runtime"]["status"], "ready");
    let encoded = serde_json::to_string(&stored.metadata).unwrap();
    assert!(!encoded.contains("/Users/example/private"));
    assert!(!encoded.contains("ghp_secret"));
}

#[tokio::test]
async fn update_project_rejects_invalid_typed_fields_without_echoing_input() {
    let (_tmp, state) = test_state();
    let created = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep typed fields bounded".to_string(),
            deadline: None,
        })
        .unwrap();
    let state = std::sync::Arc::new(state);

    let response = update_project(
        axum::extract::State(state.clone()),
        axum::extract::Path("demo".to_string()),
        axum::Json(UpdateProjectReq {
            name: None,
            goal: Some("   ".to_string()),
            status: None,
            deadline: None,
            metadata: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let stored = state
        .kernel
        .memory
        .project_get(&created.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.goal, "Keep typed fields bounded");

    let response = update_project(
        axum::extract::State(state.clone()),
        axum::extract::Path("demo".to_string()),
        axum::Json(UpdateProjectReq {
            name: None,
            goal: None,
            status: Some("invalid-/Users/example/private-ghp_secret".to_string()),
            deadline: None,
            metadata: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("unknown project status"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
}
