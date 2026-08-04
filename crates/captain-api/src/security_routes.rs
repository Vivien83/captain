//! Security status route handlers.

use crate::state::AppState;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use captain_types::config::{DockerSandboxConfig, ExecPolicy};
use std::sync::Arc;

pub(crate) fn execution_status(
    policy: &ExecPolicy,
    docker: &DockerSandboxConfig,
) -> serde_json::Value {
    let mut status = serde_json::to_value(policy.host_execution_posture()).unwrap_or_else(|_| {
        serde_json::json!({
            "profile": "unknown",
            "backend": "host_process",
            "isolation_level": "unknown",
            "os_isolation": false,
        })
    });
    status["docker"] =
        serde_json::to_value(docker.isolation_posture(policy.profile)).unwrap_or_default();
    status
}

/// GET /api/security - Security feature status.
pub async fn security_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let auth_mode = match (
        state.kernel.config.api_key.trim().is_empty(),
        state.kernel.config.auth.enabled,
        state.kernel.config.auth.allow_unauthenticated_loopback,
    ) {
        (false, true, _) => "api_key+session",
        (false, false, _) => "api_key",
        (true, true, _) => "session",
        (true, false, true) => "unauthenticated_loopback",
        (true, false, false) => "unconfigured",
    };
    let audit = state.kernel.audit_log.integrity_status();
    let execution = execution_status(
        &state.kernel.config.exec_policy,
        &state.kernel.config.docker,
    );

    Json(serde_json::json!({
        "core_protections": {
            "path_traversal": true,
            "ssrf_protection": true,
            "capability_system": true,
            "privilege_escalation_prevention": true,
            "subprocess_environment_scrub": true,
            "subprocess_os_isolation": false,
            "security_headers": true,
            "wire_hmac_auth": true,
            "request_id_tracking": true
        },
        "execution": execution,
        "configurable": {
            "rate_limiter": {
                "enabled": true,
                "tokens_per_minute": 500,
                "algorithm": "GCRA"
            },
            "websocket_limits": {
                "max_per_ip": 5,
                "idle_timeout_secs": 1800,
                "max_message_size": 65536,
                "max_messages_per_minute": 10
            },
            "wasm_sandbox": {
                "fuel_metering": true,
                "epoch_interruption": true,
                "default_timeout_secs": 30,
                "default_fuel_limit": 1_000_000u64
            },
            "auth": {
                "mode": auth_mode,
                "api_key_set": !state.kernel.config.api_key.trim().is_empty(),
                "session_auth_enabled": state.kernel.config.auth.enabled,
                "unauthenticated_loopback_allowed":
                    state.kernel.config.auth.allow_unauthenticated_loopback
            }
        },
        "monitoring": {
            "audit_trail": {
                "enabled": true,
                "algorithm": "versioned SHA-256 hash chain",
                "entry_count": audit.entry_count,
                "integrity": audit.status,
                "active_epoch": audit.active_epoch,
                "active_epoch_valid": audit.active_epoch_valid,
                "invalid_epochs": audit.invalid_epochs
            },
            "content_pattern_guards": {
                "enabled": true,
                "mode": "heuristic",
                "provenance_tracking": false,
                "protected_sinks": ["shell_command", "network_url", "browser_navigation"]
            },
            "manifest_signing": {
                "algorithm": "Ed25519",
                "available": true
            }
        },
        "secret_zeroization": true,
        "total_features": 16
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::config::{CriticalMode, ExecSecurityMode, ExecutionProfile};

    #[test]
    fn host_execution_status_is_explicit_about_missing_os_isolation() {
        let status = execution_status(&ExecPolicy::default(), &DockerSandboxConfig::default());

        assert_eq!(
            status["profile"],
            ExecutionProfile::PersonalWorkstation.as_str()
        );
        assert_eq!(status["backend"], "host_process");
        assert_eq!(status["isolation_level"], "environment_scrub");
        assert_eq!(status["os_isolation"], false);
        assert_eq!(status["environment_scrub"], true);
        assert_eq!(
            status["dangerous_command_guard"],
            "normalized_lexical_heuristic"
        );
        assert_eq!(
            status["configured_policy_mode"],
            ExecSecurityMode::Allowlist.as_str()
        );
        assert_eq!(status["policy_mode"], ExecSecurityMode::Allowlist.as_str());
        assert_eq!(status["critical_mode"], CriticalMode::Safe.as_str());
        assert_eq!(status["host_execution_allowed"], true);
        assert_eq!(status["isolation_routing"], "explicit_only");
        assert_eq!(
            status["docker"]["runtime_availability"],
            "checked_on_invocation"
        );
        assert_eq!(status["docker"]["enabled"], false);
        assert!(status.get("subprocess_isolation").is_none());
    }
}
