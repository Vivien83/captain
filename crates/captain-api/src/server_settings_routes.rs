use crate::routes::{self, AppState};
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_settings_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/tools", axum::routing::get(routes::list_tools))
        .route("/api/config", axum::routing::get(routes::get_config))
        .route(
            "/api/config/schema",
            axum::routing::get(routes::config_schema),
        )
        .route(
            "/api/config/template",
            axum::routing::get(routes::config_template_get),
        )
        .route(
            "/api/config/validate",
            axum::routing::post(routes::config_validate),
        )
        .route("/api/config/set", axum::routing::post(routes::config_set))
        .route(
            "/api/stt",
            axum::routing::get(routes::get_stt).put(routes::update_stt),
        )
}
