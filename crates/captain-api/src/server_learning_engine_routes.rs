use crate::routes::AppState;
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_learning_engine_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route(
            "/api/learning/committed",
            axum::routing::get(crate::learning_routes::list_committed),
        )
        .route(
            "/api/learning/review",
            axum::routing::get(crate::learning_routes::list_review),
        )
        .route(
            "/api/learning/review/{id}/decide",
            axum::routing::post(crate::learning_routes::decide_review),
        )
        .route(
            "/api/learning/metrics",
            axum::routing::get(crate::learning_routes::metrics),
        )
        .route(
            "/api/skills/proposals",
            axum::routing::get(crate::skill_routes::list_proposals),
        )
        .route(
            "/api/skills/patterns",
            axum::routing::get(crate::skill_routes::list_patterns),
        )
        .route(
            "/api/skills/proposals/{id}/decide",
            axum::routing::post(crate::skill_routes::decide_proposal),
        )
        .route(
            "/api/skills/metrics",
            axum::routing::get(crate::skill_routes::metrics),
        )
}
