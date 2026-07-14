use crate::routes::{self, AppState};
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_skill_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/skills", axum::routing::get(routes::list_skills))
        .route(
            "/api/skills/install",
            axum::routing::post(routes::install_skill),
        )
        .route(
            "/api/skills/uninstall",
            axum::routing::post(routes::uninstall_skill),
        )
        .route(
            "/api/marketplace/search",
            axum::routing::get(routes::marketplace_search),
        )
        .route(
            "/api/clawhub/search",
            axum::routing::get(routes::clawhub_search),
        )
        .route(
            "/api/clawhub/browse",
            axum::routing::get(routes::clawhub_browse),
        )
        .route(
            "/api/clawhub/skill/{slug}",
            axum::routing::get(routes::clawhub_skill_detail),
        )
        .route(
            "/api/clawhub/skill/{slug}/code",
            axum::routing::get(routes::clawhub_skill_code),
        )
        .route(
            "/api/clawhub/install",
            axum::routing::post(routes::clawhub_install),
        )
}
