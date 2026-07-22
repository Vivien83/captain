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
            "/api/learning/workflows",
            axum::routing::get(crate::learning_routes::list_workflows),
        )
        .route(
            "/api/learning/workflows/{token}/decide",
            axum::routing::post(crate::learning_routes::decide_workflow),
        )
        .route(
            "/api/skills/proposals",
            axum::routing::get(crate::skill_routes::retired_skill_synthesizer),
        )
        .route(
            "/api/skills/patterns",
            axum::routing::get(crate::skill_routes::retired_skill_synthesizer),
        )
        .route(
            "/api/skills/proposals/{id}/decide",
            axum::routing::post(crate::skill_routes::retired_skill_synthesizer),
        )
        .route(
            "/api/skills/metrics",
            axum::routing::get(crate::skill_routes::retired_skill_synthesizer),
        )
}
