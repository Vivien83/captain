//! Operator-safe aggregate snapshot for inbound channel queues.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct InboundQueueSnapshot {
    pub active_sessions: usize,
    pub pending_sessions: usize,
    pub pending_messages: usize,
    pub inflight_sessions: usize,
    pub inflight_messages: usize,
    pub dead_letter_sessions: usize,
    pub dead_letter_messages: usize,
    pub dead_letter_oldest_age_secs: Option<u64>,
    pub interjected_sessions: usize,
    pub interjected_messages: usize,
    pub channels: Vec<InboundQueueChannelSnapshot>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct InboundQueueChannelSnapshot {
    pub channel: String,
    pub active_sessions: usize,
    pub pending_sessions: usize,
    pub pending_messages: usize,
    pub inflight_sessions: usize,
    pub inflight_messages: usize,
    pub dead_letter_sessions: usize,
    pub dead_letter_messages: usize,
    pub dead_letter_oldest_age_secs: Option<u64>,
    pub interjected_sessions: usize,
    pub interjected_messages: usize,
}
