use crate::routes::AppState;
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_learning_engine_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route(
            "/api/learning/committed",
            axum::routing::get(crate::learning_routes::list_committed),
        )
        .route(
            "/api/learning/review",
            axum::routing::get(crate::learning_routes::list_review),
        )
        .route(
            "/api/learning/review/{id}/decide",
            axum::routing::post(crate::learning_routes::decide_review),
        )
        .route(
            "/api/learning/metrics",
            axum::routing::get(crate::learning_routes::metrics),
        )
        .route(
            "/api/learning/status",
            axum::routing::get(crate::learning_routes::workflow_status),
        )
        .route(
            "/api/learning/workflows",
            axum::routing::get(crate::learning_routes::list_workflows),
        )
        .route(
            "/api/learning/workflows/{token}/decide",
            axum::routing::post(crate::learning_routes::decide_workflow),
        )
        .route(
            "/api/skills/proposals",
            axum::routing::get(crate::skill_routes::retired_skill_synthesizer),
        )
        .route(
            "/api/skills/patterns",
            axum::routing::get(crate::skill_routes::retired_skill_synthesizer),
        )
        .route(
            "/api/skills/proposals/{id}/decide",
            axum::routing::post(crate::skill_routes::retired_skill_synthesizer),
        )
        .route(
            "/api/skills/metrics",
            axum::routing::get(crate::skill_routes::retired_skill_synthesizer),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use captain_kernel::CaptainKernel;
    use captain_types::config::{DefaultModelConfig, KernelConfig};
    use std::time::Instant;
    use tower::ServiceExt;

    fn test_state() -> (tempfile::TempDir, Arc<AppState>) {
        let tmp = tempfile::tempdir().unwrap();
        let config = KernelConfig {
            home_dir: tmp.path().join("home"),
            data_dir: tmp.path().join("data"),
            default_model: DefaultModelConfig {
                provider: "ollama".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OLLAMA_API_KEY".to_string(),
                base_url: None,
            },
            ..KernelConfig::default()
        };
        let kernel = Arc::new(CaptainKernel::boot_with_config(config).unwrap());
        kernel.set_self_handle();
        let state = Arc::new(AppState {
            kernel,
            started_at: Instant::now(),
            peer_registry: None,
            bridge_manager: tokio::sync::Mutex::new(None),
            channels_config: tokio::sync::RwLock::new(Default::default()),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            ask_user_channels: dashmap::DashMap::new(),
            provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
        });
        (tmp, state)
    }

    #[tokio::test]
    async fn learning_status_route_is_mounted_and_returns_the_shared_contract() {
        let (_tmp, state) = test_state();
        let app = mount_learning_engine_routes(Router::new()).with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/learning/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["schema_version"], 1);
        assert_eq!(payload["expected_model"]["provider"], "ollama");
        assert_eq!(payload["expected_model"]["model"], "test-model");
        assert!(payload.get("state").is_some());
        assert!(payload.get("recovery").is_some());
    }
}
