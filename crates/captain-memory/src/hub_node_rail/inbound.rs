use super::{
    acknowledge_hub_sequence_in_tx, ensure_active_node, record_inbound_receipt_in_tx, run_by_id,
    sha256_hex, terminal_status_str, u64_to_i64, HubNodeRailError, HubNodeRailStore,
    HubNodeRunRecord, HubNodeRunStatus, InboundReceipt,
};
use captain_wire::hub_protocol::{
    HubNodeEnvelope, HubNodeMessage, RunApprovalRequest, RunCompletion, RunRejection,
    HUB_NODE_PROTOCOL_VERSION,
};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};

const MAX_LEASE_DURATION_MS: u64 = 15 * 60 * 1_000;

#[derive(Debug, Clone, PartialEq)]
pub enum HubNodeInboundOutcome {
    Duplicate,
    Heartbeat { renewed_runs: usize },
    RunAccepted(HubNodeRunRecord),
    RunApprovalRequired(super::HubNodeRunApprovalRecord),
    RunRejected(HubNodeRunRecord),
    RunProgress(HubNodeRunRecord),
    RunCompleted(HubNodeRunRecord),
    Acknowledged,
    ProtocolError { code: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppliedHubNodeEnvelope {
    pub outcome: HubNodeInboundOutcome,
    pub acknowledged_node_sequence: u64,
}

impl HubNodeRailStore {
    /// Apply an authenticated Node envelope as one receipt, acknowledgement,
    /// presence update, and run-state transaction.
    pub fn apply_node_envelope(
        &self,
        envelope: &HubNodeEnvelope,
        lease_duration_ms: u64,
        now_ms: i64,
    ) -> Result<AppliedHubNodeEnvelope, HubNodeRailError> {
        envelope
            .validate()
            .map_err(|error| HubNodeRailError::InvalidInput(error.to_string()))?;
        super::validate_now(now_ms)?;
        if lease_duration_ms == 0 || lease_duration_ms > MAX_LEASE_DURATION_MS {
            return Err(HubNodeRailError::InvalidInput(
                "lease duration is outside the supported range".to_string(),
            ));
        }
        if matches!(
            &envelope.message,
            HubNodeMessage::Hello { .. }
                | HubNodeMessage::Welcome { .. }
                | HubNodeMessage::Superseded { .. }
                | HubNodeMessage::RunOffer(_)
                | HubNodeMessage::RunApprovalDecision(_)
                | HubNodeMessage::CancelRun { .. }
        ) {
            return Err(HubNodeRailError::InvalidMessageDirection);
        }
        let serialized = serde_json::to_vec(envelope)
            .map_err(|error| HubNodeRailError::InvalidInput(error.to_string()))?;
        let digest = sha256_hex(&serialized);
        let lease_expires_at_ms = now_ms
            .checked_add(i64::try_from(lease_duration_ms).map_err(|_| {
                HubNodeRailError::InvalidInput("lease duration is invalid".to_string())
            })?)
            .ok_or_else(|| HubNodeRailError::InvalidInput("lease expiry overflow".to_string()))?;

        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_active_node(&tx, &envelope.device_id)?;
        ensure_active_connection(&tx, envelope)?;
        if let Some(acknowledged) = envelope.ack_sequence {
            acknowledge_hub_sequence_in_tx(
                &tx,
                &envelope.device_id,
                u64_to_i64(acknowledged)?,
                now_ms,
            )?;
        }
        let receipt = record_inbound_receipt_in_tx(
            &tx,
            &envelope.device_id,
            &envelope.connection_id,
            u64_to_i64(envelope.sequence)?,
            inbound_kind(&envelope.message),
            &digest,
            now_ms,
        )?;
        if receipt == InboundReceipt::Duplicate {
            let acknowledged_node_sequence = last_node_sequence(&tx, &envelope.device_id)?;
            tx.commit()?;
            return Ok(AppliedHubNodeEnvelope {
                outcome: HubNodeInboundOutcome::Duplicate,
                acknowledged_node_sequence,
            });
        }

        touch_presence(&tx, &envelope.device_id, &envelope.connection_id, now_ms)?;
        let outcome = match &envelope.message {
            HubNodeMessage::Heartbeat { active_run_ids } => HubNodeInboundOutcome::Heartbeat {
                renewed_runs: renew_heartbeat_runs(
                    &tx,
                    &envelope.device_id,
                    &envelope.connection_id,
                    active_run_ids,
                    lease_expires_at_ms,
                    now_ms,
                )?,
            },
            HubNodeMessage::RunAccepted { run_id, attempt } => {
                HubNodeInboundOutcome::RunAccepted(mark_accepted_in_tx(
                    &tx,
                    &envelope.device_id,
                    &envelope.connection_id,
                    run_id,
                    *attempt,
                    lease_expires_at_ms,
                    now_ms,
                )?)
            }
            HubNodeMessage::RunApprovalRequired(request) => {
                HubNodeInboundOutcome::RunApprovalRequired(record_approval_required_in_tx(
                    &tx,
                    &envelope.device_id,
                    &envelope.connection_id,
                    request,
                    now_ms,
                )?)
            }
            HubNodeMessage::RunRejected(rejection) => {
                HubNodeInboundOutcome::RunRejected(reject_run_in_tx(
                    &tx,
                    &envelope.device_id,
                    &envelope.connection_id,
                    rejection,
                    now_ms,
                )?)
            }
            HubNodeMessage::RunProgress {
                run_id,
                attempt,
                progress_sequence,
                message,
                ..
            } => HubNodeInboundOutcome::RunProgress(record_progress_in_tx(
                &tx,
                &envelope.device_id,
                &envelope.connection_id,
                run_id,
                *attempt,
                *progress_sequence,
                message,
                lease_expires_at_ms,
                now_ms,
            )?),
            HubNodeMessage::RunCompleted(completion) => HubNodeInboundOutcome::RunCompleted(
                complete_run_in_tx(&tx, &envelope.device_id, completion, now_ms)?,
            ),
            HubNodeMessage::AckOnly => HubNodeInboundOutcome::Acknowledged,
            HubNodeMessage::ProtocolError { code, .. } => {
                record_protocol_error(
                    &tx,
                    &envelope.device_id,
                    &envelope.connection_id,
                    code,
                    now_ms,
                )?;
                HubNodeInboundOutcome::ProtocolError { code: code.clone() }
            }
            HubNodeMessage::Hello { .. }
            | HubNodeMessage::Welcome { .. }
            | HubNodeMessage::Superseded { .. }
            | HubNodeMessage::RunOffer(_)
            | HubNodeMessage::RunApprovalDecision(_)
            | HubNodeMessage::CancelRun { .. } => {
                return Err(HubNodeRailError::InvalidMessageDirection);
            }
        };
        let acknowledged_node_sequence = last_node_sequence(&tx, &envelope.device_id)?;
        tx.commit()?;
        Ok(AppliedHubNodeEnvelope {
            outcome,
            acknowledged_node_sequence,
        })
    }
}

fn ensure_active_connection(
    tx: &Transaction<'_>,
    envelope: &HubNodeEnvelope,
) -> Result<(), HubNodeRailError> {
    let stored = tx
        .query_row(
            "SELECT connection_id, protocol_major, protocol_minor, status
             FROM hub_node_connections WHERE device_id = ?1",
            [envelope.device_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u16>(1)?,
                    row.get::<_, u16>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?
        .ok_or(HubNodeRailError::ConnectionConflict)?;
    if stored.0 != envelope.connection_id || stored.3 != "active" {
        return Err(HubNodeRailError::ConnectionConflict);
    }
    HUB_NODE_PROTOCOL_VERSION
        .negotiate(captain_wire::hub_protocol::ProtocolVersion {
            major: stored.1,
            minor: stored.2,
        })
        .and_then(|version| version.negotiate(envelope.protocol_version))
        .map_err(|error| HubNodeRailError::InvalidInput(error.to_string()))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn mark_accepted_in_tx(
    tx: &Transaction<'_>,
    device_id: &str,
    connection_id: &str,
    run_id: &str,
    attempt: u32,
    lease_expires_at_ms: i64,
    now_ms: i64,
) -> Result<HubNodeRunRecord, HubNodeRailError> {
    let changed = tx.execute(
        "UPDATE hub_node_runs
         SET status = 'accepted', effect_state = 'started',
             lease_expires_at_ms = ?6, updated_at_ms = ?7
         WHERE device_id = ?1 AND run_id = ?2 AND attempt = ?3
           AND status = 'leased' AND lease_owner = ?4
           AND lease_expires_at_ms > ?5
           AND NOT EXISTS (
               SELECT 1 FROM hub_node_run_approvals approval
               WHERE approval.run_id = hub_node_runs.run_id
                 AND approval.attempt = hub_node_runs.attempt
                 AND approval.status <> 'approved'
           )",
        params![
            device_id,
            run_id,
            attempt,
            connection_id,
            now_ms,
            lease_expires_at_ms,
            now_ms,
        ],
    )?;
    if changed != 1 {
        return Err(HubNodeRailError::LeaseConflict);
    }
    run_by_id(tx, run_id)?.ok_or(HubNodeRailError::RunNotFound)
}

fn record_approval_required_in_tx(
    tx: &Transaction<'_>,
    device_id: &str,
    connection_id: &str,
    request: &RunApprovalRequest,
    now_ms: i64,
) -> Result<super::HubNodeRunApprovalRecord, HubNodeRailError> {
    request
        .validate()
        .map_err(|error| HubNodeRailError::InvalidInput(error.to_string()))?;
    let current = run_by_id(tx, &request.run_id)?.ok_or(HubNodeRailError::RunNotFound)?;
    if current.device_id != device_id
        || current.attempt != request.attempt
        || current.lease_owner.as_deref() != Some(connection_id)
        || current.status != HubNodeRunStatus::Leased
        || current.effect_state != super::HubNodeEffectState::NotStarted
        || current
            .lease_expires_at_ms
            .is_none_or(|expiry| expiry <= now_ms || request.expires_at_ms > expiry)
        || request.expires_at_ms <= now_ms
    {
        return Err(HubNodeRailError::LeaseConflict);
    }

    if let Some(existing) = super::approval_by_id(tx, &request.approval_id)? {
        if existing.run_id == request.run_id
            && existing.attempt == request.attempt
            && existing.action_digest == request.action_digest
            && existing.action_summary == request.action_summary
            && existing.risk_level == request.risk_level
            && existing.expires_at_ms == request.expires_at_ms
        {
            return Ok(existing);
        }
        return Err(HubNodeRailError::ReplayConflict);
    }
    if super::latest_run_approval(tx, &request.run_id)?
        .is_some_and(|existing| existing.attempt == request.attempt)
    {
        return Err(HubNodeRailError::ReplayConflict);
    }

    tx.execute(
        "INSERT INTO hub_node_run_approvals (
             approval_id, run_id, attempt, action_digest, action_summary,
             risk_level, requested_at_ms, expires_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            request.approval_id,
            request.run_id,
            request.attempt,
            request.action_digest,
            request.action_summary,
            super::risk_level_str(request.risk_level),
            now_ms,
            request.expires_at_ms,
        ],
    )?;
    super::approval_by_id(tx, &request.approval_id)?.ok_or(HubNodeRailError::StorageInvariant)
}

fn reject_run_in_tx(
    tx: &Transaction<'_>,
    device_id: &str,
    connection_id: &str,
    rejection: &RunRejection,
    now_ms: i64,
) -> Result<HubNodeRunRecord, HubNodeRailError> {
    rejection
        .validate()
        .map_err(|error| HubNodeRailError::InvalidInput(error.to_string()))?;
    if rejection.code.len() > 96 {
        return Err(HubNodeRailError::InvalidInput(
            "run rejection code exceeds storage limit".to_string(),
        ));
    }
    let current = run_by_id(tx, &rejection.run_id)?.ok_or(HubNodeRailError::RunNotFound)?;
    if current.device_id != device_id || current.attempt != rejection.attempt {
        return Err(HubNodeRailError::LeaseConflict);
    }
    if current.status.is_terminal() {
        if current.rejection.as_ref() == Some(rejection) {
            return Ok(current);
        }
        return Err(HubNodeRailError::TerminalConflict);
    }
    if current.lease_owner.as_deref() != Some(connection_id)
        || current.status != HubNodeRunStatus::Leased
        || current.effect_state != super::HubNodeEffectState::NotStarted
    {
        return Err(HubNodeRailError::LeaseConflict);
    }
    let rejection_json = serde_json::to_string(rejection)
        .map_err(|error| HubNodeRailError::InvalidInput(error.to_string()))?;
    let rejection_sha256 = sha256_hex(rejection_json.as_bytes());
    let changed = tx.execute(
        "UPDATE hub_node_runs
         SET status = 'failed', effect_state = 'completed', lease_owner = NULL,
             lease_expires_at_ms = NULL, rejection_json = ?2,
             rejection_sha256 = ?3, error_code = ?4,
             terminal_at_ms = ?5, updated_at_ms = ?5
         WHERE run_id = ?1 AND status = 'leased' AND effect_state = 'not_started'",
        params![
            rejection.run_id,
            rejection_json,
            rejection_sha256,
            rejection.code,
            now_ms,
        ],
    )?;
    if changed != 1 {
        return Err(HubNodeRailError::LeaseConflict);
    }
    run_by_id(tx, &rejection.run_id)?.ok_or(HubNodeRailError::RunNotFound)
}

#[allow(clippy::too_many_arguments)]
fn record_progress_in_tx(
    tx: &Transaction<'_>,
    device_id: &str,
    connection_id: &str,
    run_id: &str,
    attempt: u32,
    progress_sequence: u64,
    message: &str,
    lease_expires_at_ms: i64,
    now_ms: i64,
) -> Result<HubNodeRunRecord, HubNodeRailError> {
    let current = run_by_id(tx, run_id)?.ok_or(HubNodeRailError::RunNotFound)?;
    if current.device_id != device_id
        || current.attempt != attempt
        || current.lease_owner.as_deref() != Some(connection_id)
        || current.status != HubNodeRunStatus::Accepted
        || current
            .lease_expires_at_ms
            .is_none_or(|expiry| expiry <= now_ms)
    {
        return Err(HubNodeRailError::LeaseConflict);
    }
    if progress_sequence <= current.progress_sequence {
        return Err(HubNodeRailError::ReplayConflict);
    }
    tx.execute(
        "UPDATE hub_node_runs
         SET progress_sequence = ?2, progress_message = ?3,
             lease_expires_at_ms = ?4, updated_at_ms = ?5
         WHERE run_id = ?1",
        params![
            run_id,
            progress_sequence,
            message,
            lease_expires_at_ms,
            now_ms,
        ],
    )?;
    run_by_id(tx, run_id)?.ok_or(HubNodeRailError::RunNotFound)
}

fn complete_run_in_tx(
    tx: &Transaction<'_>,
    device_id: &str,
    completion: &RunCompletion,
    now_ms: i64,
) -> Result<HubNodeRunRecord, HubNodeRailError> {
    let current = run_by_id(tx, &completion.run_id)?.ok_or(HubNodeRailError::RunNotFound)?;
    if current.device_id != device_id || current.attempt != completion.attempt {
        return Err(HubNodeRailError::LeaseConflict);
    }
    if current.status.is_terminal() {
        if current.completion.as_ref() == Some(completion) {
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
    let completion_json = serde_json::to_string(completion)
        .map_err(|error| HubNodeRailError::InvalidInput(error.to_string()))?;
    let completion_sha256 = sha256_hex(completion_json.as_bytes());
    tx.execute(
        "UPDATE hub_node_runs
         SET status = ?2, effect_state = 'completed', lease_owner = NULL,
             lease_expires_at_ms = NULL, completion_json = ?3,
             completion_sha256 = ?4, terminal_at_ms = ?5, updated_at_ms = ?5
         WHERE run_id = ?1",
        params![
            completion.run_id,
            terminal_status_str(completion.status),
            completion_json,
            completion_sha256,
            now_ms,
        ],
    )?;
    run_by_id(tx, &completion.run_id)?.ok_or(HubNodeRailError::RunNotFound)
}

fn renew_heartbeat_runs(
    tx: &Transaction<'_>,
    device_id: &str,
    connection_id: &str,
    active_run_ids: &[String],
    lease_expires_at_ms: i64,
    now_ms: i64,
) -> Result<usize, HubNodeRailError> {
    let mut renewed = 0;
    for run_id in active_run_ids {
        let changed = tx.execute(
            "UPDATE hub_node_runs
             SET lease_expires_at_ms = ?5, updated_at_ms = ?6
             WHERE device_id = ?1 AND run_id = ?2
               AND lease_owner = ?3
               AND (
                   status IN ('accepted', 'cancel_requested')
                   OR (
                       status = 'leased'
                       AND EXISTS (
                           SELECT 1 FROM hub_node_run_approvals approval
                           WHERE approval.run_id = hub_node_runs.run_id
                             AND approval.attempt = hub_node_runs.attempt
                             AND approval.status = 'pending'
                             AND approval.expires_at_ms > ?4
                       )
                   )
               )
               AND lease_expires_at_ms > ?4",
            params![
                device_id,
                run_id,
                connection_id,
                now_ms,
                lease_expires_at_ms,
                now_ms,
            ],
        )?;
        if changed != 1 {
            return Err(HubNodeRailError::LeaseConflict);
        }
        renewed += 1;
    }
    Ok(renewed)
}

fn touch_presence(
    tx: &Transaction<'_>,
    device_id: &str,
    connection_id: &str,
    now_ms: i64,
) -> Result<(), HubNodeRailError> {
    let changed = tx.execute(
        "UPDATE hub_node_connections
         SET last_seen_ms = MAX(last_seen_ms, ?3),
             updated_at_ms = MAX(updated_at_ms, ?3), last_error_code = NULL
         WHERE device_id = ?1 AND connection_id = ?2 AND status = 'active'",
        params![device_id, connection_id, now_ms],
    )?;
    if changed != 1 {
        return Err(HubNodeRailError::ConnectionConflict);
    }
    tx.execute(
        "UPDATE captain_devices
         SET last_seen_ms = MAX(last_seen_ms, ?2),
             updated_at_ms = MAX(updated_at_ms, ?2), last_error_code = NULL
         WHERE device_id = ?1 AND status = 'active'",
        params![device_id, now_ms],
    )?;
    Ok(())
}

fn record_protocol_error(
    tx: &Transaction<'_>,
    device_id: &str,
    connection_id: &str,
    code: &str,
    now_ms: i64,
) -> Result<(), HubNodeRailError> {
    tx.execute(
        "UPDATE hub_node_connections
         SET last_error_code = ?3, updated_at_ms = MAX(updated_at_ms, ?4)
         WHERE device_id = ?1 AND connection_id = ?2 AND status = 'active'",
        params![device_id, connection_id, code, now_ms],
    )?;
    tx.execute(
        "UPDATE captain_devices
         SET last_error_code = ?2, updated_at_ms = MAX(updated_at_ms, ?3)
         WHERE device_id = ?1 AND status = 'active'",
        params![device_id, code, now_ms],
    )?;
    Ok(())
}

fn last_node_sequence(tx: &Transaction<'_>, device_id: &str) -> Result<u64, HubNodeRailError> {
    let sequence: i64 = tx.query_row(
        "SELECT last_node_sequence FROM hub_node_cursors WHERE device_id = ?1",
        [device_id],
        |row| row.get(0),
    )?;
    u64::try_from(sequence).map_err(|_| HubNodeRailError::StorageInvariant)
}

fn inbound_kind(message: &HubNodeMessage) -> &'static str {
    match message {
        HubNodeMessage::Heartbeat { .. } => "heartbeat",
        HubNodeMessage::RunAccepted { .. } => "run_accepted",
        HubNodeMessage::RunApprovalRequired(_) => "run_approval_required",
        HubNodeMessage::RunRejected(_) => "run_rejected",
        HubNodeMessage::RunProgress { .. } => "run_progress",
        HubNodeMessage::RunCompleted(_) => "run_completed",
        HubNodeMessage::AckOnly => "ack_only",
        HubNodeMessage::ProtocolError { .. } => "protocol_error",
        HubNodeMessage::Hello { .. }
        | HubNodeMessage::Welcome { .. }
        | HubNodeMessage::Superseded { .. }
        | HubNodeMessage::RunOffer(_)
        | HubNodeMessage::RunApprovalDecision(_)
        | HubNodeMessage::CancelRun { .. } => "invalid_direction",
    }
}
