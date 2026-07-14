//! Internal state for one inbound channel session.

use crate::inbound_queue_store::DeadInboundRecord;
use crate::inbound_queue_types::PendingInboundMessage;
use captain_types::agent::AgentId;
use std::time::Instant;

#[derive(Debug)]
pub(crate) struct SessionState {
    pub(crate) channel: String,
    pub(crate) active_agent_id: Option<AgentId>,
    pub(crate) pending: Option<PendingInboundMessage>,
    pub(crate) inflight: Option<PendingInboundMessage>,
    pub(crate) dead_letter: Option<DeadInboundRecord>,
    pub(crate) recovery_attempts: u32,
    pub(crate) interjected_count: usize,
    pub(crate) last_ack_at: Option<Instant>,
}
