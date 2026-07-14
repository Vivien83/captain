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
async fn resume_project_resolves_slug_and_returns_public_resume_views() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Resume me".to_string(),
            slug: "resume-me".to_string(),
            goal: "Resume safely".to_string(),
            deadline: None,
        })
        .unwrap();
    let goal = crate::project_goal_runtime::build_project_goal(
        &state,
        &project,
        Some("resume-goal".to_string()),
        Some("Resume Goal".to_string()),
        Some("private operator detail".to_string()),
        "echo ghp_secret".to_string(),
        Some("echo /Users/example/private".to_string()),
        Some(60),
        Some(3),
        Some(5),
        None,
    );
    state.kernel.goal_store.add(goal).unwrap();
    let state = std::sync::Arc::new(state);

    let response = resume_project(
        axum::extract::State(state),
        axum::extract::Path(format!(" {} ", project.slug)),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["project"]["id"], project.id);
    assert_eq!(body["project"]["slug"], "resume-me");
    assert_eq!(body["latest_checkpoint"], serde_json::Value::Null);
    assert_eq!(body["tasks"].as_array().unwrap().len(), 0);
    assert_eq!(body["goals"][0]["id"], "resume-goal");
    assert_eq!(body["goals"][0]["check_command_configured"], true);
    assert_eq!(body["goals"][0]["recovery_command_configured"], true);
    let body_text = body.to_string();
    assert!(!body_text.contains("ghp_secret"));
    assert!(!body_text.contains("/Users/example"));
    assert!(!body_text.contains("private operator detail"));
}
