use crate::project_detail_routes::get_project_by_slug;
use crate::project_resume_routes::resume_project;
use crate::project_runtime_routes::{project_runtime, ProjectRuntimeQuery};
use crate::routes::AppState;
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

fn create_project_row(state: &AppState) -> project::Project {
    state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep project healthy".to_string(),
            deadline: None,
        })
        .unwrap()
}

async fn response_body(response: axum::response::Response) -> String {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

#[tokio::test]
async fn project_runtime_rejects_invalid_identifier_without_echoing_input() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);

    let response = project_runtime(
        axum::extract::State(state),
        axum::extract::Path("bad-/Users/example/private-ghp_secret".to_string()),
        axum::extract::Query(ProjectRuntimeQuery::default()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = response_body(response).await;
    assert!(body.contains("project identifier is invalid"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
}

#[tokio::test]
async fn project_runtime_missing_project_does_not_echo_identifier() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);

    let response = project_runtime(
        axum::extract::State(state),
        axum::extract::Path("missing-project".to_string()),
        axum::extract::Query(ProjectRuntimeQuery::default()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    let body = response_body(response).await;
    assert!(body.contains("project not found"));
    assert!(!body.contains("missing-project"));
}

#[tokio::test]
async fn resume_project_trims_identifier_before_lookup() {
    let (_tmp, state) = test_state();
    let project = create_project_row(&state);
    let state = std::sync::Arc::new(state);

    let response = resume_project(
        axum::extract::State(state),
        axum::extract::Path(format!(" {} ", project.slug)),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
    assert_eq!(body["project"]["slug"], "demo");
}

#[tokio::test]
async fn get_project_by_slug_rejects_invalid_slug_without_echoing_input() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);

    let response = get_project_by_slug(
        axum::extract::State(state),
        axum::extract::Path("bad-/Users/example/private-ghp_secret".to_string()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = response_body(response).await;
    assert!(body.contains("slug must be lowercase alphanumeric with hyphens"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
}
