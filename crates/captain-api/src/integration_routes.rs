use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::collections::HashMap;
use std::sync::Arc;

/// GET /api/integrations - List installed integrations with status.
pub async fn list_integrations(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let registry = state
        .kernel
        .extension_registry
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let health = &state.kernel.extension_health;

    let mut entries = Vec::new();
    for info in registry.list_all_info() {
        let h = health.get_health(&info.template.id);
        let status = match &info.installed {
            Some(inst) if !inst.enabled => "disabled",
            Some(_) => match h.as_ref().map(|h| &h.status) {
                Some(captain_extensions::IntegrationStatus::Ready) => "ready",
                Some(captain_extensions::IntegrationStatus::Error(_)) => "error",
                _ => "installed",
            },
            None => continue,
        };
        entries.push(serde_json::json!({
            "id": info.template.id,
            "name": info.template.name,
            "icon": info.template.icon,
            "category": info.template.category.to_string(),
            "status": status,
            "tool_count": h.as_ref().map(|h| h.tool_count).unwrap_or(0),
            "installed_at": info.installed.as_ref().map(|i| i.installed_at.to_rfc3339()),
        }));
    }

    Json(serde_json::json!({
        "installed": entries,
        "count": entries.len(),
    }))
}

/// GET /api/integrations/available - List all available templates.
pub async fn list_available_integrations(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let registry = state
        .kernel
        .extension_registry
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let templates: Vec<serde_json::Value> = registry
        .list_templates()
        .iter()
        .map(|t| {
            let installed = registry.is_installed(&t.id);
            serde_json::json!({
                "id": t.id,
                "name": t.name,
                "description": t.description,
                "icon": t.icon,
                "category": t.category.to_string(),
                "installed": installed,
                "tags": t.tags,
                "required_env": t.required_env.iter().map(|e| serde_json::json!({
                    "name": e.name,
                    "label": e.label,
                    "help": e.help,
                    "is_secret": e.is_secret,
                    "get_url": e.get_url,
                })).collect::<Vec<_>>(),
                "has_oauth": t.oauth.is_some(),
                "setup_instructions": t.setup_instructions,
            })
        })
        .collect();

    Json(serde_json::json!({
        "integrations": templates,
        "count": templates.len(),
    }))
}

/// POST /api/integrations/add - Install an integration.
pub async fn add_integration(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let id = match integration_id_from_body(&req) {
        Some(id) => id,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'id' field"})),
            );
        }
    };

    let install_err = {
        let mut registry = state
            .kernel
            .extension_registry
            .write()
            .unwrap_or_else(|e| e.into_inner());

        if registry.is_installed(&id) {
            Some((
                StatusCode::CONFLICT,
                format!("Integration '{}' already installed", id),
            ))
        } else if registry.get_template(&id).is_none() {
            Some((
                StatusCode::NOT_FOUND,
                format!("Unknown integration: '{}'", id),
            ))
        } else {
            let entry = captain_extensions::InstalledIntegration {
                id: id.clone(),
                installed_at: chrono::Utc::now(),
                enabled: true,
                oauth_provider: None,
                config: HashMap::new(),
            };
            match registry.install(entry) {
                Ok(_) => None,
                Err(e) => Some((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
            }
        }
    };

    if let Some((status, error)) = install_err {
        return (status, Json(serde_json::json!({"error": error})));
    }

    state.kernel.extension_health.register(&id);
    let connected = state.kernel.reload_extension_mcps().await.unwrap_or(0);

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": id,
            "status": "installed",
            "connected": connected > 0,
            "message": format!("Integration '{}' installed", id),
        })),
    )
}

/// DELETE /api/integrations/:id - Remove an integration.
pub async fn remove_integration(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uninstall_err = {
        let mut registry = state
            .kernel
            .extension_registry
            .write()
            .unwrap_or_else(|e| e.into_inner());
        registry.uninstall(&id).err()
    };

    if let Some(e) = uninstall_err {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        );
    }

    state.kernel.extension_health.unregister(&id);
    let _ = state.kernel.reload_extension_mcps().await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "id": id,
            "status": "removed",
        })),
    )
}

/// POST /api/integrations/:id/reconnect - Reconnect an MCP server.
pub async fn reconnect_integration(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let is_installed = {
        let registry = state
            .kernel
            .extension_registry
            .read()
            .unwrap_or_else(|e| e.into_inner());
        registry.is_installed(&id)
    };

    if !is_installed {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Integration '{}' not installed", id)})),
        );
    }

    match state.kernel.reconnect_extension_mcp(&id).await {
        Ok(tool_count) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": id,
                "status": "connected",
                "tool_count": tool_count,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "id": id,
                "status": "error",
                "error": e,
            })),
        ),
    }
}

/// GET /api/integrations/health - Health status for all integrations.
pub async fn integrations_health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let health_entries = state.kernel.extension_health.all_health();
    let entries: Vec<serde_json::Value> = health_entries
        .iter()
        .map(|h| {
            serde_json::json!({
                "id": h.id,
                "status": h.status.to_string(),
                "tool_count": h.tool_count,
                "last_ok": h.last_ok.map(|t| t.to_rfc3339()),
                "last_error": h.last_error,
                "consecutive_failures": h.consecutive_failures,
                "reconnecting": h.reconnecting,
                "reconnect_attempts": h.reconnect_attempts,
                "connected_since": h.connected_since.map(|t| t.to_rfc3339()),
            })
        })
        .collect();

    Json(serde_json::json!({
        "health": entries,
        "count": entries.len(),
    }))
}

/// POST /api/integrations/reload - Hot-reload integration configs and reconnect MCP.
pub async fn reload_integrations(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.kernel.reload_extension_mcps().await {
        Ok(connected) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "reloaded",
                "new_connections": connected,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

fn integration_id_from_body(req: &serde_json::Value) -> Option<String> {
    req.get("id").and_then(|v| v.as_str()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integration_id_from_body_reads_string_id() {
        let body = serde_json::json!({"id": "github"});

        assert_eq!(integration_id_from_body(&body).as_deref(), Some("github"));
    }

    #[test]
    fn integration_id_from_body_rejects_missing_or_non_string_id() {
        assert!(integration_id_from_body(&serde_json::json!({})).is_none());
        assert!(integration_id_from_body(&serde_json::json!({"id": 42})).is_none());
    }
}
