use crate::routes::{self, AppState};
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_coordination_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route(
            "/api/mcp/servers",
            axum::routing::get(routes::list_mcp_servers),
        )
        .route(
            "/api/audit/recent",
            axum::routing::get(routes::audit_recent),
        )
        .route(
            "/api/audit/verify",
            axum::routing::get(routes::audit_verify),
        )
        .route(
            "/api/audit/repair",
            axum::routing::post(routes::audit_repair),
        )
        .route("/api/logs/stream", axum::routing::get(routes::logs_stream))
        .route("/api/peers", axum::routing::get(routes::list_peers))
        .route(
            "/api/network/status",
            axum::routing::get(routes::network_status),
        )
        .route(
            "/api/comms/topology",
            axum::routing::get(routes::comms_topology),
        )
        .route(
            "/api/comms/events",
            axum::routing::get(routes::comms_events),
        )
        .route(
            "/api/comms/events/stream",
            axum::routing::get(routes::comms_events_stream),
        )
        .route("/api/comms/send", axum::routing::post(routes::comms_send))
        .route("/api/comms/task", axum::routing::post(routes::comms_task))
}
