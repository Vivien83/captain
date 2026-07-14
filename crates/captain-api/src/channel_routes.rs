//! Active channel configuration route handlers.

use crate::channel_config_store::{remove_channel_config, upsert_channel_config};
use crate::channel_readiness::channel_readiness;
use crate::channel_registry::{
    active_channel_names, build_field_json, channel_config_values, field_is_ready,
    find_channel_meta, is_channel_configured, is_frozen_channel, FieldType, CHANNEL_REGISTRY,
    FROZEN_CHANNEL_NAMES,
};
use crate::channel_test_delivery::send_channel_test_message;
use crate::secret_env::{remove_secret_env, write_secret_env};
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path as FsPath;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct ClearInboundDeadLettersQuery {
    channel: Option<String>,
}

/// GET /api/channels - List active channel adapters with status and field metadata.
pub async fn list_channels(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let live_channels = state.channels_config.read().await;
    let mut channels = Vec::new();
    let mut configured_count = 0u32;

    for meta in CHANNEL_REGISTRY {
        let configured = is_channel_configured(&live_channels, meta.name);
        configured_count += u32::from(configured);
        let config_values = channel_config_values(&live_channels, meta.name);
        let fields: Vec<serde_json::Value> = meta
            .fields
            .iter()
            .map(|field| build_field_json(field, config_values.as_ref()))
            .collect();
        let readiness = channel_readiness(meta, config_values.as_ref());

        channels.push(serde_json::json!({
            "name": meta.name,
            "display_name": meta.display_name,
            "icon": meta.icon,
            "description": meta.description,
            "category": "messaging",
            "difficulty": meta.difficulty,
            "setup_time": meta.setup_time,
            "quick_setup": meta.quick_setup,
            "setup_type": "form",
            "configured": configured,
            "ready": readiness.ready,
            "has_token": readiness.has_required_secrets,
            "has_required_secrets": readiness.has_required_secrets,
            "missing_required_fields": readiness.missing_required_fields,
            "operator_actions": readiness.operator_actions,
            "security_state": readiness.security_state,
            "fields": fields,
            "setup_steps": meta.setup_steps,
            "operator_notes": meta.operator_notes,
            "config_template": meta.config_template,
        }));
    }

    Json(serde_json::json!({
        "channels": channels,
        "total": channels.len(),
        "configured_count": configured_count,
        "active_channels": active_channel_names(),
        "active_only": true,
        "frozen_channels": {
            "hidden": true,
            "count": FROZEN_CHANNEL_NAMES.len(),
            "state": "frozen",
            "reason": frozen_channel_reason(),
        },
    }))
}

/// POST /api/channels/{name}/configure - Save channel secrets and config fields.
pub async fn configure_channel(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let meta = match find_channel_meta(&name) {
        Some(meta) => meta,
        None => return unknown_channel(&name),
    };
    let fields = match body.get("fields").and_then(|value| value.as_object()) {
        Some(fields) => fields,
        None => return bad_request("Missing 'fields' object"),
    };

    let home = captain_kernel::config::captain_home();
    let secrets_path = home.join("secrets.env");
    let config_path = home.join("config.toml");
    let config_fields = match collect_channel_config_fields(meta, fields, &secrets_path) {
        Ok(fields) => fields,
        Err(e) => return server_error(e),
    };

    if let Err(e) = upsert_channel_config(&config_path, &name, &config_fields) {
        return server_error(format!("Failed to write config: {e}"));
    }

    match crate::channel_bridge::reload_channels_from_disk(&state).await {
        Ok(started) => {
            let activated = started
                .iter()
                .any(|channel| channel.eq_ignore_ascii_case(&name));
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "configured",
                    "channel": name,
                    "activated": activated,
                    "started_channels": started,
                })),
            )
        }
        Err(e) => {
            tracing::warn!(error = %e, "Channel hot-reload failed after configure");
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "configured",
                    "channel": name,
                    "activated": false,
                    "note": format!("Configured, but hot-reload failed: {e}. Restart daemon to activate.")
                })),
            )
        }
    }
}

/// DELETE /api/channels/{name}/configure - Remove channel secrets and config.
pub async fn remove_channel(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let meta = match find_channel_meta(&name) {
        Some(meta) => meta,
        None => return unknown_channel(&name),
    };
    let home = captain_kernel::config::captain_home();
    let secrets_path = home.join("secrets.env");
    let config_path = home.join("config.toml");

    for field in meta.fields {
        if let Some(env_var) = field.env_var {
            let _ = remove_secret_env(&secrets_path, env_var);
            unsafe {
                std::env::remove_var(env_var);
            }
        }
    }
    if let Err(e) = remove_channel_config(&config_path, &name) {
        return server_error(format!("Failed to remove config: {e}"));
    }

    match crate::channel_bridge::reload_channels_from_disk(&state).await {
        Ok(started) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "removed",
                "channel": name,
                "remaining_channels": started,
            })),
        ),
        Err(e) => {
            tracing::warn!(error = %e, "Channel hot-reload failed after remove");
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "removed",
                    "channel": name,
                    "note": format!("Removed, but hot-reload failed: {e}. Restart daemon to fully deactivate.")
                })),
            )
        }
    }
}

/// POST /api/channels/{name}/test - Connectivity check and optional test message.
pub async fn test_channel(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    raw_body: Bytes,
) -> impl IntoResponse {
    let meta = match find_channel_meta(&name) {
        Some(meta) => meta,
        None => return unknown_channel_status(&name),
    };
    let live_channels = state.channels_config.read().await;
    let config_values = channel_config_values(&live_channels, &name);
    let missing = meta
        .fields
        .iter()
        .filter(|field| !field_is_ready(field, config_values.as_ref()))
        .map(|field| field.env_var.unwrap_or(field.key))
        .collect::<Vec<_>>();
    drop(live_channels);

    if !missing.is_empty() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "error",
                "message": format!("Missing required fields: {}", missing.join(", "))
            })),
        );
    }

    let body: serde_json::Value = if raw_body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&raw_body).unwrap_or(serde_json::Value::Null)
    };
    let target = body
        .get("channel_id")
        .or_else(|| body.get("chat_id"))
        .or_else(|| body.get("recipient"))
        .and_then(|value| value.as_str());

    if let Some(target_id) = target {
        return match send_channel_test_message(&name, target_id, config_values.as_ref()).await {
            Ok(()) => (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "message": "Test message sent."})),
            ),
            Err(e) => (
                StatusCode::OK,
                Json(serde_json::json!({"status": "error", "message": e})),
            ),
        };
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "message": format!("{} is configured. Provide channel_id, chat_id, or recipient to send a test message.", meta.display_name)
        })),
    )
}

/// POST /api/channels/reload - Manually trigger channel hot-reload from config.
pub async fn reload_channels(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match crate::channel_bridge::reload_channels_from_disk(&state).await {
        Ok(started) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "started": started})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"status": "error", "error": e})),
        ),
    }
}

/// DELETE /api/channels/inbound-queue/dead-letters - Clear handled dead letters.
pub async fn clear_inbound_dead_letters(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ClearInboundDeadLettersQuery>,
) -> impl IntoResponse {
    let channel = query
        .channel
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);
    let bridge = state.bridge_manager.lock().await;
    let Some(manager) = bridge.as_ref() else {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "unavailable",
                "bridge_running": false,
                "cleared_dead_letter_sessions": 0,
                "cleared_dead_letter_messages": 0,
                "message": "Channel bridge is not running; no inbound dead letters are loaded."
            })),
        );
    };

    let body = manager.clear_inbound_dead_letters(channel.as_deref());
    crate::channel_audit::record_inbound_dead_letters_cleared(
        state.kernel.audit_log.as_ref(),
        channel.as_deref(),
        body["cleared_dead_letter_sessions"].as_u64().unwrap_or(0),
        body["cleared_dead_letter_messages"].as_u64().unwrap_or(0),
        body["remaining_dead_letter_messages"].as_u64().unwrap_or(0),
    );

    (StatusCode::OK, Json(body))
}

fn collect_channel_config_fields(
    meta: &crate::channel_registry::ChannelMeta,
    fields: &serde_json::Map<String, serde_json::Value>,
    secrets_path: &FsPath,
) -> Result<HashMap<String, (String, FieldType)>, String> {
    let mut config_fields = HashMap::new();

    for field in meta.fields {
        let value = fields
            .get(field.key)
            .and_then(|field_value| field_value.as_str())
            .unwrap_or("");
        if value.is_empty() {
            continue;
        }
        if let Some(env_var) = field.env_var {
            write_secret_env(secrets_path, env_var, value)
                .map_err(|e| format!("Failed to write secret: {e}"))?;
            unsafe {
                std::env::set_var(env_var, value);
            }
            config_fields.insert(
                field.key.to_string(),
                (env_var.to_string(), FieldType::Text),
            );
        } else {
            config_fields.insert(field.key.to_string(), (value.to_string(), field.field_type));
        }
    }

    Ok(config_fields)
}

fn bad_request(message: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": message})),
    )
}

fn server_error(message: String) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": message})),
    )
}

fn unknown_channel(name: &str) -> (StatusCode, Json<serde_json::Value>) {
    if is_frozen_channel(name) {
        return frozen_channel_response(name, "error");
    }
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "error": "Unknown channel",
            "active_channels": active_channel_names(),
        })),
    )
}

fn unknown_channel_status(name: &str) -> (StatusCode, Json<serde_json::Value>) {
    if is_frozen_channel(name) {
        return frozen_channel_response(name, "error");
    }
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "status": "error",
            "message": "Unknown channel",
            "active_channels": active_channel_names(),
        })),
    )
}

pub(crate) fn frozen_channel_response(
    name: &str,
    status: &'static str,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::GONE,
        Json(serde_json::json!({
            "status": status,
            "error": "Channel is frozen",
            "channel": name,
            "state": "frozen",
            "message": frozen_channel_reason(),
            "active_channels": active_channel_names(),
        })),
    )
}

pub(crate) fn frozen_channel_reason() -> &'static str {
    "Non-core channels are frozen from the active setup surface until Captain's core is Hermes-level. Use Telegram, Discord, Signal, Email, web, CLI, or the per-agent API."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_channel_returns_actionable_response() {
        let (status, Json(body)) = unknown_channel("slack");

        assert_eq!(status, StatusCode::GONE);
        assert_eq!(body["state"], "frozen");
        assert_eq!(body["channel"], "slack");
        assert_eq!(
            body["active_channels"],
            serde_json::json!(["telegram", "discord", "signal", "email"])
        );
    }

    #[test]
    fn unknown_channel_stays_not_found() {
        let (status, Json(body)) = unknown_channel("made-up");

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "Unknown channel");
    }

    #[test]
    fn email_config_stores_password_as_secret_env_pointer() {
        let meta = find_channel_meta("email").unwrap();
        let temp = tempfile::tempdir().unwrap();
        let secrets_path = temp.path().join("secrets.env");
        let fields = serde_json::json!({
            "username": "captain@example.com",
            "password_env": "app-secret",
            "imap_host": "imap.example.com",
            "smtp_host": "smtp.example.com",
            "allowed_senders": "operator@example.com"
        });
        let fields = fields.as_object().unwrap();

        let config_fields =
            collect_channel_config_fields(meta, fields, &secrets_path).expect("fields");

        assert_eq!(
            config_fields.get("password_env"),
            Some(&(String::from("EMAIL_PASSWORD"), FieldType::Text))
        );
        assert_eq!(
            config_fields.get("username"),
            Some(&(String::from("captain@example.com"), FieldType::Text))
        );
        let secrets = std::fs::read_to_string(secrets_path).unwrap();
        assert!(secrets.contains("EMAIL_PASSWORD=app-secret"));
        unsafe {
            std::env::remove_var("EMAIL_PASSWORD");
        }
    }

    #[test]
    fn email_config_rejects_multiline_secret() {
        let meta = find_channel_meta("email").unwrap();
        let temp = tempfile::tempdir().unwrap();
        let secrets_path = temp.path().join("secrets.env");
        let fields = serde_json::json!({
            "password_env": "app-secret\nINJECTED=value"
        });
        let fields = fields.as_object().unwrap();

        let err = collect_channel_config_fields(meta, fields, &secrets_path).unwrap_err();

        assert!(err.contains("single line"));
    }
}
