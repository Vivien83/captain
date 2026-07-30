use crate::routes::{self, AppState};
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_coordination_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route(
            "/api/mcp/servers",
            axum::routing::get(routes::list_mcp_servers),
        )
        .route(
            "/api/audit/recent",
            axum::routing::get(routes::audit_recent),
        )
        .route(
            "/api/audit/verify",
            axum::routing::get(routes::audit_verify),
        )
        .route("/api/logs/stream", axum::routing::get(routes::logs_stream))
        .route("/api/peers", axum::routing::get(routes::list_peers))
        .route(
            "/api/network/status",
            axum::routing::get(routes::network_status),
        )
        .route(
            "/api/comms/topology",
            axum::routing::get(routes::comms_topology),
        )
        .route(
            "/api/comms/events",
            axum::routing::get(routes::comms_events),
        )
        .route(
            "/api/comms/events/stream",
            axum::routing::get(routes::comms_events_stream),
        )
        .route("/api/comms/send", axum::routing::post(routes::comms_send))
        .route("/api/comms/task", axum::routing::post(routes::comms_task))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use captain_kernel::CaptainKernel;
    use captain_types::config::{DefaultModelConfig, KernelConfig};
    use std::time::Instant;
    use tower::ServiceExt;

    #[tokio::test]
    async fn audit_repair_endpoint_is_not_mounted() {
        let temp = tempfile::tempdir().unwrap();
        let config = KernelConfig {
            home_dir: temp.path().to_path_buf(),
            data_dir: temp.path().join("data"),
            default_model: DefaultModelConfig {
                provider: "ollama".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OLLAMA_API_KEY".to_string(),
                base_url: None,
            },
            ..KernelConfig::default()
        };
        let kernel = Arc::new(CaptainKernel::boot_with_config(config).unwrap());
        let state = Arc::new(AppState {
            kernel: Arc::clone(&kernel),
            started_at: Instant::now(),
            peer_registry: None,
            bridge_manager: tokio::sync::Mutex::new(None),
            channels_config: tokio::sync::RwLock::new(Default::default()),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            ask_user_channels: dashmap::DashMap::new(),
            provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
        });
        let app = mount_coordination_routes(Router::new()).with_state(state);

        let response = app
            .oneshot(
                Request::post("/api/audit/repair")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        kernel.shutdown();
    }
}
