//! Runtime status route handlers.

use crate::agent_api_egress_queue::agent_api_egress_queue_entries;
use crate::channel_readiness::channel_readiness;
use crate::channel_registry::{
    active_channel_names, channel_config_values, is_channel_configured, CHANNEL_REGISTRY,
};
use crate::state::AppState;
use crate::status_agent_api::{build_agent_api_status, unavailable_agent_api_status};
use crate::status_consciousness::build_consciousness_status;
use crate::status_disk::build_disk_status;
use crate::status_payload::{
    build_active_runs, build_deployment_status, build_status_agents, build_status_auth,
    build_status_budget, build_status_media, build_status_workload, status_workspace_dirs,
};
use crate::status_runtime_health::build_runtime_health_status;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use captain_types::version::captain_version;
use std::sync::Arc;

fn build_channel_status(config: &captain_types::config::ChannelsConfig) -> serde_json::Value {
    let mut configured = Vec::new();
    let mut ready = Vec::new();
    let mut locked = Vec::new();
    let mut items = Vec::new();
    for meta in CHANNEL_REGISTRY {
        let is_configured = is_channel_configured(config, meta.name);
        let values = channel_config_values(config, meta.name);
        let readiness = channel_readiness(meta, values.as_ref());
        let name = meta.name.to_string();
        if is_configured {
            configured.push(name.clone());
        }
        if readiness.ready {
            ready.push(name.clone());
        } else if is_configured {
            locked.push(name.clone());
        }
        items.push(serde_json::json!({
            "name": meta.name,
            "configured": is_configured,
            "ready": readiness.ready,
            "security_state": readiness.security_state,
            "missing_required_fields": readiness.missing_required_fields,
            "operator_actions": readiness.operator_actions,
        }));
    }

    serde_json::json!({
        "active": active_channel_names(),
        "total": CHANNEL_REGISTRY.len(),
        "configured_count": configured.len(),
        "ready_count": ready.len(),
        "configured": configured,
        "ready": ready,
        "locked": locked,
        "items": items,
    })
}
fn empty_channel_inbound_queue_status() -> serde_json::Value {
    serde_json::json!({"bridge_running": false, "active_sessions": 0, "pending_sessions": 0, "pending_messages": 0, "inflight_sessions": 0, "inflight_messages": 0, "dead_letter_sessions": 0, "dead_letter_messages": 0, "dead_letter_oldest_age_secs": null, "interjected_sessions": 0, "interjected_messages": 0, "operator_actions": [], "channels": []})
}

fn status_channel_names(status: &serde_json::Value, key: &str) -> Vec<String> {
    status
        .get(key)
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// GET /api/status - Kernel status.
pub async fn status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = &state.kernel.config;
    let agents = build_status_agents(state.as_ref());

    let uptime = state.started_at.elapsed().as_secs();
    let agent_count = agents.len();
    let default_model = state.kernel.effective_default_model();
    let (llm_driver_ready, llm_driver_error) = state.kernel.default_llm_driver_status();
    let (workspaces_dir, workflows_dir) = status_workspace_dirs(config);
    let channel_status = build_channel_status_with_queue(state.as_ref()).await;
    let configured_channels = status_channel_names(&channel_status, "configured");

    let auth = build_status_auth(config);
    let media = build_status_media(config, state.kernel.tts_engine.config_snapshot());
    let now = chrono::Utc::now();
    let active_runs = build_active_runs(state.as_ref(), now);
    let registry_entries = state.kernel.registry.list();
    let (active_processes, active_process_count) =
        crate::status_processes::build_active_process_status(&state.kernel, &registry_entries);
    let shutdown_status = crate::shutdown_guard::shutdown_drain_state().status_json(
        crate::shutdown_guard::ActiveShutdownWork::new(active_runs.len(), active_process_count),
    );
    let workload_snapshot = build_status_workload(state.as_ref(), now);
    let workload_status = workload_snapshot.workload;
    let budget_status = build_status_budget(state.as_ref());
    let consciousness_status = build_consciousness_status(
        &state.kernel,
        active_runs.len(),
        active_processes.len(),
        workload_snapshot.goal_active,
        workload_snapshot.goal_escalated,
        &workload_snapshot.all_project_attention,
    );
    let agent_api_status = match agent_api_egress_queue_entries(&config.home_dir).await {
        Ok(entries) => build_agent_api_status(&entries, now),
        Err(err) => unavailable_agent_api_status(err),
    };
    let disk_status = build_disk_status(&config.home_dir);
    let runtime_health_status = build_runtime_health_status(
        llm_driver_ready,
        &channel_status,
        &workload_status,
        &agent_api_status,
        &consciousness_status,
        &disk_status,
        &shutdown_status,
        &budget_status,
    );

    let mut status_payload = serde_json::json!({
        "status": "running",
        "version": captain_version(),
        "agent_count": agent_count,
        "active_run_count": active_runs.len(),
        "active_runs": active_runs,
        "process_count": active_processes.len(),
        "active_processes": active_processes,
        "default_provider": default_model.provider,
        "default_model": default_model.model,
        "llm_driver_ready": llm_driver_ready,
        "llm_driver_error": llm_driver_error,
        "fallback_provider_count": config.fallback_providers.len(),
        "uptime_seconds": uptime,
        "timezone": config.timezone,
        "api_listen": config.api_listen,
        "home_dir": config.home_dir.display().to_string(),
        "data_dir": config.data_dir.display().to_string(),
        "config_path": config.home_dir.join("config.toml").display().to_string(),
        "log_file": config.home_dir.join("captain.log").display().to_string(),
        "deployment": build_deployment_status(config),
        "workspaces_dir": workspaces_dir.display().to_string(),
        "workflows_dir": workflows_dir.display().to_string(),
        "sessions_dir": config.home_dir.join("sessions").display().to_string(),
        "log_level": config.log_level,
        "network_enabled": config.network_enabled,
        "auth_enabled": auth.auth_enabled,
        "auth_mode": auth.auth_mode,
        "api_key_configured": auth.api_key_configured,
        "session_auth_enabled": auth.session_auth_enabled,
        "channel_total": CHANNEL_REGISTRY.len(),
        "channel_configured_count": configured_channels.len(),
        "configured_channels": configured_channels,
        "channels": channel_status,
        "tts": media.tts,
        "media": media.media,
        "native_voice": media.native_voice,
        "native_embeddings": media.native_embeddings,
        "workload": workload_status,
        "agent_api": agent_api_status,
        "consciousness": consciousness_status,
        "agents": agents,
    });
    status_payload["shutdown"] = shutdown_status;
    status_payload["disk"] = disk_status;
    status_payload["budget"] = budget_status;
    status_payload["streaming"] = crate::stream_metrics::status_json();
    status_payload["tool_runs"] = captain_runtime::tool_runs::global_registry().status_summary();
    status_payload["runtime_health"] = runtime_health_status;
    Json(status_payload)
}

async fn build_channel_status_with_queue(state: &AppState) -> serde_json::Value {
    let mut channel_status = {
        let live_channels = state.channels_config.read().await;
        build_channel_status(&live_channels)
    };
    let inbound_queue_status = {
        let bridge = state.bridge_manager.lock().await;
        bridge
            .as_ref()
            .map(|manager| manager.inbound_queue_status())
            .unwrap_or_else(empty_channel_inbound_queue_status)
    };
    if let Some(channels) = channel_status.as_object_mut() {
        channels.insert("inbound_queue".to_string(), inbound_queue_status);
    }
    channel_status
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::config::{ChannelsConfig, EmailConfig, TelegramConfig};

    #[test]
    fn channel_status_marks_configured_but_locked_channels() {
        let status = build_channel_status(&ChannelsConfig {
            telegram: Some(TelegramConfig::default()),
            ..Default::default()
        });

        assert_eq!(status["total"], serde_json::json!(4));
        assert_eq!(status["configured"], serde_json::json!(["telegram"]));
        assert_eq!(status["ready"], serde_json::json!([]));
        assert_eq!(status["locked"], serde_json::json!(["telegram"]));
    }

    #[test]
    fn channel_status_treats_email_allowed_senders_as_ready_gate() {
        unsafe {
            std::env::set_var("EMAIL_PASSWORD", "secret");
        }
        let status = build_channel_status(&ChannelsConfig {
            email: Some(EmailConfig {
                username: "captain@example.com".to_string(),
                imap_host: "imap.example.com".to_string(),
                smtp_host: "smtp.example.com".to_string(),
                allowed_senders: vec!["operator@example.com".to_string()],
                ..Default::default()
            }),
            ..Default::default()
        });

        assert_eq!(status["configured"], serde_json::json!(["email"]));
        assert_eq!(status["ready"], serde_json::json!(["email"]));
        assert_eq!(status["locked"], serde_json::json!([]));
        unsafe {
            std::env::remove_var("EMAIL_PASSWORD");
        }
    }

    #[test]
    fn empty_channel_inbound_queue_status_is_operator_safe() {
        let status = empty_channel_inbound_queue_status();
        assert_eq!(status["bridge_running"], serde_json::json!(false));
        assert_eq!(status["active_sessions"], serde_json::json!(0));
        assert_eq!(status["pending_messages"], serde_json::json!(0));
        assert_eq!(status["inflight_messages"], serde_json::json!(0));
        assert_eq!(status["dead_letter_messages"], serde_json::json!(0));
        assert_eq!(
            status["dead_letter_oldest_age_secs"],
            serde_json::json!(null)
        );
        assert_eq!(status["interjected_messages"], serde_json::json!(0));
        assert_eq!(status["operator_actions"], serde_json::json!([]));
        assert_eq!(status["channels"], serde_json::json!([]));
    }
}
