//! Durable outbound delivery contracts shared by the channel bridge and kernel.

use crate::types::{ChannelContent, ChannelUser};
use captain_types::agent::AgentId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Transport operation that must be replayed for a durable outbound message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum OutboundDeliveryTransport {
    Standard,
    Thread {
        thread_id: String,
    },
    Rich {
        metadata: HashMap<String, serde_json::Value>,
    },
}

impl OutboundDeliveryTransport {
    pub fn thread_id(&self) -> Option<&str> {
        match self {
            Self::Thread { thread_id } => Some(thread_id),
            Self::Rich { metadata } => metadata.get("thread_id").and_then(|value| value.as_str()),
            Self::Standard => None,
        }
    }
}

/// Complete payload needed to retry an outbound delivery without rerunning an agent turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundDeliveryIntent {
    pub idempotency_key: String,
    pub agent_id: Option<AgentId>,
    pub channel: String,
    pub recipient: ChannelUser,
    pub content: ChannelContent,
    pub transport: OutboundDeliveryTransport,
    pub source_message_id: String,
    pub purpose: String,
}

/// Leased delivery returned by the durable control plane.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundDeliveryClaim {
    pub delivery_id: String,
    pub lease_token: String,
    pub intent: OutboundDeliveryIntent,
    pub attempt_count: u32,
    /// A prior send may have reached the remote channel before Captain lost its receipt.
    pub possible_duplicate: bool,
}

/// Result of atomically persisting and claiming a new delivery intent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "claim")]
#[allow(clippy::large_enum_variant)]
pub enum OutboundDeliveryPreparation {
    /// Persistence is unavailable on this handle (primarily lightweight tests).
    Bypass,
    /// The same intent is already delivered, queued, or currently leased.
    AlreadyHandled,
    Claimed(OutboundDeliveryClaim),
}

/// Operator-safe durable delivery counters.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutboundDeliverySnapshot {
    pub pending: usize,
    pub attempting: usize,
    pub delivered: usize,
    pub dead: usize,
    pub possible_duplicates: usize,
    pub oldest_pending_age_secs: Option<i64>,
    pub last_error: Option<String>,
}
