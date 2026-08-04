//! Shared inbound queue types kept separate from queue state logic.

use crate::types::ChannelMessage;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub(crate) const MAX_RECOVERED_INBOUND_ATTEMPTS: u32 = 3;
pub(crate) const INBOUND_DEAD_LETTER_RETENTION_SECS: i64 = 86_400;
pub(crate) const INBOUND_ACCEPTED_ID_RETENTION_SECS: i64 = 7 * 86_400;
pub(crate) const DURABLE_INGRESS_ID_METADATA_KEY: &str = "_captain_durable_ingress_id";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AcceptedInboundId {
    pub id: String,
    pub accepted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingMergeKind {
    Inserted,
    AppendedText,
    Replaced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingInboundSummary {
    pub queued_count: usize,
    pub merge_kind: PendingMergeKind,
    pub ack_recommended: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PendingInboundMessage {
    pub message: ChannelMessage,
    pub queued_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InboundStart {
    Started { key: String },
    Queued(PendingInboundSummary),
    Duplicate { key: String },
    Rejected { reason: String },
}
