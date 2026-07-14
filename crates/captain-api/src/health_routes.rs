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

/// GET /api/health - Minimal liveness probe (public, no auth required).
pub async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let db_ok = memory_health_ok(&state).await;
    let status = if db_ok { "ok" } else { "degraded" };

    Json(serde_json::json!({
        "status": status,
        "version": captain_version(),
    }))
}

/// GET /api/health/detail - Full health diagnostics (requires auth).
pub async fn health_detail(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let health = state.kernel.supervisor.health();
    let db_ok = memory_health_ok(&state).await;
    let config_warnings = state.kernel.config.validate();
    let status = if db_ok { "ok" } else { "degraded" };

    Json(serde_json::json!({
        "status": status,
        "version": captain_version(),
        "uptime_seconds": state.started_at.elapsed().as_secs(),
        "failure_count": health.failure_count,
        "panic_count": health.panic_count,
        "restart_count": health.restart_count,
        "agent_count": state.kernel.registry.count(),
        "database": if db_ok { "connected" } else { "error" },
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
}
