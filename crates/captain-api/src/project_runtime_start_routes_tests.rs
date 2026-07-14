use super::*;
use crate::project_runtime_defaults::project_session_id;
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

async fn response_text(response: axum::response::Response) -> String {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

#[tokio::test]
async fn prepare_start_project_runtime_starts_runtime_and_requests_spawn() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Start runtime".to_string(),
            slug: "start-runtime".to_string(),
            goal: "Run workers".to_string(),
            deadline: None,
        })
        .unwrap();
    let state = std::sync::Arc::new(state);

    let ((status, Json(body)), spawn_key) =
        prepare_start_project_runtime(&state, &format!(" {} ", project.slug)).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(spawn_key.as_deref(), Some(" start-runtime "));
    assert_eq!(body["project"]["slug"], "start-runtime");
    assert_eq!(body["runtime"]["status"], "running");
    assert_eq!(body["runtime"]["current_phase"], "observe");
    assert!(body["runtime"]["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["kind"] == "project.started"));

    let stored = state
        .kernel
        .memory
        .project_get(&project.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.metadata["runtime"]["status"], "running");
}

#[tokio::test]
async fn prepare_start_project_runtime_resumes_stale_run_and_replays_events() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Restart replay".to_string(),
            slug: "restart-replay".to_string(),
            goal: "Recover project runtime without losing timeline".to_string(),
            deadline: None,
        })
        .unwrap();
    let stored_event = serde_json::json!({
        "id": "event-before-restart",
        "ts": "2026-06-16T10:00:00Z",
        "kind": "worker.done",
        "title": "Observe done",
        "detail": "Pre-restart progress is durable.",
        "actor": "captain",
        "phase": "observe",
        "status": "done",
        "data": {"raw": "must-not-leak"}
    });
    let runtime = serde_json::json!({
        "status": "running",
        "current_phase": "verify",
        "orchestrator": {
            "run_id": "run-before-restart",
            "active": true,
            "trigger": "start"
        },
        "control": {
            "paused": true,
            "takeover": true
        },
        "workers": [
            {
                "id": "worker-observe",
                "role": "observer",
                "phase": "observe",
                "status": "done",
                "summary": "Observed before restart"
            },
            {
                "id": "worker-verify",
                "role": "verifier",
                "phase": "verify",
                "status": "running"
            }
        ],
        "worker_results": {
            "observe": {
                "status": "done",
                "summary": "Observed before restart"
            }
        },
        "timeline": [stored_event.clone()]
    });
    let project = state
        .kernel
        .memory
        .project_update(
            &project.id,
            project::ProjectPatch {
                metadata: Some(serde_json::json!({ "runtime": runtime })),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
    state
        .kernel
        .memory
        .append_session_event(
            &project_session_id(&project),
            "project_runtime_event",
            &serde_json::json!({ "event": stored_event }),
        )
        .unwrap();
    let state = std::sync::Arc::new(state);

    let ((status, Json(body)), spawn_key) =
        prepare_start_project_runtime(&state, &project.slug).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(spawn_key.as_deref(), Some("restart-replay"));
    assert_eq!(body["runtime"]["status"], "running");
    assert_eq!(body["runtime"]["current_phase"], "verify");
    assert_eq!(body["runtime"]["orchestrator"]["active"], true);
    assert_eq!(body["runtime"]["control"]["paused"], false);
    assert_eq!(body["runtime"]["control"]["takeover"], false);
    assert_eq!(
        body["runtime"]["worker_results"]["observe"]["summary"],
        "Observed before restart"
    );

    let transcript_events = body["transcript"]["events"].as_array().unwrap();
    assert_eq!(body["transcript"]["stored_count"], 2);
    assert_eq!(body["transcript"]["count"], 2);
    assert!(transcript_events
        .iter()
        .any(|event| event["id"] == "event-before-restart"));
    assert!(transcript_events
        .iter()
        .any(|event| event["kind"] == "orchestrator.resume_after_restart"));
    let persisted = state
        .kernel
        .memory
        .project_get(&project.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        persisted.metadata["runtime"]["orchestrator"]["trigger"],
        "resume_after_restart"
    );
    let encoded = serde_json::to_string(&body).unwrap();
    assert!(!encoded.contains("must-not-leak"));
    assert!(!encoded.contains("\"data\""));
}

#[tokio::test]
async fn start_project_runtime_rejects_invalid_identifier_without_spawn() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);

    let response = start_project_runtime(
        axum::extract::State(state),
        axum::extract::Path("bad-/Users/example/private-ghp_secret".to_string()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response_text(response).await;
    assert!(body.contains("project identifier is invalid"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
}
