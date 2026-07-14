use crate::routes::{self, AppState};
use crate::ws;
use axum::Router;
use std::sync::Arc;

type ApiRouter = Router<Arc<AppState>>;

pub(crate) fn mount_agent_routes(router: ApiRouter) -> ApiRouter {
    let router = mount_agent_collection_routes(router);
    let router = mount_agent_api_surface_routes(router);
    let router = mount_agent_session_routes(router);
    let router = mount_agent_model_capability_routes(router);
    mount_agent_workspace_routes(router)
}

fn mount_agent_collection_routes(router: ApiRouter) -> ApiRouter {
    router
        .route(
            "/api/agents",
            axum::routing::get(routes::list_agents).post(routes::spawn_agent),
        )
        .route("/api/fleets", axum::routing::get(routes::list_fleets))
        .route(
            "/api/fleets/{id}/metrics",
            axum::routing::get(routes::fleet_metrics),
        )
        .route(
            "/api/agents/{id}",
            axum::routing::get(routes::get_agent)
                .delete(routes::kill_agent)
                .patch(routes::patch_agent),
        )
        .route(
            "/api/agents/{id}/mode",
            axum::routing::put(routes::set_agent_mode),
        )
        .route("/api/profiles", axum::routing::get(routes::list_profiles))
        .route(
            "/api/agents/{id}/restart",
            axum::routing::post(routes::restart_agent),
        )
        .route(
            "/api/agents/{id}/start",
            axum::routing::post(routes::restart_agent),
        )
}

fn mount_agent_api_surface_routes(router: ApiRouter) -> ApiRouter {
    router
        .route(
            "/api/agents/{id}/api",
            axum::routing::get(routes::agent_api_status),
        )
        .route(
            "/api/agents/{id}/api/token/rotate",
            axum::routing::post(routes::rotate_agent_api_token),
        )
        .route(
            "/api/agents/{id}/api/manifest",
            axum::routing::get(routes::agent_api_manifest),
        )
        .route(
            "/api/agents/{id}/api/egress",
            axum::routing::get(routes::agent_api_egress_status),
        )
        .route(
            "/api/agents/{id}/api/egress/configure",
            axum::routing::post(routes::configure_agent_api_egress),
        )
        .route(
            "/api/agents/{id}/api/egress/test",
            axum::routing::post(routes::test_agent_api_egress),
        )
        .route(
            "/api/agents/{id}/api/egress/{queue_id}/retry",
            axum::routing::post(routes::agent_api_egress_retry),
        )
        .route(
            "/api/agents/{id}/api/events",
            axum::routing::get(routes::agent_api_events),
        )
}

fn mount_agent_session_routes(router: ApiRouter) -> ApiRouter {
    router
        .route(
            "/api/agents/{id}/message",
            axum::routing::post(routes::send_message),
        )
        .route(
            "/api/agents/{id}/message/stream",
            axum::routing::post(routes::send_message_stream),
        )
        .route(
            "/api/agents/{id}/message/answer",
            axum::routing::post(routes::answer_message),
        )
        .route(
            "/api/agents/{id}/session",
            axum::routing::get(routes::get_agent_session),
        )
        .route(
            "/api/agents/{id}/sessions",
            axum::routing::get(routes::list_agent_sessions).post(routes::create_agent_session),
        )
        .route(
            "/api/agents/{id}/sessions/{session_id}/switch",
            axum::routing::post(routes::switch_agent_session),
        )
        .route(
            "/api/agents/{id}/session/reset",
            axum::routing::post(routes::reset_session),
        )
        .route(
            "/api/agents/{id}/session/restore",
            axum::routing::post(routes::restore_session),
        )
        .route(
            "/api/agents/{id}/history",
            axum::routing::delete(routes::clear_agent_history),
        )
        .route(
            "/api/agents/{id}/session/compact",
            axum::routing::post(routes::compact_session),
        )
        .route("/api/agents/{id}/ws", axum::routing::get(ws::agent_ws))
        .route(
            "/api/agents/{id}/interrupt",
            axum::routing::post(routes::interrupt_agent),
        )
}

fn mount_agent_model_capability_routes(router: ApiRouter) -> ApiRouter {
    router
        .route(
            "/api/agents/{id}/model-switch/plan",
            axum::routing::post(routes::model_switch_plan),
        )
        .route(
            "/api/agents/{id}/model-switch/apply",
            axum::routing::post(routes::model_switch_apply),
        )
        .route(
            "/api/agents/{id}/stop",
            axum::routing::post(routes::stop_agent),
        )
        .route(
            "/api/agents/{id}/model",
            axum::routing::put(routes::set_model),
        )
        .route(
            "/api/agents/{id}/tools",
            axum::routing::get(routes::get_agent_tools).put(routes::set_agent_tools),
        )
        .route(
            "/api/agents/{id}/skills",
            axum::routing::get(routes::get_agent_skills).put(routes::set_agent_skills),
        )
        .route(
            "/api/agents/{id}/mcp_servers",
            axum::routing::get(routes::get_agent_mcp_servers).put(routes::set_agent_mcp_servers),
        )
        .route(
            "/api/agents/{id}/identity",
            axum::routing::patch(routes::update_agent_identity),
        )
        .route(
            "/api/agents/{id}/config",
            axum::routing::patch(routes::patch_agent_config),
        )
        .route(
            "/api/agents/{id}/clone",
            axum::routing::post(routes::clone_agent),
        )
}

fn mount_agent_workspace_routes(router: ApiRouter) -> ApiRouter {
    router
        .route(
            "/api/agents/{id}/files",
            axum::routing::get(routes::list_agent_files),
        )
        .route(
            "/api/agents/{id}/files/{filename}",
            axum::routing::get(routes::get_agent_file).put(routes::set_agent_file),
        )
        .route(
            "/api/agents/{id}/deliveries",
            axum::routing::get(routes::get_agent_deliveries),
        )
        .route(
            "/api/agents/{id}/upload",
            axum::routing::post(routes::upload_file),
        )
}
