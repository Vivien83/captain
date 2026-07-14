//! Shared memory KV route handlers.

use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

/// GET /api/memory/agents/:id/kv - List KV pairs for an agent.
///
/// `memory_store` writes to a shared namespace, so this route reads from that
/// same namespace regardless of which agent ID is in the URL.
pub async fn get_agent_kv(
    State(state): State<Arc<AppState>>,
    Path(_id): Path<String>,
) -> impl IntoResponse {
    let agent_id = captain_kernel::shared_memory_agent_id();

    match state.kernel.memory.list_kv(agent_id) {
        Ok(pairs) => {
            let kv: Vec<serde_json::Value> = pairs
                .into_iter()
                .map(|(key, value)| serde_json::json!({"key": key, "value": value}))
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"kv_pairs": kv})))
        }
        Err(error) => {
            tracing::warn!("Memory list_kv failed: {error}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Memory operation failed"})),
            )
        }
    }
}

/// GET /api/memory/agents/:id/kv/:key - Get a specific KV value.
pub async fn get_agent_kv_key(
    State(state): State<Arc<AppState>>,
    Path((_id, key)): Path<(String, String)>,
) -> impl IntoResponse {
    let agent_id = captain_kernel::shared_memory_agent_id();

    match state.kernel.memory.structured_get(agent_id, &key) {
        Ok(Some(value)) => (
            StatusCode::OK,
            Json(serde_json::json!({"key": key, "value": value})),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Key not found"})),
        ),
        Err(error) => {
            tracing::warn!("Memory get failed for key '{key}': {error}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Memory operation failed"})),
            )
        }
    }
}

/// PUT /api/memory/agents/:id/kv/:key - Set a KV value.
pub async fn set_agent_kv_key(
    State(state): State<Arc<AppState>>,
    Path((_id, key)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = captain_kernel::shared_memory_agent_id();
    let value = body.get("value").cloned().unwrap_or(body);

    match state.kernel.memory.structured_set(agent_id, &key, value) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "stored", "key": key})),
        ),
        Err(error) => {
            tracing::warn!("Memory set failed for key '{key}': {error}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Memory operation failed"})),
            )
        }
    }
}

/// DELETE /api/memory/agents/:id/kv/:key - Delete a KV value.
pub async fn delete_agent_kv_key(
    State(state): State<Arc<AppState>>,
    Path((_id, key)): Path<(String, String)>,
) -> impl IntoResponse {
    let agent_id = captain_kernel::shared_memory_agent_id();

    match state.kernel.memory.structured_delete(agent_id, &key) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "deleted", "key": key})),
        ),
        Err(error) => {
            tracing::warn!("Memory delete failed for key '{key}': {error}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Memory operation failed"})),
            )
        }
    }
}
