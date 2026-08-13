//! Crash-safe local half of the Hub/Node delivery rail.

mod storage;

use crate::pairing::{NodePairingError, NodePairingStore, NodeStateRoot};
use captain_wire::hub_protocol::{RunApprovalRequest, RunCompletion, RunRejection};
use captain_wire::{
    CapabilityDescriptor, HubNodeDeliveryBatch, HubNodeEnvelope, HubNodeMessage, RunEffect,
    RunLease,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fmt, fs,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, OnceLock},
};
use thiserror::Error;

const NODE_RAIL_SCHEMA_VERSION: i64 = 5;
const MAX_LOCAL_RAIL_RECORDS: usize = 4_096;
const MAX_LOCAL_RAIL_BYTES: usize = 64 * 1024 * 1024;
const MAX_LOCAL_RAIL_PAGE: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRailSnapshot {
    pub device_id: String,
    pub connection_id: String,
    pub last_node_sequence: u64,
    pub acknowledged_node_sequence: u64,
    pub last_hub_sequence: u64,
    pub confirmed_hub_ack_sequence: u64,
    pub pending_outbound: usize,
    pub pending_inbound: usize,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeInboundRecord {
    pub envelope: HubNodeEnvelope,
    pub received_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeBootstrapCapabilityState {
    Current,
    RotationDeferred,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeBootstrap {
    pub envelope: HubNodeEnvelope,
    pub capability_state: NodeBootstrapCapabilityState,
}

impl fmt::Debug for NodeBootstrap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeBootstrap")
            .field("envelope", &self.envelope)
            .field("capability_state", &self.capability_state)
            .finish()
    }
}

impl fmt::Debug for NodeInboundRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeInboundRecord")
            .field("sequence", &self.envelope.sequence)
            .field("message_type", &message_kind(&self.envelope.message))
            .field("received_at_ms", &self.received_at_ms)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDeliveryOutcome {
    pub newly_recorded: usize,
    pub duplicate_messages: usize,
    pub acknowledgement_advanced: bool,
    pub acknowledgement_enqueued: bool,
    pub acknowledged_node_sequence: u64,
    pub last_hub_sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRunStatus {
    ApprovalPending,
    Accepted,
    Running,
    CancelRequested,
    Rejected,
    Succeeded,
    Failed,
    Cancelled,
    Uncertain,
}

impl NodeRunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Rejected | Self::Succeeded | Self::Failed | Self::Cancelled | Self::Uncertain
        )
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeRunRecord {
    pub lease: RunLease,
    pub input_sha256: String,
    pub status: NodeRunStatus,
    pub effect_started: bool,
    pub inbound_sequence: u64,
    pub decision_outbound_sequence: Option<u64>,
    pub approval_decision_inbound_sequence: Option<u64>,
    pub acceptance_outbound_sequence: Option<u64>,
    pub cancel_inbound_sequence: Option<u64>,
    pub cancel_sha256: Option<String>,
    #[serde(default, skip_serializing)]
    pub execution_claim_id: Option<String>,
    #[serde(default, skip_serializing)]
    pub execution_claim_started_at_ms: Option<i64>,
    pub terminal_outbound_sequence: Option<u64>,
    pub terminal_sha256: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub terminal_at_ms: Option<i64>,
}

impl fmt::Debug for NodeRunRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeRunRecord")
            .field("run_id", &self.lease.run_id)
            .field("attempt", &self.lease.attempt)
            .field("idempotency_key", &self.lease.idempotency_key)
            .field("workspace_id", &self.lease.workspace_id)
            .field("tool_name", &self.lease.tool_name)
            .field("input", &"[REDACTED]")
            .field("input_sha256", &self.input_sha256)
            .field("effect", &self.lease.effect)
            .field("status", &self.status)
            .field("effect_started", &self.effect_started)
            .field("inbound_sequence", &self.inbound_sequence)
            .field(
                "decision_outbound_sequence",
                &self.decision_outbound_sequence,
            )
            .field(
                "approval_decision_inbound_sequence",
                &self.approval_decision_inbound_sequence,
            )
            .field(
                "acceptance_outbound_sequence",
                &self.acceptance_outbound_sequence,
            )
            .field("cancel_inbound_sequence", &self.cancel_inbound_sequence)
            .field("cancel_sha256", &self.cancel_sha256)
            .field(
                "execution_claim_id",
                &self.execution_claim_id.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "execution_claim_started_at_ms",
                &self.execution_claim_started_at_ms,
            )
            .field(
                "terminal_outbound_sequence",
                &self.terminal_outbound_sequence,
            )
            .field("terminal_sha256", &self.terminal_sha256)
            .field("updated_at_ms", &self.updated_at_ms)
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub enum NodeRunDisposition {
    Accept,
    RequireApproval(RunApprovalRequest),
    Reject(RunRejection),
}

impl fmt::Debug for NodeRunDisposition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accept => formatter.write_str("Accept"),
            Self::RequireApproval(request) => formatter
                .debug_tuple("RequireApproval")
                .field(request)
                .finish(),
            Self::Reject(rejection) => formatter.debug_tuple("Reject").field(rejection).finish(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeRunIntakeOutcome {
    pub run: NodeRunRecord,
    pub outbound: Option<HubNodeEnvelope>,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeRunApprovalOutcome {
    pub run: NodeRunRecord,
    pub outbound: Option<HubNodeEnvelope>,
    pub expired_locally: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeRunCancelOutcome {
    pub run: NodeRunRecord,
    pub outbound: Option<HubNodeEnvelope>,
    pub signal_runner: bool,
    pub replayed: bool,
}

#[derive(Clone, PartialEq)]
pub struct NodeRunClaimOutcome {
    pub run: NodeRunRecord,
    pub claim_id: String,
    pub claimed_at_ms: i64,
}

impl fmt::Debug for NodeRunClaimOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeRunClaimOutcome")
            .field("run", &self.run)
            .field("claim_id", &"[REDACTED]")
            .field("claimed_at_ms", &self.claimed_at_ms)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeRunCompletionOutcome {
    pub run: NodeRunRecord,
    pub outbound: Option<HubNodeEnvelope>,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeRunPreflightRejectionOutcome {
    pub run: NodeRunRecord,
    pub outbound: HubNodeEnvelope,
}

// Field order keeps the SQLite handle closed before either ownership guard drops.
struct NodeRailInner {
    connection: Mutex<Connection>,
    _open_guard: NodeRailOpenGuard,
    _state_root: Arc<NodeStateRoot>,
}

#[derive(Clone)]
pub struct NodeRailStore {
    inner: Arc<NodeRailInner>,
}

impl NodeRailStore {
    pub fn open(pairing: &NodePairingStore) -> Result<Self, NodeRailError> {
        let binding = pairing.rail_binding().map_err(map_pairing_error)?;
        let root =
            fs::canonicalize(binding.root.path()).map_err(|_| NodeRailError::StateUnavailable)?;
        let open_guard = NodeRailOpenGuard::acquire(root)?;
        let connection = storage::open_database(&binding)?;
        Ok(Self {
            inner: Arc::new(NodeRailInner {
                connection: Mutex::new(connection),
                _open_guard: open_guard,
                _state_root: binding.root,
            }),
        })
    }

    /// Returns the exact durable bootstrap Hello. Once created, retries and
    /// transport fallbacks reuse it byte-for-byte; current runs are refreshed
    /// with a Heartbeat after Welcome.
    pub fn bootstrap_hello(
        &self,
        capabilities: &CapabilityDescriptor,
        active_run_ids: &[String],
        sent_at_ms: i64,
    ) -> Result<NodeBootstrap, NodeRailError> {
        let mut connection = self.lock()?;
        storage::bootstrap_hello(&mut connection, capabilities, active_run_ids, sent_at_ms)
    }

    /// Durably allocates the next Node sequence before any network send.
    pub fn enqueue(
        &self,
        message: HubNodeMessage,
        sent_at_ms: i64,
    ) -> Result<HubNodeEnvelope, NodeRailError> {
        let mut connection = self.lock()?;
        storage::enqueue(&mut connection, message, sent_at_ms)
    }

    /// Returns an existing pending Heartbeat for the same active-run set or
    /// durably appends exactly one. Reconnect loops therefore refresh current
    /// runtime state without growing duplicate outbox records.
    pub fn ensure_heartbeat(
        &self,
        active_run_ids: &[String],
        sent_at_ms: i64,
    ) -> Result<HubNodeEnvelope, NodeRailError> {
        let mut connection = self.lock()?;
        storage::ensure_heartbeat(&mut connection, active_run_ids, sent_at_ms)
    }

    pub fn pending_outbound(&self, limit: usize) -> Result<Vec<HubNodeEnvelope>, NodeRailError> {
        let connection = self.lock()?;
        storage::pending_outbound(&connection, limit)
    }

    /// Commits a Hub delivery and its cumulative Node acknowledgement in one
    /// transaction. Any generated AckOnly is also durable before this returns.
    pub fn observe_delivery(
        &self,
        batch: &HubNodeDeliveryBatch,
        received_at_ms: i64,
    ) -> Result<NodeDeliveryOutcome, NodeRailError> {
        let mut connection = self.lock()?;
        storage::observe_delivery(&mut connection, batch, received_at_ms)
    }

    pub fn pending_inbound(&self, limit: usize) -> Result<Vec<NodeInboundRecord>, NodeRailError> {
        let connection = self.lock()?;
        storage::pending_inbound(&connection, limit)
    }

    /// Marks exactly the oldest unapplied Hub message as applied. Delivery
    /// order cannot be skipped, including across a process restart.
    pub fn mark_inbound_applied(
        &self,
        sequence: u64,
        applied_at_ms: i64,
    ) -> Result<(), NodeRailError> {
        let mut connection = self.lock()?;
        storage::mark_inbound_applied(&mut connection, sequence, applied_at_ms)
    }

    /// Atomically consumes the oldest RunOffer, persists the local attempt and
    /// appends the corresponding acceptance, approval request, or rejection.
    pub fn apply_run_offer(
        &self,
        sequence: u64,
        disposition: &NodeRunDisposition,
        applied_at_ms: i64,
    ) -> Result<NodeRunIntakeOutcome, NodeRailError> {
        let mut connection = self.lock()?;
        storage::apply_run_offer(&mut connection, sequence, disposition, applied_at_ms)
    }

    /// Atomically applies the oldest exact Hub approval decision. An approved
    /// action is accepted only while both approval and lease remain valid.
    pub fn apply_run_approval_decision(
        &self,
        sequence: u64,
        applied_at_ms: i64,
    ) -> Result<NodeRunApprovalOutcome, NodeRailError> {
        let mut connection = self.lock()?;
        storage::apply_run_approval_decision(&mut connection, sequence, applied_at_ms)
    }

    /// Applies the oldest exact cancellation. Before-effect cancellation and
    /// its terminal completion commit together; a running effect is only
    /// marked for cooperative cancellation.
    pub fn apply_cancel_run(
        &self,
        sequence: u64,
        applied_at_ms: i64,
    ) -> Result<NodeRunCancelOutcome, NodeRailError> {
        let mut connection = self.lock()?;
        storage::apply_cancel_run(&mut connection, sequence, applied_at_ms)
    }

    /// Atomically claims accepted work before any local effect begins. The Hub
    /// must already have acknowledged the exact `RunAccepted` evidence.
    pub fn claim_run(
        &self,
        run_id: &str,
        attempt: u32,
        claimed_at_ms: i64,
    ) -> Result<NodeRunClaimOutcome, NodeRailError> {
        let mut connection = self.lock()?;
        storage::claim_run(&mut connection, run_id, attempt, claimed_at_ms)
    }

    /// Returns the durable cooperative-cancellation state for one exact claim.
    pub fn cancellation_requested(&self, claim_id: &str) -> Result<bool, NodeRailError> {
        let connection = self.lock()?;
        storage::cancellation_requested(&connection, claim_id)
    }

    /// Commits exact terminal evidence and its sequenced outbox record in the
    /// same transaction. An exact replay never duplicates the completion.
    pub fn complete_run(
        &self,
        claim_id: &str,
        completion: &RunCompletion,
        completed_at_ms: i64,
    ) -> Result<NodeRunCompletionOutcome, NodeRailError> {
        let mut connection = self.lock()?;
        storage::complete_run(&mut connection, claim_id, completion, completed_at_ms)
    }

    pub fn get_run(
        &self,
        run_id: &str,
        attempt: u32,
    ) -> Result<Option<NodeRunRecord>, NodeRailError> {
        let connection = self.lock()?;
        storage::get_run(&connection, run_id, attempt)
    }

    pub fn active_run_ids(&self) -> Result<Vec<String>, NodeRailError> {
        let connection = self.lock()?;
        storage::active_run_ids(&connection)
    }

    /// Returns accepted, unclaimed runs in deterministic order so a restarted
    /// worker can reapply current local policy before claiming any effect.
    pub fn claimable_runs(&self, limit: usize) -> Result<Vec<NodeRunRecord>, NodeRailError> {
        let connection = self.lock()?;
        storage::claimable_runs(&connection, limit)
    }

    /// Returns the exact action digest durably approved for accepted work.
    /// Directly accepted runs return `None`; inconsistent approval state fails
    /// closed before an execution claim can begin.
    pub fn approved_action_digest(
        &self,
        run_id: &str,
        attempt: u32,
    ) -> Result<Option<String>, NodeRailError> {
        let connection = self.lock()?;
        storage::approved_action_digest(&connection, run_id, attempt)
    }

    /// Atomically closes accepted work before its effect starts when current
    /// local policy or runtime review no longer authorizes the exact offer.
    pub fn reject_run_before_effect(
        &self,
        run_id: &str,
        attempt: u32,
        rejection: &RunRejection,
        rejected_at_ms: i64,
    ) -> Result<NodeRunPreflightRejectionOutcome, NodeRailError> {
        let mut connection = self.lock()?;
        storage::reject_run_before_effect(
            &mut connection,
            run_id,
            attempt,
            rejection,
            rejected_at_ms,
        )
    }

    pub fn snapshot(&self) -> Result<NodeRailSnapshot, NodeRailError> {
        let connection = self.lock()?;
        storage::snapshot(&connection)
    }

    pub(crate) fn ensure_hub_identity(&self, hub_sha256: &str) -> Result<(), NodeRailError> {
        let connection = self.lock()?;
        storage::ensure_hub_identity(&connection, hub_sha256)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, NodeRailError> {
        self.inner
            .connection
            .lock()
            .map_err(|_| NodeRailError::StateUnavailable)
    }
}

struct NodeRailOpenGuard {
    root: PathBuf,
}

static OPEN_RAIL_ROOTS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

impl NodeRailOpenGuard {
    fn acquire(root: PathBuf) -> Result<Self, NodeRailError> {
        let mut open = OPEN_RAIL_ROOTS
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
            .map_err(|_| NodeRailError::StateUnavailable)?;
        if !open.insert(root.clone()) {
            return Err(NodeRailError::StateUnavailable);
        }
        Ok(Self { root })
    }
}

impl Drop for NodeRailOpenGuard {
    fn drop(&mut self) {
        if let Ok(mut open) = OPEN_RAIL_ROOTS
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
        {
            open.remove(&self.root);
        }
    }
}

impl fmt::Debug for NodeRailStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeRailStore")
            .field("state_root", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Error)]
pub enum NodeRailError {
    #[error("Node must be paired before its delivery rail can start")]
    PairingRequired,
    #[error("Node local state is unavailable")]
    StateUnavailable,
    #[error("Node local state path is unsafe")]
    UnsafeStatePath,
    #[error("Node delivery state is corrupt")]
    StateCorrupt,
    #[error("Node delivery state version is unsupported")]
    StateVersionUnsupported,
    #[error("Node delivery state belongs to another pairing identity")]
    IdentityConflict,
    #[error("Node delivery message is invalid")]
    InvalidMessage,
    #[error("Node connection has not received its Welcome acknowledgement")]
    ConnectionNotReady,
    #[error("Node sequence is exhausted")]
    SequenceExhausted,
    #[error("Hub delivery contains a sequence gap")]
    SequenceGap,
    #[error("Hub delivery replays a sequence with different content")]
    ReplayConflict,
    #[error("Hub delivery acknowledgement conflicts with durable state")]
    InvalidAcknowledgement,
    #[error("Node outbound queue is full; pending evidence was preserved")]
    OutboxFull,
    #[error("Node inbound queue is full; delivery was not acknowledged")]
    InboxFull,
    #[error("Node inbound messages must be applied in delivery order")]
    ApplyOrderConflict,
    #[error("Node run offer conflicts with durable local execution state")]
    RunConflict,
    #[error("Node run decision does not match the offered work")]
    RunDecisionConflict,
    #[error("Node run is not durably acknowledged or its lease is no longer valid")]
    RunNotReady,
    #[error("Node run has a durable cancellation waiting to be applied")]
    RunCancellationPending,
    #[error("Node execution claim conflicts with durable local state")]
    RunClaimConflict,
    #[error("Node delivery database operation failed")]
    Database(#[source] rusqlite::Error),
}

impl From<rusqlite::Error> for NodeRailError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

fn map_pairing_error(error: NodePairingError) -> NodeRailError {
    match error {
        NodePairingError::PairingNotStarted | NodePairingError::PairingNotApproved => {
            NodeRailError::PairingRequired
        }
        NodePairingError::UnsafeStatePath => NodeRailError::UnsafeStatePath,
        NodePairingError::StateCorrupt => NodeRailError::StateCorrupt,
        NodePairingError::StateVersionUnsupported => NodeRailError::StateVersionUnsupported,
        NodePairingError::HubIdentityMismatch => NodeRailError::IdentityConflict,
        _ => NodeRailError::StateUnavailable,
    }
}

fn message_kind(message: &HubNodeMessage) -> &'static str {
    match message {
        HubNodeMessage::Hello { .. } => "hello",
        HubNodeMessage::Welcome { .. } => "welcome",
        HubNodeMessage::Superseded { .. } => "superseded",
        HubNodeMessage::Heartbeat { .. } => "heartbeat",
        HubNodeMessage::RunOffer(_) => "run_offer",
        HubNodeMessage::RunAccepted { .. } => "run_accepted",
        HubNodeMessage::RunApprovalRequired(_) => "run_approval_required",
        HubNodeMessage::RunApprovalDecision(_) => "run_approval_decision",
        HubNodeMessage::RunRejected(_) => "run_rejected",
        HubNodeMessage::RunProgress { .. } => "run_progress",
        HubNodeMessage::RunCompleted(_) => "run_completed",
        HubNodeMessage::CancelRun { .. } => "cancel_run",
        HubNodeMessage::AckOnly => "ack_only",
        HubNodeMessage::ProtocolError { .. } => "protocol_error",
    }
}

fn is_node_message(message: &HubNodeMessage) -> bool {
    matches!(
        message,
        HubNodeMessage::Heartbeat { .. }
            | HubNodeMessage::RunAccepted { .. }
            | HubNodeMessage::RunApprovalRequired(_)
            | HubNodeMessage::RunRejected(_)
            | HubNodeMessage::RunProgress { .. }
            | HubNodeMessage::RunCompleted(_)
            | HubNodeMessage::AckOnly
            | HubNodeMessage::ProtocolError { .. }
    )
}
