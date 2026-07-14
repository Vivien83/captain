use crate::routes::{self, AppState};
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_a2a_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        // A2A (Agent-to-Agent) Protocol endpoints
        .route(
            "/.well-known/agent.json",
            axum::routing::get(routes::a2a_agent_card),
        )
        .route("/a2a/agents", axum::routing::get(routes::a2a_list_agents))
        .route(
            "/a2a/tasks/send",
            axum::routing::post(routes::a2a_send_task),
        )
        .route("/a2a/tasks/{id}", axum::routing::get(routes::a2a_get_task))
        .route(
            "/a2a/tasks/{id}/cancel",
            axum::routing::post(routes::a2a_cancel_task),
        )
        // A2A management (outbound) endpoints
        .route(
            "/api/a2a/agents",
            axum::routing::get(routes::a2a_list_external_agents),
        )
        .route(
            "/api/a2a/discover",
            axum::routing::post(routes::a2a_discover_external),
        )
        .route(
            "/api/a2a/send",
            axum::routing::post(routes::a2a_send_external),
        )
        .route(
            "/api/a2a/tasks/{id}/status",
            axum::routing::get(routes::a2a_external_task_status),
        )
}
