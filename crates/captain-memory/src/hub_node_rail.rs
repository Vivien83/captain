//! Durable authority for Hub-to-Node runs and sequenced message delivery.

mod connection;
mod inbound;
pub use connection::HubNodeDeliverySnapshot;
pub use connection::{HubNodeConnectionRecord, HubNodeConnectionStatus, OpenHubNodeConnection};
pub use inbound::{AppliedHubNodeEnvelope, HubNodeInboundOutcome};

use captain_types::approval::{ApprovalDecision, RiskLevel};
use captain_wire::hub_protocol::{
    HubNodeMessage, RunApprovalDecision, RunApprovalRequest, RunCompletion, RunEffect, RunLease,
    RunRejection,
};
use rusqlite::{
    params, types::Type, Connection, OptionalExtension, Row, Transaction, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex, MutexGuard};
use thiserror::Error;

const MAX_LEASE_DURATION_MS: i64 = 15 * 60 * 1_000;
const MAX_OUTBOX_PAGE: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HubNodeRunStatus {
    Queued,
    Leased,
    Accepted,
    CancelRequested,
    Succeeded,
    Failed,
    Cancelled,
    Uncertain,
}

impl HubNodeRunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Uncertain
        )
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "queued" => Self::Queued,
            "leased" => Self::Leased,
            "accepted" => Self::Accepted,
            "cancel_requested" => Self::CancelRequested,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "uncertain" => Self::Uncertain,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HubNodeEffectState {
    NotStarted,
    Started,
    Completed,
}

impl HubNodeEffectState {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "not_started" => Self::NotStarted,
            "started" => Self::Started,
            "completed" => Self::Completed,
            _ => return None,
        })
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct NewHubNodeRun {
    pub run_id: String,
    pub device_id: String,
    pub idempotency_key: String,
    pub workspace_id: String,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub effect: RunEffect,
    pub created_at_ms: i64,
}

impl std::fmt::Debug for NewHubNodeRun {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NewHubNodeRun")
            .field("run_id", &self.run_id)
            .field("device_id", &self.device_id)
            .field("idempotency_key", &self.idempotency_key)
            .field("workspace_id", &self.workspace_id)
            .field("tool_name", &self.tool_name)
            .field("input", &"[REDACTED]")
            .field("effect", &self.effect)
            .field("created_at_ms", &self.created_at_ms)
            .finish()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct HubNodeRunRecord {
    pub run_id: String,
    pub device_id: String,
    pub idempotency_key: String,
    pub workspace_id: String,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub effect: RunEffect,
    pub status: HubNodeRunStatus,
    pub attempt: u32,
    pub lease_owner: Option<String>,
    pub lease_expires_at_ms: Option<i64>,
    pub effect_state: HubNodeEffectState,
    pub progress_sequence: u64,
    pub progress_message: Option<String>,
    pub completion: Option<RunCompletion>,
    pub rejection: Option<RunRejection>,
    pub error_code: Option<String>,
    pub cancel_requested_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub terminal_at_ms: Option<i64>,
}

impl std::fmt::Debug for HubNodeRunRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HubNodeRunRecord")
            .field("run_id", &self.run_id)
            .field("device_id", &self.device_id)
            .field("workspace_id", &self.workspace_id)
            .field("tool_name", &self.tool_name)
            .field("input", &"[REDACTED]")
            .field("effect", &self.effect)
            .field("status", &self.status)
            .field("attempt", &self.attempt)
            .field("effect_state", &self.effect_state)
            .field("progress_sequence", &self.progress_sequence)
            .field(
                "completion",
                &self.completion.as_ref().map(|_| "[REDACTED]"),
            )
            .field("rejection", &self.rejection.as_ref().map(|_| "[REDACTED]"))
            .field("error_code", &self.error_code)
            .field("updated_at_ms", &self.updated_at_ms)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HubNodeRunApprovalStatus {
    Pending,
    Approved,
    Denied,
    TimedOut,
}

impl HubNodeRunApprovalStatus {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "approved" => Self::Approved,
            "denied" => Self::Denied,
            "timed_out" => Self::TimedOut,
            _ => return None,
        })
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubNodeRunApprovalRecord {
    pub approval_id: String,
    pub run_id: String,
    pub attempt: u32,
    pub action_digest: String,
    pub action_summary: String,
    pub risk_level: RiskLevel,
    pub status: HubNodeRunApprovalStatus,
    pub decision: Option<ApprovalDecision>,
    pub reason: Option<String>,
    pub requested_at_ms: i64,
    pub expires_at_ms: i64,
    pub decided_at_ms: Option<i64>,
}

impl std::fmt::Debug for HubNodeRunApprovalRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HubNodeRunApprovalRecord")
            .field("approval_id", &self.approval_id)
            .field("run_id", &self.run_id)
            .field("attempt", &self.attempt)
            .field("action_digest", &self.action_digest)
            .field("action_summary", &"[REDACTED]")
            .field("risk_level", &self.risk_level)
            .field("status", &self.status)
            .field("decision", &self.decision)
            .field("reason", &self.reason.as_ref().map(|_| "[REDACTED]"))
            .field("requested_at_ms", &self.requested_at_ms)
            .field("expires_at_ms", &self.expires_at_ms)
            .field("decided_at_ms", &self.decided_at_ms)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubNodeOutboxRecord {
    pub device_id: String,
    pub sequence: u64,
    pub message_kind: String,
    pub message_json: String,
    pub message_sha256: String,
    pub run_id: Option<String>,
    pub created_at_ms: i64,
    pub acked_at_ms: Option<i64>,
    pub superseded_at_ms: Option<i64>,
}

impl std::fmt::Debug for HubNodeOutboxRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HubNodeOutboxRecord")
            .field("device_id", &self.device_id)
            .field("sequence", &self.sequence)
            .field("message_kind", &self.message_kind)
            .field("message_json", &"[REDACTED]")
            .field("message_sha256", &self.message_sha256)
            .field("run_id", &self.run_id)
            .field("created_at_ms", &self.created_at_ms)
            .field("acked_at_ms", &self.acked_at_ms)
            .field("superseded_at_ms", &self.superseded_at_ms)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LeasedHubNodeRun {
    pub run: HubNodeRunRecord,
    pub lease: RunLease,
    pub outbox: HubNodeOutboxRecord,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CancelledHubNodeRun {
    pub run: HubNodeRunRecord,
    pub outbox: Option<HubNodeOutboxRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecidedHubNodeRunApproval {
    pub approval: HubNodeRunApprovalRecord,
    pub outbox: HubNodeOutboxRecord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundReceipt {
    Recorded,
    Duplicate,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HubNodeRecoverySummary {
    pub requeued_read_only: usize,
    pub cancelled_before_effect: usize,
    pub uncertain_side_effects: usize,
}

#[derive(Debug, Error)]
pub enum HubNodeRailError {
    #[error("invalid Hub Node rail input: {0}")]
    InvalidInput(String),
    #[error("paired Node is unavailable")]
    NodeUnavailable,
    #[error("Hub Node run not found")]
    RunNotFound,
    #[error("run id already exists")]
    RunIdConflict,
    #[error("idempotency key was reused with different work")]
    IdempotencyConflict,
    #[error("run lease is stale or owned by another connection")]
    LeaseConflict,
    #[error("run already has conflicting terminal evidence")]
    TerminalConflict,
    #[error("Node connection identity conflicts with durable state")]
    ConnectionConflict,
    #[error("Hub Node message is invalid for this direction")]
    InvalidMessageDirection,
    #[error("Hub Node durable state is incomplete")]
    StorageInvariant,
    #[error("message sequence contains a gap")]
    SequenceGap,
    #[error("message sequence was replayed with different content")]
    ReplayConflict,
    #[error("acknowledgement exceeds the durable outbox")]
    InvalidAcknowledgement,
    #[error("Hub Node rail lock failed: {0}")]
    Lock(String),
    #[error("Hub Node rail database error: {0}")]
    Database(#[from] rusqlite::Error),
}

#[derive(Clone)]
pub struct HubNodeRailStore {
    conn: Arc<Mutex<Connection>>,
}

impl HubNodeRailStore {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn enqueue_run(&self, input: &NewHubNodeRun) -> Result<HubNodeRunRecord, HubNodeRailError> {
        validate_new_run(input)?;
        let input_json = serde_json::to_string(&input.input)
            .map_err(|error| HubNodeRailError::InvalidInput(error.to_string()))?;
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_active_node(&tx, &input.device_id)?;

        if let Some(existing) = run_by_idempotency(&tx, &input.device_id, &input.idempotency_key)? {
            if same_run_request(&existing, input) {
                tx.commit()?;
                return Ok(existing);
            }
            return Err(HubNodeRailError::IdempotencyConflict);
        }
        if run_by_id(&tx, &input.run_id)?.is_some() {
            return Err(HubNodeRailError::RunIdConflict);
        }

        tx.execute(
            "INSERT INTO hub_node_runs (
                 run_id, device_id, idempotency_key, workspace_id, tool_name,
                 input_json, effect, created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![
                input.run_id,
                input.device_id,
                input.idempotency_key,
                input.workspace_id,
                input.tool_name,
                input_json,
                effect_str(input.effect),
                input.created_at_ms,
            ],
        )?;
        let record = run_by_id(&tx, &input.run_id)?.ok_or(HubNodeRailError::RunNotFound)?;
        tx.commit()?;
        Ok(record)
    }

    pub fn get_run(&self, run_id: &str) -> Result<Option<HubNodeRunRecord>, HubNodeRailError> {
        validate_identifier("run id", run_id)?;
        let conn = self.lock()?;
        run_by_id(&conn, run_id)
    }

    pub fn get_run_approval(
        &self,
        run_id: &str,
    ) -> Result<Option<HubNodeRunApprovalRecord>, HubNodeRailError> {
        validate_identifier("run id", run_id)?;
        let conn = self.lock()?;
        latest_run_approval(&conn, run_id)
    }

    /// Persist an operator decision and its Hub-to-Node delivery atomically.
    /// The exact digest and attempt must match the pending local request.
    pub fn decide_run_approval(
        &self,
        decision: &RunApprovalDecision,
    ) -> Result<DecidedHubNodeRunApproval, HubNodeRailError> {
        decision
            .validate()
            .map_err(|error| HubNodeRailError::InvalidInput(error.to_string()))?;
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let approval =
            approval_by_id(&tx, &decision.approval_id)?.ok_or(HubNodeRailError::RunNotFound)?;
        if approval.run_id != decision.run_id
            || approval.attempt != decision.attempt
            || approval.action_digest != decision.action_digest
        {
            return Err(HubNodeRailError::LeaseConflict);
        }
        if approval.status != HubNodeRunApprovalStatus::Pending {
            if approval.decision == Some(decision.decision) && approval.reason == decision.reason {
                let outbox = outbox_for_run_approval(&tx, &decision.run_id, &decision.approval_id)?
                    .ok_or(HubNodeRailError::StorageInvariant)?;
                tx.commit()?;
                return Ok(DecidedHubNodeRunApproval { approval, outbox });
            }
            return Err(HubNodeRailError::TerminalConflict);
        }
        if decision.decided_at_ms < approval.requested_at_ms
            || (decision.decided_at_ms > approval.expires_at_ms
                && decision.decision != ApprovalDecision::TimedOut)
        {
            return Err(HubNodeRailError::LeaseConflict);
        }
        let run = run_by_id(&tx, &decision.run_id)?.ok_or(HubNodeRailError::RunNotFound)?;
        if run.attempt != decision.attempt
            || run.status != HubNodeRunStatus::Leased
            || run.effect_state != HubNodeEffectState::NotStarted
        {
            return Err(HubNodeRailError::LeaseConflict);
        }
        let status = approval_status_for_decision(decision.decision);
        let changed = tx.execute(
            "UPDATE hub_node_run_approvals
             SET status = ?2, decision = ?3, reason = ?4, decided_at_ms = ?5
             WHERE approval_id = ?1 AND status = 'pending'",
            params![
                decision.approval_id,
                approval_status_str(status),
                approval_decision_str(decision.decision),
                decision.reason,
                decision.decided_at_ms,
            ],
        )?;
        if changed != 1 {
            return Err(HubNodeRailError::TerminalConflict);
        }
        if !decision.decision.is_approved() {
            let error_code = if decision.decision == ApprovalDecision::TimedOut {
                "approval_timed_out"
            } else {
                "approval_denied"
            };
            let changed = tx.execute(
                "UPDATE hub_node_runs
                 SET status = 'cancelled', effect_state = 'completed',
                     lease_owner = NULL, lease_expires_at_ms = NULL,
                     error_code = ?2, terminal_at_ms = ?3, updated_at_ms = ?3
                 WHERE run_id = ?1 AND status = 'leased'
                   AND effect_state = 'not_started'",
                params![decision.run_id, error_code, decision.decided_at_ms],
            )?;
            if changed != 1 {
                return Err(HubNodeRailError::LeaseConflict);
            }
        }
        let message = HubNodeMessage::RunApprovalDecision(decision.clone());
        let outbox = append_outbox_in_tx(
            &tx,
            &run.device_id,
            Some(&decision.run_id),
            &message,
            decision.decided_at_ms,
        )?;
        let approval = approval_by_id(&tx, &decision.approval_id)?
            .ok_or(HubNodeRailError::StorageInvariant)?;
        tx.commit()?;
        Ok(DecidedHubNodeRunApproval { approval, outbox })
    }

    pub fn lease_next(
        &self,
        device_id: &str,
        lease_owner: &str,
        now_ms: i64,
        lease_duration_ms: i64,
    ) -> Result<Option<LeasedHubNodeRun>, HubNodeRailError> {
        validate_identifier("device id", device_id)?;
        validate_identifier("lease owner", lease_owner)?;
        validate_now(now_ms)?;
        if !(1..=MAX_LEASE_DURATION_MS).contains(&lease_duration_ms) {
            return Err(HubNodeRailError::InvalidInput(
                "lease duration is outside the supported range".to_string(),
            ));
        }
        let lease_expires_at_ms = now_ms
            .checked_add(lease_duration_ms)
            .ok_or_else(|| HubNodeRailError::InvalidInput("lease expiry overflow".to_string()))?;

        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_active_node(&tx, device_id)?;
        reconcile_in_tx(&tx, Some(device_id), now_ms, false)?;
        let run_id = tx
            .query_row(
                "SELECT run_id FROM hub_node_runs
                 WHERE device_id = ?1 AND status = 'queued'
                 ORDER BY created_at_ms, run_id LIMIT 1",
                [device_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(run_id) = run_id else {
            tx.commit()?;
            return Ok(None);
        };
        let leased = lease_queued_run_in_tx(
            &tx,
            device_id,
            &run_id,
            lease_owner,
            lease_expires_at_ms,
            now_ms,
        )?;
        tx.commit()?;
        Ok(Some(leased))
    }

    /// Lease one exact queued run and persist its RunOffer atomically.
    ///
    /// The production dispatcher uses this targeted form so a recovered older
    /// queue entry cannot cause the caller to wait on a run that was never
    /// offered. `None` means the run already advanced beyond `queued`; callers
    /// must re-read its durable status instead of offering it twice.
    pub fn lease_run(
        &self,
        device_id: &str,
        run_id: &str,
        lease_owner: &str,
        now_ms: i64,
        lease_duration_ms: i64,
    ) -> Result<Option<LeasedHubNodeRun>, HubNodeRailError> {
        validate_identifier("device id", device_id)?;
        validate_identifier("run id", run_id)?;
        validate_identifier("lease owner", lease_owner)?;
        validate_now(now_ms)?;
        if !(1..=MAX_LEASE_DURATION_MS).contains(&lease_duration_ms) {
            return Err(HubNodeRailError::InvalidInput(
                "lease duration is outside the supported range".to_string(),
            ));
        }
        let lease_expires_at_ms = now_ms
            .checked_add(lease_duration_ms)
            .ok_or_else(|| HubNodeRailError::InvalidInput("lease expiry overflow".to_string()))?;

        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_active_node(&tx, device_id)?;
        reconcile_in_tx(&tx, Some(device_id), now_ms, false)?;
        let current = run_by_id(&tx, run_id)?.ok_or(HubNodeRailError::RunNotFound)?;
        if current.device_id != device_id {
            return Err(HubNodeRailError::LeaseConflict);
        }
        if current.status != HubNodeRunStatus::Queued {
            tx.commit()?;
            return Ok(None);
        }
        let leased = lease_queued_run_in_tx(
            &tx,
            device_id,
            run_id,
            lease_owner,
            lease_expires_at_ms,
            now_ms,
        )?;
        tx.commit()?;
        Ok(Some(leased))
    }

    #[cfg(test)]
    pub(crate) fn mark_accepted(
        &self,
        device_id: &str,
        run_id: &str,
        attempt: u32,
        lease_owner: &str,
        now_ms: i64,
    ) -> Result<HubNodeRunRecord, HubNodeRailError> {
        validate_run_transition(device_id, run_id, attempt, lease_owner, now_ms)?;
        let conn = self.lock()?;
        let changed = conn.execute(
            "UPDATE hub_node_runs
             SET status = 'accepted', effect_state = 'started', updated_at_ms = ?5
             WHERE device_id = ?1 AND run_id = ?2 AND attempt = ?3
               AND status = 'leased' AND lease_owner = ?4
               AND lease_expires_at_ms > ?5",
            params![device_id, run_id, attempt, lease_owner, now_ms],
        )?;
        if changed != 1 {
            return Err(HubNodeRailError::LeaseConflict);
        }
        run_by_id(&conn, run_id)?.ok_or(HubNodeRailError::RunNotFound)
    }

    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    pub(crate) fn record_progress(
        &self,
        device_id: &str,
        run_id: &str,
        attempt: u32,
        lease_owner: &str,
        progress_sequence: u64,
        message: &str,
        now_ms: i64,
    ) -> Result<HubNodeRunRecord, HubNodeRailError> {
        validate_run_transition(device_id, run_id, attempt, lease_owner, now_ms)?;
        if progress_sequence == 0 || message.len() > 4096 || message.contains(['\n', '\r']) {
            return Err(HubNodeRailError::InvalidInput(
                "progress update is invalid".to_string(),
            ));
        }
        let conn = self.lock()?;
        let current = run_by_id(&conn, run_id)?.ok_or(HubNodeRailError::RunNotFound)?;
        if current.device_id != device_id
            || current.attempt != attempt
            || current.lease_owner.as_deref() != Some(lease_owner)
            || current.status != HubNodeRunStatus::Accepted
            || current
                .lease_expires_at_ms
                .is_none_or(|expiry| expiry <= now_ms)
        {
            return Err(HubNodeRailError::LeaseConflict);
        }
        if progress_sequence == current.progress_sequence
            && current.progress_message.as_deref() == Some(message)
        {
            return Ok(current);
        }
        if progress_sequence <= current.progress_sequence {
            return Err(HubNodeRailError::ReplayConflict);
        }
        conn.execute(
            "UPDATE hub_node_runs
             SET progress_sequence = ?2, progress_message = ?3, updated_at_ms = ?4
             WHERE run_id = ?1",
            params![run_id, progress_sequence, message, now_ms],
        )?;
        run_by_id(&conn, run_id)?.ok_or(HubNodeRailError::RunNotFound)
    }

    #[cfg(test)]
    pub(crate) fn complete_run(
        &self,
        device_id: &str,
        completion: &RunCompletion,
        now_ms: i64,
    ) -> Result<HubNodeRunRecord, HubNodeRailError> {
        completion
            .validate()
            .map_err(|error| HubNodeRailError::InvalidInput(error.to_string()))?;
        validate_identifier("device id", device_id)?;
        validate_now(now_ms)?;
        let completion_json = serde_json::to_string(completion)
            .map_err(|error| HubNodeRailError::InvalidInput(error.to_string()))?;
        let completion_sha256 = sha256_hex(completion_json.as_bytes());
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = run_by_id(&tx, &completion.run_id)?.ok_or(HubNodeRailError::RunNotFound)?;
        if current.device_id != device_id || current.attempt != completion.attempt {
            return Err(HubNodeRailError::LeaseConflict);
        }
        if current.status.is_terminal() {
            if current.completion.as_ref() == Some(completion) {
                tx.commit()?;
                return Ok(current);
            }
            if current.status != HubNodeRunStatus::Uncertain || current.completion.is_some() {
                return Err(HubNodeRailError::TerminalConflict);
            }
        }
        if !matches!(
            current.status,
            HubNodeRunStatus::Accepted
                | HubNodeRunStatus::CancelRequested
                | HubNodeRunStatus::Uncertain
        ) {
            return Err(HubNodeRailError::LeaseConflict);
        }
        let status = terminal_status_str(completion.status);
        tx.execute(
            "UPDATE hub_node_runs
             SET status = ?2, effect_state = 'completed', lease_owner = NULL,
                 lease_expires_at_ms = NULL, completion_json = ?3,
                 completion_sha256 = ?4, terminal_at_ms = ?5, updated_at_ms = ?5
             WHERE run_id = ?1",
            params![
                completion.run_id,
                status,
                completion_json,
                completion_sha256,
                now_ms,
            ],
        )?;
        let completed = run_by_id(&tx, &completion.run_id)?.ok_or(HubNodeRailError::RunNotFound)?;
        tx.commit()?;
        Ok(completed)
    }

    pub fn request_cancel(
        &self,
        run_id: &str,
        reason: &str,
        now_ms: i64,
    ) -> Result<CancelledHubNodeRun, HubNodeRailError> {
        validate_identifier("run id", run_id)?;
        validate_now(now_ms)?;
        if reason.is_empty()
            || reason.len() > 160
            || reason.contains(['\n', '\r'])
            || reason.chars().any(char::is_control)
        {
            return Err(HubNodeRailError::InvalidInput(
                "cancellation reason is invalid".to_string(),
            ));
        }
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = run_by_id(&tx, run_id)?.ok_or(HubNodeRailError::RunNotFound)?;
        if current.status.is_terminal() {
            tx.commit()?;
            return Ok(CancelledHubNodeRun {
                run: current,
                outbox: None,
            });
        }
        if current.status == HubNodeRunStatus::CancelRequested {
            let outbox = latest_outbox_for_run_kind(&tx, run_id, "cancel_run")?;
            tx.commit()?;
            return Ok(CancelledHubNodeRun {
                run: current,
                outbox,
            });
        }
        let mut outbox = None;
        match current.status {
            HubNodeRunStatus::Queued => {
                tx.execute(
                    "UPDATE hub_node_runs
                     SET status = 'cancelled', effect_state = 'completed',
                         cancel_requested_at_ms = ?2, terminal_at_ms = ?2,
                         error_code = 'cancelled_before_delivery', updated_at_ms = ?2
                     WHERE run_id = ?1 AND status = 'queued'",
                    params![run_id, now_ms],
                )?;
            }
            HubNodeRunStatus::Leased | HubNodeRunStatus::Accepted => {
                tx.execute(
                    "UPDATE hub_node_runs
                     SET status = 'cancel_requested', cancel_requested_at_ms = ?2,
                         updated_at_ms = ?2
                     WHERE run_id = ?1 AND status IN ('leased', 'accepted')",
                    params![run_id, now_ms],
                )?;
                let message = HubNodeMessage::CancelRun {
                    run_id: current.run_id.clone(),
                    attempt: current.attempt,
                    reason: reason.to_string(),
                };
                outbox = Some(append_outbox_in_tx(
                    &tx,
                    &current.device_id,
                    Some(run_id),
                    &message,
                    now_ms,
                )?);
            }
            _ => {
                tx.commit()?;
                return Ok(CancelledHubNodeRun {
                    run: current,
                    outbox: None,
                });
            }
        }
        let run = run_by_id(&tx, run_id)?.ok_or(HubNodeRailError::RunNotFound)?;
        tx.commit()?;
        Ok(CancelledHubNodeRun { run, outbox })
    }

    pub fn reconcile_after_restart(
        &self,
        now_ms: i64,
    ) -> Result<HubNodeRecoverySummary, HubNodeRailError> {
        validate_now(now_ms)?;
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let summary = reconcile_in_tx(&tx, None, now_ms, true)?;
        tx.commit()?;
        Ok(summary)
    }

    /// Reconcile every in-flight run owned by a Node whose active transport
    /// was lost. Unexpired leases are included because that transport can no
    /// longer provide terminal evidence: reads may be re-offered after a new
    /// connection, while side effects become explicitly uncertain.
    pub fn reconcile_after_disconnect(
        &self,
        device_id: &str,
        now_ms: i64,
    ) -> Result<HubNodeRecoverySummary, HubNodeRailError> {
        validate_identifier("device id", device_id)?;
        validate_now(now_ms)?;
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let summary = reconcile_in_tx(&tx, Some(device_id), now_ms, true)?;
        tx.commit()?;
        Ok(summary)
    }

    pub fn pending_outbox(
        &self,
        device_id: &str,
        after_sequence: u64,
        limit: usize,
    ) -> Result<Vec<HubNodeOutboxRecord>, HubNodeRailError> {
        validate_identifier("device id", device_id)?;
        let conn = self.lock()?;
        let mut statement = conn.prepare(
            "SELECT device_id, sequence, message_kind, message_json,
                    message_sha256, run_id, created_at_ms, acked_at_ms,
                    superseded_at_ms
             FROM hub_node_outbox
             WHERE device_id = ?1 AND sequence > ?2
               AND acked_at_ms IS NULL AND superseded_at_ms IS NULL
             ORDER BY sequence LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![
                device_id,
                u64_to_i64(after_sequence)?,
                limit.clamp(1, MAX_OUTBOX_PAGE)
            ],
            outbox_from_row,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    #[cfg(test)]
    pub(crate) fn acknowledge_hub_sequence(
        &self,
        device_id: &str,
        ack_sequence: u64,
        now_ms: i64,
    ) -> Result<(), HubNodeRailError> {
        validate_identifier("device id", device_id)?;
        validate_now(now_ms)?;
        let ack = u64_to_i64(ack_sequence)?;
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        acknowledge_hub_sequence_in_tx(&tx, device_id, ack, now_ms)?;
        tx.commit()?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    pub(crate) fn record_inbound_receipt(
        &self,
        device_id: &str,
        connection_id: &str,
        sequence: u64,
        message_kind: &str,
        message_sha256: &str,
        received_at_ms: i64,
    ) -> Result<InboundReceipt, HubNodeRailError> {
        validate_identifier("device id", device_id)?;
        validate_identifier("connection id", connection_id)?;
        validate_kind(message_kind)?;
        validate_sha256(message_sha256)?;
        validate_now(received_at_ms)?;
        if sequence == 0 {
            return Err(HubNodeRailError::InvalidInput(
                "message sequence starts at one".to_string(),
            ));
        }
        let sequence = u64_to_i64(sequence)?;
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_active_node(&tx, device_id)?;
        let receipt = record_inbound_receipt_in_tx(
            &tx,
            device_id,
            connection_id,
            sequence,
            message_kind,
            message_sha256,
            received_at_ms,
        )?;
        tx.commit()?;
        Ok(receipt)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, HubNodeRailError> {
        self.conn
            .lock()
            .map_err(|error| HubNodeRailError::Lock(error.to_string()))
    }
}

fn lease_queued_run_in_tx(
    tx: &Transaction<'_>,
    device_id: &str,
    run_id: &str,
    lease_owner: &str,
    lease_expires_at_ms: i64,
    now_ms: i64,
) -> Result<LeasedHubNodeRun, HubNodeRailError> {
    let changed = tx.execute(
        "UPDATE hub_node_runs
         SET status = 'leased', attempt = attempt + 1,
             lease_owner = ?2, lease_expires_at_ms = ?3,
             effect_state = 'not_started', progress_sequence = 0,
             progress_message = NULL, error_code = NULL, updated_at_ms = ?4
         WHERE run_id = ?1 AND device_id = ?5 AND status = 'queued'",
        params![run_id, lease_owner, lease_expires_at_ms, now_ms, device_id,],
    )?;
    if changed != 1 {
        return Err(HubNodeRailError::LeaseConflict);
    }
    let run = run_by_id(tx, run_id)?.ok_or(HubNodeRailError::RunNotFound)?;
    let lease = lease_from_record(&run)?;
    let message = HubNodeMessage::RunOffer(lease.clone());
    let outbox = append_outbox_in_tx(tx, device_id, Some(run_id), &message, now_ms)?;
    Ok(LeasedHubNodeRun { run, lease, outbox })
}

fn reconcile_in_tx(
    tx: &Transaction<'_>,
    device_id: Option<&str>,
    now_ms: i64,
    include_unexpired: bool,
) -> Result<HubNodeRecoverySummary, HubNodeRailError> {
    let device = device_id.unwrap_or("");
    let all_leases = i64::from(include_unexpired);
    let interrupted_approvals = tx.execute(
        "UPDATE hub_node_runs
         SET status = 'cancelled', effect_state = 'completed',
             lease_owner = NULL, lease_expires_at_ms = NULL,
             error_code = 'approval_not_granted', terminal_at_ms = ?1,
             updated_at_ms = ?1
         WHERE (?2 = '' OR device_id = ?2) AND status = 'leased'
           AND effect_state = 'not_started'
           AND EXISTS (
               SELECT 1 FROM hub_node_run_approvals approval
               WHERE approval.run_id = hub_node_runs.run_id
                 AND approval.attempt = hub_node_runs.attempt
                 AND (
                     approval.status IN ('denied', 'timed_out')
                     OR (
                         approval.status = 'pending'
                         AND (?3 = 1 OR approval.expires_at_ms <= ?1)
                     )
                 )
           )",
        params![now_ms, device, all_leases],
    )?;
    tx.execute(
        "UPDATE hub_node_run_approvals
         SET status = 'timed_out', decision = 'timed_out',
             reason = 'Approval interrupted before execution', decided_at_ms = ?1
         WHERE status = 'pending'
           AND EXISTS (
               SELECT 1 FROM hub_node_runs run
               WHERE run.run_id = hub_node_run_approvals.run_id
                 AND run.attempt = hub_node_run_approvals.attempt
                 AND run.status = 'cancelled'
                 AND run.error_code = 'approval_not_granted'
           )",
        [now_ms],
    )?;
    let cancelled_before_effect = interrupted_approvals
        + tx.execute(
            "UPDATE hub_node_runs
         SET status = 'cancelled', effect_state = 'completed',
             lease_owner = NULL, lease_expires_at_ms = NULL,
             error_code = 'cancelled_before_effect', terminal_at_ms = ?1,
             updated_at_ms = ?1
         WHERE (?2 = '' OR device_id = ?2) AND status = 'cancel_requested'
           AND (effect = 'read_only' OR effect_state = 'not_started')
           AND (?3 = 1 OR lease_expires_at_ms <= ?1)",
            params![now_ms, device, all_leases],
        )?;
    let requeued_read_only = tx.execute(
        "UPDATE hub_node_runs
         SET status = 'queued', lease_owner = NULL, lease_expires_at_ms = NULL,
             error_code = 'lease_recovered', updated_at_ms = ?1
         WHERE (?2 = '' OR device_id = ?2)
           AND status IN ('leased', 'accepted') AND effect = 'read_only'
           AND (?3 = 1 OR lease_expires_at_ms <= ?1)",
        params![now_ms, device, all_leases],
    )?;
    let uncertain_side_effects = tx.execute(
        "UPDATE hub_node_runs
         SET status = 'uncertain', effect_state = 'started',
             lease_owner = NULL, lease_expires_at_ms = NULL,
             error_code = 'delivery_or_effect_interrupted', terminal_at_ms = ?1,
             updated_at_ms = ?1
         WHERE (?2 = '' OR device_id = ?2)
           AND status IN ('leased', 'accepted', 'cancel_requested')
           AND effect <> 'read_only'
           AND (?3 = 1 OR lease_expires_at_ms <= ?1)",
        params![now_ms, device, all_leases],
    )?;
    Ok(HubNodeRecoverySummary {
        requeued_read_only,
        cancelled_before_effect,
        uncertain_side_effects,
    })
}

fn append_outbox_in_tx(
    tx: &Transaction<'_>,
    device_id: &str,
    run_id: Option<&str>,
    message: &HubNodeMessage,
    now_ms: i64,
) -> Result<HubNodeOutboxRecord, HubNodeRailError> {
    ensure_cursor(tx, device_id, now_ms)?;
    let sequence: i64 = tx.query_row(
        "SELECT next_hub_sequence FROM hub_node_cursors WHERE device_id = ?1",
        [device_id],
        |row| row.get(0),
    )?;
    let message_json = serde_json::to_string(message)
        .map_err(|error| HubNodeRailError::InvalidInput(error.to_string()))?;
    let message_sha256 = sha256_hex(message_json.as_bytes());
    let message_kind = message_kind(message);
    tx.execute(
        "INSERT INTO hub_node_outbox (
             device_id, sequence, message_kind, message_json,
             message_sha256, run_id, created_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            device_id,
            sequence,
            message_kind,
            message_json,
            message_sha256,
            run_id,
            now_ms,
        ],
    )?;
    tx.execute(
        "UPDATE hub_node_cursors
         SET next_hub_sequence = next_hub_sequence + 1, updated_at_ms = ?2
         WHERE device_id = ?1",
        params![device_id, now_ms],
    )?;
    Ok(HubNodeOutboxRecord {
        device_id: device_id.to_string(),
        sequence: i64_to_u64(sequence, 1)?,
        message_kind: message_kind.to_string(),
        message_json,
        message_sha256,
        run_id: run_id.map(str::to_string),
        created_at_ms: now_ms,
        acked_at_ms: None,
        superseded_at_ms: None,
    })
}

fn ensure_cursor(
    tx: &Transaction<'_>,
    device_id: &str,
    now_ms: i64,
) -> Result<(), HubNodeRailError> {
    tx.execute(
        "INSERT OR IGNORE INTO hub_node_cursors (device_id, updated_at_ms)
         VALUES (?1, ?2)",
        params![device_id, now_ms],
    )?;
    Ok(())
}

fn acknowledge_hub_sequence_in_tx(
    tx: &Transaction<'_>,
    device_id: &str,
    ack: i64,
    now_ms: i64,
) -> Result<(), HubNodeRailError> {
    ensure_cursor(tx, device_id, now_ms)?;
    let (next, current): (i64, i64) = tx.query_row(
        "SELECT next_hub_sequence, last_hub_ack_sequence
         FROM hub_node_cursors WHERE device_id = ?1",
        [device_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if ack >= next {
        return Err(HubNodeRailError::InvalidAcknowledgement);
    }
    if ack <= current {
        return Ok(());
    }
    tx.execute(
        "UPDATE hub_node_outbox SET acked_at_ms = COALESCE(acked_at_ms, ?3)
         WHERE device_id = ?1 AND sequence <= ?2",
        params![device_id, ack, now_ms],
    )?;
    tx.execute(
        "UPDATE hub_node_cursors
         SET last_hub_ack_sequence = ?2, updated_at_ms = ?3
         WHERE device_id = ?1",
        params![device_id, ack, now_ms],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn record_inbound_receipt_in_tx(
    tx: &Transaction<'_>,
    device_id: &str,
    connection_id: &str,
    sequence: i64,
    message_kind: &str,
    message_sha256: &str,
    received_at_ms: i64,
) -> Result<InboundReceipt, HubNodeRailError> {
    ensure_cursor(tx, device_id, received_at_ms)?;
    let last: i64 = tx.query_row(
        "SELECT last_node_sequence FROM hub_node_cursors WHERE device_id = ?1",
        [device_id],
        |row| row.get(0),
    )?;
    if sequence <= last {
        let stored = tx
            .query_row(
                "SELECT message_sha256 FROM hub_node_inbox
                 WHERE device_id = ?1 AND sequence = ?2",
                params![device_id, sequence],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        return match stored.as_deref() {
            Some(value) if value == message_sha256 => Ok(InboundReceipt::Duplicate),
            _ => Err(HubNodeRailError::ReplayConflict),
        };
    }
    if sequence != last + 1 {
        return Err(HubNodeRailError::SequenceGap);
    }
    tx.execute(
        "INSERT INTO hub_node_inbox (
             device_id, sequence, connection_id, message_kind,
             message_sha256, received_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            device_id,
            sequence,
            connection_id,
            message_kind,
            message_sha256,
            received_at_ms,
        ],
    )?;
    tx.execute(
        "UPDATE hub_node_cursors
         SET last_node_sequence = ?2, updated_at_ms = ?3
         WHERE device_id = ?1",
        params![device_id, sequence, received_at_ms],
    )?;
    Ok(InboundReceipt::Recorded)
}

fn ensure_active_node(conn: &Connection, device_id: &str) -> Result<(), HubNodeRailError> {
    let active = conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM captain_devices
             WHERE device_id = ?1 AND role = 'node' AND status = 'active'
         )",
        [device_id],
        |row| row.get::<_, bool>(0),
    )?;
    if active {
        Ok(())
    } else {
        Err(HubNodeRailError::NodeUnavailable)
    }
}

const RUN_APPROVAL_SELECT: &str = "SELECT approval_id, run_id, attempt, action_digest,
            action_summary, risk_level, status, decision, reason,
            requested_at_ms, expires_at_ms, decided_at_ms
     FROM hub_node_run_approvals";

fn approval_by_id(
    conn: &Connection,
    approval_id: &str,
) -> Result<Option<HubNodeRunApprovalRecord>, HubNodeRailError> {
    conn.query_row(
        &format!("{RUN_APPROVAL_SELECT} WHERE approval_id = ?1"),
        [approval_id],
        approval_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn latest_run_approval(
    conn: &Connection,
    run_id: &str,
) -> Result<Option<HubNodeRunApprovalRecord>, HubNodeRailError> {
    conn.query_row(
        &format!("{RUN_APPROVAL_SELECT} WHERE run_id = ?1 ORDER BY attempt DESC LIMIT 1"),
        [run_id],
        approval_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn approval_from_row(row: &Row<'_>) -> rusqlite::Result<HubNodeRunApprovalRecord> {
    let risk_level = parse_risk_level(&row.get::<_, String>(5)?)
        .ok_or_else(|| corrupt(5, "unknown approval risk"))?;
    let status = HubNodeRunApprovalStatus::parse(&row.get::<_, String>(6)?)
        .ok_or_else(|| corrupt(6, "unknown approval status"))?;
    let decision = row
        .get::<_, Option<String>>(7)?
        .map(|value| {
            parse_approval_decision(&value).ok_or_else(|| corrupt(7, "unknown approval decision"))
        })
        .transpose()?;
    let record = HubNodeRunApprovalRecord {
        approval_id: row.get(0)?,
        run_id: row.get(1)?,
        attempt: i64_to_u32(row.get(2)?, 2)?,
        action_digest: row.get(3)?,
        action_summary: row.get(4)?,
        risk_level,
        status,
        decision,
        reason: row.get(8)?,
        requested_at_ms: row.get(9)?,
        expires_at_ms: row.get(10)?,
        decided_at_ms: row.get(11)?,
    };
    RunApprovalRequest {
        run_id: record.run_id.clone(),
        attempt: record.attempt,
        approval_id: record.approval_id.clone(),
        action_digest: record.action_digest.clone(),
        action_summary: record.action_summary.clone(),
        risk_level: record.risk_level,
        expires_at_ms: record.expires_at_ms,
        path_policy_applied: true,
    }
    .validate()
    .map_err(|error| corrupt(0, error.to_string()))?;
    if record.expires_at_ms <= record.requested_at_ms
        || record.decision.is_some() != record.decided_at_ms.is_some()
        || record.status == HubNodeRunApprovalStatus::Pending && record.decision.is_some()
        || record.status != HubNodeRunApprovalStatus::Pending && record.decision.is_none()
        || record
            .decision
            .is_some_and(|decision| approval_status_for_decision(decision) != record.status)
    {
        return Err(corrupt(6, "inconsistent approval state"));
    }
    if let (Some(decision), Some(decided_at_ms)) = (record.decision, record.decided_at_ms) {
        RunApprovalDecision {
            run_id: record.run_id.clone(),
            attempt: record.attempt,
            approval_id: record.approval_id.clone(),
            action_digest: record.action_digest.clone(),
            decision,
            reason: record.reason.clone(),
            decided_at_ms,
        }
        .validate()
        .map_err(|error| corrupt(7, error.to_string()))?;
    }
    Ok(record)
}

const RUN_SELECT: &str = "SELECT run_id, device_id, idempotency_key, workspace_id, tool_name,
            input_json, effect, status, attempt, lease_owner,
            lease_expires_at_ms, effect_state, progress_sequence,
            progress_message, completion_json, error_code,
            cancel_requested_at_ms, created_at_ms, updated_at_ms, terminal_at_ms,
            rejection_json, rejection_sha256
     FROM hub_node_runs";

fn run_by_id(
    conn: &Connection,
    run_id: &str,
) -> Result<Option<HubNodeRunRecord>, HubNodeRailError> {
    conn.query_row(
        &format!("{RUN_SELECT} WHERE run_id = ?1"),
        [run_id],
        run_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn run_by_idempotency(
    conn: &Connection,
    device_id: &str,
    idempotency_key: &str,
) -> Result<Option<HubNodeRunRecord>, HubNodeRailError> {
    conn.query_row(
        &format!("{RUN_SELECT} WHERE device_id = ?1 AND idempotency_key = ?2"),
        params![device_id, idempotency_key],
        run_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn run_from_row(row: &Row<'_>) -> rusqlite::Result<HubNodeRunRecord> {
    let input_json: String = row.get(5)?;
    let effect: String = row.get(6)?;
    let status: String = row.get(7)?;
    let effect_state: String = row.get(11)?;
    let completion_json: Option<String> = row.get(14)?;
    let rejection_json: Option<String> = row.get(20)?;
    let rejection_sha256: Option<String> = row.get(21)?;
    let rejection = match (rejection_json, rejection_sha256) {
        (None, None) => None,
        (Some(value), Some(expected_digest)) if sha256_hex(value.as_bytes()) == expected_digest => {
            Some(serde_json::from_str(&value).map_err(|error| corrupt(20, error.to_string()))?)
        }
        _ => return Err(corrupt(20, "invalid run rejection evidence")),
    };
    Ok(HubNodeRunRecord {
        run_id: row.get(0)?,
        device_id: row.get(1)?,
        idempotency_key: row.get(2)?,
        workspace_id: row.get(3)?,
        tool_name: row.get(4)?,
        input: serde_json::from_str(&input_json).map_err(|error| corrupt(5, error.to_string()))?,
        effect: parse_effect(&effect).ok_or_else(|| corrupt(6, "unknown run effect"))?,
        status: HubNodeRunStatus::parse(&status).ok_or_else(|| corrupt(7, "unknown run status"))?,
        attempt: i64_to_u32(row.get(8)?, 8)?,
        lease_owner: row.get(9)?,
        lease_expires_at_ms: row.get(10)?,
        effect_state: HubNodeEffectState::parse(&effect_state)
            .ok_or_else(|| corrupt(11, "unknown effect state"))?,
        progress_sequence: i64_to_u64(row.get(12)?, 12)?,
        progress_message: row.get(13)?,
        completion: completion_json
            .map(|value| {
                serde_json::from_str(&value).map_err(|error| corrupt(14, error.to_string()))
            })
            .transpose()?,
        rejection,
        error_code: row.get(15)?,
        cancel_requested_at_ms: row.get(16)?,
        created_at_ms: row.get(17)?,
        updated_at_ms: row.get(18)?,
        terminal_at_ms: row.get(19)?,
    })
}

fn outbox_from_row(row: &Row<'_>) -> rusqlite::Result<HubNodeOutboxRecord> {
    Ok(HubNodeOutboxRecord {
        device_id: row.get(0)?,
        sequence: i64_to_u64(row.get(1)?, 1)?,
        message_kind: row.get(2)?,
        message_json: row.get(3)?,
        message_sha256: row.get(4)?,
        run_id: row.get(5)?,
        created_at_ms: row.get(6)?,
        acked_at_ms: row.get(7)?,
        superseded_at_ms: row.get(8)?,
    })
}

fn latest_outbox_for_run_kind(
    conn: &Connection,
    run_id: &str,
    message_kind: &str,
) -> Result<Option<HubNodeOutboxRecord>, HubNodeRailError> {
    conn.query_row(
        "SELECT device_id, sequence, message_kind, message_json,
                message_sha256, run_id, created_at_ms, acked_at_ms,
                superseded_at_ms
         FROM hub_node_outbox
         WHERE run_id = ?1 AND message_kind = ?2
         ORDER BY sequence DESC LIMIT 1",
        params![run_id, message_kind],
        outbox_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn outbox_for_run_approval(
    conn: &Connection,
    run_id: &str,
    approval_id: &str,
) -> Result<Option<HubNodeOutboxRecord>, HubNodeRailError> {
    conn.query_row(
        "SELECT device_id, sequence, message_kind, message_json,
                message_sha256, run_id, created_at_ms, acked_at_ms,
                superseded_at_ms
         FROM hub_node_outbox
         WHERE run_id = ?1 AND message_kind = 'run_approval_decision'
           AND json_extract(message_json, '$.payload.approval_id') = ?2
         ORDER BY sequence DESC LIMIT 1",
        params![run_id, approval_id],
        outbox_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn latest_outbox_for_device_kind(
    conn: &Connection,
    device_id: &str,
    message_kind: &str,
    not_before_ms: i64,
) -> Result<Option<HubNodeOutboxRecord>, HubNodeRailError> {
    conn.query_row(
        "SELECT device_id, sequence, message_kind, message_json,
                message_sha256, run_id, created_at_ms, acked_at_ms,
                superseded_at_ms
         FROM hub_node_outbox
         WHERE device_id = ?1 AND message_kind = ?2 AND created_at_ms >= ?3
         ORDER BY sequence DESC LIMIT 1",
        params![device_id, message_kind, not_before_ms],
        outbox_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn same_run_request(existing: &HubNodeRunRecord, input: &NewHubNodeRun) -> bool {
    existing.run_id == input.run_id
        && existing.device_id == input.device_id
        && existing.idempotency_key == input.idempotency_key
        && existing.workspace_id == input.workspace_id
        && existing.tool_name == input.tool_name
        && existing.input == input.input
        && existing.effect == input.effect
}

fn lease_from_record(run: &HubNodeRunRecord) -> Result<RunLease, HubNodeRailError> {
    let lease = RunLease {
        run_id: run.run_id.clone(),
        attempt: run.attempt,
        idempotency_key: run.idempotency_key.clone(),
        workspace_id: run.workspace_id.clone(),
        tool_name: run.tool_name.clone(),
        input: run.input.clone(),
        effect: run.effect,
        lease_expires_at_ms: run
            .lease_expires_at_ms
            .ok_or(HubNodeRailError::LeaseConflict)?,
    };
    lease
        .validate()
        .map_err(|error| HubNodeRailError::InvalidInput(error.to_string()))?;
    Ok(lease)
}

fn validate_new_run(input: &NewHubNodeRun) -> Result<(), HubNodeRailError> {
    validate_identifier("device id", &input.device_id)?;
    validate_now(input.created_at_ms)?;
    RunLease {
        run_id: input.run_id.clone(),
        attempt: 1,
        idempotency_key: input.idempotency_key.clone(),
        workspace_id: input.workspace_id.clone(),
        tool_name: input.tool_name.clone(),
        input: input.input.clone(),
        effect: input.effect,
        lease_expires_at_ms: 1,
    }
    .validate()
    .map_err(|error| HubNodeRailError::InvalidInput(error.to_string()))
}

#[cfg(test)]
fn validate_run_transition(
    device_id: &str,
    run_id: &str,
    attempt: u32,
    lease_owner: &str,
    now_ms: i64,
) -> Result<(), HubNodeRailError> {
    validate_identifier("device id", device_id)?;
    validate_identifier("run id", run_id)?;
    validate_identifier("lease owner", lease_owner)?;
    validate_now(now_ms)?;
    if attempt == 0 {
        return Err(HubNodeRailError::InvalidInput(
            "run attempt starts at one".to_string(),
        ));
    }
    Ok(())
}

fn validate_identifier(label: &str, value: &str) -> Result<(), HubNodeRailError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(HubNodeRailError::InvalidInput(format!(
            "{label} is invalid"
        )));
    }
    Ok(())
}

fn validate_kind(value: &str) -> Result<(), HubNodeRailError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(HubNodeRailError::InvalidInput(
            "message kind is invalid".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
fn validate_sha256(value: &str) -> Result<(), HubNodeRailError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(HubNodeRailError::InvalidInput(
            "message digest is invalid".to_string(),
        ));
    }
    Ok(())
}

fn validate_now(now_ms: i64) -> Result<(), HubNodeRailError> {
    if now_ms < 0 {
        Err(HubNodeRailError::InvalidInput(
            "timestamp is invalid".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn effect_str(effect: RunEffect) -> &'static str {
    match effect {
        RunEffect::ReadOnly => "read_only",
        RunEffect::LocalMutation => "local_mutation",
        RunEffect::ExternalEffect => "external_effect",
    }
}

fn parse_effect(value: &str) -> Option<RunEffect> {
    Some(match value {
        "read_only" => RunEffect::ReadOnly,
        "local_mutation" => RunEffect::LocalMutation,
        "external_effect" => RunEffect::ExternalEffect,
        _ => return None,
    })
}

fn terminal_status_str(status: captain_wire::hub_protocol::RunTerminalStatus) -> &'static str {
    use captain_wire::hub_protocol::RunTerminalStatus;
    match status {
        RunTerminalStatus::Succeeded => "succeeded",
        RunTerminalStatus::Failed => "failed",
        RunTerminalStatus::Cancelled => "cancelled",
        RunTerminalStatus::Uncertain => "uncertain",
    }
}

fn risk_level_str(risk: RiskLevel) -> &'static str {
    match risk {
        RiskLevel::Low => "low",
        RiskLevel::Medium => "medium",
        RiskLevel::High => "high",
        RiskLevel::Critical => "critical",
    }
}

fn parse_risk_level(value: &str) -> Option<RiskLevel> {
    Some(match value {
        "low" => RiskLevel::Low,
        "medium" => RiskLevel::Medium,
        "high" => RiskLevel::High,
        "critical" => RiskLevel::Critical,
        _ => return None,
    })
}

fn approval_decision_str(decision: ApprovalDecision) -> &'static str {
    match decision {
        ApprovalDecision::Approved => "approved",
        ApprovalDecision::ApprovedSession => "approved_session",
        ApprovalDecision::ApprovedAlways => "approved_always",
        ApprovalDecision::Denied => "denied",
        ApprovalDecision::DeniedSession => "denied_session",
        ApprovalDecision::DeniedAlways => "denied_always",
        ApprovalDecision::TimedOut => "timed_out",
    }
}

fn parse_approval_decision(value: &str) -> Option<ApprovalDecision> {
    Some(match value {
        "approved" => ApprovalDecision::Approved,
        "approved_session" => ApprovalDecision::ApprovedSession,
        "approved_always" => ApprovalDecision::ApprovedAlways,
        "denied" => ApprovalDecision::Denied,
        "denied_session" => ApprovalDecision::DeniedSession,
        "denied_always" => ApprovalDecision::DeniedAlways,
        "timed_out" => ApprovalDecision::TimedOut,
        _ => return None,
    })
}

fn approval_status_for_decision(decision: ApprovalDecision) -> HubNodeRunApprovalStatus {
    if decision.is_approved() {
        HubNodeRunApprovalStatus::Approved
    } else if decision == ApprovalDecision::TimedOut {
        HubNodeRunApprovalStatus::TimedOut
    } else {
        HubNodeRunApprovalStatus::Denied
    }
}

fn approval_status_str(status: HubNodeRunApprovalStatus) -> &'static str {
    match status {
        HubNodeRunApprovalStatus::Pending => "pending",
        HubNodeRunApprovalStatus::Approved => "approved",
        HubNodeRunApprovalStatus::Denied => "denied",
        HubNodeRunApprovalStatus::TimedOut => "timed_out",
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

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn u64_to_i64(value: u64) -> Result<i64, HubNodeRailError> {
    i64::try_from(value)
        .map_err(|_| HubNodeRailError::InvalidInput("sequence is too large".to_string()))
}

fn i64_to_u64(value: i64, column: usize) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|_| corrupt(column, "negative integer"))
}

fn i64_to_u32(value: i64, column: usize) -> rusqlite::Result<u32> {
    u32::try_from(value).map_err(|_| corrupt(column, "integer exceeds u32"))
}

fn corrupt(column: usize, message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message.into(),
        )),
    )
}
