use crate::routes::{self, AppState};
use crate::ws_terminal;
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_session_io_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route(
            "/api/sessions/{id}/terminal",
            axum::routing::get(ws_terminal::terminal_ws),
        )
        .route(
            "/api/terminal/sessions",
            axum::routing::get(ws_terminal::list_terminal_sessions),
        )
        .route(
            "/api/terminal/sessions/{id}",
            axum::routing::delete(ws_terminal::terminate_terminal_session),
        )
        .route(
            "/api/realtime/calls",
            axum::routing::get(crate::realtime_call::get_call_config)
                .post(crate::realtime_call::create_call),
        )
        .route(
            "/api/sessions/{id}/events",
            axum::routing::get(routes::list_session_events),
        )
        .route(
            "/api/uploads/{file_id}",
            axum::routing::get(routes::serve_upload),
        )
}
