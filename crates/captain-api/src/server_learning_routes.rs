use crate::routes::{self, AppState};
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_learning_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/templates", axum::routing::get(routes::list_templates))
        .route(
            "/api/templates/{name}",
            axum::routing::get(routes::get_template),
        )
        .route(
            "/api/memory/agents/{id}/kv",
            axum::routing::get(routes::get_agent_kv),
        )
        .route(
            "/api/memory/agents/{id}/kv/{key}",
            axum::routing::get(routes::get_agent_kv_key)
                .put(routes::set_agent_kv_key)
                .delete(routes::delete_agent_kv_key),
        )
        .route(
            "/api/agents/{id}/feedback",
            axum::routing::post(routes::submit_feedback).get(routes::get_feedback),
        )
}
