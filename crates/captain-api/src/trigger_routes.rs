//! Trigger and file-trigger route handlers.

use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_kernel::triggers::{FileChangeTrigger, TriggerId, TriggerPatch, TriggerPattern};
use captain_types::agent::AgentId;
use captain_types::event::FileEventKind;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

type JsonResponse = (StatusCode, Json<serde_json::Value>);

fn parse_trigger_id(id: &str) -> Result<TriggerId, JsonResponse> {
    id.parse().map(TriggerId).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid trigger ID"})),
        )
    })
}

/// POST /api/triggers - Register a new event trigger.
pub async fn create_trigger(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id_str = match req["agent_id"].as_str() {
        Some(id) => id,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'agent_id'"})),
            );
        }
    };

    let agent_id: AgentId = match agent_id_str.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent_id"})),
            );
        }
    };

    let pattern: TriggerPattern = match req.get("pattern") {
        Some(p) => match serde_json::from_value(p.clone()) {
            Ok(pattern) => pattern,
            Err(error) => {
                tracing::warn!("Invalid trigger pattern: {error}");
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "Invalid trigger pattern"})),
                );
            }
        },
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'pattern'"})),
            );
        }
    };

    let prompt_template = req["prompt_template"]
        .as_str()
        .unwrap_or("Event: {{event}}")
        .to_string();
    let max_fires = req["max_fires"].as_u64().unwrap_or(0);

    match state
        .kernel
        .register_trigger(agent_id, pattern, prompt_template, max_fires)
    {
        Ok(trigger_id) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "trigger_id": trigger_id.to_string(),
                "agent_id": agent_id.to_string(),
            })),
        ),
        Err(error) => {
            tracing::warn!("Trigger registration failed: {error}");
            (
                StatusCode::NOT_FOUND,
                Json(
                    serde_json::json!({"error": "Trigger registration failed (agent not found?)"}),
                ),
            )
        }
    }
}

/// GET /api/triggers - List all triggers.
pub async fn list_triggers(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let agent_filter = params
        .get("agent_id")
        .and_then(|id| id.parse::<AgentId>().ok());

    let triggers = state.kernel.list_triggers(agent_filter);
    let list: Vec<serde_json::Value> = triggers.iter().map(trigger_json).collect();
    Json(list)
}

/// PUT /api/triggers/:id - Update a trigger.
pub async fn update_trigger(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let trigger_id = match parse_trigger_id(&id) {
        Ok(trigger_id) => trigger_id,
        Err(response) => return response,
    };

    let pattern = match req.get("pattern") {
        Some(value) => match serde_json::from_value::<TriggerPattern>(value.clone()) {
            Ok(pattern) => Some(pattern),
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": format!("Invalid trigger pattern: {error}")})),
                )
            }
        },
        None => None,
    };
    let prompt_template = req
        .get("prompt_template")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    let enabled = req.get("enabled").and_then(|value| value.as_bool());
    let max_fires = req.get("max_fires").and_then(|value| value.as_u64());

    if pattern.is_none() && prompt_template.is_none() && enabled.is_none() && max_fires.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing editable trigger field"})),
        );
    }

    if let Some(trigger) = state.kernel.update_trigger(
        trigger_id,
        TriggerPatch {
            pattern,
            prompt_template,
            enabled,
            max_fires,
        },
    ) {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "updated",
                "trigger": trigger_json(&trigger),
            })),
        )
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Trigger not found"})),
        )
    }
}

/// DELETE /api/triggers/:id - Remove a trigger.
pub async fn delete_trigger(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let trigger_id = match parse_trigger_id(&id) {
        Ok(trigger_id) => trigger_id,
        Err(response) => return response,
    };

    if state.kernel.remove_trigger(trigger_id) {
        (
            StatusCode::OK,
            Json(serde_json::json!({"status": "removed", "trigger_id": id})),
        )
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Trigger not found"})),
        )
    }
}

/// POST /api/file-triggers - Register a file-change trigger.
pub async fn create_file_trigger(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = match req["agent_id"].as_str().and_then(|id| id.parse().ok()) {
        Some(id) => id,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing or invalid 'agent_id'"})),
            );
        }
    };

    let paths = match parse_file_trigger_paths(&req) {
        Ok(paths) => paths,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": error })),
            )
        }
    };
    let events = match parse_file_event_kinds(&req) {
        Ok(events) => events,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": error })),
            )
        }
    };

    let trigger = FileChangeTrigger {
        id: TriggerId::new(),
        paths,
        recursive: req["recursive"].as_bool().unwrap_or(true),
        events,
        agent_id,
        prompt_template: req["prompt_template"]
            .as_str()
            .unwrap_or("File {kind}: {path}")
            .to_string(),
        debounce_ms: req["debounce_ms"]
            .as_u64()
            .unwrap_or(captain_kernel::triggers::DEFAULT_FILE_WATCH_DEBOUNCE_MS),
        enabled: req["enabled"].as_bool().unwrap_or(true),
    };

    match state.kernel.register_file_change_trigger(trigger) {
        Ok(trigger_id) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "trigger_id": trigger_id.to_string(),
                "agent_id": agent_id.to_string(),
            })),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": error.to_string()})),
        ),
    }
}

/// GET /api/file-triggers - List file-change triggers.
pub async fn list_file_triggers(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let agent_filter = params
        .get("agent_id")
        .and_then(|id| id.parse::<AgentId>().ok());
    let list: Vec<serde_json::Value> = state
        .kernel
        .list_file_change_triggers(agent_filter)
        .iter()
        .map(file_trigger_json)
        .collect();
    Json(list)
}

/// PUT /api/file-triggers/:id - Enable or disable a file-change trigger.
pub async fn update_file_trigger(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let trigger_id = match parse_trigger_id(&id) {
        Ok(trigger_id) => trigger_id,
        Err(response) => return response,
    };
    let Some(enabled) = req.get("enabled").and_then(|value| value.as_bool()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing 'enabled' field"})),
        );
    };

    match state
        .kernel
        .set_file_change_trigger_enabled(trigger_id, enabled)
    {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "updated",
                "trigger_id": id,
                "enabled": enabled,
            })),
        ),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "File trigger not found"})),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": error.to_string()})),
        ),
    }
}

/// DELETE /api/file-triggers/:id - Remove a file-change trigger.
pub async fn delete_file_trigger(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let trigger_id = match parse_trigger_id(&id) {
        Ok(trigger_id) => trigger_id,
        Err(response) => return response,
    };

    match state.kernel.remove_file_change_trigger(trigger_id) {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "removed", "trigger_id": id})),
        ),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "File trigger not found"})),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": error.to_string()})),
        ),
    }
}

fn parse_file_trigger_paths(req: &serde_json::Value) -> Result<Vec<PathBuf>, String> {
    let Some(values) = req["paths"].as_array() else {
        return Err("Missing 'paths' array".to_string());
    };
    let mut paths = Vec::with_capacity(values.len());
    for value in values {
        let Some(path) = value.as_str() else {
            return Err("'paths' must contain strings".to_string());
        };
        paths.push(PathBuf::from(path));
    }
    if paths.is_empty() {
        Err("'paths' must contain at least one path".to_string())
    } else {
        Ok(paths)
    }
}

fn parse_file_event_kinds(req: &serde_json::Value) -> Result<Vec<FileEventKind>, String> {
    let Some(values) = req.get("events").and_then(|value| value.as_array()) else {
        return Ok(vec![FileEventKind::Any]);
    };
    let mut events = Vec::with_capacity(values.len());
    for value in values {
        let Some(kind) = value.as_str() else {
            return Err("'events' must contain strings".to_string());
        };
        events.push(kind.parse::<FileEventKind>()?);
    }
    if events.is_empty() {
        Ok(vec![FileEventKind::Any])
    } else {
        Ok(events)
    }
}

fn trigger_json(trigger: &captain_kernel::triggers::Trigger) -> serde_json::Value {
    serde_json::json!({
        "id": trigger.id.to_string(),
        "agent_id": trigger.agent_id.to_string(),
        "pattern": serde_json::to_value(&trigger.pattern).unwrap_or_default(),
        "prompt_template": trigger.prompt_template,
        "enabled": trigger.enabled,
        "fire_count": trigger.fire_count,
        "max_fires": trigger.max_fires,
        "created_at": trigger.created_at.to_rfc3339(),
    })
}

fn file_trigger_json(trigger: &FileChangeTrigger) -> serde_json::Value {
    serde_json::json!({
        "id": trigger.id.to_string(),
        "agent_id": trigger.agent_id.to_string(),
        "paths": trigger
            .paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>(),
        "recursive": trigger.recursive,
        "events": trigger
            .events
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>(),
        "prompt_template": trigger.prompt_template,
        "debounce_ms": trigger.debounce_ms,
        "enabled": trigger.enabled,
    })
}
