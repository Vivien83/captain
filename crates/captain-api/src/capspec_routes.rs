//! Authenticated operator API for native Captain Forge capabilities.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_runtime::kernel_handle::CapSpecForgeScope;
use serde::{Deserialize, Deserializer};

use crate::state::AppState;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityScopeQuery {
    pub scope: Option<CapSpecForgeScope>,
    pub workspace: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityInspectQuery {
    pub scope: Option<CapSpecForgeScope>,
    pub workspace: Option<String>,
    #[serde(default)]
    pub include_source: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidateCapabilityRequest {
    pub source: String,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallCapabilityRequest {
    pub source: String,
    pub name: Option<String>,
    pub scope: CapSpecForgeScope,
    pub workspace: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDecision {
    Approve,
    Reject,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecideCapabilityRequest {
    pub decision: CapabilityDecision,
    pub expected_hash: String,
    pub scope: CapSpecForgeScope,
    pub workspace: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollbackCapabilityRequest {
    pub target_hash: String,
    pub scope: CapSpecForgeScope,
    pub workspace: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRunsQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UncertainRunDecision {
    ConfirmSucceeded,
    Retry,
    MarkFailed,
}

#[derive(Debug, Default)]
pub enum PresentJson {
    #[default]
    Missing,
    Present(serde_json::Value),
}

fn deserialize_present_json<'de, D>(deserializer: D) -> Result<PresentJson, D::Error>
where
    D: Deserializer<'de>,
{
    serde_json::Value::deserialize(deserializer).map(PresentJson::Present)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveUncertainRunRequest {
    pub node_id: String,
    pub expected_tool_use_id: String,
    pub expected_attempt: u32,
    pub decision: UncertainRunDecision,
    #[serde(default, deserialize_with = "deserialize_present_json")]
    pub output: PresentJson,
    pub reason: Option<String>,
}

impl ResolveUncertainRunRequest {
    fn into_parts(
        self,
    ) -> Result<
        (
            String,
            captain_capspec::UncertainNodeExpectation,
            captain_capspec::UncertainResolution,
        ),
        String,
    > {
        let node_id = required_trimmed(self.node_id, "node_id")?;
        let tool_use_id = required_trimmed(self.expected_tool_use_id, "expected_tool_use_id")?;
        if self.expected_attempt == 0 {
            return Err("expected_attempt must be greater than zero".to_string());
        }
        let expectation = captain_capspec::UncertainNodeExpectation {
            tool_use_id,
            attempt: self.expected_attempt,
        };
        let resolution = match self.decision {
            UncertainRunDecision::ConfirmSucceeded => {
                if self.reason.is_some() {
                    return Err("confirm_succeeded does not accept reason".to_string());
                }
                captain_capspec::UncertainResolution::ConfirmSucceeded {
                    output: match self.output {
                        PresentJson::Present(output) => output,
                        PresentJson::Missing => {
                            return Err(
                                "confirm_succeeded requires the observed tool output".to_string()
                            );
                        }
                    },
                }
            }
            UncertainRunDecision::Retry => {
                if matches!(self.output, PresentJson::Present(_)) || self.reason.is_some() {
                    return Err("retry does not accept output or reason".to_string());
                }
                captain_capspec::UncertainResolution::Retry
            }
            UncertainRunDecision::MarkFailed => {
                if matches!(self.output, PresentJson::Present(_)) {
                    return Err("mark_failed does not accept output".to_string());
                }
                captain_capspec::UncertainResolution::MarkFailed {
                    reason: required_trimmed(
                        self.reason.unwrap_or_default(),
                        "reason for mark_failed",
                    )?,
                }
            }
        };
        Ok((node_id, expectation, resolution))
    }
}

/// GET /api/capabilities/native
pub async fn list_native_capabilities(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CapabilityScopeQuery>,
) -> impl IntoResponse {
    let workspace = query.workspace.as_deref().map(std::path::Path::new);
    api_result(state.kernel.capspec_management_list(
        query.scope.unwrap_or(CapSpecForgeScope::Effective),
        workspace,
    ))
}

/// GET /api/capabilities/native/{name}
pub async fn inspect_native_capability(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(query): Query<CapabilityInspectQuery>,
) -> impl IntoResponse {
    let workspace = query.workspace.as_deref().map(std::path::Path::new);
    api_result(state.kernel.capspec_management_inspect(
        &name,
        query.scope.unwrap_or(CapSpecForgeScope::Effective),
        workspace,
        query.include_source,
    ))
}

/// POST /api/capabilities/native/validate
pub async fn validate_native_capability(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ValidateCapabilityRequest>,
) -> impl IntoResponse {
    api_result(state.kernel.capspec_management_validate(
        &request.source,
        request.name.as_deref(),
        captain_kernel::CaptainKernel::capspec_control_actor(),
    ))
}

/// POST /api/capabilities/native/install
pub async fn install_native_capability(
    State(state): State<Arc<AppState>>,
    Json(request): Json<InstallCapabilityRequest>,
) -> impl IntoResponse {
    let workspace = request.workspace.as_deref().map(std::path::Path::new);
    api_result(state.kernel.capspec_management_install(
        &request.source,
        request.name.as_deref(),
        request.scope,
        workspace,
        captain_kernel::CaptainKernel::capspec_control_actor(),
    ))
}

/// POST /api/capabilities/native/{name}/decision
pub async fn decide_native_capability(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(request): Json<DecideCapabilityRequest>,
) -> impl IntoResponse {
    let workspace = request.workspace.as_deref().map(std::path::Path::new);
    api_result(state.kernel.capspec_management_decide(
        &name,
        request.scope,
        workspace,
        request.expected_hash.trim(),
        matches!(request.decision, CapabilityDecision::Approve),
        captain_kernel::CaptainKernel::capspec_control_actor(),
    ))
}

/// POST /api/capabilities/native/{name}/rollback
pub async fn rollback_native_capability(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(request): Json<RollbackCapabilityRequest>,
) -> impl IntoResponse {
    let workspace = request.workspace.as_deref().map(std::path::Path::new);
    api_result(state.kernel.capspec_management_rollback(
        &name,
        request.scope,
        workspace,
        request.target_hash.trim(),
        captain_kernel::CaptainKernel::capspec_control_actor(),
    ))
}

/// DELETE /api/capabilities/native/{name}
pub async fn disable_native_capability(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(query): Query<CapabilityScopeQuery>,
) -> impl IntoResponse {
    let workspace = query.workspace.as_deref().map(std::path::Path::new);
    api_result(state.kernel.capspec_management_disable(
        &name,
        query.scope.unwrap_or(CapSpecForgeScope::Global),
        workspace,
        captain_kernel::CaptainKernel::capspec_control_actor(),
    ))
}

/// GET /api/capabilities/native/runs
pub async fn list_native_capability_runs(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CapabilityRunsQuery>,
) -> impl IntoResponse {
    api_result(
        state
            .kernel
            .capspec_management_runs(query.limit.unwrap_or(100)),
    )
}

/// GET /api/capabilities/native/runs/{run_id}
pub async fn inspect_native_capability_run(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    api_result(state.kernel.capspec_management_run(&run_id))
}

/// POST /api/capabilities/native/runs/{run_id}/decision
pub async fn resolve_uncertain_native_capability_run(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
    Json(request): Json<ResolveUncertainRunRequest>,
) -> impl IntoResponse {
    match request.into_parts() {
        Ok((node_id, expectation, resolution)) => api_result(
            state
                .kernel
                .capspec_management_resolve_run(
                    &run_id,
                    &node_id,
                    expectation,
                    resolution,
                    captain_kernel::CaptainKernel::capspec_control_actor(),
                )
                .await,
        ),
        Err(error) => api_result(Err(error)),
    }
}

fn required_trimmed(value: String, field: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        Err(format!("{field} must not be empty"))
    } else {
        Ok(value.to_string())
    }
}

fn api_result(result: Result<serde_json::Value, String>) -> impl IntoResponse {
    match result {
        Ok(value) => (StatusCode::OK, Json(value)),
        Err(error) => {
            let status = error_status(&error);
            (status, Json(serde_json::json!({"error": error})))
        }
    }
}

fn error_status(error: &str) -> StatusCode {
    let error = error.to_ascii_lowercase();
    if error.contains("was not found") || error.contains("not found") {
        StatusCode::NOT_FOUND
    } else if error.contains("expects pending hash")
        || error.contains("waiting")
        || error.contains("already")
        || error.contains("stale capspec decision")
    {
        StatusCode::CONFLICT
    } else if error.contains("database error")
        || error.contains("filesystem error")
        || error.contains("reload failed")
        || error.contains("cannot persist")
    {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::BAD_REQUEST
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use captain_kernel::CaptainKernel;
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

    fn write_source() -> String {
        r#"format = 1
name = "writer"
description = "Write an explicitly approved file."
version = "1.0.0"

[permissions]
tools = ["file_write"]
write_paths = ["/tmp/captain-forge-test.txt"]

[[steps]]
id = "write"
tool = "file_write"
with = { path = "/tmp/captain-forge-test.txt", content = "approved" }
"#
        .to_string()
    }

    async fn json_body(response: axum::response::Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn operator_api_requires_exact_hash_and_preserves_rollback() {
        let (_tmp, state) = test_state();
        let installed = install_native_capability(
            State(state.clone()),
            Json(InstallCapabilityRequest {
                source: write_source(),
                name: Some("writer".to_string()),
                scope: CapSpecForgeScope::Global,
                workspace: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(installed.status(), StatusCode::OK);
        let installed = json_body(installed).await;
        assert_eq!(installed["status"], "pending_approval");
        assert_eq!(installed["ready"], false);
        assert_eq!(installed["human_action_required"], true);
        let hash = installed["pending_hash"].as_str().unwrap().to_string();

        let mismatched = decide_native_capability(
            State(state.clone()),
            Path("writer".to_string()),
            Json(DecideCapabilityRequest {
                decision: CapabilityDecision::Approve,
                expected_hash: "wrong".to_string(),
                scope: CapSpecForgeScope::Global,
                workspace: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(mismatched.status(), StatusCode::CONFLICT);

        let approved = decide_native_capability(
            State(state.clone()),
            Path("writer".to_string()),
            Json(DecideCapabilityRequest {
                decision: CapabilityDecision::Approve,
                expected_hash: hash.clone(),
                scope: CapSpecForgeScope::Global,
                workspace: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(approved.status(), StatusCode::OK);
        assert_eq!(json_body(approved).await["ready"], true);

        let expanded_source = write_source()
            .replace("version = \"1.0.0\"", "version = \"2.0.0\"")
            .replace(
                "write_paths = [\"/tmp/captain-forge-test.txt\"]",
                "write_paths = [\"/tmp/captain-forge-test.txt\", \"/tmp/captain-forge-extra.txt\"]",
            );
        let expanded = install_native_capability(
            State(state.clone()),
            Json(InstallCapabilityRequest {
                source: expanded_source,
                name: Some("writer".to_string()),
                scope: CapSpecForgeScope::Global,
                workspace: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(expanded.status(), StatusCode::OK);
        let expanded = json_body(expanded).await;
        assert_eq!(expanded["status"], "update_pending_approval");
        assert_eq!(expanded["ready"], true);
        let pending_hash = expanded["pending_hash"].as_str().unwrap().to_string();

        let rejected = decide_native_capability(
            State(state.clone()),
            Path("writer".to_string()),
            Json(DecideCapabilityRequest {
                decision: CapabilityDecision::Reject,
                expected_hash: pending_hash,
                scope: CapSpecForgeScope::Global,
                workspace: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(rejected.status(), StatusCode::OK);
        let rejected = json_body(rejected).await;
        assert_eq!(rejected["status"], "update_rejected");
        assert_eq!(rejected["active_hash"], hash);
        assert_eq!(rejected["ready"], true);

        let disabled = disable_native_capability(
            State(state.clone()),
            Path("writer".to_string()),
            Query(CapabilityScopeQuery {
                scope: Some(CapSpecForgeScope::Global),
                ..CapabilityScopeQuery::default()
            }),
        )
        .await
        .into_response();
        assert_eq!(disabled.status(), StatusCode::OK);
        assert_eq!(json_body(disabled).await["status"], "disabled");

        let restored = rollback_native_capability(
            State(state.clone()),
            Path("writer".to_string()),
            Json(RollbackCapabilityRequest {
                target_hash: hash,
                scope: CapSpecForgeScope::Global,
                workspace: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(restored.status(), StatusCode::OK);
        assert_eq!(json_body(restored).await["ready"], true);
        assert!(state
            .kernel
            .audit_log
            .recent(20)
            .iter()
            .any(|entry| entry.agent_id == "control-web"
                && entry.detail.contains("exact-hash-decision")));
        state.kernel.shutdown();
    }

    #[test]
    fn operator_requests_reject_forged_actor_and_unknown_fields() {
        let forged = serde_json::json!({
            "decision": "approve",
            "expected_hash": "hash",
            "scope": "global",
            "actor": "captain"
        });
        let error = serde_json::from_value::<DecideCapabilityRequest>(forged).unwrap_err();
        assert!(
            error.to_string().contains("unknown field `actor`"),
            "{error}"
        );

        let unknown_install = serde_json::json!({
            "source": write_source(),
            "scope": "global",
            "auto_approve": true
        });
        let error =
            serde_json::from_value::<InstallCapabilityRequest>(unknown_install).unwrap_err();
        assert!(
            error.to_string().contains("unknown field `auto_approve`"),
            "{error}"
        );

        let list_query = serde_json::json!({"include_source": true});
        let error = serde_json::from_value::<CapabilityScopeQuery>(list_query).unwrap_err();
        assert!(
            error.to_string().contains("unknown field `include_source`"),
            "{error}"
        );

        let forged_run_actor = serde_json::json!({
            "node_id": "write",
            "expected_tool_use_id": "capspec:run:write:1",
            "expected_attempt": 1,
            "decision": "retry",
            "actor": "captain"
        });
        let error =
            serde_json::from_value::<ResolveUncertainRunRequest>(forged_run_actor).unwrap_err();
        assert!(
            error.to_string().contains("unknown field `actor`"),
            "{error}"
        );
    }

    #[test]
    fn uncertain_run_decisions_have_strict_payload_contracts() {
        let retry: ResolveUncertainRunRequest = serde_json::from_value(serde_json::json!({
            "node_id": "write",
            "expected_tool_use_id": "capspec:run:write:1",
            "expected_attempt": 1,
            "decision": "retry"
        }))
        .unwrap();
        let (_, expectation, resolution) = retry.into_parts().unwrap();
        assert_eq!(expectation.attempt, 1);
        assert!(matches!(
            resolution,
            captain_capspec::UncertainResolution::Retry
        ));

        let missing_output: ResolveUncertainRunRequest =
            serde_json::from_value(serde_json::json!({
                "node_id": "write",
                "expected_tool_use_id": "capspec:run:write:1",
                "expected_attempt": 1,
                "decision": "confirm_succeeded"
            }))
            .unwrap();
        assert!(missing_output
            .into_parts()
            .unwrap_err()
            .contains("requires the observed tool output"));

        let null_output: ResolveUncertainRunRequest = serde_json::from_value(serde_json::json!({
            "node_id": "write",
            "expected_tool_use_id": "capspec:run:write:1",
            "expected_attempt": 1,
            "decision": "confirm_succeeded",
            "output": null
        }))
        .unwrap();
        let (_, _, resolution) = null_output.into_parts().unwrap();
        assert!(matches!(
            resolution,
            captain_capspec::UncertainResolution::ConfirmSucceeded {
                output: serde_json::Value::Null
            }
        ));

        let retry_with_null: ResolveUncertainRunRequest =
            serde_json::from_value(serde_json::json!({
                "node_id": "write",
                "expected_tool_use_id": "capspec:run:write:1",
                "expected_attempt": 1,
                "decision": "retry",
                "output": null
            }))
            .unwrap();
        assert!(retry_with_null.into_parts().is_err());

        let mixed_retry: ResolveUncertainRunRequest = serde_json::from_value(serde_json::json!({
            "node_id": "write",
            "expected_tool_use_id": "capspec:run:write:1",
            "expected_attempt": 1,
            "decision": "retry",
            "reason": "not allowed"
        }))
        .unwrap();
        assert!(mixed_retry.into_parts().is_err());
    }

    #[test]
    fn operator_errors_have_stable_http_classes() {
        assert_eq!(
            error_status("pending revision expects pending hash abc"),
            StatusCode::CONFLICT
        );
        assert_eq!(
            error_status("CapSpec 'missing' was not found"),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            error_status("mutating a CapSpec requires an explicit scope"),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            error_status("CapSpec database error: unavailable"),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
