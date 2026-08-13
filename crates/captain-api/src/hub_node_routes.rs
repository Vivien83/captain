//! Device-bearer HTTP adapters for the outbound-only Hub/Node rail.

use crate::state::AppState;
use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use captain_kernel::hub_node_service::{
    HubNodeService, HubNodeServiceError, HubNodeTransportPermit,
};
use captain_memory::hub_node_rail::HubNodeRailError;
use captain_runtime::audit::AuditAction;
use captain_wire::{
    HubNodeCloseRequest, HubNodeConnectRequest, HubNodeDeliveryBatch, HubNodeIngressRequest,
    HubNodePullRequest, HubNodeStreamRequest, NodeTransport, HUB_NODE_CLOSE_PATH,
    HUB_NODE_CONNECT_PATH, HUB_NODE_ENVELOPE_PATH, HUB_NODE_PULL_PATH, HUB_NODE_STREAM_PATH,
    MAX_HUB_NODE_FRAME_BYTES,
};
use futures::stream;
use serde::de::DeserializeOwned;
use std::{convert::Infallible, sync::Arc, time::Duration};
use zeroize::Zeroizing;

pub(crate) const MAX_HUB_NODE_BODY_SIZE: usize = MAX_HUB_NODE_FRAME_BYTES;
const STREAM_KEEP_ALIVE_SECS: u64 = 10;

pub(crate) struct AuthorizedNode {
    pub(crate) access_token: Zeroizing<String>,
    pub(crate) device_id: String,
}

struct HubNodeStreamState {
    service: HubNodeService,
    access_token: Zeroizing<String>,
    request: HubNodePullRequest,
    pending: Option<HubNodeDeliveryBatch>,
    last_sent_sequence: u64,
    _permit: HubNodeTransportPermit,
}

impl Drop for HubNodeStreamState {
    fn drop(&mut self) {
        if let Err(error) = self.service.close_permitted_connection(
            &self._permit,
            &self.request.device_id,
            &self.request.connection_id,
            NodeTransport::HttpStream,
            Some("http_stream_closed"),
        ) {
            tracing::info!(
                error_kind = transport_error_kind(&error),
                "Hub Node HTTPS stream durable close was already resolved"
            );
        }
    }
}

pub fn is_hub_node_http_transport_route(method: &Method, path: &str) -> bool {
    (*method == Method::POST
        && matches!(
            path,
            HUB_NODE_CONNECT_PATH
                | HUB_NODE_ENVELOPE_PATH
                | HUB_NODE_PULL_PATH
                | HUB_NODE_CLOSE_PATH
        ))
        || (*method == Method::GET && path == HUB_NODE_STREAM_PATH)
}

pub async fn hub_node_connect(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let authorized = match authorize_node(&state, &headers) {
        Ok(authorized) => authorized,
        Err(error) => return transport_error_response(error),
    };
    let request = match parse_body::<HubNodeConnectRequest>(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if request.validate().is_err() || request.transport == NodeTransport::WebSocket {
        return invalid_transport_request();
    }
    match state.kernel.hub_nodes.open_connection(
        authorized.access_token.as_str(),
        &request.hello,
        request.transport,
    ) {
        Ok(batch) => {
            state.kernel.audit_log.record_or_alert(
                "hub_node",
                AuditAction::WireConnect,
                "Hub Node HTTP connection opened",
                format!(
                    "device_id={} connection_id={} transport={:?}",
                    authorized.device_id, request.hello.connection_id, request.transport
                ),
            );
            (StatusCode::OK, Json(batch)).into_response()
        }
        Err(error) => transport_error_response(error),
    }
}

pub async fn hub_node_envelope(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let authorized = match authorize_node(&state, &headers) {
        Ok(authorized) => authorized,
        Err(error) => return transport_error_response(error),
    };
    let request = match parse_body::<HubNodeIngressRequest>(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if request.validate().is_err() || request.transport == NodeTransport::WebSocket {
        return invalid_transport_request();
    }
    match state.kernel.hub_nodes.apply_envelope(
        authorized.access_token.as_str(),
        &request.envelope,
        request.transport,
    ) {
        Ok((_, batch)) => (StatusCode::OK, Json(batch)).into_response(),
        Err(error) => transport_error_response(error),
    }
}

pub async fn hub_node_pull(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let authorized = match authorize_node(&state, &headers) {
        Ok(authorized) => authorized,
        Err(error) => return transport_error_response(error),
    };
    let request = match parse_body::<HubNodePullRequest>(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if request.validate().is_err() {
        return invalid_transport_request();
    }
    let _permit = match state.kernel.hub_nodes.acquire_transport_permit(
        authorized.access_token.as_str(),
        &request.device_id,
        NodeTransport::LongPoll,
    ) {
        Ok(permit) => permit,
        Err(error) => return transport_error_response(error),
    };

    let deadline = tokio::time::Instant::now() + Duration::from_millis(request.wait_ms);
    loop {
        let batch = match state.kernel.hub_nodes.pull(
            authorized.access_token.as_str(),
            &request,
            NodeTransport::LongPoll,
        ) {
            Ok(batch) => batch,
            Err(error) => return transport_error_response(error),
        };
        if !batch.messages.is_empty()
            || request.wait_ms == 0
            || tokio::time::Instant::now() >= deadline
        {
            return (StatusCode::OK, Json(batch)).into_response();
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let retry = Duration::from_millis(batch.retry_after_ms.unwrap_or(1_000));
        tokio::time::sleep(retry.min(remaining)).await;
    }
}

pub async fn hub_node_stream(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(stream_request): Query<HubNodeStreamRequest>,
) -> Response {
    let authorized = match authorize_node(&state, &headers) {
        Ok(authorized) => authorized,
        Err(error) => return transport_error_response(error),
    };
    if stream_request.validate().is_err() {
        return invalid_transport_request();
    }
    let request = stream_request.pull_request();
    let permit = match state.kernel.hub_nodes.acquire_transport_permit(
        authorized.access_token.as_str(),
        &request.device_id,
        NodeTransport::HttpStream,
    ) {
        Ok(permit) => permit,
        Err(error) => return transport_error_response(error),
    };
    let initial = match state.kernel.hub_nodes.pull(
        authorized.access_token.as_str(),
        &request,
        NodeTransport::HttpStream,
    ) {
        Ok(batch) => batch,
        Err(error) => return transport_error_response(error),
    };
    let stream = stream::unfold(
        HubNodeStreamState {
            service: state.kernel.hub_nodes.clone(),
            access_token: authorized.access_token,
            request,
            pending: Some(initial),
            last_sent_sequence: 0,
            _permit: permit,
        },
        next_stream_event,
    );
    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(STREAM_KEEP_ALIVE_SECS))
                .text("keepalive"),
        )
        .into_response()
}

pub async fn hub_node_close(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let authorized = match authorize_node(&state, &headers) {
        Ok(authorized) => authorized,
        Err(error) => return transport_error_response(error),
    };
    let request = match parse_body::<HubNodeCloseRequest>(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if request.validate().is_err() {
        return invalid_transport_request();
    }
    match state.kernel.hub_nodes.close_connection(
        authorized.access_token.as_str(),
        &request.device_id,
        &request.connection_id,
        request.error_code.as_deref(),
    ) {
        Ok(connection) => {
            state.kernel.audit_log.record_or_alert(
                "hub_node",
                AuditAction::WireConnect,
                "Hub Node connection closed",
                format!(
                    "device_id={} connection_id={} status={:?}",
                    authorized.device_id, connection.connection_id, connection.status
                ),
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "offline"})),
            )
                .into_response()
        }
        Err(error) => transport_error_response(error),
    }
}

async fn next_stream_event(
    mut state: HubNodeStreamState,
) -> Option<(Result<Event, Infallible>, HubNodeStreamState)> {
    loop {
        let batch = match state.pending.take() {
            Some(batch) => batch,
            None => match state.service.pull(
                state.access_token.as_str(),
                &state.request,
                NodeTransport::HttpStream,
            ) {
                Ok(batch) => batch,
                Err(error) => {
                    tracing::info!(
                        error_kind = transport_error_kind(&error),
                        "Hub Node HTTPS stream closed"
                    );
                    return None;
                }
            },
        };
        let newest_sequence = batch
            .messages
            .last()
            .map(|message| message.sequence)
            .unwrap_or(0);
        if newest_sequence > state.last_sent_sequence {
            state.last_sent_sequence = newest_sequence;
            let payload = match serde_json::to_string(&batch) {
                Ok(payload) => payload,
                Err(_) => {
                    tracing::error!("Hub Node HTTPS stream could not serialize a validated batch");
                    return None;
                }
            };
            return Some((Ok(Event::default().event("delivery").data(payload)), state));
        }
        tokio::time::sleep(Duration::from_millis(batch.retry_after_ms.unwrap_or(1_000))).await;
    }
}

pub(crate) fn authorize_node(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthorizedNode, HubNodeServiceError> {
    let access_token = Zeroizing::new(
        bearer_token(headers)
            .ok_or(HubNodeServiceError::AuthenticationFailed)?
            .to_string(),
    );
    let device_id = state
        .kernel
        .hub_nodes
        .authorize_transport_token(access_token.as_str())?;
    Ok(AuthorizedNode {
        access_token,
        device_id,
    })
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer")
        || token.is_empty()
        || token.chars().any(char::is_whitespace)
    {
        return None;
    }
    Some(token)
}

#[allow(clippy::result_large_err)]
fn parse_body<T: DeserializeOwned>(body: &[u8]) -> Result<T, Response> {
    if body.len() > MAX_HUB_NODE_BODY_SIZE {
        return Err(api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "hub_node_body_too_large",
            "Hub Node request is too large",
        ));
    }
    serde_json::from_slice(body).map_err(|_| invalid_transport_request())
}

fn invalid_transport_request() -> Response {
    api_error(
        StatusCode::BAD_REQUEST,
        "invalid_hub_node_request",
        "Invalid Hub Node transport request",
    )
}

fn authentication_failed() -> Response {
    let mut response = api_error(
        StatusCode::UNAUTHORIZED,
        "hub_node_authentication_failed",
        "Device authentication failed",
    );
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static("Bearer realm=\"captain-hub-node\""),
    );
    response
}

pub(crate) fn transport_error_response(error: HubNodeServiceError) -> Response {
    tracing::warn!(
        error_kind = transport_error_kind(&error),
        "Hub Node transport operation rejected"
    );
    match error {
        HubNodeServiceError::Disabled => api_error(
            StatusCode::NOT_FOUND,
            "hub_node_disabled",
            "Hub Node transport is not enabled",
        ),
        HubNodeServiceError::AuthenticationFailed
        | HubNodeServiceError::NodeRoleRequired
        | HubNodeServiceError::DeviceIdentityMismatch
        | HubNodeServiceError::Rail(HubNodeRailError::NodeUnavailable) => authentication_failed(),
        HubNodeServiceError::InvalidTransportRequest
        | HubNodeServiceError::Rail(HubNodeRailError::InvalidInput(_))
        | HubNodeServiceError::Rail(HubNodeRailError::InvalidMessageDirection) => {
            invalid_transport_request()
        }
        HubNodeServiceError::TransportMismatch
        | HubNodeServiceError::NodeUnavailable
        | HubNodeServiceError::NodeOffline
        | HubNodeServiceError::NodeIncompatible
        | HubNodeServiceError::WorkspaceNotGranted
        | HubNodeServiceError::ToolFamilyNotGranted
        | HubNodeServiceError::MutationNotGranted
        | HubNodeServiceError::ToolNotSupported
        | HubNodeServiceError::PathPolicyViolation
        | HubNodeServiceError::EffectMismatch
        | HubNodeServiceError::Rail(HubNodeRailError::RunNotFound)
        | HubNodeServiceError::Rail(HubNodeRailError::RunIdConflict)
        | HubNodeServiceError::Rail(HubNodeRailError::IdempotencyConflict)
        | HubNodeServiceError::Rail(HubNodeRailError::LeaseConflict)
        | HubNodeServiceError::Rail(HubNodeRailError::TerminalConflict)
        | HubNodeServiceError::Rail(HubNodeRailError::ConnectionConflict)
        | HubNodeServiceError::Rail(HubNodeRailError::SequenceGap)
        | HubNodeServiceError::Rail(HubNodeRailError::ReplayConflict)
        | HubNodeServiceError::Rail(HubNodeRailError::InvalidAcknowledgement) => api_error(
            StatusCode::CONFLICT,
            "hub_node_state_conflict",
            "Hub Node transport state conflict",
        ),
        HubNodeServiceError::TransportBusy => {
            let mut response = api_error(
                StatusCode::TOO_MANY_REQUESTS,
                "hub_node_transport_busy",
                "Hub Node transport already has an active receive loop",
            );
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
            response
        }
        HubNodeServiceError::DeliveryInvariant
        | HubNodeServiceError::StorageUnavailable
        | HubNodeServiceError::Rail(HubNodeRailError::StorageInvariant)
        | HubNodeServiceError::Rail(HubNodeRailError::Lock(_))
        | HubNodeServiceError::Rail(HubNodeRailError::Database(_)) => api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "hub_node_unavailable",
            "Hub Node transport is temporarily unavailable",
        ),
    }
}

pub(crate) fn transport_error_kind(error: &HubNodeServiceError) -> &'static str {
    match error {
        HubNodeServiceError::Disabled => "disabled",
        HubNodeServiceError::AuthenticationFailed => "authentication",
        HubNodeServiceError::NodeRoleRequired => "role",
        HubNodeServiceError::DeviceIdentityMismatch => "identity",
        HubNodeServiceError::NodeUnavailable
        | HubNodeServiceError::NodeOffline
        | HubNodeServiceError::NodeIncompatible
        | HubNodeServiceError::WorkspaceNotGranted
        | HubNodeServiceError::ToolFamilyNotGranted
        | HubNodeServiceError::MutationNotGranted => "execution_target",
        HubNodeServiceError::ToolNotSupported
        | HubNodeServiceError::PathPolicyViolation
        | HubNodeServiceError::EffectMismatch => "execution_contract",
        HubNodeServiceError::InvalidTransportRequest => "invalid_request",
        HubNodeServiceError::TransportMismatch => "transport_mismatch",
        HubNodeServiceError::TransportBusy => "transport_busy",
        HubNodeServiceError::DeliveryInvariant => "delivery_invariant",
        HubNodeServiceError::StorageUnavailable => "storage",
        HubNodeServiceError::Rail(_) => "rail",
    }
}

fn api_error(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    (
        status,
        Json(serde_json::json!({
            "error": {
                "code": code,
                "message": message,
            }
        })),
    )
        .into_response()
}

#[cfg(test)]
#[path = "hub_node_routes_tests.rs"]
mod tests;
