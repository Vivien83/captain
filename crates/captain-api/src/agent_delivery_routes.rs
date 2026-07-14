//! Agent delivery receipt route handlers.

use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use captain_types::agent::AgentId;
use std::collections::HashMap;
use std::sync::Arc;

/// GET /api/agents/:id/deliveries - List recent delivery receipts for an agent.
pub async fn get_agent_deliveries(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id_or_name(&state, &id) {
        Some(agent_id) => agent_id,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            )
        }
    };

    let limit = delivery_limit(&params);
    let receipts = state.kernel.delivery_tracker.get_receipts(agent_id, limit);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "count": receipts.len(),
            "receipts": receipts,
        })),
    )
}

fn parse_agent_id_or_name(state: &AppState, id: &str) -> Option<AgentId> {
    id.parse()
        .ok()
        .or_else(|| state.kernel.registry.find_by_name(id).map(|entry| entry.id))
}

fn delivery_limit(params: &HashMap<String, String>) -> usize {
    params
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(50)
        .min(500)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_limit_defaults_to_50() {
        assert_eq!(delivery_limit(&HashMap::new()), 50);
    }

    #[test]
    fn delivery_limit_clamps_to_500() {
        let params = HashMap::from([("limit".to_string(), "9999".to_string())]);

        assert_eq!(delivery_limit(&params), 500);
    }
}
