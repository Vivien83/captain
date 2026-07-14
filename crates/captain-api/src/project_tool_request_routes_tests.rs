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

async fn response_body(response: axum::response::Response) -> String {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

#[tokio::test]
async fn tool_request_rejects_invalid_project_identifier_without_echoing_input() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);

    let response = respond_project_tool_request(
        axum::extract::State(state),
        axum::extract::Path("bad-/Users/example/private-ghp_secret".to_string()),
        axum::Json(ProjectToolRequestDecisionReq {
            phase: Some("build".to_string()),
            decision: "deny".to_string(),
            tools: None,
            reason: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = response_body(response).await;
    assert!(body.contains("invalid_project_identifier"));
    assert!(body.contains("project identifier is invalid"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
}

#[test]
fn prepare_tool_request_update_selects_pending_phase_and_normalizes_tools() {
    let project = project::Project {
        id: "project-1".to_string(),
        name: "Demo".to_string(),
        slug: "demo".to_string(),
        goal: "Approve required tools".to_string(),
        status: project::ProjectStatus::Active,
        deadline: None,
        created_at: 1,
        updated_at: 2,
        metadata: serde_json::json!({
            "runtime": {
                "status": "blocked",
                "worker_results": {
                    "build": {
                        "status": "blocked",
                        "blocked": true,
                        "tool_request": {
                            "status": "pending_captain_decision",
                            "tools": ["`shell_exec`", "browser", "shell_exec"]
                        }
                    }
                }
            }
        }),
    };

    let update = prepare_project_tool_request_update(
        &project,
        ProjectToolRequestDecisionReq {
            phase: None,
            decision: "allow-once".to_string(),
            tools: None,
            reason: Some("Use the requested tools once.".to_string()),
        },
    )
    .unwrap();

    assert_eq!(update.phase, "build");
    assert_eq!(update.tools, vec!["browser", "shell_exec"]);
    assert_eq!(update.metadata["runtime"]["status"], "ready");
    assert_eq!(
        update.metadata["runtime"]["resume_pending"]["reason"],
        "tool_request_approved"
    );
}
