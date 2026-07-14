//! Peer and network status route handlers.

use crate::state::AppState;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

/// GET /api/peers - List known OFP peers.
pub async fn list_peers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if let Some(ref peer_registry) = state.peer_registry {
        let peers: Vec<serde_json::Value> = peer_registry
            .all_peers()
            .iter()
            .map(|peer| {
                serde_json::json!({
                    "node_id": peer.node_id,
                    "node_name": peer.node_name,
                    "address": peer.address.to_string(),
                    "state": format!("{:?}", peer.state),
                    "agents": peer.agents.iter().map(|agent| serde_json::json!({
                        "id": agent.id,
                        "name": agent.name,
                    })).collect::<Vec<_>>(),
                    "connected_at": peer.connected_at.to_rfc3339(),
                    "protocol_version": peer.protocol_version,
                })
            })
            .collect();
        Json(serde_json::json!({"peers": peers, "total": peers.len()}))
    } else {
        Json(serde_json::json!({"peers": [], "total": 0}))
    }
}

/// GET /api/network/status - OFP network status summary.
pub async fn network_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let enabled = state.kernel.config.network_enabled
        && !state.kernel.config.network.shared_secret.is_empty();

    let (node_id, listen_address, connected_peers, total_peers) =
        if let Some(peer_node) = state.kernel.peer_node.get() {
            let registry = peer_node.registry();
            (
                peer_node.node_id().to_string(),
                peer_node.local_addr().to_string(),
                registry.connected_count(),
                registry.total_count(),
            )
        } else {
            (String::new(), String::new(), 0, 0)
        };

    Json(serde_json::json!({
        "enabled": enabled,
        "node_id": node_id,
        "listen_address": listen_address,
        "connected_peers": connected_peers,
        "total_peers": total_peers,
    }))
}
