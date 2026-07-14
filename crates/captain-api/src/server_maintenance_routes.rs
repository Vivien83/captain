use crate::routes::{self, AppState};
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_maintenance_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route(
            "/api/cron/jobs",
            axum::routing::get(routes::list_cron_jobs).post(routes::create_cron_job),
        )
        .route(
            "/api/cron/jobs/{id}",
            axum::routing::delete(routes::delete_cron_job).put(routes::update_cron_job),
        )
        .route(
            "/api/cron/jobs/{id}/enable",
            axum::routing::put(routes::toggle_cron_job),
        )
        .route(
            "/api/cron/jobs/{id}/run",
            axum::routing::post(routes::run_cron_job),
        )
        .route(
            "/api/cron/jobs/{id}/status",
            axum::routing::get(routes::cron_job_status),
        )
}
