//! Per-agent API ingress routes.

use crate::{
    agent_api_audit::{
        recent_agent_api_events, record_ingress_accepted, record_ingress_completed,
        record_ingress_denied, record_ingress_duplicate, record_ingress_failed,
        record_ingress_rejected,
    },
    agent_api_config_status::agent_api_config_status,
    agent_api_egress::{
        agent_api_egress_descriptor, clip_for_callback, deliver_agent_api_callback,
        AgentApiEgressDescriptor,
    },
    agent_api_egress_queue::agent_api_egress_queue_summary,
    agent_api_idempotency::{
        request_fingerprint, reserve_agent_api_request, AgentApiIdempotencyDecision,
        AgentApiIdempotencyStatus, AGENT_API_IDEMPOTENCY_TTL_SECS,
    },
    agent_api_ingress_support::{
        conflict_response, duplicate_response, queue_failed_callback, record_callback_audit,
        update_idempotency_status,
    },
    state::AppState,
};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode},
    response::IntoResponse,
    Json,
};
use captain_runtime::{agent_loop::AgentLoopResult, kernel_handle::KernelHandle};
use captain_types::agent::AgentId;
use captain_types::agent_api::{
    agent_api_audit_events_url, agent_api_ingress_url, agent_api_manifest_url,
    agent_api_token_env as shared_agent_api_token_env, agent_api_token_rotate_url,
    AGENT_API_AUTH_SCHEME, AGENT_API_CHANNEL_TYPE,
};
use governor::{clock::DefaultClock, state::keyed::DashMapStateStore, Quota, RateLimiter};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    num::NonZeroU32,
    sync::{Arc, LazyLock},
};

const MAX_AGENT_API_BODY_SIZE: usize = 128 * 1024;
const MAX_AGENT_API_MESSAGE_SIZE: usize = 64 * 1024;
const MIN_AGENT_API_TOKEN_LEN: usize = 32;
const AGENT_API_RATE_LIMIT_PER_MINUTE: u32 = 60;

type AgentApiLimiter = RateLimiter<String, DashMapStateStore<String>, DefaultClock>;

static AGENT_API_RATE_LIMITER: LazyLock<AgentApiLimiter> = LazyLock::new(|| {
    RateLimiter::keyed(Quota::per_minute(
        NonZeroU32::new(AGENT_API_RATE_LIMIT_PER_MINUTE).expect("non-zero rate limit"),
    ))
});

#[derive(Debug, Deserialize)]
struct AgentApiIngressRequest {
    #[serde(default)]
    request_id: Option<String>,
    message: String,
    #[serde(default)]
    sender_id: Option<String>,
    #[serde(default)]
    sender_name: Option<String>,
    #[serde(default)]
    metadata: Option<serde_json::Value>,
}

struct PreparedAgentApiIngress {
    message: String,
    request_id: Option<String>,
    sender_id: Option<String>,
    sender_name: Option<String>,
    metadata: Option<serde_json::Value>,
    metadata_present: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentApiIngressValidationError {
    EmptyMessage,
    MessageTooLarge,
}

#[derive(Debug, Serialize)]
pub struct AgentApiDescriptor {
    pub ingress_url: String,
    pub token_env: String,
    pub token_configured: bool,
    pub token_rotate_url: String,
    pub auth_scheme: &'static str,
    pub channel_type: &'static str,
    pub max_body_bytes: usize,
    pub max_message_bytes: usize,
    pub rate_limit_per_minute: u32,
    pub egress: AgentApiEgressDescriptor,
    pub audit_events_url: String,
    pub manifest_url: String,
    pub idempotency_key: &'static str,
    pub idempotency_ttl_secs: i64,
}

/// GET /api/agents/:id/api - Describe the dedicated external API surface.
pub async fn agent_api_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    let entry = match state.kernel.registry.get(agent_id) {
        Some(entry) => entry,
        None => return error(StatusCode::NOT_FOUND, "Agent not found"),
    };
    let agent_name = entry.name.clone();
    let api = agent_api_descriptor(&agent_id);
    let config_status =
        agent_api_config_status(&state.kernel.config.home_dir, &agent_id, &api).await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "agent_name": agent_name,
            "api": api,
            "config_status": config_status,
        })),
    )
        .into_response()
}

/// GET /api/agents/:id/api/egress - Inspect queued/dead callback deliveries.
pub async fn agent_api_egress_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    if state.kernel.registry.get(agent_id).is_none() {
        return error(StatusCode::NOT_FOUND, "Agent not found");
    }

    match agent_api_egress_queue_summary(&state.kernel.config.home_dir, &agent_id).await {
        Ok(summary) => (StatusCode::OK, Json(serde_json::json!(summary))).into_response(),
        Err(err) => error(StatusCode::INTERNAL_SERVER_ERROR, &err),
    }
}

/// GET /api/agents/:id/api/events - Inspect recent per-agent API audit events.
pub async fn agent_api_events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    if state.kernel.registry.get(agent_id).is_none() {
        return error(StatusCode::NOT_FOUND, "Agent not found");
    }

    let limit = params
        .get("n")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(50);
    let summary = recent_agent_api_events(state.kernel.audit_log.as_ref(), &agent_id, limit);
    (StatusCode::OK, Json(serde_json::json!(summary))).into_response()
}

/// POST /hooks/agents/:id/ingress - Trigger one agent turn from an external service.
pub async fn agent_api_ingress(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    let prepared = match prepare_agent_api_ingress(&state, &agent_id, &headers, &body).await {
        Ok(prepared) => prepared,
        Err(response) => return response,
    };

    let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone() as Arc<dyn KernelHandle>;
    match state
        .kernel
        .send_message_full(
            agent_id,
            &prepared.message,
            Some(kernel_handle),
            None,
            agent_api_sender_id(&agent_id, prepared.sender_id.clone()),
            agent_api_sender_name(prepared.sender_name.clone()),
            Some("agent_api".to_string()),
        )
        .await
    {
        Ok(result) => complete_agent_api_ingress(&state, &agent_id, prepared, result).await,
        Err(e) => {
            tracing::warn!("agent API ingress failed for agent {id}: {e}");
            fail_agent_api_ingress(&state, &agent_id, prepared, format!("{e}")).await
        }
    }
}

async fn prepare_agent_api_ingress(
    state: &AppState,
    agent_id: &AgentId,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<PreparedAgentApiIngress, axum::response::Response> {
    // Rate limit first: it's keyed on the agent_id path segment alone, so
    // it bounds the cost of *any* traffic to this endpoint — including
    // wrong-token or agent-enumeration probes — before the registry
    // lookup or the token compare run at all. Checking it last (as before)
    // meant unauthenticated requests were never throttled, since a 401
    // short-circuited before reaching the rate-limit check.
    ensure_agent_api_rate_limit(state, agent_id)?;
    ensure_agent_api_agent_exists(state, agent_id)?;
    ensure_agent_api_authorized(state, agent_id, headers)?;
    let req = parse_agent_api_ingress_body(state, agent_id, body)?;

    let request_id = req.request_id.clone();
    reserve_agent_api_ingress_request(state, agent_id, request_id.as_deref(), body).await?;

    let metadata_present = req.metadata.as_ref().is_some_and(|value| !value.is_null());
    record_ingress_accepted(
        state.kernel.audit_log.as_ref(),
        agent_id,
        request_id.as_deref(),
        req.message.len(),
        metadata_present,
    );

    Ok(PreparedAgentApiIngress {
        message: req.message,
        request_id,
        sender_id: req.sender_id,
        sender_name: req.sender_name,
        metadata: req.metadata,
        metadata_present,
    })
}

#[allow(clippy::result_large_err)]
fn ensure_agent_api_agent_exists(
    state: &AppState,
    agent_id: &AgentId,
) -> Result<(), axum::response::Response> {
    if state.kernel.registry.get(*agent_id).is_some() {
        Ok(())
    } else {
        Err(error(StatusCode::NOT_FOUND, "Agent not found"))
    }
}

#[allow(clippy::result_large_err)]
fn ensure_agent_api_authorized(
    state: &AppState,
    agent_id: &AgentId,
    headers: &HeaderMap,
) -> Result<(), axum::response::Response> {
    if validate_agent_api_token(headers, agent_id) {
        return Ok(());
    }
    record_ingress_denied(
        state.kernel.audit_log.as_ref(),
        agent_id,
        "invalid_or_missing_token",
    );
    Err(error(
        StatusCode::UNAUTHORIZED,
        "Invalid or missing agent API token",
    ))
}

#[allow(clippy::result_large_err)]
fn ensure_agent_api_rate_limit(
    state: &AppState,
    agent_id: &AgentId,
) -> Result<(), axum::response::Response> {
    if AGENT_API_RATE_LIMITER
        .check_key(&agent_id.to_string())
        .is_ok()
    {
        return Ok(());
    }
    record_ingress_denied(state.kernel.audit_log.as_ref(), agent_id, "rate_limited");
    Err(error(
        StatusCode::TOO_MANY_REQUESTS,
        "Agent API rate limit exceeded",
    ))
}

#[allow(clippy::result_large_err)]
fn parse_agent_api_ingress_body(
    state: &AppState,
    agent_id: &AgentId,
    body: &Bytes,
) -> Result<AgentApiIngressRequest, axum::response::Response> {
    if body.len() > MAX_AGENT_API_BODY_SIZE {
        record_ingress_rejected(
            state.kernel.audit_log.as_ref(),
            agent_id,
            None,
            "body_too_large",
        );
        return Err(error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Request too large (max 128KB)",
        ));
    }

    let req: AgentApiIngressRequest = serde_json::from_slice(body).map_err(|_| {
        record_ingress_rejected(
            state.kernel.audit_log.as_ref(),
            agent_id,
            None,
            "invalid_json",
        );
        error(StatusCode::BAD_REQUEST, "Invalid JSON body")
    })?;

    validate_agent_api_ingress_payload(&req).map_err(|err| {
        let (status, reason, message) = match err {
            AgentApiIngressValidationError::EmptyMessage => (
                StatusCode::BAD_REQUEST,
                "empty_message",
                "Message is required",
            ),
            AgentApiIngressValidationError::MessageTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "message_too_large",
                "Message too large (max 64KB)",
            ),
        };
        record_ingress_rejected(
            state.kernel.audit_log.as_ref(),
            agent_id,
            req.request_id.as_deref(),
            reason,
        );
        error(status, message)
    })?;

    Ok(req)
}

fn validate_agent_api_ingress_payload(
    req: &AgentApiIngressRequest,
) -> Result<(), AgentApiIngressValidationError> {
    if req.message.trim().is_empty() {
        return Err(AgentApiIngressValidationError::EmptyMessage);
    }
    if req.message.len() > MAX_AGENT_API_MESSAGE_SIZE {
        return Err(AgentApiIngressValidationError::MessageTooLarge);
    }
    Ok(())
}

async fn reserve_agent_api_ingress_request(
    state: &AppState,
    agent_id: &AgentId,
    request_id: Option<&str>,
    body: &Bytes,
) -> Result<(), axum::response::Response> {
    let fingerprint = request_fingerprint(body);
    match reserve_agent_api_request(
        &state.kernel.config.home_dir,
        agent_id,
        request_id,
        &fingerprint,
    )
    .await
    {
        Ok(AgentApiIdempotencyDecision::Fresh) => Ok(()),
        Ok(AgentApiIdempotencyDecision::Duplicate(entry)) => {
            record_ingress_duplicate(
                state.kernel.audit_log.as_ref(),
                agent_id,
                Some(&entry.request_id),
                entry.status.as_str(),
            );
            Err(duplicate_response(agent_id, entry))
        }
        Ok(AgentApiIdempotencyDecision::Conflict(entry)) => {
            record_ingress_rejected(
                state.kernel.audit_log.as_ref(),
                agent_id,
                Some(&entry.request_id),
                "request_id_conflict",
            );
            Err(conflict_response(agent_id, entry))
        }
        Err(err) => {
            tracing::warn!(error = %err, "agent API idempotency reserve failed");
            record_ingress_rejected(
                state.kernel.audit_log.as_ref(),
                agent_id,
                request_id,
                "idempotency_store_error",
            );
            Err(error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Idempotency guard unavailable",
            ))
        }
    }
}

async fn complete_agent_api_ingress(
    state: &AppState,
    agent_id: &AgentId,
    prepared: PreparedAgentApiIngress,
    result: AgentLoopResult,
) -> axum::response::Response {
    let response = crate::ws::strip_think_tags(&result.response);
    let callback_payload =
        agent_api_completed_callback_payload(agent_id, &prepared, &result, &response);
    let request_id = callback_request_id(&callback_payload);

    record_ingress_completed(
        state.kernel.audit_log.as_ref(),
        agent_id,
        request_id,
        result.iterations,
    );
    update_idempotency_status(
        state,
        agent_id,
        request_id,
        AgentApiIdempotencyStatus::Completed,
    )
    .await;
    let egress = deliver_agent_api_ingress_callback(state, agent_id, &callback_payload).await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "completed",
            "agent_id": agent_id.to_string(),
            "request_id": callback_payload.get("request_id").cloned().unwrap_or(serde_json::Value::Null),
            "response": response,
            "usage": {
                "input_tokens": result.total_usage.input_tokens,
                "output_tokens": result.total_usage.output_tokens,
            },
            "iterations": result.iterations,
            "cost_usd": result.cost_usd,
            "metadata_received": prepared.metadata_present,
            "egress": egress,
        })),
    )
        .into_response()
}

fn agent_api_completed_callback_payload(
    agent_id: &AgentId,
    prepared: &PreparedAgentApiIngress,
    result: &AgentLoopResult,
    response: &str,
) -> serde_json::Value {
    serde_json::json!({
        "event": "agent_api.completed",
        "status": "completed",
        "agent_id": agent_id.to_string(),
        "request_id": prepared.request_id,
        "response": clip_for_callback(response),
        "usage": {
            "input_tokens": result.total_usage.input_tokens,
            "output_tokens": result.total_usage.output_tokens,
        },
        "iterations": result.iterations,
        "cost_usd": result.cost_usd,
        "metadata": prepared.metadata,
    })
}

async fn fail_agent_api_ingress(
    state: &AppState,
    agent_id: &AgentId,
    prepared: PreparedAgentApiIngress,
    message: String,
) -> axum::response::Response {
    let status = agent_api_execution_failure_status(&message);
    let callback_payload = agent_api_failed_callback_payload(agent_id, &prepared, &message);
    let request_id = callback_request_id(&callback_payload);

    record_ingress_failed(
        state.kernel.audit_log.as_ref(),
        agent_id,
        request_id,
        callback_payload
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or("agent execution failed"),
    );
    update_idempotency_status(
        state,
        agent_id,
        request_id,
        AgentApiIdempotencyStatus::Failed,
    )
    .await;
    let egress = deliver_agent_api_ingress_callback(state, agent_id, &callback_payload).await;

    (
        status,
        Json(serde_json::json!({
            "error": format!("Agent API execution failed: {message}"),
            "egress": egress,
        })),
    )
        .into_response()
}

fn agent_api_execution_failure_status(message: &str) -> StatusCode {
    if message.contains("quota") || message.contains("Quota") {
        StatusCode::TOO_MANY_REQUESTS
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

fn agent_api_failed_callback_payload(
    agent_id: &AgentId,
    prepared: &PreparedAgentApiIngress,
    message: &str,
) -> serde_json::Value {
    serde_json::json!({
        "event": "agent_api.failed",
        "status": "failed",
        "agent_id": agent_id.to_string(),
        "request_id": prepared.request_id,
        "error": message,
        "metadata": prepared.metadata,
    })
}

async fn deliver_agent_api_ingress_callback(
    state: &AppState,
    agent_id: &AgentId,
    payload: &serde_json::Value,
) -> crate::agent_api_egress::AgentApiCallbackDelivery {
    let mut egress = deliver_agent_api_callback(agent_id, payload).await;
    queue_failed_callback(state, agent_id, payload, &mut egress).await;
    record_callback_audit(state, agent_id, payload, &egress);
    egress
}

fn callback_request_id(payload: &serde_json::Value) -> Option<&str> {
    payload.get("request_id").and_then(|value| value.as_str())
}

fn agent_api_sender_id(agent_id: &AgentId, sender_id: Option<String>) -> Option<String> {
    Some(sender_id.unwrap_or_else(|| format!("agent-api:{}", short_agent_id(agent_id))))
}

fn agent_api_sender_name(sender_name: Option<String>) -> Option<String> {
    Some(sender_name.unwrap_or_else(|| "Agent API".to_string()))
}

pub fn agent_api_descriptor(agent_id: &AgentId) -> AgentApiDescriptor {
    AgentApiDescriptor {
        ingress_url: agent_api_ingress_url(agent_id),
        token_env: agent_api_token_env(agent_id),
        token_configured: agent_api_token_configured(agent_id),
        token_rotate_url: agent_api_token_rotate_url(agent_id),
        auth_scheme: AGENT_API_AUTH_SCHEME,
        channel_type: AGENT_API_CHANNEL_TYPE,
        max_body_bytes: MAX_AGENT_API_BODY_SIZE,
        max_message_bytes: MAX_AGENT_API_MESSAGE_SIZE,
        rate_limit_per_minute: AGENT_API_RATE_LIMIT_PER_MINUTE,
        egress: agent_api_egress_descriptor(agent_id),
        audit_events_url: agent_api_audit_events_url(agent_id),
        manifest_url: agent_api_manifest_url(agent_id),
        idempotency_key: "request_id",
        idempotency_ttl_secs: AGENT_API_IDEMPOTENCY_TTL_SECS,
    }
}

pub fn is_agent_api_ingress_route(method: &Method, path: &str) -> bool {
    method == Method::POST && path.starts_with("/hooks/agents/") && path.ends_with("/ingress")
}

pub(crate) fn agent_api_token_env(agent_id: &AgentId) -> String {
    shared_agent_api_token_env(agent_id)
}

fn agent_api_token_configured(agent_id: &AgentId) -> bool {
    std::env::var(agent_api_token_env(agent_id))
        .map(|token| token.len() >= MIN_AGENT_API_TOKEN_LEN)
        .unwrap_or(false)
}

fn validate_agent_api_token(headers: &HeaderMap, agent_id: &AgentId) -> bool {
    let expected = match std::env::var(agent_api_token_env(agent_id)) {
        Ok(token) if token.len() >= MIN_AGENT_API_TOKEN_LEN => token,
        _ => return false,
    };

    let Some(provided) = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|header| header.strip_prefix("Bearer "))
    else {
        return false;
    };

    use subtle::ConstantTimeEq;
    provided.len() == expected.len() && provided.as_bytes().ct_eq(expected.as_bytes()).into()
}

#[allow(clippy::result_large_err)]
fn parse_agent_id(id: &str) -> Result<AgentId, axum::response::Response> {
    id.parse()
        .map_err(|_| error(StatusCode::BAD_REQUEST, "Invalid agent ID"))
}

fn short_agent_id(agent_id: &AgentId) -> String {
    agent_id.to_string().chars().take(8).collect()
}

fn error(status: StatusCode, message: &str) -> axum::response::Response {
    (status, Json(serde_json::json!({"error": message}))).into_response()
}

#[cfg(test)]
#[path = "agent_api_route_tests.rs"]
mod agent_api_route_tests;
