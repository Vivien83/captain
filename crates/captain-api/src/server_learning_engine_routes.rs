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
            "/api/learning/workflows/{proposal_id}/retry",
            axum::routing::post(crate::learning_routes::retry_workflow),
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
    use captain_memory::workflow_learning_control::{
        NewWorkflowProposal, WorkflowLearningStore, WorkflowProposalState,
        WorkflowProposalTransition,
    };
    use captain_memory::workflow_learning_queue::{
        NewWorkflowJob, WorkflowJobKind, WorkflowJobStatus,
    };
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

    fn seed_retryable_dead_draft(state: &Arc<AppState>) {
        let store = WorkflowLearningStore::new(state.kernel.memory.usage_conn());
        store
            .create_observed(&NewWorkflowProposal {
                id: "proposal-retry".to_string(),
                idempotency_key: "proposal-retry:observed".to_string(),
                workflow_signature: "a".repeat(64),
                source_agent_id: "captain".to_string(),
                origin_channel: None,
                evidence_json: "{}".to_string(),
                created_at_unix_ms: 1,
            })
            .unwrap();
        for (from, version, to, key, at) in [
            (
                WorkflowProposalState::Observed,
                0,
                WorkflowProposalState::Eligible,
                "proposal-retry:eligible",
                2,
            ),
            (
                WorkflowProposalState::Eligible,
                1,
                WorkflowProposalState::Drafting,
                "proposal-retry:drafting",
                3,
            ),
        ] {
            store
                .transition(&WorkflowProposalTransition {
                    proposal_id: "proposal-retry".to_string(),
                    expected_state: from,
                    expected_version: version,
                    expected_revision_sha256: None,
                    to_state: to,
                    actor: "captain:test".to_string(),
                    reason: "API retry fixture".to_string(),
                    idempotency_key: key.to_string(),
                    snoozed_until_unix_ms: None,
                    occurred_at_unix_ms: at,
                })
                .unwrap();
        }
        store
            .enqueue_job(&NewWorkflowJob {
                id: "draft-retry".to_string(),
                idempotency_key: "job:draft-retry".to_string(),
                proposal_id: "proposal-retry".to_string(),
                revision_sha256: None,
                kind: WorkflowJobKind::Draft,
                payload_json: "{}".to_string(),
                max_attempts: 1,
                run_after_unix_ms: 4,
                created_at_unix_ms: 4,
            })
            .unwrap();
        store.claim_due_job("worker", 4, 1_000).unwrap();
        store
            .mark_job_effect_started("draft-retry", "worker", 5)
            .unwrap();
        let dead = store
            .fail_job_after_known_effect(
                "draft-retry",
                "worker",
                "model_timeout",
                "active model completion timed out",
                true,
                100,
                6,
                None,
            )
            .unwrap();
        assert_eq!(dead.status, WorkflowJobStatus::Dead);
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
        assert_eq!(
            payload["schema_version"],
            captain_types::workflow_learning::WORKFLOW_LEARNING_STATUS_SCHEMA_VERSION
        );
        assert_eq!(payload["expected_model"]["provider"], "ollama");
        assert_eq!(payload["expected_model"]["model"], "test-model");
        assert!(payload.get("state").is_some());
        assert!(payload.get("recovery").is_some());
        assert!(payload["attention"].is_array());
    }

    #[tokio::test]
    async fn learning_retry_route_is_mounted_and_rejects_a_stale_proposal() {
        let (_tmp, state) = test_state();
        let app = mount_learning_engine_routes(Router::new()).with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/learning/workflows/missing/retry")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"expected_error_code":"model_timeout","surface":"web"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn learning_retry_route_requeues_only_the_same_safe_dead_job() {
        let (_tmp, state) = test_state();
        seed_retryable_dead_draft(&state);
        let app = mount_learning_engine_routes(Router::new()).with_state(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/learning/workflows/proposal-retry/retry")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"expected_error_code":"model_timeout","surface":"web"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["proposal_id"], "proposal-retry");
        assert_eq!(payload["stage"], "draft");
        assert_eq!(payload["replayed"], false);
        let job = WorkflowLearningStore::new(state.kernel.memory.usage_conn())
            .get_job("draft-retry")
            .unwrap()
            .unwrap();
        assert_eq!(job.status, WorkflowJobStatus::Pending);
        assert_eq!(job.attempt_count, 0);
    }
}
