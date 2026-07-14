use crate::routes::{self, AppState};
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_channel_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/channels", axum::routing::get(routes::list_channels))
        .route(
            "/api/channels/inbound-queue/dead-letters",
            axum::routing::delete(routes::clear_inbound_dead_letters),
        )
        .route(
            "/api/channels/telegram/topics",
            axum::routing::get(routes::list_telegram_topics).post(routes::set_telegram_topic),
        )
        .route(
            "/api/channels/telegram/topics/{thread_id}",
            axum::routing::delete(routes::delete_telegram_topic),
        )
        .route(
            "/api/channels/{name}/configure",
            axum::routing::post(routes::configure_channel).delete(routes::remove_channel),
        )
        .route(
            "/api/channels/{name}/test",
            axum::routing::post(routes::test_channel),
        )
        .route(
            "/api/channels/reload",
            axum::routing::post(routes::reload_channels),
        )
        .route(
            "/api/channels/whatsapp/qr/start",
            axum::routing::post(routes::whatsapp_qr_start),
        )
        .route(
            "/api/channels/whatsapp/qr/status",
            axum::routing::get(routes::whatsapp_qr_status),
        )
}
