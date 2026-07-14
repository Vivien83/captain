use crate::routes::AppState;
use crate::webchat;
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_web_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/", axum::routing::get(webchat::app_page))
        .route("/assets/logo.png", axum::routing::get(webchat::logo_png))
        .route(
            "/assets/app/{*path}",
            axum::routing::get(webchat::app_asset),
        )
        .route("/terminal", axum::routing::get(webchat::terminal_page))
        .route("/config", axum::routing::get(webchat::config_page))
        .route("/embed/chat.js", axum::routing::get(webchat::embed_chat_js))
        .route("/logo.svg", axum::routing::get(webchat::logo_svg))
        .route("/favicon.ico", axum::routing::get(webchat::favicon_ico))
        .route("/manifest.json", axum::routing::get(webchat::manifest_json))
        .route("/sw.js", axum::routing::get(webchat::sw_js))
}
