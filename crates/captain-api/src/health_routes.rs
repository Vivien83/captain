//! Health and metrics route handlers.

use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_types::version::captain_version;
use std::sync::Arc;

fn health_agent_id() -> captain_types::agent::AgentId {
    captain_types::agent::AgentId(uuid::Uuid::from_bytes([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
    ]))
}

async fn memory_health_ok(state: &AppState) -> bool {
    // Run the database check on a blocking thread so we never hold the
    // std::sync::Mutex<Connection> on a tokio worker thread.
    let memory = state.kernel.memory.clone();
    tokio::task::spawn_blocking(move || {
        memory
            .structured_get(health_agent_id(), "__health_check__")
            .is_ok()
    })
    .await
    .unwrap_or(false)
}

fn overall_health_status(database_ok: bool, audit_ok: bool) -> &'static str {
    if database_ok && audit_ok {
        "ok"
    } else {
        "degraded"
    }
}

/// GET /api/health - Minimal liveness probe (public, no auth required).
pub async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let db_ok = memory_health_ok(&state).await;
    let audit = state.kernel.audit_log.integrity_status();
    let status = overall_health_status(db_ok, audit.valid);

    Json(serde_json::json!({
        "status": status,
        "version": captain_version(),
    }))
}

/// GET /api/health/detail - Full health diagnostics (requires auth).
pub async fn health_detail(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let health = state.kernel.supervisor.health();
    let db_ok = memory_health_ok(&state).await;
    let audit = state.kernel.audit_log.integrity_status();
    let config_warnings = state.kernel.config.validate();
    let status = overall_health_status(db_ok, audit.valid);

    Json(serde_json::json!({
        "status": status,
        "version": captain_version(),
        "uptime_seconds": state.started_at.elapsed().as_secs(),
        "failure_count": health.failure_count,
        "panic_count": health.panic_count,
        "restart_count": health.restart_count,
        "agent_count": state.kernel.registry.count(),
        "database": if db_ok { "connected" } else { "error" },
        "audit": audit,
        "execution": crate::security_routes::execution_status(
            &state.kernel.config.exec_policy,
            &state.kernel.config.docker,
        ),
        "config_warnings": config_warnings,
    }))
}

/// GET /api/metrics - Prometheus text-format metrics.
pub async fn prometheus_metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut out = String::with_capacity(2048);

    let uptime = state.started_at.elapsed().as_secs();
    out.push_str("# HELP captain_uptime_seconds Time since daemon started.\n");
    out.push_str("# TYPE captain_uptime_seconds gauge\n");
    out.push_str(&format!("captain_uptime_seconds {uptime}\n\n"));

    let agents = state.kernel.registry.list();
    let active = agents
        .iter()
        .filter(|a| matches!(a.state, captain_types::agent::AgentState::Running))
        .count();
    out.push_str("# HELP captain_agents_active Number of active agents.\n");
    out.push_str("# TYPE captain_agents_active gauge\n");
    out.push_str(&format!("captain_agents_active {active}\n"));
    out.push_str("# HELP captain_agents_total Total number of registered agents.\n");
    out.push_str("# TYPE captain_agents_total gauge\n");
    out.push_str(&format!("captain_agents_total {}\n\n", agents.len()));

    out.push_str("# HELP captain_tokens_total Total tokens consumed (rolling hourly window).\n");
    out.push_str("# TYPE captain_tokens_total gauge\n");
    out.push_str("# HELP captain_tool_calls_total Total tool calls (rolling hourly window).\n");
    out.push_str("# TYPE captain_tool_calls_total gauge\n");
    for agent in &agents {
        let name = &agent.name;
        let provider = &agent.manifest.model.provider;
        let model = &agent.manifest.model.model;
        if let Some((tokens, tools)) = state.kernel.scheduler.get_usage(agent.id) {
            out.push_str(&format!(
                "captain_tokens_total{{agent=\"{name}\",provider=\"{provider}\",model=\"{model}\"}} {tokens}\n"
            ));
            out.push_str(&format!(
                "captain_tool_calls_total{{agent=\"{name}\"}} {tools}\n"
            ));
        }
    }
    out.push('\n');

    let health = state.kernel.supervisor.health();
    append_supervisor_metrics(&mut out, &health);
    append_audit_metrics(&mut out, &state.kernel.audit_log.integrity_status());

    out.push_str("# HELP captain_info Captain version and build info.\n");
    out.push_str("# TYPE captain_info gauge\n");
    out.push_str(&format!(
        "captain_info{{version=\"{}\"}} 1\n",
        captain_version()
    ));

    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        out,
    )
}

fn append_audit_metrics(
    out: &mut String,
    integrity: &captain_runtime::audit::AuditIntegrityStatus,
) {
    out.push_str("# HELP captain_audit_integrity Audit history integrity state (1 healthy).\n");
    out.push_str("# TYPE captain_audit_integrity gauge\n");
    out.push_str(&format!(
        "captain_audit_integrity {}\n",
        u8::from(integrity.valid)
    ));
    out.push_str("# HELP captain_audit_invalid_epochs Number of sealed invalid audit epochs.\n");
    out.push_str("# TYPE captain_audit_invalid_epochs gauge\n");
    out.push_str(&format!(
        "captain_audit_invalid_epochs {}\n",
        integrity.invalid_epochs.len()
    ));
    out.push_str("# HELP captain_audit_active_epoch Current writable audit epoch.\n");
    out.push_str("# TYPE captain_audit_active_epoch gauge\n");
    out.push_str(&format!(
        "captain_audit_active_epoch {}\n\n",
        integrity.active_epoch
    ));
}

fn append_supervisor_metrics(
    out: &mut String,
    health: &captain_kernel::supervisor::SupervisorHealth,
) {
    out.push_str(
        "# HELP captain_agent_failures_total Total recoverable agent failures since start.\n",
    );
    out.push_str("# TYPE captain_agent_failures_total counter\n");
    out.push_str(&format!(
        "captain_agent_failures_total {}\n",
        health.failure_count
    ));
    out.push_str("# HELP captain_panics_total Total supervisor panics since start.\n");
    out.push_str("# TYPE captain_panics_total counter\n");
    out.push_str(&format!("captain_panics_total {}\n", health.panic_count));
    out.push_str("# HELP captain_restarts_total Total supervisor restarts since start.\n");
    out.push_str("# TYPE captain_restarts_total counter\n");
    out.push_str(&format!(
        "captain_restarts_total {}\n\n",
        health.restart_count
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use axum::routing::get;
    use axum::Router;
    use captain_kernel::CaptainKernel;
    use captain_types::config::{DefaultModelConfig, KernelConfig};
    use std::time::Instant;
    use tower::ServiceExt;

    #[test]
    fn supervisor_metrics_keep_recoverable_failures_separate_from_panics() {
        let mut output = String::new();
        append_supervisor_metrics(
            &mut output,
            &captain_kernel::supervisor::SupervisorHealth {
                is_shutting_down: false,
                failure_count: 7,
                panic_count: 2,
                restart_count: 1,
            },
        );

        assert!(output.contains("captain_agent_failures_total 7"));
        assert!(output.contains("captain_panics_total 2"));
        assert!(output.contains("captain_restarts_total 1"));
    }

    #[test]
    fn audit_degradation_changes_overall_health_and_metrics() {
        assert_eq!(overall_health_status(true, true), "ok");
        assert_eq!(overall_health_status(true, false), "degraded");
        assert_eq!(overall_health_status(false, true), "degraded");

        let mut output = String::new();
        append_audit_metrics(
            &mut output,
            &captain_runtime::audit::AuditIntegrityStatus {
                valid: false,
                status: "degraded".to_string(),
                active_epoch: 2,
                active_epoch_valid: true,
                invalid_epochs: vec![0, 1],
                entry_count: 12,
                tip_hash: "a".repeat(64),
                last_error: Some("sealed history".to_string()),
            },
        );
        assert!(output.contains("captain_audit_integrity 0"));
        assert!(output.contains("captain_audit_invalid_epochs 2"));
        assert!(output.contains("captain_audit_active_epoch 2"));
    }

    #[tokio::test]
    async fn tampering_is_exposed_by_health_detail_after_restart() {
        let temp = tempfile::tempdir().unwrap();
        let config = KernelConfig {
            home_dir: temp.path().to_path_buf(),
            data_dir: temp.path().join("data"),
            default_model: DefaultModelConfig {
                provider: "ollama".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OLLAMA_API_KEY".to_string(),
                base_url: None,
            },
            ..KernelConfig::default()
        };
        let first = CaptainKernel::boot_with_config(config.clone()).unwrap();
        first
            .audit_log
            .record(
                "captain",
                captain_runtime::audit::AuditAction::ConfigChange,
                "original detail",
                "ok",
            )
            .unwrap();
        first
            .memory
            .usage_conn()
            .lock()
            .unwrap()
            .execute(
                "UPDATE audit_entries SET detail = 'tampered' WHERE seq = (
                     SELECT MIN(seq) FROM audit_entries
                 )",
                [],
            )
            .unwrap();
        first.shutdown();
        drop(first);

        let kernel = Arc::new(CaptainKernel::boot_with_config(config).unwrap());
        let state = Arc::new(AppState {
            kernel: Arc::clone(&kernel),
            started_at: Instant::now(),
            peer_registry: None,
            bridge_manager: tokio::sync::Mutex::new(None),
            channels_config: tokio::sync::RwLock::new(Default::default()),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            ask_user_channels: dashmap::DashMap::new(),
            provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
        });
        let app = Router::new()
            .route("/api/health/detail", get(health_detail))
            .with_state(state);
        let response = app
            .oneshot(
                Request::get("/api/health/detail")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let payload: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();

        assert_eq!(payload["status"], "degraded");
        assert_eq!(payload["audit"]["valid"], false);
        assert_eq!(payload["audit"]["active_epoch_valid"], true);
        assert_eq!(payload["audit"]["active_epoch"], 1);
        assert_eq!(payload["audit"]["invalid_epochs"], serde_json::json!([0]));
        assert_eq!(payload["execution"]["backend"], "host_process");
        assert_eq!(payload["execution"]["isolation_level"], "environment_scrub");
        assert_eq!(payload["execution"]["os_isolation"], false);
        assert_eq!(payload["execution"]["profile"], "personal_workstation");
        assert_eq!(payload["execution"]["configured_policy_mode"], "allowlist");
        assert_eq!(payload["execution"]["policy_mode"], "allowlist");
        assert_eq!(payload["execution"]["critical_mode"], "safe");
        assert_eq!(payload["execution"]["host_execution_allowed"], true);
        assert_eq!(payload["execution"]["isolation_routing"], "explicit_only");
        assert_eq!(payload["execution"]["docker"]["enabled"], false);
        kernel.shutdown();
    }
}
