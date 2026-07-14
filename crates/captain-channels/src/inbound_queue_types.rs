//! Shared inbound queue types kept separate from queue state logic.

use crate::types::ChannelMessage;
use serde::{Deserialize, Serialize};

pub(crate) const MAX_RECOVERED_INBOUND_ATTEMPTS: u32 = 3;
pub(crate) const INBOUND_DEAD_LETTER_RETENTION_SECS: i64 = 86_400;

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
}
