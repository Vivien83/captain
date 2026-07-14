use crate::routes::{self, AppState};
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_control_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        // Webhook trigger endpoints (external event injection)
        .route("/hooks/wake", axum::routing::post(routes::webhook_wake))
        .route("/hooks/agent", axum::routing::post(routes::webhook_agent))
        .route(
            "/hooks/agents/{id}/ingress",
            axum::routing::post(routes::agent_api_ingress),
        )
        .route("/api/shutdown", axum::routing::post(routes::shutdown))
        // Chat commands endpoint (dynamic slash menu)
        .route("/api/commands", axum::routing::get(routes::list_commands))
        // Config reload endpoint
        .route(
            "/api/config/reload",
            axum::routing::post(routes::config_reload),
        )
        // Raw config file read/write
        .route(
            "/api/config/raw",
            axum::routing::get(routes::config_raw_get).put(routes::config_raw_put),
        )
        // Agent binding routes
        .route(
            "/api/bindings",
            axum::routing::get(routes::list_bindings).post(routes::add_binding),
        )
        .route(
            "/api/bindings/{index}",
            axum::routing::delete(routes::remove_binding),
        )
}
