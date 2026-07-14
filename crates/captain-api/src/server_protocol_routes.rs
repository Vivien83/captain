use crate::routes::{self, AppState};
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_protocol_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        // MCP HTTP endpoint (exposes MCP protocol over HTTP)
        .route("/mcp", axum::routing::post(routes::mcp_http))
        // OpenAI-compatible API
        .route(
            "/v1/chat/completions",
            axum::routing::post(crate::openai_compat::chat_completions),
        )
        .route(
            "/v1/models",
            axum::routing::get(crate::openai_compat::list_models),
        )
        // Web authentication endpoints
        .route("/api/auth/login", axum::routing::post(routes::auth_login))
        .route("/api/auth/logout", axum::routing::post(routes::auth_logout))
        .route("/api/auth/check", axum::routing::get(routes::auth_check))
}
