//! Authenticated transport orchestration for the durable local Node rail.

use crate::{
    network::{NodeHttpClient, NodeHttpStream, NodeNetworkError, NodeWebSocket},
    pairing::NodeAccessToken,
    rail::{
        NodeBootstrap, NodeBootstrapCapabilityState, NodeRailError, NodeRailSnapshot, NodeRailStore,
    },
};
use captain_wire::{
    CapabilityDescriptor, HubNodeCloseRequest, HubNodeDeliveryBatch, HubNodeEnvelope,
    HubNodeMessage, HubNodePullRequest, HubNodeStreamRequest, NodeTransport,
    HUB_NODE_PROTOCOL_VERSION, MAX_HUB_NODE_BATCH_MESSAGES, MAX_HUB_NODE_LONG_POLL_WAIT_MS,
};
use serde::{Deserialize, Serialize};
use std::{fmt, time::Duration};
use thiserror::Error;

const PREFERRED_TRANSPORTS: [NodeTransport; 3] = [
    NodeTransport::WebSocket,
    NodeTransport::HttpStream,
    NodeTransport::LongPoll,
];
const MAX_HANDSHAKE_ROUNDS: usize = 128;
const MAX_FLUSH_ENVELOPES: usize = 4_096;
const MAX_FALLBACK_HISTORY: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeTransportFailure {
    UpgradeFailed,
    TimedOut,
    NetworkUnavailable,
    TransportClosed,
    HubUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeTransportFallback {
    pub transport: NodeTransport,
    pub reason: NodeTransportFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeHeartbeatPolicy {
    interval_ms: u64,
    lease_duration_ms: u64,
}

impl NodeHeartbeatPolicy {
    pub fn interval(self) -> Duration {
        Duration::from_millis(self.interval_ms)
    }

    pub fn lease_duration(self) -> Duration {
        Duration::from_millis(self.lease_duration_ms)
    }

    fn from_welcome(interval_ms: u64, lease_duration_ms: u64) -> Result<Self, NodeLinkError> {
        if interval_ms == 0 || lease_duration_ms <= interval_ms {
            return Err(NodeLinkError::InvalidHandshake);
        }
        Ok(Self {
            interval_ms,
            lease_duration_ms,
        })
    }
}

enum ActiveTransport {
    WebSocket(Box<NodeWebSocket>),
    HttpStream(Box<NodeHttpStream>),
    LongPoll,
    Disconnected(NodeTransport),
}

impl ActiveTransport {
    fn kind(&self) -> NodeTransport {
        match self {
            Self::WebSocket(_) => NodeTransport::WebSocket,
            Self::HttpStream(_) => NodeTransport::HttpStream,
            Self::LongPoll => NodeTransport::LongPoll,
            Self::Disconnected(transport) => *transport,
        }
    }
}

impl fmt::Debug for ActiveTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ActiveTransport")
            .field(&self.kind())
            .finish()
    }
}

pub struct NodeRailLink {
    client: NodeHttpClient,
    rail: NodeRailStore,
    access_token: NodeAccessToken,
    bootstrap: NodeBootstrap,
    active: ActiveTransport,
    heartbeat_policy: NodeHeartbeatPolicy,
    active_run_ids: Vec<String>,
    fallbacks: Vec<NodeTransportFallback>,
}

impl NodeRailLink {
    pub async fn connect(
        client: NodeHttpClient,
        rail: NodeRailStore,
        access_token: NodeAccessToken,
        capabilities: &CapabilityDescriptor,
        active_run_ids: &[String],
    ) -> Result<Self, NodeLinkError> {
        ensure_token_valid(&access_token)?;
        rail.ensure_hub_identity(&client.hub_sha256())?;
        let bootstrap = rail.bootstrap_hello(capabilities, active_run_ids, current_time_ms()?)?;
        let transports = advertised_transports(&bootstrap.envelope)?;
        let mut normalized_active_run_ids = active_run_ids.to_vec();
        normalized_active_run_ids.sort();
        let mut fallbacks = Vec::new();

        for transport in transports {
            let opened = open_transport(
                &client,
                access_token.as_str(),
                &bootstrap.envelope,
                transport,
            )
            .await;
            let (mut active, initial) = match opened {
                Ok(opened) => opened,
                Err(error) if is_fallback_safe(&error) => {
                    fallbacks.push(fallback_for(transport, &error));
                    continue;
                }
                Err(error) => return Err(NodeLinkError::Network(error)),
            };

            let established = complete_handshake(
                &client,
                &rail,
                access_token.as_str(),
                transport,
                &mut active,
                initial,
            )
            .await;
            if let Err(NodeLinkError::Network(error)) = established {
                if is_fallback_safe(&error) {
                    fallbacks.push(fallback_for(transport, &error));
                    continue;
                }
                return Err(NodeLinkError::Network(error));
            }
            let heartbeat_policy = established?;

            rail.ensure_heartbeat(active_run_ids, current_time_ms()?)?;
            match flush_pending(
                &client,
                &rail,
                access_token.as_str(),
                transport,
                &mut active,
            )
            .await
            {
                Ok(_) => {
                    return Ok(Self {
                        client,
                        rail,
                        access_token,
                        bootstrap,
                        active,
                        heartbeat_policy,
                        active_run_ids: normalized_active_run_ids,
                        fallbacks,
                    });
                }
                Err(NodeLinkError::Network(error)) if is_fallback_safe(&error) => {
                    fallbacks.push(fallback_for(transport, &error));
                }
                Err(error) => return Err(error),
            }
        }

        Err(NodeLinkError::TransportsUnavailable {
            attempts: fallbacks,
        })
    }

    pub fn transport(&self) -> NodeTransport {
        self.active.kind()
    }

    pub fn heartbeat_policy(&self) -> NodeHeartbeatPolicy {
        self.heartbeat_policy
    }

    pub fn active_runs_match(&self, active_run_ids: &[String]) -> bool {
        let mut normalized = active_run_ids.to_vec();
        normalized.sort();
        normalized == self.active_run_ids
    }

    pub fn capability_state(&self) -> NodeBootstrapCapabilityState {
        self.bootstrap.capability_state
    }

    pub fn fallbacks(&self) -> &[NodeTransportFallback] {
        &self.fallbacks
    }

    pub fn snapshot(&self) -> Result<NodeRailSnapshot, NodeLinkError> {
        self.rail.snapshot().map_err(Into::into)
    }

    pub fn replace_access_token(
        &mut self,
        access_token: NodeAccessToken,
    ) -> Result<(), NodeLinkError> {
        ensure_token_valid(&access_token)?;
        if HUB_NODE_PROTOCOL_VERSION
            .negotiate(access_token.protocol_version)
            .is_err()
        {
            return Err(NodeLinkError::InvalidAccessToken);
        }
        self.access_token = access_token;
        Ok(())
    }

    pub async fn flush_pending(&mut self) -> Result<NodeRailSnapshot, NodeLinkError> {
        ensure_token_valid(&self.access_token)?;
        let flushed = flush_pending(
            &self.client,
            &self.rail,
            self.access_token.as_str(),
            self.active.kind(),
            &mut self.active,
        )
        .await;
        match flushed {
            Ok(_) => self.snapshot(),
            Err(NodeLinkError::Network(error)) if is_fallback_safe(&error) => {
                self.recover_from(error).await?;
                self.snapshot()
            }
            Err(error) => Err(error),
        }
    }

    pub async fn set_active_runs(
        &mut self,
        active_run_ids: &[String],
    ) -> Result<NodeRailSnapshot, NodeLinkError> {
        let mut normalized = active_run_ids.to_vec();
        normalized.sort();
        if normalized == self.active_run_ids {
            return self.snapshot();
        }
        self.refresh_presence(&normalized).await
    }

    /// Refresh the Hub lease even when the active-run set is unchanged.
    ///
    /// The outbox still deduplicates an unacknowledged heartbeat, while a
    /// previously acknowledged one receives a fresh monotonic sequence. This
    /// keeps idle WebSocket and long-poll Nodes selectable without weakening
    /// the durable rail contract.
    pub async fn refresh_presence(
        &mut self,
        active_run_ids: &[String],
    ) -> Result<NodeRailSnapshot, NodeLinkError> {
        let mut normalized = active_run_ids.to_vec();
        normalized.sort();
        self.rail
            .ensure_heartbeat(&normalized, current_time_ms()?)?;
        self.active_run_ids = normalized;
        self.flush_pending().await
    }

    /// Receives one transport batch, commits it, and drains every resulting
    /// ACK or existing durable output before returning the coherent snapshot.
    pub async fn synchronize_once(&mut self) -> Result<NodeRailSnapshot, NodeLinkError> {
        ensure_token_valid(&self.access_token)?;
        let received = receive_batch(
            &self.client,
            &self.rail,
            self.access_token.as_str(),
            &mut self.active,
            self.heartbeat_policy
                .interval_ms
                .min(MAX_HUB_NODE_LONG_POLL_WAIT_MS),
        )
        .await;
        let batch = match received {
            Ok(batch) => batch,
            Err(NodeLinkError::Network(error)) if is_fallback_safe(&error) => {
                self.recover_from(error).await?;
                return self.snapshot();
            }
            Err(error) => return Err(error),
        };
        observe_batch(&self.rail, &batch, self.active.kind())?;
        self.flush_pending().await
    }

    pub async fn close(mut self, error_code: Option<&str>) -> Result<(), NodeLinkError> {
        ensure_token_valid(&self.access_token)?;
        self.flush_pending().await?;
        let snapshot = self.rail.snapshot()?;
        self.client
            .close_http(
                self.access_token.as_str(),
                &HubNodeCloseRequest {
                    protocol_version: HUB_NODE_PROTOCOL_VERSION,
                    device_id: snapshot.device_id,
                    connection_id: snapshot.connection_id,
                    error_code: error_code.map(ToString::to_string),
                },
            )
            .await
            .map_err(Into::into)
    }

    async fn recover_from(&mut self, initial_error: NodeNetworkError) -> Result<(), NodeLinkError> {
        ensure_token_valid(&self.access_token)?;
        let failed_transport = self.active.kind();
        let previous = std::mem::replace(
            &mut self.active,
            ActiveTransport::Disconnected(failed_transport),
        );
        drop(previous);
        let first = fallback_for(failed_transport, &initial_error);
        self.record_fallback(first);
        let mut attempts = vec![first];
        let transports = recovery_order(&self.bootstrap.envelope, failed_transport)?;

        for transport in transports {
            let opened = open_transport(
                &self.client,
                self.access_token.as_str(),
                &self.bootstrap.envelope,
                transport,
            )
            .await;
            let (mut active, initial) = match opened {
                Ok(opened) => opened,
                Err(error) if is_fallback_safe(&error) => {
                    let fallback = fallback_for(transport, &error);
                    self.record_fallback(fallback);
                    attempts.push(fallback);
                    continue;
                }
                Err(error) => return Err(NodeLinkError::Network(error)),
            };
            let established = complete_handshake(
                &self.client,
                &self.rail,
                self.access_token.as_str(),
                transport,
                &mut active,
                initial,
            )
            .await;
            if let Err(NodeLinkError::Network(error)) = established {
                if is_fallback_safe(&error) {
                    let fallback = fallback_for(transport, &error);
                    self.record_fallback(fallback);
                    attempts.push(fallback);
                    continue;
                }
                return Err(NodeLinkError::Network(error));
            }
            let heartbeat_policy = established?;
            self.rail
                .ensure_heartbeat(&self.active_run_ids, current_time_ms()?)?;
            match flush_pending(
                &self.client,
                &self.rail,
                self.access_token.as_str(),
                transport,
                &mut active,
            )
            .await
            {
                Ok(_) => {
                    self.active = active;
                    self.heartbeat_policy = heartbeat_policy;
                    return Ok(());
                }
                Err(NodeLinkError::Network(error)) if is_fallback_safe(&error) => {
                    let fallback = fallback_for(transport, &error);
                    self.record_fallback(fallback);
                    attempts.push(fallback);
                }
                Err(error) => return Err(error),
            }
        }
        Err(NodeLinkError::TransportsUnavailable { attempts })
    }

    fn record_fallback(&mut self, fallback: NodeTransportFallback) {
        if self.fallbacks.len() == MAX_FALLBACK_HISTORY {
            self.fallbacks.remove(0);
        }
        self.fallbacks.push(fallback);
    }
}

impl fmt::Debug for NodeRailLink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeRailLink")
            .field("transport", &self.transport())
            .field("capability_state", &self.capability_state())
            .field("fallbacks", &self.fallbacks)
            .field("access_token", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

async fn open_transport(
    client: &NodeHttpClient,
    access_token: &str,
    hello: &HubNodeEnvelope,
    transport: NodeTransport,
) -> Result<(ActiveTransport, HubNodeDeliveryBatch), NodeNetworkError> {
    match transport {
        NodeTransport::WebSocket => {
            let mut socket = client.open_rail_websocket(access_token).await?;
            socket.send_envelope(hello).await?;
            let initial = socket.next_delivery(client.request_timeout()).await?;
            Ok((ActiveTransport::WebSocket(Box::new(socket)), initial))
        }
        NodeTransport::HttpStream => {
            let initial = client
                .connect_http(access_token, NodeTransport::HttpStream, hello)
                .await?;
            let stream = client
                .open_http_stream(access_token, &stream_request(hello))
                .await?;
            Ok((ActiveTransport::HttpStream(Box::new(stream)), initial))
        }
        NodeTransport::LongPoll => {
            let initial = client
                .connect_http(access_token, NodeTransport::LongPoll, hello)
                .await?;
            Ok((ActiveTransport::LongPoll, initial))
        }
    }
}

async fn complete_handshake(
    client: &NodeHttpClient,
    rail: &NodeRailStore,
    access_token: &str,
    transport: NodeTransport,
    active: &mut ActiveTransport,
    mut batch: HubNodeDeliveryBatch,
) -> Result<NodeHeartbeatPolicy, NodeLinkError> {
    for _ in 0..MAX_HANDSHAKE_ROUNDS {
        if inspect_welcome(&batch, transport)?.is_none()
            && batch
                .messages
                .iter()
                .any(|envelope| !matches!(&envelope.message, HubNodeMessage::Superseded { .. }))
        {
            return Err(NodeLinkError::InvalidHandshake);
        }
        if let Some(policy) = observe_batch(rail, &batch, transport)? {
            return Ok(policy);
        }
        batch = exchange_next(
            client,
            rail,
            access_token,
            active,
            MAX_HUB_NODE_LONG_POLL_WAIT_MS,
        )
        .await?;
    }
    Err(NodeLinkError::HandshakeBudgetExceeded)
}

async fn flush_pending(
    client: &NodeHttpClient,
    rail: &NodeRailStore,
    access_token: &str,
    transport: NodeTransport,
    active: &mut ActiveTransport,
) -> Result<usize, NodeLinkError> {
    let mut sent = 0usize;
    loop {
        let Some(envelope) = rail.pending_outbound(1)?.into_iter().next() else {
            return Ok(sent);
        };
        if sent >= MAX_FLUSH_ENVELOPES {
            return Err(NodeLinkError::FlushBudgetExceeded);
        }
        let sequence = envelope.sequence;
        let batch = send_envelope(client, access_token, active, &envelope).await?;
        observe_batch(rail, &batch, transport)?;
        if rail.snapshot()?.acknowledged_node_sequence < sequence {
            return Err(NodeLinkError::EnvelopeNotAcknowledged { sequence });
        }
        sent += 1;
    }
}

async fn exchange_next(
    client: &NodeHttpClient,
    rail: &NodeRailStore,
    access_token: &str,
    active: &mut ActiveTransport,
    long_poll_wait_ms: u64,
) -> Result<HubNodeDeliveryBatch, NodeLinkError> {
    if let Some(envelope) = rail.pending_outbound(1)?.into_iter().next() {
        send_envelope(client, access_token, active, &envelope).await
    } else {
        receive_batch(client, rail, access_token, active, long_poll_wait_ms).await
    }
}

async fn send_envelope(
    client: &NodeHttpClient,
    access_token: &str,
    active: &mut ActiveTransport,
    envelope: &HubNodeEnvelope,
) -> Result<HubNodeDeliveryBatch, NodeLinkError> {
    match active {
        ActiveTransport::WebSocket(socket) => {
            socket.send_envelope(envelope).await?;
            socket
                .next_delivery(client.request_timeout())
                .await
                .map_err(Into::into)
        }
        ActiveTransport::HttpStream(_) => client
            .send_http_envelope(access_token, NodeTransport::HttpStream, envelope)
            .await
            .map_err(Into::into),
        ActiveTransport::LongPoll => client
            .send_http_envelope(access_token, NodeTransport::LongPoll, envelope)
            .await
            .map_err(Into::into),
        ActiveTransport::Disconnected(_) => {
            Err(NodeLinkError::Network(NodeNetworkError::TransportClosed))
        }
    }
}

async fn receive_batch(
    client: &NodeHttpClient,
    rail: &NodeRailStore,
    access_token: &str,
    active: &mut ActiveTransport,
    long_poll_wait_ms: u64,
) -> Result<HubNodeDeliveryBatch, NodeLinkError> {
    match active {
        ActiveTransport::WebSocket(socket) => socket
            .next_delivery(client.request_timeout())
            .await
            .map_err(Into::into),
        ActiveTransport::HttpStream(stream) => stream
            .next_delivery(client.request_timeout())
            .await
            .map_err(Into::into),
        ActiveTransport::LongPoll => {
            let snapshot = rail.snapshot()?;
            client
                .pull_long_poll(
                    access_token,
                    &HubNodePullRequest {
                        protocol_version: HUB_NODE_PROTOCOL_VERSION,
                        device_id: snapshot.device_id,
                        connection_id: snapshot.connection_id,
                        max_messages: MAX_HUB_NODE_BATCH_MESSAGES as u16,
                        wait_ms: long_poll_wait_ms.min(MAX_HUB_NODE_LONG_POLL_WAIT_MS),
                    },
                )
                .await
                .map_err(Into::into)
        }
        ActiveTransport::Disconnected(_) => {
            Err(NodeLinkError::Network(NodeNetworkError::TransportClosed))
        }
    }
}

fn observe_batch(
    rail: &NodeRailStore,
    batch: &HubNodeDeliveryBatch,
    expected_transport: NodeTransport,
) -> Result<Option<NodeHeartbeatPolicy>, NodeLinkError> {
    let welcome = inspect_welcome(batch, expected_transport)?;
    let now_ms = current_time_ms()?;
    rail.observe_delivery(batch, now_ms)?;
    if let Some((sequence, _)) = welcome {
        rail.mark_inbound_applied(sequence, now_ms)?;
    }
    Ok(welcome.map(|(_, policy)| policy))
}

fn inspect_welcome(
    batch: &HubNodeDeliveryBatch,
    expected_transport: NodeTransport,
) -> Result<Option<(u64, NodeHeartbeatPolicy)>, NodeLinkError> {
    let mut selected = None;
    for envelope in &batch.messages {
        if let HubNodeMessage::Welcome {
            transport,
            heartbeat_interval_ms,
            lease_duration_ms,
            ..
        } = &envelope.message
        {
            let policy =
                NodeHeartbeatPolicy::from_welcome(*heartbeat_interval_ms, *lease_duration_ms)?;
            if selected
                .replace((envelope.sequence, *transport, policy))
                .is_some()
                || *transport != expected_transport
            {
                return Err(NodeLinkError::InvalidHandshake);
            }
        }
    }
    Ok(selected.map(|(sequence, _, policy)| (sequence, policy)))
}

fn advertised_transports(hello: &HubNodeEnvelope) -> Result<Vec<NodeTransport>, NodeLinkError> {
    let HubNodeMessage::Hello { capabilities, .. } = &hello.message else {
        return Err(NodeLinkError::InvalidBootstrap);
    };
    let selected = PREFERRED_TRANSPORTS
        .into_iter()
        .filter(|transport| capabilities.transports.contains(transport))
        .collect::<Vec<_>>();
    if selected.is_empty() {
        Err(NodeLinkError::InvalidBootstrap)
    } else {
        Ok(selected)
    }
}

fn recovery_order(
    hello: &HubNodeEnvelope,
    failed_transport: NodeTransport,
) -> Result<Vec<NodeTransport>, NodeLinkError> {
    let advertised = advertised_transports(hello)?;
    let failed_index = advertised
        .iter()
        .position(|transport| *transport == failed_transport)
        .ok_or(NodeLinkError::InvalidBootstrap)?;
    Ok((1..=advertised.len())
        .map(|offset| advertised[(failed_index + offset) % advertised.len()])
        .collect())
}

fn stream_request(hello: &HubNodeEnvelope) -> HubNodeStreamRequest {
    HubNodeStreamRequest {
        protocol_major: hello.protocol_version.major,
        protocol_minor: hello.protocol_version.minor,
        device_id: hello.device_id.clone(),
        connection_id: hello.connection_id.clone(),
    }
}

fn is_fallback_safe(error: &NodeNetworkError) -> bool {
    matches!(
        error,
        NodeNetworkError::WebSocketUpgradeFailed
            | NodeNetworkError::RequestTimedOut
            | NodeNetworkError::NetworkUnavailable
            | NodeNetworkError::TransportClosed
            | NodeNetworkError::HubUnavailable
    )
}

fn fallback_for(transport: NodeTransport, error: &NodeNetworkError) -> NodeTransportFallback {
    let reason = match error {
        NodeNetworkError::WebSocketUpgradeFailed => NodeTransportFailure::UpgradeFailed,
        NodeNetworkError::RequestTimedOut => NodeTransportFailure::TimedOut,
        NodeNetworkError::NetworkUnavailable => NodeTransportFailure::NetworkUnavailable,
        NodeNetworkError::TransportClosed => NodeTransportFailure::TransportClosed,
        NodeNetworkError::HubUnavailable => NodeTransportFailure::HubUnavailable,
        _ => unreachable!("fallback_for requires a fallback-safe network error"),
    };
    NodeTransportFallback { transport, reason }
}

fn ensure_token_valid(access_token: &NodeAccessToken) -> Result<(), NodeLinkError> {
    let now_ms = current_time_ms()?;
    if access_token.issued_at_ms > now_ms
        || access_token.expires_at_ms <= now_ms
        || HUB_NODE_PROTOCOL_VERSION
            .negotiate(access_token.protocol_version)
            .is_err()
    {
        return Err(NodeLinkError::InvalidAccessToken);
    }
    Ok(())
}

fn current_time_ms() -> Result<i64, NodeLinkError> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| NodeLinkError::ClockUnavailable)?;
    i64::try_from(duration.as_millis()).map_err(|_| NodeLinkError::ClockUnavailable)
}

#[derive(Debug, Error)]
pub enum NodeLinkError {
    #[error(transparent)]
    Rail(#[from] NodeRailError),
    #[error(transparent)]
    Network(#[from] NodeNetworkError),
    #[error("Node access token is expired or incompatible")]
    InvalidAccessToken,
    #[error("Node durable bootstrap is invalid")]
    InvalidBootstrap,
    #[error("Hub Node handshake is invalid")]
    InvalidHandshake,
    #[error("Hub Node handshake exceeded its bounded page budget")]
    HandshakeBudgetExceeded,
    #[error("Node outbound flush exceeded its bounded record budget")]
    FlushBudgetExceeded,
    #[error("Hub did not acknowledge Node envelope sequence {sequence}")]
    EnvelopeNotAcknowledged { sequence: u64 },
    #[error("all advertised Hub Node transports are unavailable")]
    TransportsUnavailable {
        attempts: Vec<NodeTransportFallback>,
    },
    #[error("system clock is unavailable")]
    ClockUnavailable,
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;
