use super::*;
use captain_kernel::CaptainKernel;
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
        goal: "Keep runtime transcript bounded".to_string(),
        status: project::ProjectStatus::Active,
        deadline: None,
        created_at: 1,
        updated_at: 2,
        metadata: serde_json::json!({}),
    }
}

#[test]
fn project_runtime_query_events_limit_defaults_and_clamps() {
    assert_eq!(
        project_runtime_transcript_limit(None),
        PROJECT_RUNTIME_TRANSCRIPT_LIMIT
    );
    assert_eq!(project_runtime_transcript_limit(Some(0)), 1);
    assert_eq!(project_runtime_transcript_limit(Some(42)), 42);
    assert_eq!(
        project_runtime_transcript_limit(Some(PROJECT_RUNTIME_TRANSCRIPT_LIMIT + 1)),
        PROJECT_RUNTIME_TRANSCRIPT_LIMIT
    );
}

#[test]
fn project_runtime_transcript_keeps_latest_persisted_events_when_limited() {
    let (_tmp, state) = test_state();
    let project = project();
    let session_id = project_session_id(&project);
    for idx in 0..5 {
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
                        "data": {"secret": "do-not-print"}
                    }
                }),
            )
            .unwrap();
    }
    for idx in 0..3 {
        state
            .kernel
            .memory
            .append_session_event(
                &session_id,
                "unrelated_runtime_event",
                &serde_json::json!({
                    "event": {
                        "id": format!("noise-{idx}"),
                        "ts": format!("2026-05-24T10:01:0{idx}Z"),
                        "title": "Noise"
                    }
                }),
            )
            .unwrap();
    }

    let transcript =
        project_runtime_transcript_with_limit(&state, &project, &serde_json::json!({}), 2);
    let events = transcript["events"].as_array().unwrap();

    assert_eq!(transcript["stored_count"], 5);
    assert_eq!(transcript["limit"], 2);
    assert_eq!(transcript["truncated"], true);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["id"], "event-3");
    assert_eq!(events[1]["id"], "event-4");
    let encoded = serde_json::to_string(&transcript).unwrap();
    assert!(!encoded.contains("do-not-print"));
    assert!(!encoded.contains("\"data\""));
}

#[test]
fn project_runtime_transcript_limits_after_merging_runtime_timeline() {
    let (_tmp, state) = test_state();
    let project = project();
    let session_id = project_session_id(&project);
    for idx in 0..2 {
        state
            .kernel
            .memory
            .append_session_event(
                &session_id,
                "project_runtime_event",
                &serde_json::json!({
                    "event": {
                        "id": format!("persisted-{idx}"),
                        "ts": format!("2026-05-24T10:00:0{idx}Z"),
                        "kind": "worker.note",
                        "title": format!("Persisted {idx}")
                    }
                }),
            )
            .unwrap();
    }
    let runtime = serde_json::json!({
        "timeline": [
            {
                "id": "runtime-old",
                "ts": "2026-05-24T09:59:59Z",
                "kind": "worker.note",
                "title": "Older runtime",
                "data": {"secret": "do-not-print"}
            },
            {
                "id": "runtime-new",
                "ts": "2026-05-24T10:00:02Z",
                "kind": "worker.note",
                "title": "Newer runtime"
            }
        ]
    });

    let transcript = project_runtime_transcript_with_limit(&state, &project, &runtime, 2);
    let events = transcript["events"].as_array().unwrap();

    assert_eq!(transcript["stored_count"], 2);
    assert_eq!(transcript["limit"], 2);
    assert_eq!(transcript["truncated"], true);
    assert_eq!(transcript["count"], 2);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["id"], "persisted-1");
    assert_eq!(events[1]["id"], "runtime-new");
    let encoded = serde_json::to_string(&transcript).unwrap();
    assert!(!encoded.contains("do-not-print"));
    assert!(!encoded.contains("\"data\""));
}
