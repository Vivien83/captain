use crate::routes::{self, AppState};
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_hand_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/hands", axum::routing::get(routes::list_hands))
        .route(
            "/api/hands/install",
            axum::routing::post(routes::install_hand),
        )
        .route(
            "/api/hands/upsert",
            axum::routing::post(routes::upsert_hand),
        )
        .route(
            "/api/hands/active",
            axum::routing::get(routes::list_active_hands),
        )
        .route("/api/hands/{hand_id}", axum::routing::get(routes::get_hand))
        .route(
            "/api/hands/{hand_id}/activate",
            axum::routing::post(routes::activate_hand),
        )
        .route(
            "/api/hands/{hand_id}/check-deps",
            axum::routing::post(routes::check_hand_deps),
        )
        .route(
            "/api/hands/{hand_id}/install-deps",
            axum::routing::post(routes::install_hand_deps),
        )
        .route(
            "/api/hands/{hand_id}/settings",
            axum::routing::get(routes::get_hand_settings).put(routes::update_hand_settings),
        )
        .route(
            "/api/hands/instances/{id}/pause",
            axum::routing::post(routes::pause_hand),
        )
        .route(
            "/api/hands/instances/{id}/resume",
            axum::routing::post(routes::resume_hand),
        )
        .route(
            "/api/hands/instances/{id}",
            axum::routing::delete(routes::deactivate_hand),
        )
        .route(
            "/api/hands/instances/{id}/stats",
            axum::routing::get(routes::hand_stats),
        )
        .route(
            "/api/hands/instances/{id}/browser",
            axum::routing::get(routes::hand_instance_browser),
        )
}
