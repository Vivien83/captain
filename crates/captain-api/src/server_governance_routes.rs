use crate::routes::{self, AppState};
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_governance_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route(
            "/api/approvals",
            axum::routing::get(routes::list_approvals).post(routes::create_approval),
        )
        .route(
            "/api/approvals/{id}/approve",
            axum::routing::post(routes::approve_request),
        )
        .route(
            "/api/approvals/{id}/reject",
            axum::routing::post(routes::reject_request),
        )
        .route(
            "/api/approvals/{id}/approve_session",
            axum::routing::post(routes::approve_session_request),
        )
        .route(
            "/api/approvals/{id}/approve_always",
            axum::routing::post(routes::approve_always_request),
        )
        .route(
            "/api/approvals/clear_session",
            axum::routing::post(routes::clear_session_approvals),
        )
        .route("/api/usage", axum::routing::get(routes::usage_stats))
        .route(
            "/api/usage/summary",
            axum::routing::get(routes::usage_summary),
        )
        .route(
            "/api/usage/by-model",
            axum::routing::get(routes::usage_by_model),
        )
        .route("/api/usage/daily", axum::routing::get(routes::usage_daily))
        .route(
            "/api/budget",
            axum::routing::get(routes::budget_status).put(routes::update_budget),
        )
        .route(
            "/api/budget/agents",
            axum::routing::get(routes::agent_budget_ranking),
        )
        .route(
            "/api/budget/agents/{id}",
            axum::routing::get(routes::agent_budget_status).put(routes::update_agent_budget),
        )
}
