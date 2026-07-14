//! Graph memory and operational consciousness route handlers.

use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::collections::HashMap;
use std::sync::Arc;

/// GET /api/graph/stats - Graph statistics.
pub async fn graph_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let stats = state.kernel.graph_memory.extended_stats();
    (StatusCode::OK, Json(stats))
}

/// GET /api/graph/entities - List entities.
pub async fn graph_entities(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let limit = parse_limit(&params, 200);
    let entities = state.kernel.graph_memory.list_entities(limit);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "entities": entities,
            "total": entities.len(),
        })),
    )
}

/// GET /api/graph/facts - List facts/edges.
pub async fn graph_facts(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let limit = parse_limit(&params, 200);
    let facts = state.kernel.graph_memory.list_facts(limit);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "facts": facts,
            "total": facts.len(),
        })),
    )
}

/// GET /api/graph/entity/{id} - Entity detail with facts and neighbors.
pub async fn graph_entity_detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    match state.kernel.graph_memory.get_entity_detail(id) {
        Some((entity, facts, neighbors)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "entity": entity,
                "facts": facts,
                "neighbors": neighbors,
                "activation": state.kernel.graph_memory.get_activation(id),
                "memory_phase": state.kernel.graph_memory.get_memory_phase(id),
            })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Entity not found"})),
        ),
    }
}

/// GET /api/graph/search?q=... - Search entities by text (BM25 + hybrid).
pub async fn graph_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let query = params.get("q").map(|s| s.as_str()).unwrap_or("");
    let top_k = parse_limit(&params, 20);
    if query.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing 'q' parameter"})),
        );
    }
    let hits = state.kernel.graph_memory.search_graph(query, top_k);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "hits": hits,
            "total": hits.len(),
            "query": query,
        })),
    )
}

/// DELETE /api/graph/entity/{id} - Delete an entity.
pub async fn graph_delete_entity(
    State(state): State<Arc<AppState>>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    match state.kernel.graph_memory.delete_entity(id) {
        Ok(()) => {
            let _ = state.kernel.graph_memory.save();
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "deleted", "id": id})),
            )
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": error})),
        ),
    }
}

/// POST /api/graph/fact/{id}/invalidate - Invalidate a fact.
pub async fn graph_invalidate_fact(
    State(state): State<Arc<AppState>>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    match state.kernel.graph_memory.invalidate_fact(id) {
        Ok(()) => {
            let _ = state.kernel.graph_memory.save();
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "invalidated", "id": id})),
            )
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": error})),
        ),
    }
}

/// POST /api/graph/dream - Run a dream cycle (consolidation).
pub async fn graph_dream_cycle(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.kernel.graph_memory.dream_cycle() {
        Ok(stats) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "stats": stats})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": error})),
        ),
    }
}

/// GET /api/graph/consciousness - Full consciousness state.
pub async fn graph_consciousness(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mood = state.kernel.graph_memory.get_mood();
    let nm = state.kernel.graph_memory.get_neuromodulators();
    let (accuracy, correct, total) = state.kernel.graph_memory.prediction_accuracy();
    let patterns = state.kernel.graph_memory.detect_patterns(3);
    let user_state = state.kernel.graph_memory.get_user_state();
    let queued = state.kernel.graph_memory.queued_thought_count();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "mood": {
                "confidence": mood.confidence,
                "streak": mood.streak,
                "error_rate": mood.error_rate,
                "prediction_accuracy": mood.prediction_accuracy,
            },
            "neuromodulators": {
                "dopamine": nm.dopamine,
                "serotonin": nm.serotonin,
                "norepinephrine": nm.norepinephrine,
                "cortisol": nm.cortisol,
            },
            "predictions": {
                "total": total,
                "correct": correct,
                "accuracy": accuracy,
            },
            "patterns": patterns
                .iter()
                .map(|pattern| {
                    serde_json::json!({
                        "action": pattern.action,
                        "hour": pattern.hour,
                        "weekday": pattern.weekday,
                        "occurrences": pattern.occurrences,
                    })
                })
                .collect::<Vec<_>>(),
            "user_state": {
                "pace": user_state.pace,
                "frustration": user_state.frustration,
                "mode": user_state.mode,
                "interaction_count": user_state.interaction_count,
            },
            "queued_thoughts": queued,
        })),
    )
}

/// GET /api/graph/consciousness/digest - Preview the next digest without sending it.
pub async fn graph_consciousness_digest_preview(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match state.kernel.graph_memory.peek_telegram_digest() {
        Some(message) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "has_content": true,
                "message": message,
            })),
        ),
        None => (
            StatusCode::OK,
            Json(serde_json::json!({
                "has_content": false,
                "message": null,
            })),
        ),
    }
}

/// POST /api/graph/consciousness/digest/send - Force-send the digest to Telegram now.
pub async fn graph_consciousness_digest_send(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let digest_msg = match state.kernel.graph_memory.flush_telegram_digest() {
        Some(message) => message,
        None => {
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "sent": false,
                    "reason": "nothing interesting to report",
                })),
            )
        }
    };

    if let Some(telegram) = state.kernel.channel_adapters.get("telegram") {
        let chat_id = state
            .kernel
            .config
            .channels
            .telegram
            .as_ref()
            .and_then(|config| config.default_chat_id.clone())
            .unwrap_or_default();
        if !chat_id.is_empty() {
            let user = captain_channels::types::ChannelUser {
                platform_id: chat_id,
                display_name: "system".to_string(),
                captain_user: None,
            };
            let content = captain_channels::types::ChannelContent::Text(digest_msg.clone());
            let _ = telegram.send(&user, content).await;
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "sent": true,
                    "message": digest_msg,
                })),
            );
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "sent": false,
            "reason": "telegram not configured",
        })),
    )
}

/// GET /api/consciousness/mood - Raw system mood data.
pub async fn get_consciousness_mood(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mood = state.kernel.graph_memory.get_mood();
    Json(serde_json::json!({
        "confidence": mood.confidence,
        "streak": mood.streak,
        "error_rate": mood.error_rate,
        "prediction_accuracy": mood.prediction_accuracy,
    }))
}

/// GET /api/consciousness/state - User state (pace, frustration, mode).
pub async fn get_consciousness_user_state(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let user_state = state.kernel.graph_memory.get_user_state();
    Json(serde_json::json!({
        "pace": user_state.pace,
        "frustration": user_state.frustration,
        "mode": user_state.mode,
        "interaction_count": user_state.interaction_count,
    }))
}

/// GET /api/consciousness/neuromodulators - Current neuromodulator values.
pub async fn get_consciousness_neuromodulators(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let nm = state.kernel.graph_memory.get_neuromodulators();
    Json(serde_json::json!({
        "dopamine": nm.dopamine,
        "serotonin": nm.serotonin,
        "norepinephrine": nm.norepinephrine,
        "cortisol": nm.cortisol,
    }))
}

fn parse_limit(params: &HashMap<String, String>, default: usize) -> usize {
    params
        .get("limit")
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_limit_uses_query_value() {
        let params = HashMap::from([("limit".to_string(), "42".to_string())]);

        assert_eq!(parse_limit(&params, 200), 42);
    }

    #[test]
    fn parse_limit_falls_back_to_default() {
        let params = HashMap::from([("limit".to_string(), "bad".to_string())]);

        assert_eq!(parse_limit(&params, 200), 200);
    }
}
