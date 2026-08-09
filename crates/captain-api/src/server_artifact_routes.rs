use crate::routes::{self, AppState};
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_artifact_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/artifacts", axum::routing::get(routes::list_artifacts))
        .route(
            "/api/artifacts/{id}",
            axum::routing::get(routes::inspect_artifact),
        )
        .route(
            "/api/artifacts/{id}/versions",
            axum::routing::get(routes::list_artifact_versions),
        )
        .route(
            "/api/artifacts/{id}/versions/{version}/download",
            axum::routing::get(routes::download_artifact),
        )
        .route(
            "/api/artifacts/{id}/versions/{version}/preview",
            axum::routing::get(routes::preview_artifact),
        )
}
