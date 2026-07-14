use crate::routes::{self, AppState};
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_automation_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route(
            "/api/triggers",
            axum::routing::get(routes::list_triggers).post(routes::create_trigger),
        )
        .route(
            "/api/triggers/{id}",
            axum::routing::delete(routes::delete_trigger).put(routes::update_trigger),
        )
        .route(
            "/api/file-triggers",
            axum::routing::get(routes::list_file_triggers).post(routes::create_file_trigger),
        )
        .route(
            "/api/file-triggers/{id}",
            axum::routing::delete(routes::delete_file_trigger).put(routes::update_file_trigger),
        )
        .route(
            "/api/workspace/add",
            axum::routing::post(routes::add_workspace_path),
        )
        .route(
            "/api/schedules",
            axum::routing::get(routes::list_schedules).post(routes::create_schedule),
        )
        .route(
            "/api/schedules/{id}",
            axum::routing::delete(routes::delete_schedule).put(routes::update_schedule),
        )
        .route(
            "/api/schedules/{id}/run",
            axum::routing::post(routes::run_schedule),
        )
        .route(
            "/api/workflows",
            axum::routing::get(routes::list_workflows).post(routes::create_workflow),
        )
        .route(
            "/api/workflows/{id}",
            axum::routing::get(routes::get_workflow)
                .put(routes::update_workflow)
                .delete(routes::delete_workflow),
        )
        .route(
            "/api/workflows/{id}/run",
            axum::routing::post(routes::run_workflow),
        )
        .route(
            "/api/workflows/{id}/runs",
            axum::routing::get(routes::list_workflow_runs),
        )
}
