use super::*;
use captain_types::approval::{is_valid_approval_action_digest, ApprovalDecision};
use captain_wire::hub_protocol::{RunApprovalDecision, RunRejection};

struct StoredApproval {
    approval_id: String,
    run_id: String,
    attempt: u32,
    action_digest: String,
    status: String,
    requested_at_ms: i64,
    expires_at_ms: i64,
}

pub(in crate::rail) fn apply_run_approval_decision(
    connection: &mut Connection,
    sequence: u64,
    applied_at_ms: i64,
) -> Result<NodeRunApprovalOutcome, NodeRailError> {
    validate_timestamp(applied_at_ms)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut meta = read_meta(&transaction)?;
    let inbound = runs::pending_inbound_for_sequence(&transaction, sequence)?;
    let HubNodeMessage::RunApprovalDecision(decision) = &inbound.envelope.message else {
        return Err(NodeRailError::RunDecisionConflict);
    };
    decision
        .validate()
        .map_err(|_| NodeRailError::RunDecisionConflict)?;
    let run = runs::get_run_in_tx(&transaction, &decision.run_id, decision.attempt)?
        .ok_or(NodeRailError::RunConflict)?;
    if run.status != NodeRunStatus::ApprovalPending
        || run.effect_started
        || run.approval_decision_inbound_sequence.is_some()
        || run
            .decision_outbound_sequence
            .is_none_or(|outbound| outbound > meta.acknowledged_node_sequence)
    {
        return Err(NodeRailError::RunDecisionConflict);
    }
    let approval = approval_for_run(&transaction, &decision.run_id, decision.attempt)?
        .ok_or(NodeRailError::StateCorrupt)?;
    validate_decision(&approval, decision)?;

    let decision_json = serde_json::to_vec(decision).map_err(|_| NodeRailError::InvalidMessage)?;
    let decision_sha256 = sha256_hex(&decision_json);
    let approval_status = approval_status(decision.decision);
    let changed = transaction.execute(
        "UPDATE node_run_approvals
         SET status = ?2, decision_json = ?3, decision_sha256 = ?4,
             decided_at_ms = ?5
         WHERE approval_id = ?1 AND status = 'pending'",
        params![
            decision.approval_id,
            approval_status,
            decision_json,
            decision_sha256,
            decision.decided_at_ms,
        ],
    )?;
    if changed != 1 {
        return Err(NodeRailError::RunDecisionConflict);
    }

    let approved_in_time = decision.decision.is_approved()
        && applied_at_ms < approval.expires_at_ms
        && applied_at_ms < run.lease.lease_expires_at_ms;
    let (outbound, expired_locally) = if approved_in_time {
        let message = HubNodeMessage::RunAccepted {
            run_id: decision.run_id.clone(),
            attempt: decision.attempt,
        };
        let outbound = append_next_outbox(&transaction, &mut meta, message, applied_at_ms)?;
        let changed = transaction.execute(
            "UPDATE node_runs
             SET status = 'accepted', approval_decision_inbound_sequence = ?3,
                 acceptance_outbound_sequence = ?4,
                 updated_at_ms = MAX(updated_at_ms, ?5)
             WHERE run_id = ?1 AND attempt = ?2 AND status = 'approval_pending'",
            params![
                decision.run_id,
                decision.attempt,
                u64_to_i64(sequence)?,
                u64_to_i64(outbound.sequence)?,
                applied_at_ms,
            ],
        )?;
        require_run_transition(changed)?;
        (Some(outbound), false)
    } else if decision.decision.is_approved() {
        let rejection = HubNodeMessage::RunRejected(RunRejection {
            run_id: decision.run_id.clone(),
            attempt: decision.attempt,
            code: "approval_expired".to_string(),
            message: "The local approval or execution lease expired before execution".to_string(),
            retryable: true,
            path_policy_applied: true,
        });
        let terminal_json =
            serde_json::to_vec(&rejection).map_err(|_| NodeRailError::InvalidMessage)?;
        let terminal_sha256 = sha256_hex(&terminal_json);
        let outbound = append_next_outbox(&transaction, &mut meta, rejection, applied_at_ms)?;
        let changed = transaction.execute(
            "UPDATE node_runs
             SET status = 'rejected', approval_decision_inbound_sequence = ?3,
                 terminal_outbound_sequence = ?4, terminal_json = ?5,
                 terminal_sha256 = ?6, terminal_at_ms = ?7,
                 updated_at_ms = MAX(updated_at_ms, ?7)
             WHERE run_id = ?1 AND attempt = ?2 AND status = 'approval_pending'",
            params![
                decision.run_id,
                decision.attempt,
                u64_to_i64(sequence)?,
                u64_to_i64(outbound.sequence)?,
                terminal_json,
                terminal_sha256,
                applied_at_ms,
            ],
        )?;
        require_run_transition(changed)?;
        (Some(outbound), true)
    } else {
        let terminal = HubNodeMessage::RunApprovalDecision(decision.clone());
        let terminal_json =
            serde_json::to_vec(&terminal).map_err(|_| NodeRailError::InvalidMessage)?;
        let terminal_sha256 = sha256_hex(&terminal_json);
        let changed = transaction.execute(
            "UPDATE node_runs
             SET status = 'cancelled', approval_decision_inbound_sequence = ?3,
                 terminal_json = ?4, terminal_sha256 = ?5,
                 terminal_at_ms = ?6, updated_at_ms = MAX(updated_at_ms, ?6)
             WHERE run_id = ?1 AND attempt = ?2 AND status = 'approval_pending'",
            params![
                decision.run_id,
                decision.attempt,
                u64_to_i64(sequence)?,
                terminal_json,
                terminal_sha256,
                applied_at_ms,
            ],
        )?;
        require_run_transition(changed)?;
        (None, false)
    };

    mark_inbound_applied_in_tx(&transaction, &mut meta, sequence, applied_at_ms)?;
    write_meta_cursors(&transaction, &meta, applied_at_ms)?;
    let run = runs::get_run_in_tx(&transaction, &decision.run_id, decision.attempt)?
        .ok_or(NodeRailError::StateCorrupt)?;
    transaction.commit()?;
    Ok(NodeRunApprovalOutcome {
        run,
        outbound,
        expired_locally,
    })
}

pub(in crate::rail) fn approved_action_digest(
    connection: &Connection,
    run_id: &str,
    attempt: u32,
) -> Result<Option<String>, NodeRailError> {
    let run =
        runs::get_run_in_tx(connection, run_id, attempt)?.ok_or(NodeRailError::RunClaimConflict)?;
    if run.status != NodeRunStatus::Accepted
        || run.effect_started
        || run.execution_claim_id.is_some()
        || run.execution_claim_started_at_ms.is_some()
        || run.cancel_inbound_sequence.is_some()
        || run.terminal_sha256.is_some()
    {
        return Err(NodeRailError::RunNotReady);
    }

    let approval = approval_for_run(connection, run_id, attempt)?;
    match approval {
        None if run.approval_decision_inbound_sequence.is_none() => Ok(None),
        Some(approval)
            if approval.status == "approved"
                && run.approval_decision_inbound_sequence.is_some()
                && is_valid_approval_action_digest(&approval.action_digest) =>
        {
            Ok(Some(approval.action_digest))
        }
        _ => Err(NodeRailError::StateCorrupt),
    }
}

fn approval_for_run(
    connection: &Connection,
    run_id: &str,
    attempt: u32,
) -> Result<Option<StoredApproval>, NodeRailError> {
    connection
        .query_row(
            "SELECT approval_id, run_id, attempt, action_digest, status,
                    requested_at_ms, expires_at_ms
             FROM node_run_approvals WHERE run_id = ?1 AND attempt = ?2",
            params![run_id, attempt],
            |row| {
                Ok(StoredApproval {
                    approval_id: row.get(0)?,
                    run_id: row.get(1)?,
                    attempt: row.get(2)?,
                    action_digest: row.get(3)?,
                    status: row.get(4)?,
                    requested_at_ms: row.get(5)?,
                    expires_at_ms: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

fn validate_decision(
    approval: &StoredApproval,
    decision: &RunApprovalDecision,
) -> Result<(), NodeRailError> {
    if approval.status != "pending"
        || approval.approval_id != decision.approval_id
        || approval.run_id != decision.run_id
        || approval.attempt != decision.attempt
        || approval.action_digest != decision.action_digest
        || decision.decided_at_ms < approval.requested_at_ms
        || (decision.decided_at_ms > approval.expires_at_ms
            && decision.decision != ApprovalDecision::TimedOut)
    {
        return Err(NodeRailError::RunDecisionConflict);
    }
    Ok(())
}

fn approval_status(decision: ApprovalDecision) -> &'static str {
    if decision.is_approved() {
        "approved"
    } else if decision == ApprovalDecision::TimedOut {
        "timed_out"
    } else {
        "denied"
    }
}

fn require_run_transition(changed: usize) -> Result<(), NodeRailError> {
    if changed == 1 {
        Ok(())
    } else {
        Err(NodeRailError::RunDecisionConflict)
    }
}
