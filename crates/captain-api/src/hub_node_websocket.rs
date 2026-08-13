//! Authenticated WebSocket primary transport for outbound Hub/Node links.

use crate::{
    hub_node_routes::{authorize_node, transport_error_kind, transport_error_response},
    state::AppState,
};
use axum::{
    extract::{
        ws::{CloseFrame, Message, WebSocket},
        ConnectInfo, FromRequestParts, Request, State, WebSocketUpgrade,
    },
    http::{header, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
};
use captain_kernel::hub_node_service::{
    HubNodeService, HubNodeServiceError, HubNodeTransportPermit, HUB_NODE_LEASE_DURATION_MS,
};
use captain_runtime::audit::AuditAction;
use captain_wire::{
    HubNodeConnectRequest, HubNodeDeliveryBatch, HubNodeEnvelope, HubNodeIngressRequest,
    HubNodeMessage, HubNodePullRequest, HubNodeWebSocketFrame, NodeTransport,
    HUB_NODE_WEBSOCKET_PATH, MAX_HUB_NODE_BATCH_MESSAGES, MAX_HUB_NODE_FRAME_BYTES,
};
use dashmap::DashMap;
use futures::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const HUB_NODE_WS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const HUB_NODE_WS_POLL_INTERVAL: Duration = Duration::from_secs(1);
const MAX_HUB_NODE_WS_PER_IP: usize = 64;
const MAX_HUB_NODE_WS_MESSAGES_PER_MINUTE: usize = 600;

type WebSocketSender = SplitSink<WebSocket, Message>;
type WebSocketReceiver = SplitStream<WebSocket>;

struct HubNodeWsIpGuard {
    ip: IpAddr,
}

struct HubNodeWebSocketGuards {
    device: HubNodeTransportPermit,
    _ip: HubNodeWsIpGuard,
}

struct IngressRateWindow {
    started_at: Instant,
    messages: usize,
}

impl IngressRateWindow {
    fn new(now: Instant) -> Self {
        Self {
            started_at: now,
            messages: 0,
        }
    }

    fn allow(&mut self, now: Instant) -> bool {
        if now.duration_since(self.started_at) >= Duration::from_secs(60) {
            self.started_at = now;
            self.messages = 0;
        }
        if self.messages >= MAX_HUB_NODE_WS_MESSAGES_PER_MINUTE {
            return false;
        }
        self.messages += 1;
        true
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct DeliveryCursor {
    acknowledged_node_sequence: u64,
    hub_sequence: u64,
}

impl DeliveryCursor {
    fn needs_delivery(self, batch: &HubNodeDeliveryBatch) -> bool {
        batch.acknowledged_node_sequence > self.acknowledged_node_sequence
            || newest_hub_sequence(batch) > self.hub_sequence
    }

    fn observe(&mut self, batch: &HubNodeDeliveryBatch) {
        self.acknowledged_node_sequence = self
            .acknowledged_node_sequence
            .max(batch.acknowledged_node_sequence);
        self.hub_sequence = self.hub_sequence.max(newest_hub_sequence(batch));
    }
}

pub fn is_hub_node_websocket_route(method: &Method, path: &str) -> bool {
    *method == Method::GET && path == HUB_NODE_WEBSOCKET_PATH
}

pub async fn hub_node_websocket(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    let authorized = match authorize_node(&state, request.headers()) {
        Ok(authorized) => authorized,
        Err(error) => return transport_error_response(error),
    };
    let ip = crate::web_auth_security::request_client_ip(
        Some(peer),
        request.headers(),
        &state.kernel.config.deployment,
    );
    let (mut parts, _) = request.into_parts();
    let ws = match WebSocketUpgrade::from_request_parts(&mut parts, &state).await {
        Ok(ws) => ws,
        Err(rejection) => return rejection.into_response(),
    };
    let ip_guard = match try_acquire_ip_slot(ip) {
        Some(guard) => guard,
        None => return websocket_capacity_response(),
    };
    let device_guard = match state.kernel.hub_nodes.acquire_transport_permit(
        authorized.access_token.as_str(),
        &authorized.device_id,
        NodeTransport::WebSocket,
    ) {
        Ok(guard) => guard,
        Err(error) => return transport_error_response(error),
    };
    let guards = HubNodeWebSocketGuards {
        device: device_guard,
        _ip: ip_guard,
    };
    ws.max_message_size(MAX_HUB_NODE_FRAME_BYTES)
        .max_frame_size(MAX_HUB_NODE_FRAME_BYTES)
        .accept_unmasked_frames(false)
        .on_upgrade(move |socket| {
            run_hub_node_websocket(socket, state, authorized.access_token, guards)
        })
        .into_response()
}

async fn run_hub_node_websocket(
    socket: WebSocket,
    state: Arc<AppState>,
    access_token: Zeroizing<String>,
    guards: HubNodeWebSocketGuards,
) {
    let (mut sender, mut receiver) = socket.split();
    let mut ingress_rate = IngressRateWindow::new(Instant::now());
    let hello = match receive_hello(&mut sender, &mut receiver, &mut ingress_rate).await {
        Ok(hello) => hello,
        Err(failure) => {
            close_with_failure(&mut sender, failure).await;
            return;
        }
    };
    let device_id = hello.device_id.clone();
    let connection_id = hello.connection_id.clone();
    let initial = match state.kernel.hub_nodes.open_connection(
        access_token.as_str(),
        &hello,
        NodeTransport::WebSocket,
    ) {
        Ok(batch) => batch,
        Err(error) => {
            close_with_service_error(&mut sender, &error).await;
            return;
        }
    };
    state.kernel.audit_log.record_or_alert(
        "hub_node",
        AuditAction::WireConnect,
        "Hub Node WebSocket connection opened",
        format!("device_id={device_id} connection_id={connection_id}"),
    );

    let mut cursor = DeliveryCursor::default();
    if send_delivery(&mut sender, &initial, &mut cursor)
        .await
        .is_err()
    {
        close_durable_connection(
            &state.kernel.hub_nodes,
            &guards.device,
            &device_id,
            &connection_id,
            NodeTransport::WebSocket,
            "initial_delivery_failed",
        );
        return;
    }

    let pull = HubNodePullRequest {
        protocol_version: hello.protocol_version,
        device_id: device_id.clone(),
        connection_id: connection_id.clone(),
        max_messages: MAX_HUB_NODE_BATCH_MESSAGES as u16,
        wait_ms: 0,
    };
    let mut poll = tokio::time::interval(HUB_NODE_WS_POLL_INTERVAL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    poll.tick().await;
    let mut last_protocol_activity = tokio::time::Instant::now();
    let mut exit_reason = "connection_lost";

    loop {
        tokio::select! {
            frame = receiver.next() => {
                let Some(frame) = frame else { break };
                if let Ok(message) = &frame {
                    if !matches!(message, Message::Close(_))
                        && !ingress_rate.allow(Instant::now())
                    {
                        exit_reason = "message_rate_exceeded";
                        close_with_failure(
                            &mut sender,
                            WebSocketFailure::MessageRateExceeded,
                        )
                        .await;
                        break;
                    }
                }
                match frame {
                    Ok(Message::Text(text)) => {
                        let envelope = match parse_node_frame(text.as_str()) {
                            Ok(envelope) => envelope,
                            Err(failure) => {
                                exit_reason = failure.reason();
                                close_with_failure(&mut sender, failure).await;
                                break;
                            }
                        };
                        let request = HubNodeIngressRequest {
                            transport: NodeTransport::WebSocket,
                            envelope,
                        };
                        if request.validate().is_err() {
                            exit_reason = "invalid_message_direction";
                            close_with_failure(&mut sender, WebSocketFailure::InvalidDirection).await;
                            break;
                        }
                        let batch = match state.kernel.hub_nodes.apply_envelope(
                            access_token.as_str(),
                            &request.envelope,
                            NodeTransport::WebSocket,
                        ) {
                            Ok((_, batch)) => batch,
                            Err(error) => {
                                exit_reason = service_close_reason(&error);
                                close_with_service_error(&mut sender, &error).await;
                                break;
                            }
                        };
                        last_protocol_activity = tokio::time::Instant::now();
                        if send_delivery(&mut sender, &batch, &mut cursor).await.is_err() {
                            exit_reason = "delivery_failed";
                            break;
                        }
                    }
                    Ok(Message::Ping(payload)) => {
                        if sender.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Ok(Message::Pong(_)) => {}
                    Ok(Message::Close(_)) => {
                        exit_reason = "client_closed";
                        break;
                    }
                    Ok(Message::Binary(_)) => {
                        exit_reason = "binary_frame_unsupported";
                        close_with_failure(&mut sender, WebSocketFailure::UnsupportedFrame).await;
                        break;
                    }
                    Err(_) => break,
                }
            }
            _ = poll.tick() => {
                if last_protocol_activity.elapsed()
                    >= Duration::from_millis(HUB_NODE_LEASE_DURATION_MS)
                {
                    exit_reason = "heartbeat_timeout";
                    close_with_failure(&mut sender, WebSocketFailure::HeartbeatTimeout).await;
                    break;
                }
                let batch = match state.kernel.hub_nodes.pull(
                    access_token.as_str(),
                    &pull,
                    NodeTransport::WebSocket,
                ) {
                    Ok(batch) => batch,
                    Err(error) => {
                        exit_reason = service_close_reason(&error);
                        close_with_service_error(&mut sender, &error).await;
                        break;
                    }
                };
                if send_delivery(&mut sender, &batch, &mut cursor).await.is_err() {
                    exit_reason = "delivery_failed";
                    break;
                }
            }
        }
    }

    close_durable_connection(
        &state.kernel.hub_nodes,
        &guards.device,
        &device_id,
        &connection_id,
        NodeTransport::WebSocket,
        exit_reason,
    );
    state.kernel.audit_log.record_or_alert(
        "hub_node",
        AuditAction::WireConnect,
        "Hub Node WebSocket connection closed",
        format!("device_id={device_id} connection_id={connection_id} reason={exit_reason}"),
    );
}

async fn receive_hello(
    sender: &mut WebSocketSender,
    receiver: &mut WebSocketReceiver,
    ingress_rate: &mut IngressRateWindow,
) -> Result<HubNodeEnvelope, WebSocketFailure> {
    let deadline = tokio::time::Instant::now() + HUB_NODE_WS_HANDSHAKE_TIMEOUT;
    loop {
        let frame = tokio::time::timeout_at(deadline, receiver.next())
            .await
            .map_err(|_| WebSocketFailure::HandshakeTimeout)?
            .ok_or(WebSocketFailure::PeerClosed)?
            .map_err(|_| WebSocketFailure::PeerClosed)?;
        if !matches!(frame, Message::Close(_)) && !ingress_rate.allow(Instant::now()) {
            return Err(WebSocketFailure::MessageRateExceeded);
        }
        match frame {
            Message::Text(text) => {
                let envelope = parse_node_frame(text.as_str())?;
                if !matches!(&envelope.message, HubNodeMessage::Hello { .. }) {
                    return Err(WebSocketFailure::ExpectedHello);
                }
                let request = HubNodeConnectRequest {
                    transport: NodeTransport::WebSocket,
                    hello: envelope.clone(),
                };
                request
                    .validate()
                    .map_err(|_| WebSocketFailure::InvalidFrame)?;
                return Ok(envelope);
            }
            Message::Ping(payload) => sender
                .send(Message::Pong(payload))
                .await
                .map_err(|_| WebSocketFailure::PeerClosed)?,
            Message::Pong(_) => {}
            Message::Close(_) => return Err(WebSocketFailure::PeerClosed),
            Message::Binary(_) => return Err(WebSocketFailure::UnsupportedFrame),
        }
    }
}

fn parse_node_frame(text: &str) -> Result<HubNodeEnvelope, WebSocketFailure> {
    if text.len() > MAX_HUB_NODE_FRAME_BYTES {
        return Err(WebSocketFailure::FrameTooLarge);
    }
    let frame: HubNodeWebSocketFrame =
        serde_json::from_str(text).map_err(|_| WebSocketFailure::InvalidFrame)?;
    frame
        .validate()
        .map_err(|_| WebSocketFailure::InvalidFrame)?;
    match frame {
        HubNodeWebSocketFrame::NodeEnvelope { envelope } => Ok(envelope),
        HubNodeWebSocketFrame::HubDelivery { .. } => Err(WebSocketFailure::InvalidDirection),
    }
}

async fn send_delivery(
    sender: &mut WebSocketSender,
    batch: &HubNodeDeliveryBatch,
    cursor: &mut DeliveryCursor,
) -> Result<bool, ()> {
    if !cursor.needs_delivery(batch) {
        return Ok(false);
    }
    let frame = HubNodeWebSocketFrame::HubDelivery {
        batch: batch.clone(),
    };
    frame.validate().map_err(|_| ())?;
    let payload = serde_json::to_string(&frame).map_err(|_| ())?;
    if payload.len() > MAX_HUB_NODE_FRAME_BYTES {
        return Err(());
    }
    sender
        .send(Message::Text(payload.into()))
        .await
        .map_err(|_| ())?;
    cursor.observe(batch);
    Ok(true)
}

fn newest_hub_sequence(batch: &HubNodeDeliveryBatch) -> u64 {
    batch
        .messages
        .last()
        .map(|envelope| envelope.sequence)
        .unwrap_or(0)
}

fn close_durable_connection(
    service: &HubNodeService,
    permit: &HubNodeTransportPermit,
    device_id: &str,
    connection_id: &str,
    transport: NodeTransport,
    reason: &str,
) {
    if let Err(error) = service.close_permitted_connection(
        permit,
        device_id,
        connection_id,
        transport,
        Some(reason),
    ) {
        tracing::info!(
            error_kind = transport_error_kind(&error),
            "Hub Node WebSocket durable close was already resolved"
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebSocketFailure {
    HandshakeTimeout,
    PeerClosed,
    FrameTooLarge,
    InvalidFrame,
    ExpectedHello,
    InvalidDirection,
    UnsupportedFrame,
    HeartbeatTimeout,
    MessageRateExceeded,
}

impl WebSocketFailure {
    fn code(self) -> u16 {
        match self {
            Self::FrameTooLarge => 1009,
            Self::UnsupportedFrame => 1003,
            Self::HeartbeatTimeout | Self::MessageRateExceeded => 1008,
            Self::PeerClosed => 1000,
            Self::HandshakeTimeout
            | Self::InvalidFrame
            | Self::ExpectedHello
            | Self::InvalidDirection => 1002,
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::HandshakeTimeout => "hello_timeout",
            Self::PeerClosed => "peer_closed",
            Self::FrameTooLarge => "frame_too_large",
            Self::InvalidFrame => "invalid_frame",
            Self::ExpectedHello => "hello_required",
            Self::InvalidDirection => "invalid_message_direction",
            Self::UnsupportedFrame => "unsupported_frame",
            Self::HeartbeatTimeout => "heartbeat_timeout",
            Self::MessageRateExceeded => "message_rate_exceeded",
        }
    }
}

async fn close_with_failure(sender: &mut WebSocketSender, failure: WebSocketFailure) {
    if failure == WebSocketFailure::PeerClosed {
        return;
    }
    let _ = sender
        .send(Message::Close(Some(CloseFrame {
            code: failure.code(),
            reason: failure.reason().into(),
        })))
        .await;
}

async fn close_with_service_error(sender: &mut WebSocketSender, error: &HubNodeServiceError) {
    tracing::info!(
        error_kind = transport_error_kind(error),
        "Hub Node WebSocket operation rejected"
    );
    let _ = sender
        .send(Message::Close(Some(CloseFrame {
            code: service_close_code(error),
            reason: service_close_reason(error).into(),
        })))
        .await;
}

fn service_close_code(error: &HubNodeServiceError) -> u16 {
    match error {
        HubNodeServiceError::DeliveryInvariant
        | HubNodeServiceError::StorageUnavailable
        | HubNodeServiceError::Rail(
            captain_memory::hub_node_rail::HubNodeRailError::StorageInvariant
            | captain_memory::hub_node_rail::HubNodeRailError::Lock(_)
            | captain_memory::hub_node_rail::HubNodeRailError::Database(_),
        ) => 1011,
        _ => 1008,
    }
}

fn service_close_reason(error: &HubNodeServiceError) -> &'static str {
    match service_close_code(error) {
        1011 => "transport_unavailable",
        _ => "transport_rejected",
    }
}

fn websocket_capacity_response() -> Response {
    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        axum::Json(serde_json::json!({
            "error": {
                "code": "hub_node_websocket_capacity",
                "message": "Too many Hub Node WebSocket connections from this address"
            }
        })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
    response
}

fn websocket_ip_counts() -> &'static DashMap<IpAddr, AtomicUsize> {
    static COUNTS: OnceLock<DashMap<IpAddr, AtomicUsize>> = OnceLock::new();
    COUNTS.get_or_init(DashMap::new)
}

fn try_acquire_ip_slot(ip: IpAddr) -> Option<HubNodeWsIpGuard> {
    let entry = websocket_ip_counts()
        .entry(ip)
        .or_insert_with(|| AtomicUsize::new(0));
    let previous = entry.value().fetch_add(1, Ordering::AcqRel);
    if previous >= MAX_HUB_NODE_WS_PER_IP {
        entry.value().fetch_sub(1, Ordering::AcqRel);
        return None;
    }
    Some(HubNodeWsIpGuard { ip })
}

impl Drop for HubNodeWsIpGuard {
    fn drop(&mut self) {
        let counts = websocket_ip_counts();
        let returned_to_zero = if let Some(entry) = counts.get(&self.ip) {
            entry.value().fetch_sub(1, Ordering::AcqRel) == 1
        } else {
            false
        };
        if returned_to_zero {
            counts.remove_if(&self.ip, |_, count| count.load(Ordering::Acquire) == 0);
        }
    }
}

#[cfg(test)]
#[path = "hub_node_websocket_tests.rs"]
mod tests;
