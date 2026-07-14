use crate::routes::{self, AppState};
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_memory_graph_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        // Graph memory API
        .route("/api/graph/stats", axum::routing::get(routes::graph_stats))
        .route(
            "/api/graph/entities",
            axum::routing::get(routes::graph_entities),
        )
        .route("/api/graph/facts", axum::routing::get(routes::graph_facts))
        .route(
            "/api/graph/entity/{id}",
            axum::routing::get(routes::graph_entity_detail).delete(routes::graph_delete_entity),
        )
        .route(
            "/api/graph/fact/{id}/invalidate",
            axum::routing::post(routes::graph_invalidate_fact),
        )
        .route(
            "/api/graph/search",
            axum::routing::get(routes::graph_search),
        )
        .route(
            "/api/graph/dream",
            axum::routing::post(routes::graph_dream_cycle),
        )
        .route(
            "/api/memory/migrate",
            axum::routing::post(routes::memory_migrate_to_mempalace),
        )
        // Phase O.3 - SSE broadcast des commits auto-memorize.
        .route(
            "/api/memory/events",
            axum::routing::get(routes::memory_events_stream),
        )
        .route(
            "/api/graph/consciousness",
            axum::routing::get(routes::graph_consciousness),
        )
        .route(
            "/api/graph/consciousness/digest",
            axum::routing::get(routes::graph_consciousness_digest_preview),
        )
        .route(
            "/api/graph/consciousness/digest/send",
            axum::routing::post(routes::graph_consciousness_digest_send),
        )
        // Focused consciousness state endpoints
        .route(
            "/api/consciousness/mood",
            axum::routing::get(routes::get_consciousness_mood),
        )
        .route(
            "/api/consciousness/state",
            axum::routing::get(routes::get_consciousness_user_state),
        )
        .route(
            "/api/consciousness/neuromodulators",
            axum::routing::get(routes::get_consciousness_neuromodulators),
        )
}
