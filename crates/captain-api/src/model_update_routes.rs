//! Durable Codex catalog update decisions.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_kernel::error::KernelError;
use captain_kernel::model_switch::ModelSwitchSessionStrategy;
use captain_types::agent::AgentId;
use captain_types::error::CaptainError;
use serde::Deserialize;

use crate::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelUpdateDecision {
    Keep,
    Switch,
}

#[derive(Debug, Deserialize)]
pub struct ModelUpdateDecisionRequest {
    pub model_id: String,
    pub decision: ModelUpdateDecision,
    pub agent_id: Option<String>,
    pub session_strategy: Option<ModelSwitchSessionStrategy>,
}

/// GET /api/models/updates - pending live Codex catalog additions.
pub async fn list_model_updates(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.kernel.codex_model_update_snapshot() {
        Ok(snapshot) => (StatusCode::OK, Json(serde_json::json!(snapshot))),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": error.to_string()})),
        ),
    }
}

/// POST /api/models/updates/decision - keep the current model or switch safely.
pub async fn decide_model_update(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ModelUpdateDecisionRequest>,
) -> impl IntoResponse {
    let model_id = request.model_id.trim();
    if model_id.is_empty() {
        return error(StatusCode::BAD_REQUEST, "Missing 'model_id' field");
    }

    match request.decision {
        ModelUpdateDecision::Keep => match state.kernel.keep_codex_model_update(Some(model_id)) {
            Ok(resolved) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "kept",
                    "resolved": resolved,
                    "message": "Current model retained; no automatic switch was performed."
                })),
            ),
            Err(error) => keep_error_response(error),
        },
        ModelUpdateDecision::Switch => {
            let snapshot = match state.kernel.codex_model_update_snapshot() {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    return error_response(StatusCode::INTERNAL_SERVER_ERROR, error);
                }
            };
            let is_pending = snapshot
                .pending
                .iter()
                .any(|update| update.model_id.eq_ignore_ascii_case(model_id));
            if !is_pending {
                return error(StatusCode::NOT_FOUND, "Codex model update is not pending");
            }
            let Some(agent_id) = request.agent_id.as_deref() else {
                return error(
                    StatusCode::BAD_REQUEST,
                    "Switch decision requires 'agent_id'",
                );
            };
            let agent_id = match agent_id.parse::<AgentId>() {
                Ok(agent_id) => agent_id,
                Err(_) => return error(StatusCode::BAD_REQUEST, "Invalid agent ID"),
            };
            let Some(strategy) = request.session_strategy else {
                return error(
                    StatusCode::BAD_REQUEST,
                    "Switch decision requires 'session_strategy'",
                );
            };

            match state
                .kernel
                .apply_model_switch(agent_id, model_id, Some("codex"), strategy)
            {
                Ok(result) => (StatusCode::OK, Json(serde_json::json!(result))),
                Err(error) => error_response(StatusCode::BAD_REQUEST, error),
            }
        }
    }
}

fn error(status: StatusCode, message: &str) -> (StatusCode, Json<serde_json::Value>) {
    (status, Json(serde_json::json!({"error": message})))
}

fn error_response(
    status: StatusCode,
    source: impl std::fmt::Display,
) -> (StatusCode, Json<serde_json::Value>) {
    error(status, &source.to_string())
}

fn keep_error_response(source: KernelError) -> (StatusCode, Json<serde_json::Value>) {
    let status = if matches!(&source, KernelError::Captain(CaptainError::Config(_))) {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    error_response(status, source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use captain_kernel::{shared_memory_agent_id, CaptainKernel};
    use captain_types::config::{DefaultModelConfig, KernelConfig};
    use std::time::Instant;

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
            clawhub_cache: dashmap::DashMap::new(),
            ask_user_channels: dashmap::DashMap::new(),
            provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
        });
        (tmp, state)
    }

    fn seed_pending_update(state: &AppState) {
        state
            .kernel
            .memory
            .structured_set(
                shared_memory_agent_id(),
                "__captain_codex_model_updates_v1",
                serde_json::json!({
                    "schema_version": 1,
                    "baseline_ready": true,
                    "initialized_at": "2026-07-13T00:00:00Z",
                    "last_checked_at": "2026-07-13T01:00:00Z",
                    "last_success_at": "2026-07-13T01:00:00Z",
                    "consecutive_failures": 0,
                    "known_model_ids": ["codex/gpt-5.6"],
                    "pending": [{
                        "model_id": "codex/gpt-5.6",
                        "display_name": "GPT-5.6 (Codex)",
                        "discovered_at": "2026-07-13T01:00:00Z"
                    }],
                    "recent_decisions": []
                }),
            )
            .unwrap();
    }

    async fn response_json(response: axum::response::Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[test]
    fn decision_payload_requires_explicit_keep_or_switch() {
        let keep: ModelUpdateDecisionRequest = serde_json::from_value(serde_json::json!({
            "model_id": "codex/gpt-5.6",
            "decision": "keep"
        }))
        .unwrap();
        assert!(matches!(keep.decision, ModelUpdateDecision::Keep));

        assert!(
            serde_json::from_value::<ModelUpdateDecisionRequest>(serde_json::json!({
                "model_id": "codex/gpt-5.6",
                "decision": "later"
            }))
            .is_err()
        );
    }

    #[tokio::test]
    async fn pending_update_is_listed_and_requires_an_explicit_safe_decision() {
        let (_tmp, state) = test_state();
        seed_pending_update(&state);

        let listed = list_model_updates(State(state.clone()))
            .await
            .into_response();
        assert_eq!(listed.status(), StatusCode::OK);
        let listed_body = response_json(listed).await;
        assert_eq!(listed_body["pending"][0]["model_id"], "codex/gpt-5.6");

        let captain_id = state
            .kernel
            .registry
            .list()
            .into_iter()
            .find(|agent| agent.name == "captain")
            .unwrap()
            .id
            .to_string();
        let incomplete_switch = decide_model_update(
            State(state.clone()),
            Json(ModelUpdateDecisionRequest {
                model_id: "codex/gpt-5.6".to_string(),
                decision: ModelUpdateDecision::Switch,
                agent_id: Some(captain_id),
                session_strategy: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(incomplete_switch.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            state
                .kernel
                .codex_model_update_snapshot()
                .unwrap()
                .pending
                .len(),
            1
        );

        let kept = decide_model_update(
            State(state.clone()),
            Json(ModelUpdateDecisionRequest {
                model_id: "CODEX/GPT-5.6".to_string(),
                decision: ModelUpdateDecision::Keep,
                agent_id: None,
                session_strategy: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(kept.status(), StatusCode::OK);
        assert!(state
            .kernel
            .codex_model_update_snapshot()
            .unwrap()
            .pending
            .is_empty());
        state.kernel.shutdown();
    }
}
