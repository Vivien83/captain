use super::*;
use axum::body::to_bytes;
use axum::response::IntoResponse;
use captain_kernel::CaptainKernel;
use captain_memory::project;
use captain_runtime::active_project::SlashCommand;
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
    captain_runtime::active_project::install(tmp.path());
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
async fn set_active_project_normalizes_agent_and_slug_before_registry() {
    let (_tmp, state) = test_state();
    create_project_row(&state);
    let state = std::sync::Arc::new(state);
    let agent_id = "route-agent-normalizes";
    if let Some(reg) = captain_runtime::active_project::global() {
        reg.clear(agent_id);
    }

    let response = set_active_project(
        axum::extract::State(state),
        axum::extract::Path(format!(" {agent_id} ")),
        axum::Json(SetActiveProjectReq {
            slug: " demo ".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
    assert_eq!(body["agent_id"], agent_id);
    assert_eq!(body["slug"], "demo");
    let reg = captain_runtime::active_project::global().unwrap();
    assert_eq!(reg.get(agent_id).as_deref(), Some("demo"));
    reg.clear(agent_id);
}

#[tokio::test]
async fn set_active_project_rejects_invalid_slug_without_echoing_input() {
    let (_tmp, state) = test_state();
    create_project_row(&state);
    let state = std::sync::Arc::new(state);
    let agent_id = "route-agent-invalid-slug";
    if let Some(reg) = captain_runtime::active_project::global() {
        reg.clear(agent_id);
    }

    let response = set_active_project(
        axum::extract::State(state),
        axum::extract::Path(agent_id.to_string()),
        axum::Json(SetActiveProjectReq {
            slug: "bad-/Users/example/private-ghp_secret".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = response_body(response).await;
    assert!(body.contains("slug must be lowercase alphanumeric with hyphens"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
    assert!(captain_runtime::active_project::global()
        .and_then(|reg| reg.get(agent_id))
        .is_none());
}

#[tokio::test]
async fn set_active_project_missing_project_does_not_echo_slug() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);
    let agent_id = "route-agent-missing-project";
    if let Some(reg) = captain_runtime::active_project::global() {
        reg.clear(agent_id);
    }

    let response = set_active_project(
        axum::extract::State(state),
        axum::extract::Path(agent_id.to_string()),
        axum::Json(SetActiveProjectReq {
            slug: "missing-project".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    let body = response_body(response).await;
    assert!(body.contains("project not found"));
    assert!(!body.contains("missing-project"));
    assert!(captain_runtime::active_project::global()
        .and_then(|reg| reg.get(agent_id))
        .is_none());
}

#[tokio::test]
async fn get_and_clear_active_project_normalize_agent_id() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);
    let agent_id = "route-agent-get-clear";
    let reg = captain_runtime::active_project::global().unwrap();
    reg.set(agent_id.to_string(), "demo".to_string());

    let response = get_active_project(
        axum::extract::State(state.clone()),
        axum::extract::Path(format!(" {agent_id} ")),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
    assert_eq!(body["agent_id"], agent_id);
    assert_eq!(body["slug"], "demo");

    let response = clear_active_project(
        axum::extract::State(state),
        axum::extract::Path(format!(" {agent_id} ")),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
    assert_eq!(body["agent_id"], agent_id);
    assert_eq!(body["cleared"], true);
    assert!(reg.get(agent_id).is_none());
}

#[tokio::test]
async fn active_project_routes_reject_invalid_agent_id_without_echoing_input() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);

    let response = get_active_project(
        axum::extract::State(state),
        axum::extract::Path("ghp_secret".to_string()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = response_body(response).await;
    assert!(body.contains("agent_id is invalid"));
    assert!(!body.contains("ghp_secret"));
}

#[tokio::test]
async fn project_slash_switch_normalizes_slug_before_registry() {
    let (_tmp, state) = test_state();
    create_project_row(&state);
    let agent_id = "slash-agent-normalizes";
    if let Some(reg) = captain_runtime::active_project::global() {
        reg.clear(agent_id);
    }

    let reply = crate::project_slash::handle(
        &state.kernel,
        agent_id,
        SlashCommand::Switch(" demo ".to_string()),
    );

    assert!(reply.contains("Active project"));
    let reg = captain_runtime::active_project::global().unwrap();
    assert_eq!(reg.get(agent_id).as_deref(), Some("demo"));
    reg.clear(agent_id);
}

#[tokio::test]
async fn project_slash_switch_rejects_invalid_slug_without_echoing_input() {
    let (_tmp, state) = test_state();
    let reply = crate::project_slash::handle(
        &state.kernel,
        "slash-agent-invalid-slug",
        SlashCommand::Switch("bad-/Users/example/private-ghp_secret".to_string()),
    );

    assert!(reply.contains("slug must be lowercase alphanumeric with hyphens"));
    assert!(!reply.contains("/Users/example"));
    assert!(!reply.contains("ghp_secret"));
}

#[tokio::test]
async fn project_slash_missing_project_does_not_echo_slug() {
    let (_tmp, state) = test_state();
    let reply = crate::project_slash::handle(
        &state.kernel,
        "slash-agent-missing-project",
        SlashCommand::Switch("missing-project".to_string()),
    );

    assert!(reply.contains("Project not found"));
    assert!(!reply.contains("missing-project"));
}
