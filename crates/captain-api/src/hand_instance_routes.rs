use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::collections::HashMap;
use std::sync::Arc;

/// POST /api/hands/{hand_id}/activate - Activate a hand.
pub async fn activate_hand(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
    body: Option<Json<captain_hands::ActivateHandRequest>>,
) -> impl IntoResponse {
    let config = body.map(|b| b.0.config).unwrap_or_default();

    match state.kernel.activate_hand(&hand_id, config) {
        Ok(instance) => {
            if let Some(agent_id) = instance.agent_id {
                let entry = state
                    .kernel
                    .registry
                    .list()
                    .into_iter()
                    .find(|e| e.id == agent_id);
                if let Some(entry) = entry {
                    if !matches!(
                        entry.manifest.schedule,
                        captain_types::agent::ScheduleMode::Reactive
                    ) {
                        state.kernel.start_background_for_agent(
                            agent_id,
                            &entry.name,
                            &entry.manifest.schedule,
                        );
                    }
                }
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "instance_id": instance.instance_id,
                    "hand_id": instance.hand_id,
                    "status": format!("{}", instance.status),
                    "agent_id": instance.agent_id.map(|a| a.to_string()),
                    "agent_name": instance.agent_name,
                    "activated_at": instance.activated_at.to_rfc3339(),
                })),
            )
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// POST /api/hands/instances/{id}/pause - Pause a hand instance.
pub async fn pause_hand(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match state.kernel.pause_hand(id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "paused", "instance_id": id})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// POST /api/hands/instances/{id}/resume - Resume a paused hand instance.
pub async fn resume_hand(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match state.kernel.resume_hand(id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "resumed", "instance_id": id})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// DELETE /api/hands/instances/{id} - Deactivate a hand.
pub async fn deactivate_hand(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match state.kernel.deactivate_hand(id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "deactivated", "instance_id": id})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// GET /api/hands/{hand_id}/settings - Get settings for a hand.
pub async fn get_hand_settings(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
) -> impl IntoResponse {
    let settings_status = match state
        .kernel
        .hand_registry
        .check_settings_availability(&hand_id)
    {
        Ok(s) => s,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Hand not found: {hand_id}")})),
            );
        }
    };

    let instance_config: HashMap<String, serde_json::Value> = state
        .kernel
        .hand_registry
        .list_instances()
        .iter()
        .find(|i| i.hand_id == hand_id)
        .map(|i| i.config.clone())
        .unwrap_or_default();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "hand_id": hand_id,
            "settings": settings_status,
            "current_values": instance_config,
        })),
    )
}

/// PUT /api/hands/{hand_id}/settings - Update settings for a hand instance.
pub async fn update_hand_settings(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
    Json(config): Json<HashMap<String, serde_json::Value>>,
) -> impl IntoResponse {
    let instance_id = state
        .kernel
        .hand_registry
        .list_instances()
        .iter()
        .find(|i| i.hand_id == hand_id)
        .map(|i| i.instance_id);

    match instance_id {
        Some(id) => match state.kernel.hand_registry.update_config(id, config.clone()) {
            Ok(()) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "hand_id": hand_id,
                    "instance_id": id,
                    "config": config,
                })),
            ),
            Err(e) => (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("{e}")})),
            ),
        },
        None => (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::json!({"error": format!("No active instance for hand: {hand_id}. Activate the hand first.")}),
            ),
        ),
    }
}

/// GET /api/hands/instances/{id}/stats - Get stats for a hand instance.
pub async fn hand_stats(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    let instance = match state.kernel.hand_registry.get_instance(id) {
        Some(i) => i,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Instance not found"})),
            );
        }
    };

    let def = match state.kernel.hand_registry.get_definition(&instance.hand_id) {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Hand definition not found"})),
            );
        }
    };

    let agent_id = match instance.agent_id {
        Some(aid) => aid,
        None => {
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "instance_id": id,
                    "hand_id": instance.hand_id,
                    "metrics": {},
                })),
            );
        }
    };

    let shared_id = captain_kernel::shared_memory_agent_id();
    let mut metrics = serde_json::Map::new();
    for metric in &def.dashboard.metrics {
        let value = state
            .kernel
            .memory
            .structured_get(shared_id, &metric.memory_key)
            .ok()
            .flatten()
            .or_else(|| {
                state
                    .kernel
                    .memory
                    .structured_get(agent_id, &metric.memory_key)
                    .ok()
                    .flatten()
            })
            .unwrap_or(serde_json::Value::Null);
        metrics.insert(
            metric.label.clone(),
            serde_json::json!({
                "value": value,
                "format": metric.format,
            }),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "instance_id": id,
            "hand_id": instance.hand_id,
            "status": format!("{}", instance.status),
            "agent_id": agent_id.to_string(),
            "metrics": metrics,
        })),
    )
}

/// GET /api/hands/instances/{id}/browser - Get live browser state.
pub async fn hand_instance_browser(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    let instance = match state.kernel.hand_registry.get_instance(id) {
        Some(i) => i,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Instance not found"})),
            );
        }
    };

    let agent_id = match instance.agent_id {
        Some(aid) => aid,
        None => {
            return (StatusCode::OK, Json(serde_json::json!({"active": false})));
        }
    };

    let agent_id_str = agent_id.to_string();
    if !state.kernel.browser_ctx.has_session(&agent_id_str) {
        return (StatusCode::OK, Json(serde_json::json!({"active": false})));
    }

    let mut url = String::new();
    let mut title = String::new();
    let mut content = String::new();

    match state
        .kernel
        .browser_ctx
        .send_command(
            &agent_id_str,
            captain_runtime::browser::BrowserCommand::ReadPage,
        )
        .await
    {
        Ok(resp) if resp.success => {
            if let Some(data) = &resp.data {
                url = data["url"].as_str().unwrap_or("").to_string();
                title = data["title"].as_str().unwrap_or("").to_string();
                content = truncate_browser_content(data["content"].as_str().unwrap_or(""));
            }
        }
        Ok(_) => {}
        Err(_) => {}
    }

    let mut screenshot_base64 = String::new();

    match state
        .kernel
        .browser_ctx
        .send_command(
            &agent_id_str,
            captain_runtime::browser::BrowserCommand::Screenshot,
        )
        .await
    {
        Ok(resp) if resp.success => {
            if let Some(data) = &resp.data {
                screenshot_base64 = data["image_base64"].as_str().unwrap_or("").to_string();
            }
        }
        Ok(_) => {}
        Err(_) => {}
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "active": true,
            "url": url,
            "title": title,
            "content": content,
            "screenshot_base64": screenshot_base64,
        })),
    )
}

fn truncate_browser_content(content: &str) -> String {
    if content.len() > 2000 {
        format!(
            "{}... (truncated)",
            captain_types::truncate_str(content, 2000)
        )
    } else {
        content.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_browser_content_keeps_short_content() {
        assert_eq!(truncate_browser_content("hello"), "hello");
    }

    #[test]
    fn truncate_browser_content_caps_large_content() {
        let content = "a".repeat(2001);
        let truncated = truncate_browser_content(&content);

        assert!(truncated.len() < content.len() + "... (truncated)".len());
        assert!(truncated.ends_with("... (truncated)"));
    }
}
