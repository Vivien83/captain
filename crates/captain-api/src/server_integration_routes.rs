use crate::routes::{self, AppState};
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_integration_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        // Integration management endpoints
        .route(
            "/api/integrations",
            axum::routing::get(routes::list_integrations),
        )
        .route(
            "/api/integrations/available",
            axum::routing::get(routes::list_available_integrations),
        )
        .route(
            "/api/integrations/add",
            axum::routing::post(routes::add_integration),
        )
        .route(
            "/api/integrations/{id}",
            axum::routing::delete(routes::remove_integration),
        )
        .route(
            "/api/integrations/{id}/reconnect",
            axum::routing::post(routes::reconnect_integration),
        )
        .route(
            "/api/integrations/health",
            axum::routing::get(routes::integrations_health),
        )
        .route(
            "/api/integrations/reload",
            axum::routing::post(routes::reload_integrations),
        )
        // Device pairing endpoints
        .route(
            "/api/pairing/request",
            axum::routing::post(routes::pairing_request),
        )
        .route(
            "/api/pairing/complete",
            axum::routing::post(routes::pairing_complete),
        )
        .route(
            "/api/pairing/devices",
            axum::routing::get(routes::pairing_devices),
        )
        .route(
            "/api/pairing/devices/{id}",
            axum::routing::delete(routes::pairing_remove_device),
        )
        .route(
            "/api/pairing/notify",
            axum::routing::post(routes::pairing_notify),
        )
}
