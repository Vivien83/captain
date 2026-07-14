use super::{enrich_project, project_runtime_response};
use crate::routes::AppState;
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

fn project() -> project::Project {
    project::Project {
        id: "project-1".to_string(),
        name: "Demo".to_string(),
        slug: "demo".to_string(),
        goal: "Keep runtime response operator-safe".to_string(),
        status: project::ProjectStatus::Active,
        deadline: None,
        created_at: 1,
        updated_at: 2,
        metadata: serde_json::json!({
            "runtime": {
                "status": "ready",
                "current_phase": "observe",
                "timeline": [{
                    "id": "event-1",
                    "title": "Visible event",
                    "data": {"secret": "raw-event-secret"}
                }]
            },
            "workspace": {"path": "/private/project-path"}
        }),
    }
}

#[test]
fn project_runtime_response_omits_chat_agent_id_and_raw_payloads() {
    let (_tmp, state) = test_state();
    let response = project_runtime_response(&state, &project());

    assert!(response.get("chat").is_none());
    assert_eq!(response["project"]["slug"], "demo");
    assert!(response["runtime"]["timeline"][0].get("data").is_none());

    let encoded = serde_json::to_string(&response).unwrap();
    assert!(!encoded.contains("\"chat\""));
    assert!(!encoded.contains("raw-event-secret"));
    assert!(!encoded.contains("/private/project-path"));
}

#[test]
fn enrich_project_omits_metadata_and_raw_runtime_payloads() {
    let (_tmp, state) = test_state();
    let project = project::Project {
        metadata: serde_json::json!({
            "runtime": {
                "status": "running",
                "current_phase": "build",
                "workers": [{
                    "id": "worker-1",
                    "phase": "build",
                    "prompt": "raw worker prompt secret"
                }]
            },
            "source": {
                "type": "github",
                "full_name": "owner/repo",
                "path": "/private/source-path",
                "local_path": "/private/local-path",
                "clone_url": "https://token-secret@example.test/owner/repo.git",
                "unexpected_secret": "source-secret"
            },
            "workspace": {
                "path": "/tmp/demo",
                "default_root": "/private/default-root",
                "unexpected_secret": "workspace-secret"
            },
            "unexpected_secret": "metadata-secret"
        }),
        ..project()
    };

    let view = enrich_project(&state, project);

    assert!(view.get("metadata").is_none());
    assert_eq!(view["source"]["type"], "github");
    assert_eq!(view["source"]["full_name"], "owner/repo");
    assert!(view["source"].get("path").is_none());
    assert!(view["source"].get("local_path").is_none());
    assert!(view["workspace"].get("path").is_none());
    assert!(view["workspace"].get("default_root").is_none());
    assert!(view.get("workspace_path").is_none());
    assert_eq!(view["runtime"]["status"], "running");
    assert!(view["runtime"]["workers"][0].get("prompt").is_none());

    let encoded = serde_json::to_string(&view).unwrap();
    for forbidden in [
        "raw worker prompt secret",
        "token-secret",
        "source-secret",
        "/private/source-path",
        "/private/local-path",
        "workspace-secret",
        "/tmp/demo",
        "/private/default-root",
        "metadata-secret",
        "clone_url",
        "unexpected_secret",
    ] {
        assert!(!encoded.contains(forbidden), "leaked {forbidden}");
    }
}
