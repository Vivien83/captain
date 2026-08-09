use super::*;
use axum::{body::to_bytes, http::Request, Router};
use captain_kernel::CaptainKernel;
use captain_runtime::audit::AuditAction;
use captain_types::config::{DefaultModelConfig, KernelConfig};
use std::time::Instant;
use tower::ServiceExt;

fn test_state() -> (tempfile::TempDir, Arc<AppState>) {
    let temp = tempfile::tempdir().unwrap();
    let kernel = Arc::new(
        CaptainKernel::boot_with_config(KernelConfig {
            home_dir: temp.path().join("home"),
            data_dir: temp.path().join("data"),
            default_model: DefaultModelConfig {
                provider: "ollama".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OLLAMA_API_KEY".to_string(),
                base_url: None,
            },
            ..KernelConfig::default()
        })
        .unwrap(),
    );
    let state = Arc::new(AppState {
        kernel,
        started_at: Instant::now(),
        peer_registry: None,
        bridge_manager: tokio::sync::Mutex::new(None),
        channels_config: tokio::sync::RwLock::new(Default::default()),
        shutdown_notify: Arc::new(tokio::sync::Notify::new()),
        ask_user_channels: dashmap::DashMap::new(),
        provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
    });
    (temp, state)
}

async fn json_body(response: Response<Body>) -> serde_json::Value {
    serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
}

#[test]
fn operator_projection_never_serializes_results_inputs_or_paths() {
    let registry = captain_runtime::tool_runs::ToolRunRegistry::default();
    let run_id = registry.start(
        "shell_exec",
        Some("captain".to_string()),
        Some("tool-use-1".to_string()),
        false,
        Some("a".repeat(64)),
    );
    registry.append_chunk(&run_id, "stdout", "password=do-not-expose\n");
    let projection = OperatorToolRun::from(registry.snapshot(&run_id).unwrap());
    let value = serde_json::to_value(projection).unwrap();
    let object = value.as_object().unwrap();

    assert_eq!(object["input_sha256"], "a".repeat(64));
    for forbidden in ["result", "result_preview", "input", "path", "file_name"] {
        assert!(!object.contains_key(forbidden), "leaked field {forbidden}");
    }
    assert!(!value.to_string().contains("do-not-expose"));
}

#[test]
fn operator_tail_keeps_utf8_suffix_bounded_and_fails_closed_on_secret() {
    let long = format!("prefix\n{}", "é".repeat(MAX_TAIL_BYTES));
    let tail = operator_tail(
        "toolrun-long",
        ToolRunStatus::Running,
        ToolRunOutputPage {
            start_line: 1,
            end_line: 2,
            total_lines: 2,
            content: long,
        },
    );
    assert!(tail.content_truncated);
    assert!(tail.content.len() <= MAX_TAIL_BYTES);
    assert!(!tail.content_withheld);

    let withheld = operator_tail(
        "toolrun-secret",
        ToolRunStatus::Running,
        ToolRunOutputPage {
            start_line: 1,
            end_line: 1,
            total_lines: 1,
            content: "Bearer abcdefghijklmnopqrstuvwxyz123456".to_string(),
        },
    );
    assert!(withheld.content_withheld);
    assert_eq!(withheld.content, WITHHELD_TAIL);
    assert!(!withheld.content.contains("abcdefghijklmnopqrstuvwxyz"));
}

#[test]
fn run_id_validation_accepts_only_bounded_runtime_ids() {
    assert!(valid_run_id("toolrun-crashed"));
    assert!(valid_run_id("toolrun-01234567-89ab-cdef-0123-456789abcdef"));
    assert!(!valid_run_id("not-a-tool-run"));
    assert!(!valid_run_id("toolrun-../../private"));
    assert!(!valid_run_id(&format!("toolrun-{}", "a".repeat(96))));
}

#[tokio::test]
async fn mounted_routes_project_tail_and_strictly_audit_cancellation() {
    let (_temp, state) = test_state();
    let kernel = Arc::clone(&state.kernel);
    let app = crate::server_observability_routes::mount_observability_routes(Router::new())
        .with_state(state);
    let registry = global_registry();

    let foreground = registry.start(
        "shell_exec",
        Some("captain".to_string()),
        Some("tool-use-api".to_string()),
        false,
        Some("b".repeat(64)),
    );
    registry.append_chunk(
        &foreground,
        "stdout",
        "safe line\npassword=mounted-secret-value\n",
    );

    let list = app
        .clone()
        .oneshot(
            Request::get("/api/tool-runs?status=running&limit=200")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let list = json_body(list).await;
    assert!(list["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["run_id"] == foreground));

    let detail = app
        .clone()
        .oneshot(
            Request::get(format!("/api/tool-runs/{foreground}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
    let detail = json_body(detail).await;
    assert!(detail["run"].get("result_preview").is_none());
    assert!(detail["run"].get("result").is_none());
    assert!(detail["run"].get("input").is_none());

    let tail = app
        .clone()
        .oneshot(
            Request::get(format!("/api/tool-runs/{foreground}/tail?max_lines=20"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(tail.status(), StatusCode::OK);
    let tail = json_body(tail).await;
    assert!(tail["tail"]["content"]
        .as_str()
        .unwrap()
        .contains("[REDACTED]"));
    assert!(!tail.to_string().contains("mounted-secret-value"));
    assert!(tail["tail"]["content_bytes"].as_u64().unwrap() <= MAX_TAIL_BYTES as u64);

    let rejected = app
        .clone()
        .oneshot(
            Request::post(format!("/api/tool-runs/{foreground}/cancel"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::CONFLICT);
    assert_eq!(
        registry.snapshot(&foreground).unwrap().status,
        ToolRunStatus::Running
    );

    let cancellable = registry.start("shell_exec", None, None, true, None);
    let task = tokio::spawn(std::future::pending::<()>());
    registry.attach_abort_handle(&cancellable, task.abort_handle());
    let cancelled = app
        .clone()
        .oneshot(
            Request::post(format!("/api/tool-runs/{cancellable}/cancel"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancelled.status(), StatusCode::OK);
    let cancelled_body = json_body(cancelled).await;
    assert!(cancelled_body["run"].get("result").is_none());
    assert!(cancelled_body["run"].get("result_preview").is_none());
    assert!(task.await.unwrap_err().is_cancelled());

    registry.finish(
        &foreground,
        &captain_types::tool::ToolResult {
            tool_use_id: "tool-use-api".to_string(),
            content: "done".to_string(),
            is_error: false,
            transient_content: Vec::new(),
        },
    );
    let terminal = app
        .clone()
        .oneshot(
            Request::post(format!("/api/tool-runs/{foreground}/cancel"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(terminal.status(), StatusCode::CONFLICT);

    let invalid = app
        .clone()
        .oneshot(
            Request::get("/api/tool-runs/not-a-tool-run")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let missing = app
        .oneshot(
            Request::post("/api/tool-runs/toolrun-unknown/cancel")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let audit = kernel.audit_log.recent(10);
    assert!(audit
        .iter()
        .filter(|entry| entry.detail.starts_with("tool_run_cancel run_id="))
        .all(|entry| entry.agent_id == "operator:api"));
    assert!(audit.iter().any(|entry| {
        entry.action == AuditAction::ToolInvoke
            && entry.detail == format!("tool_run_cancel run_id={foreground}")
            && entry.outcome == "not_cancellable"
    }));
    assert!(audit.iter().any(|entry| {
        entry.detail == format!("tool_run_cancel run_id={cancellable}")
            && entry.outcome == "cancelled"
    }));
    assert!(audit.iter().any(|entry| {
        entry.detail == format!("tool_run_cancel run_id={foreground}")
            && entry.outcome == "not_active"
    }));
    assert!(audit.iter().any(|entry| {
        entry.detail == "tool_run_cancel run_id=toolrun-unknown" && entry.outcome == "not_found"
    }));
    kernel.shutdown();
}
