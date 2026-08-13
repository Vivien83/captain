//! Sanitized execution-target catalog and durable session/project bindings.

use crate::hub_pairing_routes::{execution_node_target_options, ExecutionNodeTargetOption};
use crate::project_lookup_input::{normalize_project_lookup_key, PROJECT_LOOKUP_NOT_FOUND_ERROR};
use crate::routes::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use captain_memory::execution_targets::{
    ExecutionTargetBinding, ExecutionTargetScope, ExecutionTargetStoreError,
};
use captain_runtime::audit::AuditAction;
use captain_types::agent::SessionId;
use captain_wire::ExecutionTarget;
use serde::Deserialize;
use std::sync::Arc;

pub(crate) const EXECUTION_TARGET_POLICY_VERSION: u16 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetExecutionTargetRequest {
    target: ExecutionTarget,
}

pub async fn list_execution_targets(State(state): State<Arc<AppState>>) -> Response {
    let node_options = match execution_node_target_options(&state) {
        Ok(options) => options,
        Err(_) => return storage_unavailable(),
    };
    let mut targets = vec![
        serde_json::json!({
            "target": ExecutionTarget::Auto,
            "label": "Auto",
            "status": "available",
            "online": true,
            "compatible": true,
            "selectable": true,
            "reason_code": null,
            "action": null,
        }),
        serde_json::json!({
            "target": ExecutionTarget::Hub,
            "label": "Hub",
            "status": "online",
            "online": true,
            "compatible": true,
            "selectable": true,
            "reason_code": null,
            "action": null,
        }),
    ];
    for option in node_options {
        let Ok(option) = serde_json::to_value(option) else {
            return storage_unavailable();
        };
        targets.push(option);
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "policy_version": EXECUTION_TARGET_POLICY_VERSION,
            "targets": targets,
        })),
    )
        .into_response()
}

pub async fn get_session_execution_target(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let session_id = match resolve_session(&state, &id) {
        Ok(id) => id,
        Err(error) => return error.into_response(),
    };
    execution_target_binding_response(
        &state,
        ExecutionTargetScope::Session,
        &session_id.to_string(),
    )
}

pub async fn set_session_execution_target(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(request): Json<SetExecutionTargetRequest>,
) -> Response {
    let session_id = match resolve_session(&state, &id) {
        Ok(id) => id,
        Err(error) => return error.into_response(),
    };
    set_execution_target_binding(
        &state,
        ExecutionTargetScope::Session,
        &session_id.to_string(),
        request.target,
    )
}

pub async fn get_project_execution_target(
    State(state): State<Arc<AppState>>,
    Path(id_or_slug): Path<String>,
) -> Response {
    let project_id = match resolve_project_id(&state, &id_or_slug) {
        Ok(id) => id,
        Err(error) => return error.into_response(),
    };
    execution_target_binding_response(&state, ExecutionTargetScope::Project, &project_id)
}

pub async fn set_project_execution_target(
    State(state): State<Arc<AppState>>,
    Path(id_or_slug): Path<String>,
    Json(request): Json<SetExecutionTargetRequest>,
) -> Response {
    let project_id = match resolve_project_id(&state, &id_or_slug) {
        Ok(id) => id,
        Err(error) => return error.into_response(),
    };
    set_execution_target_binding(
        &state,
        ExecutionTargetScope::Project,
        &project_id,
        request.target,
    )
}

fn execution_target_binding_response(
    state: &AppState,
    scope: ExecutionTargetScope,
    scope_id: &str,
) -> Response {
    let binding = match state.kernel.memory.execution_targets().get(scope, scope_id) {
        Ok(binding) => binding,
        Err(_) => return storage_unavailable(),
    };
    let options = match execution_node_target_options(state) {
        Ok(options) => options,
        Err(_) => return storage_unavailable(),
    };
    binding_response(scope, scope_id, binding, &options)
}

fn set_execution_target_binding(
    state: &AppState,
    scope: ExecutionTargetScope,
    scope_id: &str,
    target: ExecutionTarget,
) -> Response {
    if target.validate().is_err() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "invalid_execution_target",
            "Execution target is invalid",
        );
    }
    let options = match execution_node_target_options(state) {
        Ok(options) => options,
        Err(_) => return storage_unavailable(),
    };
    if let ExecutionTarget::Node { .. } = &target {
        match matching_node_option(&target, &options) {
            Some(option) if option.selectable => {}
            Some(option) => {
                return api_error_with_action(
                    StatusCode::CONFLICT,
                    option.reason_code.unwrap_or("execution_target_unavailable"),
                    "This Node workspace is not currently selectable",
                    option.action,
                )
            }
            None => {
                return api_error_with_action(
                    StatusCode::CONFLICT,
                    "execution_target_not_authorized",
                    "This Node workspace is not available to the Hub",
                    Some("Choose Auto, Hub or an available Node workspace."),
                )
            }
        }
    }

    let binding = match state.kernel.memory.execution_targets().set(
        scope,
        scope_id,
        &target,
        chrono::Utc::now().timestamp_millis(),
    ) {
        Ok(binding) => binding,
        Err(
            ExecutionTargetStoreError::InvalidScopeId
            | ExecutionTargetStoreError::InvalidTarget
            | ExecutionTargetStoreError::InvalidTimestamp,
        ) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_execution_target",
                "Execution target is invalid",
            )
        }
        Err(_) => return storage_unavailable(),
    };
    state.kernel.audit_log.record_or_alert(
        "execution_target",
        AuditAction::ConfigChange,
        "execution target changed",
        target_audit_outcome(scope, scope_id, &target),
    );
    binding_response(scope, scope_id, Some(binding), &options)
}

fn binding_response(
    scope: ExecutionTargetScope,
    scope_id: &str,
    binding: Option<ExecutionTargetBinding>,
    options: &[ExecutionNodeTargetOption],
) -> Response {
    let (target, updated_at_ms, source) = match binding {
        Some(binding) => (binding.target, Some(binding.updated_at_ms), "pinned"),
        None => (ExecutionTarget::Auto, None, "default"),
    };
    let availability = target_availability(&target, options);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "scope": scope,
            "scope_id": scope_id,
            "target": target,
            "source": source,
            "updated_at_ms": updated_at_ms,
            "availability": availability,
        })),
    )
        .into_response()
}

fn target_availability(
    target: &ExecutionTarget,
    options: &[ExecutionNodeTargetOption],
) -> serde_json::Value {
    match target {
        ExecutionTarget::Auto | ExecutionTarget::Hub => serde_json::json!({
            "selectable": true,
            "status": "available",
            "reason_code": null,
            "action": null,
        }),
        ExecutionTarget::Node { .. } => matching_node_option(target, options)
            .map(|option| {
                serde_json::json!({
                    "selectable": option.selectable,
                    "status": option.status,
                    "reason_code": option.reason_code,
                    "action": option.action,
                })
            })
            .unwrap_or_else(|| {
                serde_json::json!({
                    "selectable": false,
                    "status": "unavailable",
                    "reason_code": "execution_target_not_authorized",
                    "action": "Choose Auto, Hub or an available Node workspace.",
                })
            }),
    }
}

fn matching_node_option<'a>(
    target: &ExecutionTarget,
    options: &'a [ExecutionNodeTargetOption],
) -> Option<&'a ExecutionNodeTargetOption> {
    options.iter().find(|option| option.target == *target)
}

fn resolve_session(state: &AppState, raw: &str) -> Result<SessionId, ExecutionTargetRouteError> {
    let session_id = raw
        .parse::<uuid::Uuid>()
        .map(SessionId)
        .map_err(|_| ExecutionTargetRouteError::InvalidSession)?;
    match state.kernel.memory.get_session(session_id) {
        Ok(Some(_)) => Ok(session_id),
        Ok(None) => Err(ExecutionTargetRouteError::SessionNotFound),
        Err(_) => Err(ExecutionTargetRouteError::StorageUnavailable),
    }
}

fn resolve_project_id(state: &AppState, raw: &str) -> Result<String, ExecutionTargetRouteError> {
    let key =
        normalize_project_lookup_key(raw).map_err(|_| ExecutionTargetRouteError::InvalidProject)?;
    match state.kernel.memory.project_get(&key) {
        Ok(Some(project)) => Ok(project.id),
        Ok(None) => match state.kernel.memory.project_find_by_slug(&key) {
            Ok(Some(project)) => Ok(project.id),
            Ok(None) => Err(ExecutionTargetRouteError::ProjectNotFound),
            Err(_) => Err(ExecutionTargetRouteError::StorageUnavailable),
        },
        Err(_) => Err(ExecutionTargetRouteError::StorageUnavailable),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionTargetRouteError {
    InvalidSession,
    SessionNotFound,
    InvalidProject,
    ProjectNotFound,
    StorageUnavailable,
}

impl ExecutionTargetRouteError {
    fn into_response(self) -> Response {
        match self {
            Self::InvalidSession => api_error(
                StatusCode::BAD_REQUEST,
                "invalid_session_id",
                "Session identifier is invalid",
            ),
            Self::SessionNotFound => api_error(
                StatusCode::NOT_FOUND,
                "session_not_found",
                "Session not found",
            ),
            Self::InvalidProject => api_error(
                StatusCode::BAD_REQUEST,
                "invalid_project_id",
                "Project identifier is invalid",
            ),
            Self::ProjectNotFound => api_error(
                StatusCode::NOT_FOUND,
                "project_not_found",
                PROJECT_LOOKUP_NOT_FOUND_ERROR,
            ),
            Self::StorageUnavailable => storage_unavailable(),
        }
    }
}

fn target_audit_outcome(
    scope: ExecutionTargetScope,
    scope_id: &str,
    target: &ExecutionTarget,
) -> String {
    let target = match target {
        ExecutionTarget::Auto => "auto".to_string(),
        ExecutionTarget::Hub => "hub".to_string(),
        ExecutionTarget::Node {
            device_id,
            workspace_id,
        } => format!("node:{device_id}:{workspace_id}"),
    };
    format!("scope={scope:?} scope_id={scope_id} target={target}")
}

fn storage_unavailable() -> Response {
    api_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "execution_target_storage_unavailable",
        "Execution target data is temporarily unavailable",
    )
}

fn api_error(status: StatusCode, code: &'static str, message: &str) -> Response {
    api_error_with_action(status, code, message, None)
}

fn api_error_with_action(
    status: StatusCode,
    code: &'static str,
    message: &str,
    action: Option<&str>,
) -> Response {
    (
        status,
        Json(serde_json::json!({
            "error": {
                "code": code,
                "message": message,
                "action": action,
            }
        })),
    )
        .into_response()
}

#[cfg(test)]
#[path = "execution_target_routes_tests.rs"]
mod tests;
