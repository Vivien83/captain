use crate::routes::{self, AppState};
use axum::Router;
use std::sync::Arc;

pub(crate) fn mount_skill_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/skills", axum::routing::get(routes::list_skills))
        .route(
            "/api/skills/uninstall",
            axum::routing::post(routes::uninstall_skill),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use captain_kernel::CaptainKernel;
    use captain_types::config::{DefaultModelConfig, KernelConfig};
    use std::time::Instant;
    use tower::ServiceExt;

    #[tokio::test]
    async fn remote_skill_marketplace_routes_are_absent() {
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
        let app = mount_skill_routes(Router::new()).with_state(state);

        let removed = [
            (Method::POST, "/api/skills/install"),
            (Method::GET, "/api/marketplace/search?q=demo"),
            (Method::GET, "/api/clawhub/search?q=demo"),
            (Method::GET, "/api/clawhub/browse"),
            (Method::GET, "/api/clawhub/skill/demo"),
            (Method::GET, "/api/clawhub/skill/demo/code"),
            (Method::POST, "/api/clawhub/install"),
        ];
        for (method, path) in removed {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }

        let local_list = app
            .oneshot(Request::get("/api/skills").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(local_list.status(), StatusCode::OK);
        kernel.shutdown();
    }
}
