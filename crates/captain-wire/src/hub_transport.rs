//! Transport-neutral delivery frames for the outbound-only Hub/Node rail.

use crate::hub_protocol::{
    validate_identifier, HubNodeEnvelope, HubNodeMessage, ProtocolContractError, ProtocolVersion,
    HUB_NODE_PROTOCOL_VERSION,
};
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

pub const MAX_HUB_NODE_BATCH_MESSAGES: usize = 64;
pub const MAX_HUB_NODE_LONG_POLL_WAIT_MS: u64 = 30_000;
pub const MAX_HUB_NODE_FRAME_BYTES: usize = 1024 * 1024 + 64 * 1024;

pub const HUB_NODE_CONNECT_PATH: &str = "/api/hub/nodes/connect";
pub const HUB_NODE_ENVELOPE_PATH: &str = "/api/hub/nodes/envelope";
pub const HUB_NODE_PULL_PATH: &str = "/api/hub/nodes/pull";
pub const HUB_NODE_STREAM_PATH: &str = "/api/hub/nodes/stream";
pub const HUB_NODE_WEBSOCKET_PATH: &str = "/api/hub/nodes/ws";
pub const HUB_NODE_CLOSE_PATH: &str = "/api/hub/nodes/close";

/// Explicit WebSocket frame shape. A tag keeps clients from inferring frame
/// direction from incidental JSON fields.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HubNodeWebSocketFrame {
    NodeEnvelope { envelope: HubNodeEnvelope },
    HubDelivery { batch: HubNodeDeliveryBatch },
}

impl HubNodeWebSocketFrame {
    pub fn validate(&self) -> Result<(), HubTransportContractError> {
        match self {
            Self::NodeEnvelope { envelope } => {
                envelope.validate()?;
                if !matches!(
                    &envelope.message,
                    HubNodeMessage::Hello { .. }
                        | HubNodeMessage::Heartbeat { .. }
                        | HubNodeMessage::RunAccepted { .. }
                        | HubNodeMessage::RunApprovalRequired(_)
                        | HubNodeMessage::RunRejected(_)
                        | HubNodeMessage::RunProgress { .. }
                        | HubNodeMessage::RunCompleted(_)
                        | HubNodeMessage::AckOnly
                        | HubNodeMessage::ProtocolError { .. }
                ) {
                    return Err(HubTransportContractError::InvalidMessageDirection);
                }
            }
            Self::HubDelivery { batch } => batch.validate()?,
        }
        Ok(())
    }
}

impl fmt::Debug for HubNodeWebSocketFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NodeEnvelope { envelope } => formatter
                .debug_struct("NodeEnvelope")
                .field("envelope", envelope)
                .finish(),
            Self::HubDelivery { batch } => formatter
                .debug_struct("HubDelivery")
                .field("device_id", &batch.device_id)
                .field("connection_id", &batch.connection_id)
                .field(
                    "acknowledged_node_sequence",
                    &batch.acknowledged_node_sequence,
                )
                .field("message_count", &batch.messages.len())
                .field("retry_after_ms", &batch.retry_after_ms)
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct HubNodeConnectRequest {
    pub transport: crate::hub_protocol::NodeTransport,
    pub hello: HubNodeEnvelope,
}

impl HubNodeConnectRequest {
    pub fn validate(&self) -> Result<(), HubTransportContractError> {
        self.hello.validate()?;
        let HubNodeMessage::Hello { capabilities, .. } = &self.hello.message else {
            return Err(HubTransportContractError::InvalidMessageDirection);
        };
        if !capabilities.transports.contains(&self.transport) {
            return Err(HubTransportContractError::TransportMismatch);
        }
        Ok(())
    }
}

impl fmt::Debug for HubNodeConnectRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HubNodeConnectRequest")
            .field("transport", &self.transport)
            .field("hello", &self.hello)
            .finish()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct HubNodeIngressRequest {
    pub transport: crate::hub_protocol::NodeTransport,
    pub envelope: HubNodeEnvelope,
}

impl HubNodeIngressRequest {
    pub fn validate(&self) -> Result<(), HubTransportContractError> {
        self.envelope.validate()?;
        if !matches!(
            &self.envelope.message,
            HubNodeMessage::Heartbeat { .. }
                | HubNodeMessage::RunAccepted { .. }
                | HubNodeMessage::RunApprovalRequired(_)
                | HubNodeMessage::RunRejected(_)
                | HubNodeMessage::RunProgress { .. }
                | HubNodeMessage::RunCompleted(_)
                | HubNodeMessage::AckOnly
                | HubNodeMessage::ProtocolError { .. }
        ) {
            return Err(HubTransportContractError::InvalidMessageDirection);
        }
        Ok(())
    }
}

impl fmt::Debug for HubNodeIngressRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HubNodeIngressRequest")
            .field("transport", &self.transport)
            .field("envelope", &self.envelope)
            .finish()
    }
}

/// Query-safe identity for the unidirectional HTTPS stream fallback.
/// Credentials are never part of this structure and must stay in the
/// `Authorization` header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubNodeStreamRequest {
    pub protocol_major: u16,
    pub protocol_minor: u16,
    pub device_id: String,
    pub connection_id: String,
}

impl HubNodeStreamRequest {
    pub fn protocol_version(&self) -> ProtocolVersion {
        ProtocolVersion {
            major: self.protocol_major,
            minor: self.protocol_minor,
        }
    }

    pub fn validate(&self) -> Result<(), HubTransportContractError> {
        HUB_NODE_PROTOCOL_VERSION.negotiate(self.protocol_version())?;
        validate_identifier("device_id", &self.device_id)?;
        validate_identifier("connection_id", &self.connection_id)?;
        Ok(())
    }

    pub fn pull_request(&self) -> HubNodePullRequest {
        HubNodePullRequest {
            protocol_version: self.protocol_version(),
            device_id: self.device_id.clone(),
            connection_id: self.connection_id.clone(),
            max_messages: MAX_HUB_NODE_BATCH_MESSAGES as u16,
            wait_ms: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubNodeCloseRequest {
    pub protocol_version: ProtocolVersion,
    pub device_id: String,
    pub connection_id: String,
    pub error_code: Option<String>,
}

impl HubNodeCloseRequest {
    pub fn validate(&self) -> Result<(), HubTransportContractError> {
        HUB_NODE_PROTOCOL_VERSION.negotiate(self.protocol_version)?;
        validate_identifier("device_id", &self.device_id)?;
        validate_identifier("connection_id", &self.connection_id)?;
        if let Some(error_code) = &self.error_code {
            validate_identifier("error_code", error_code)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubNodePullRequest {
    pub protocol_version: ProtocolVersion,
    pub device_id: String,
    pub connection_id: String,
    pub max_messages: u16,
    pub wait_ms: u64,
}

impl HubNodePullRequest {
    pub fn validate(&self) -> Result<(), HubTransportContractError> {
        HUB_NODE_PROTOCOL_VERSION.negotiate(self.protocol_version)?;
        validate_identifier("device_id", &self.device_id)?;
        validate_identifier("connection_id", &self.connection_id)?;
        if self.max_messages == 0 || usize::from(self.max_messages) > MAX_HUB_NODE_BATCH_MESSAGES {
            return Err(HubTransportContractError::InvalidBatchLimit);
        }
        if self.wait_ms > MAX_HUB_NODE_LONG_POLL_WAIT_MS {
            return Err(HubTransportContractError::InvalidWait);
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct HubNodeDeliveryBatch {
    pub protocol_version: ProtocolVersion,
    pub device_id: String,
    pub connection_id: String,
    /// Highest Node-origin sequence durably committed by the Hub. This is
    /// deliberately outside Hub envelopes to prevent acknowledgement loops.
    pub acknowledged_node_sequence: u64,
    #[serde(default)]
    pub messages: Vec<HubNodeEnvelope>,
    pub retry_after_ms: Option<u64>,
}

impl HubNodeDeliveryBatch {
    pub fn validate(&self) -> Result<(), HubTransportContractError> {
        HUB_NODE_PROTOCOL_VERSION.negotiate(self.protocol_version)?;
        validate_identifier("device_id", &self.device_id)?;
        validate_identifier("connection_id", &self.connection_id)?;
        if self.acknowledged_node_sequence == 0 {
            return Err(HubTransportContractError::InvalidNodeAcknowledgement);
        }
        if self.messages.len() > MAX_HUB_NODE_BATCH_MESSAGES {
            return Err(HubTransportContractError::InvalidBatchLimit);
        }
        if self
            .retry_after_ms
            .is_some_and(|wait| wait == 0 || wait > MAX_HUB_NODE_LONG_POLL_WAIT_MS)
        {
            return Err(HubTransportContractError::InvalidWait);
        }

        let mut previous_sequence = None;
        for envelope in &self.messages {
            envelope.validate()?;
            if envelope.device_id != self.device_id {
                return Err(HubTransportContractError::DeviceMismatch);
            }
            if envelope.connection_id != self.connection_id {
                return Err(HubTransportContractError::ConnectionMismatch);
            }
            if envelope.protocol_version != self.protocol_version {
                return Err(HubTransportContractError::VersionMismatch);
            }
            if envelope.ack_sequence.is_some() {
                return Err(HubTransportContractError::AcknowledgementLoop);
            }
            if !matches!(
                &envelope.message,
                HubNodeMessage::Welcome { .. }
                    | HubNodeMessage::Superseded { .. }
                    | HubNodeMessage::RunOffer(_)
                    | HubNodeMessage::RunApprovalDecision(_)
                    | HubNodeMessage::CancelRun { .. }
                    | HubNodeMessage::ProtocolError { .. }
            ) {
                return Err(HubTransportContractError::InvalidMessageDirection);
            }
            if previous_sequence
                .is_some_and(|previous: u64| previous.checked_add(1) != Some(envelope.sequence))
            {
                return Err(HubTransportContractError::SequenceGap);
            }
            previous_sequence = Some(envelope.sequence);
        }
        Ok(())
    }
}

impl fmt::Debug for HubNodeDeliveryBatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HubNodeDeliveryBatch")
            .field("protocol_version", &self.protocol_version)
            .field("device_id", &self.device_id)
            .field("connection_id", &self.connection_id)
            .field(
                "acknowledged_node_sequence",
                &self.acknowledged_node_sequence,
            )
            .field("message_count", &self.messages.len())
            .field(
                "first_message_sequence",
                &self.messages.first().map(|message| message.sequence),
            )
            .field(
                "last_message_sequence",
                &self.messages.last().map(|message| message.sequence),
            )
            .field("retry_after_ms", &self.retry_after_ms)
            .finish()
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum HubTransportContractError {
    #[error(transparent)]
    Protocol(#[from] ProtocolContractError),
    #[error("Hub Node batch size is invalid")]
    InvalidBatchLimit,
    #[error("Hub Node transport wait is invalid")]
    InvalidWait,
    #[error("Hub Node acknowledgement must follow a received Node envelope")]
    InvalidNodeAcknowledgement,
    #[error("Hub Node batch contains a different device identity")]
    DeviceMismatch,
    #[error("Hub Node batch contains a different connection identity")]
    ConnectionMismatch,
    #[error("Hub Node batch contains a different protocol version")]
    VersionMismatch,
    #[error("Hub Node delivery envelopes cannot carry acknowledgements")]
    AcknowledgementLoop,
    #[error("Hub Node batch contains a message in the wrong direction")]
    InvalidMessageDirection,
    #[error("Hub Node request transport was not advertised or does not match the connection")]
    TransportMismatch,
    #[error("Hub Node batch message sequence contains a gap")]
    SequenceGap,
}

#[cfg(test)]
#[path = "hub_transport_tests.rs"]
mod tests;
