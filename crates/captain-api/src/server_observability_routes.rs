use crate::routes::{self, AppState};
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_observability_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route(
            "/api/metrics",
            axum::routing::get(routes::prometheus_metrics),
        )
        .route("/api/health", axum::routing::get(routes::health))
        .route(
            "/api/health/detail",
            axum::routing::get(routes::health_detail),
        )
        .route("/api/status", axum::routing::get(routes::status))
        .route("/api/version", axum::routing::get(routes::version))
        .route(
            "/api/processes/{process_id}",
            axum::routing::delete(routes::kill_process),
        )
        .route(
            "/api/events",
            axum::routing::get(crate::event_webhooks::recent_events),
        )
        .route(
            "/api/webhooks/outbound",
            axum::routing::get(crate::event_webhooks::outbound_webhooks),
        )
        .route(
            "/api/webhooks/outbound/test",
            axum::routing::post(crate::event_webhooks::test_outbound_webhook),
        )
        .route(
            "/api/webhooks/outbound/endpoints",
            axum::routing::post(crate::event_webhooks::create_outbound_webhook_endpoint),
        )
        .route(
            "/api/webhooks/outbound/endpoints/{name}",
            axum::routing::put(crate::event_webhooks::update_outbound_webhook_endpoint)
                .delete(crate::event_webhooks::delete_outbound_webhook_endpoint),
        )
}
