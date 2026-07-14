use crate::routes::{self, AppState};
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_session_management_routes(
    router: Router<Arc<AppState>>,
) -> Router<Arc<AppState>> {
    router
        .route("/api/sessions", axum::routing::get(routes::list_sessions))
        .route(
            "/api/sessions/{id}",
            axum::routing::get(routes::get_session).delete(routes::delete_session),
        )
        .route(
            "/api/sessions/{id}/label",
            axum::routing::put(routes::set_session_label),
        )
        .route(
            "/api/agents/{id}/sessions/by-label/{label}",
            axum::routing::get(routes::find_session_by_label),
        )
}
