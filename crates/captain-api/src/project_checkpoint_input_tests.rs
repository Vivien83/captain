use super::*;
use axum::body::to_bytes;
use axum::response::IntoResponse;
use captain_kernel::CaptainKernel;
use captain_types::config::{DefaultModelConfig, KernelConfig};
use std::collections::HashMap;
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
async fn create_checkpoint_normalizes_text_and_stores_empty_state() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep checkpoint input bounded".to_string(),
            deadline: None,
        })
        .unwrap();
    let state = std::sync::Arc::new(state);

    let response = create_checkpoint(
        axum::extract::State(state.clone()),
        axum::extract::Path(project.id.clone()),
        axum::Json(CreateCheckpointReq {
            summary: "  Reached verify  ".to_string(),
            state: serde_json::json!({}),
            session_id: Some(" session-1 ".to_string()),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    let stored = state
        .kernel
        .memory
        .checkpoint_history(&project.id, 10)
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].summary, "Reached verify");
    assert_eq!(stored[0].session_id.as_deref(), Some("session-1"));
    assert_eq!(stored[0].state, serde_json::json!({}));
}

#[tokio::test]
async fn create_checkpoint_rejects_state_payload_without_echoing_input() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep checkpoint state internal".to_string(),
            deadline: None,
        })
        .unwrap();
    let state = std::sync::Arc::new(state);

    let response = create_checkpoint(
        axum::extract::State(state.clone()),
        axum::extract::Path(project.id.clone()),
        axum::Json(CreateCheckpointReq {
            summary: "Checkpoint attempt".to_string(),
            state: serde_json::json!({
                "path": "/Users/example/private",
                "token": "ghp_secret"
            }),
            session_id: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("checkpoint state is managed by runtime checkpoints"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
    let stored = state
        .kernel
        .memory
        .checkpoint_history(&project.id, 10)
        .unwrap();
    assert!(stored.is_empty());
}

#[tokio::test]
async fn list_checkpoints_trims_project_id_and_respects_limit() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep checkpoint ids bounded".to_string(),
            deadline: None,
        })
        .unwrap();
    for summary in ["first", "second"] {
        state
            .kernel
            .memory
            .checkpoint_append(project_checkpoint::NewCheckpoint {
                project_id: project.id.clone(),
                session_id: None,
                summary: summary.to_string(),
                state: serde_json::json!({}),
            })
            .unwrap();
    }
    let state = std::sync::Arc::new(state);
    let mut params = HashMap::new();
    params.insert("limit".to_string(), "1".to_string());

    let response = list_checkpoints(
        axum::extract::State(state),
        axum::extract::Path(format!(" {} ", project.id)),
        axum::extract::Query(params),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["checkpoints"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn checkpoint_routes_resolve_slug_to_canonical_project_id() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Resolve checkpoint routes from slug".to_string(),
            deadline: None,
        })
        .unwrap();
    let state = std::sync::Arc::new(state);

    let create = create_checkpoint(
        axum::extract::State(state.clone()),
        axum::extract::Path(project.slug.clone()),
        axum::Json(CreateCheckpointReq {
            summary: "Slug checkpoint".to_string(),
            state: serde_json::json!({}),
            session_id: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(create.status(), axum::http::StatusCode::CREATED);

    let list = list_checkpoints(
        axum::extract::State(state.clone()),
        axum::extract::Path(project.slug.clone()),
        axum::extract::Query(HashMap::new()),
    )
    .await
    .into_response();
    assert_eq!(list.status(), axum::http::StatusCode::OK);
    let body = to_bytes(list.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["checkpoints"].as_array().unwrap().len(), 1);

    let stored = state
        .kernel
        .memory
        .checkpoint_history(&project.id, 10)
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].project_id, project.id);
}

#[tokio::test]
async fn list_checkpoints_rejects_invalid_project_id_without_echoing_input() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);

    let response = list_checkpoints(
        axum::extract::State(state),
        axum::extract::Path("bad-/Users/example/private-ghp_secret".to_string()),
        axum::extract::Query(HashMap::new()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("project checkpoint project id"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
}

#[tokio::test]
async fn create_checkpoint_rejects_invalid_project_id_without_echoing_input() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);

    let response = create_checkpoint(
        axum::extract::State(state.clone()),
        axum::extract::Path("bad-/Users/example/private-ghp_secret".to_string()),
        axum::Json(CreateCheckpointReq {
            summary: "Checkpoint attempt".to_string(),
            state: serde_json::json!({}),
            session_id: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("project checkpoint project id"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
}

#[tokio::test]
async fn list_checkpoints_rejects_invalid_limit_without_echoing_input() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep checkpoint limit bounded".to_string(),
            deadline: None,
        })
        .unwrap();
    let state = std::sync::Arc::new(state);
    let mut params = HashMap::new();
    params.insert(
        "limit".to_string(),
        "bad-/Users/example/private-ghp_secret".to_string(),
    );

    let response = list_checkpoints(
        axum::extract::State(state),
        axum::extract::Path(project.id),
        axum::extract::Query(params),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("checkpoint limit must be an integer between 1 and 100"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
}
