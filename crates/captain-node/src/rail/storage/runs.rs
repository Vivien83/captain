use super::*;
use captain_types::approval::approval_action_digest;
use std::collections::{HashMap, HashSet};

const MAX_LOCAL_RUNS: i64 = 4_096;
const MAX_ACTIVE_RUNS: i64 = 256;

const RUN_SELECT: &str = "SELECT run_id, attempt, idempotency_key, workspace_id,
        tool_name, input_json, input_sha256, effect, lease_expires_at_ms,
        status, effect_started, inbound_sequence, decision_json,
        decision_sha256, decision_outbound_sequence, terminal_outbound_sequence,
        approval_decision_inbound_sequence, acceptance_outbound_sequence,
        cancel_inbound_sequence, cancel_json, cancel_sha256,
        execution_claim_id, execution_claim_started_at_ms,
        terminal_json, terminal_sha256, created_at_ms, updated_at_ms, terminal_at_ms
    FROM node_runs";

struct StoredRun {
    run_id: String,
    attempt: i64,
    idempotency_key: String,
    workspace_id: String,
    tool_name: String,
    input_json: Vec<u8>,
    input_sha256: String,
    effect: String,
    lease_expires_at_ms: i64,
    status: String,
    effect_started: i64,
    inbound_sequence: i64,
    decision_json: Vec<u8>,
    decision_sha256: String,
    decision_outbound_sequence: Option<i64>,
    terminal_outbound_sequence: Option<i64>,
    approval_decision_inbound_sequence: Option<i64>,
    acceptance_outbound_sequence: Option<i64>,
    cancel_inbound_sequence: Option<i64>,
    cancel_json: Option<Vec<u8>>,
    cancel_sha256: Option<String>,
    execution_claim_id: Option<String>,
    execution_claim_started_at_ms: Option<i64>,
    terminal_json: Option<Vec<u8>>,
    terminal_sha256: Option<String>,
    created_at_ms: i64,
    updated_at_ms: i64,
    terminal_at_ms: Option<i64>,
}

pub(in crate::rail) fn apply_run_offer(
    connection: &mut Connection,
    sequence: u64,
    disposition: &NodeRunDisposition,
    applied_at_ms: i64,
) -> Result<NodeRunIntakeOutcome, NodeRailError> {
    validate_timestamp(applied_at_ms)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut meta = read_meta(&transaction)?;
    let inbound = pending_inbound_for_sequence(&transaction, sequence)?;
    let HubNodeMessage::RunOffer(lease) = &inbound.envelope.message else {
        return Err(NodeRailError::RunDecisionConflict);
    };
    lease
        .validate()
        .map_err(|_| NodeRailError::InvalidMessage)?;
    if lease.lease_expires_at_ms <= applied_at_ms {
        return Err(NodeRailError::RunDecisionConflict);
    }
    let input_json = serde_json::to_vec(&lease.input).map_err(|_| NodeRailError::InvalidMessage)?;
    let input_sha256 = sha256_hex(&input_json);
    let decision = decision_message(lease, disposition, &input_json, applied_at_ms)?;
    let decision_json = serde_json::to_vec(&decision).map_err(|_| NodeRailError::InvalidMessage)?;
    let decision_sha256 = sha256_hex(&decision_json);

    if let Some(existing) = query_run(&transaction, &lease.run_id, lease.attempt)? {
        let stored = decode_run(existing)?;
        if !same_offer(&stored, lease, &input_sha256)
            || stored_decision(&transaction, &stored)? != decision
        {
            return Err(NodeRailError::RunConflict);
        }
        transaction.execute(
            "UPDATE node_runs
             SET lease_expires_at_ms = MAX(lease_expires_at_ms, ?3),
                 inbound_sequence = ?4, updated_at_ms = MAX(updated_at_ms, ?5)
             WHERE run_id = ?1 AND attempt = ?2",
            params![
                lease.run_id,
                lease.attempt,
                lease.lease_expires_at_ms,
                u64_to_i64(sequence)?,
                applied_at_ms,
            ],
        )?;
        mark_inbound_applied_in_tx(&transaction, &mut meta, sequence, applied_at_ms)?;
        write_meta_cursors(&transaction, &meta, applied_at_ms)?;
        let run = get_run_in_tx(&transaction, &lease.run_id, lease.attempt)?
            .ok_or(NodeRailError::StateCorrupt)?;
        let outbound = run
            .decision_outbound_sequence
            .map(|sequence| outbound_by_sequence(&transaction, sequence))
            .transpose()?
            .flatten();
        transaction.commit()?;
        return Ok(NodeRunIntakeOutcome {
            run,
            outbound,
            replayed: true,
        });
    }

    let conflicting_active_attempt = transaction.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM node_runs
             WHERE run_id = ?1 AND attempt <> ?2
               AND status NOT IN ('rejected', 'succeeded', 'failed', 'cancelled', 'uncertain')
         )",
        params![lease.run_id, lease.attempt],
        |row| row.get::<_, bool>(0),
    )?;
    if conflicting_active_attempt {
        return Err(NodeRailError::RunConflict);
    }
    if let Some(existing) = query_run_by_idempotency(&transaction, &lease.idempotency_key)? {
        let existing = decode_run(existing)?;
        if !same_idempotent_work(&existing, lease, &input_sha256) {
            return Err(NodeRailError::RunConflict);
        }
    }

    let run_count = transaction.query_row("SELECT COUNT(*) FROM node_runs", [], |row| {
        row.get::<_, i64>(0)
    })?;
    let active_count = transaction.query_row(
        "SELECT COUNT(*) FROM node_runs
         WHERE status NOT IN ('rejected', 'succeeded', 'failed', 'cancelled', 'uncertain')",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if run_count >= MAX_LOCAL_RUNS || active_count >= MAX_ACTIVE_RUNS {
        return Err(NodeRailError::RunConflict);
    }

    let (status, terminal_at_ms) = match disposition {
        NodeRunDisposition::Accept => ("accepted", None),
        NodeRunDisposition::RequireApproval(_) => ("approval_pending", None),
        NodeRunDisposition::Reject(_) => ("rejected", Some(applied_at_ms)),
    };
    let terminal_json =
        matches!(disposition, NodeRunDisposition::Reject(_)).then(|| decision_json.clone());
    let terminal_sha256 = terminal_json.as_ref().map(|_| decision_sha256.clone());
    transaction.execute(
        "INSERT INTO node_runs (
             run_id, attempt, idempotency_key, workspace_id, tool_name,
             input_json, input_sha256, effect, lease_expires_at_ms, status,
             inbound_sequence, decision_json, decision_sha256,
             terminal_json, terminal_sha256,
             created_at_ms, updated_at_ms, terminal_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                   ?11, ?12, ?13, ?14, ?15, ?16, ?16, ?17)",
        params![
            lease.run_id,
            lease.attempt,
            lease.idempotency_key,
            lease.workspace_id,
            lease.tool_name,
            input_json,
            input_sha256,
            effect_str(lease.effect),
            lease.lease_expires_at_ms,
            status,
            u64_to_i64(sequence)?,
            decision_json,
            decision_sha256,
            terminal_json,
            terminal_sha256,
            applied_at_ms,
            terminal_at_ms,
        ],
    )?;
    if let NodeRunDisposition::RequireApproval(request) = disposition {
        let request_json =
            serde_json::to_vec(request).map_err(|_| NodeRailError::InvalidMessage)?;
        let request_sha256 = sha256_hex(&request_json);
        transaction.execute(
            "INSERT INTO node_run_approvals (
                 approval_id, run_id, attempt, action_digest, request_json,
                 request_sha256, requested_at_ms, expires_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                request.approval_id,
                request.run_id,
                request.attempt,
                request.action_digest,
                request_json,
                request_sha256,
                applied_at_ms,
                request.expires_at_ms,
            ],
        )?;
    }
    let outbound = append_next_outbox(&transaction, &mut meta, decision, applied_at_ms)?;
    transaction.execute(
        "UPDATE node_runs
         SET decision_outbound_sequence = ?3,
             acceptance_outbound_sequence = CASE
                 WHEN status = 'accepted' THEN ?3
                 ELSE acceptance_outbound_sequence
             END,
             terminal_outbound_sequence = CASE
                 WHEN status = 'rejected' THEN ?3
                 ELSE terminal_outbound_sequence
             END
         WHERE run_id = ?1 AND attempt = ?2",
        params![lease.run_id, lease.attempt, u64_to_i64(outbound.sequence)?],
    )?;
    mark_inbound_applied_in_tx(&transaction, &mut meta, sequence, applied_at_ms)?;
    write_meta_cursors(&transaction, &meta, applied_at_ms)?;
    let run = get_run_in_tx(&transaction, &lease.run_id, lease.attempt)?
        .ok_or(NodeRailError::StateCorrupt)?;
    transaction.commit()?;
    Ok(NodeRunIntakeOutcome {
        run,
        outbound: Some(outbound),
        replayed: false,
    })
}

pub(in crate::rail) fn claimable_runs(
    connection: &Connection,
    limit: usize,
) -> Result<Vec<NodeRunRecord>, NodeRailError> {
    let mut statement = connection.prepare(&format!(
        "{RUN_SELECT}
         WHERE status = 'accepted' AND effect_started = 0
           AND execution_claim_id IS NULL
           AND execution_claim_started_at_ms IS NULL
           AND cancel_inbound_sequence IS NULL AND terminal_json IS NULL
         ORDER BY created_at_ms, run_id, attempt
         LIMIT ?1"
    ))?;
    let rows = statement
        .query_map([page_limit(limit)], stored_run_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter().map(decode_run).collect()
}

pub(in crate::rail) fn reject_run_before_effect(
    connection: &mut Connection,
    run_id: &str,
    attempt: u32,
    rejection: &RunRejection,
    rejected_at_ms: i64,
) -> Result<NodeRunPreflightRejectionOutcome, NodeRailError> {
    validate_timestamp(rejected_at_ms)?;
    rejection
        .validate()
        .map_err(|_| NodeRailError::RunDecisionConflict)?;
    if rejection.run_id != run_id || rejection.attempt != attempt {
        return Err(NodeRailError::RunDecisionConflict);
    }

    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut meta = read_meta(&transaction)?;
    let run =
        get_run_in_tx(&transaction, run_id, attempt)?.ok_or(NodeRailError::RunClaimConflict)?;
    if run.status != NodeRunStatus::Accepted
        || run.effect_started
        || run.execution_claim_id.is_some()
        || run.execution_claim_started_at_ms.is_some()
        || run.cancel_inbound_sequence.is_some()
        || run.terminal_sha256.is_some()
        || super::execution::pending_cancellation_exists(&transaction, run_id, attempt)?
    {
        return Err(NodeRailError::RunClaimConflict);
    }

    let terminal = HubNodeMessage::RunRejected(rejection.clone());
    let terminal_json = serde_json::to_vec(&terminal).map_err(|_| NodeRailError::InvalidMessage)?;
    let terminal_sha256 = sha256_hex(&terminal_json);
    let outbound = append_next_outbox(&transaction, &mut meta, terminal, rejected_at_ms)?;
    let changed = transaction.execute(
        "UPDATE node_runs
         SET status = 'rejected', terminal_outbound_sequence = ?3,
             terminal_json = ?4, terminal_sha256 = ?5,
             terminal_at_ms = ?6, updated_at_ms = MAX(updated_at_ms, ?6)
         WHERE run_id = ?1 AND attempt = ?2 AND status = 'accepted'
           AND effect_started = 0 AND execution_claim_id IS NULL
           AND execution_claim_started_at_ms IS NULL
           AND cancel_inbound_sequence IS NULL AND terminal_json IS NULL",
        params![
            run_id,
            attempt,
            u64_to_i64(outbound.sequence)?,
            terminal_json,
            terminal_sha256,
            rejected_at_ms,
        ],
    )?;
    require_run_transition(changed)?;
    write_meta_cursors(&transaction, &meta, rejected_at_ms)?;
    let run = get_run_in_tx(&transaction, run_id, attempt)?.ok_or(NodeRailError::StateCorrupt)?;
    transaction.commit()?;
    Ok(NodeRunPreflightRejectionOutcome { run, outbound })
}

fn require_run_transition(changed: usize) -> Result<(), NodeRailError> {
    if changed == 1 {
        Ok(())
    } else {
        Err(NodeRailError::RunClaimConflict)
    }
}

pub(in crate::rail) fn get_run(
    connection: &Connection,
    run_id: &str,
    attempt: u32,
) -> Result<Option<NodeRunRecord>, NodeRailError> {
    if attempt == 0 {
        return Err(NodeRailError::RunConflict);
    }
    get_run_in_tx(connection, run_id, attempt)
}

pub(in crate::rail) fn active_run_ids(
    connection: &Connection,
) -> Result<Vec<String>, NodeRailError> {
    let mut statement = connection.prepare(
        "SELECT DISTINCT run_id FROM node_runs
         WHERE status IN ('approval_pending', 'accepted', 'running', 'cancel_requested')
         ORDER BY run_id LIMIT 257",
    )?;
    let runs = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if runs.len() > MAX_ACTIVE_RUNS as usize {
        return Err(NodeRailError::StateCorrupt);
    }
    Ok(runs)
}

pub(super) fn verify_run_invariants(connection: &Connection) -> Result<(), NodeRailError> {
    let meta = read_meta(connection)?;
    let mut statement = connection.prepare(&format!("{RUN_SELECT} ORDER BY run_id, attempt"))?;
    let rows = statement.query_map([], stored_run_from_row)?;
    let mut run_count = 0_i64;
    let mut active_count = 0_i64;
    let mut active_run_ids = HashSet::new();
    let mut idempotent_work = HashMap::new();
    for row in rows {
        let stored = row?;
        let run = decode_run(stored)?;
        run_count += 1;
        if run_count > MAX_LOCAL_RUNS {
            return Err(NodeRailError::StateCorrupt);
        }
        if !run.status.is_terminal() {
            active_count += 1;
            if active_count > MAX_ACTIVE_RUNS || !active_run_ids.insert(run.lease.run_id.clone()) {
                return Err(NodeRailError::StateCorrupt);
            }
        }
        let fingerprint = (
            run.lease.run_id.clone(),
            run.lease.workspace_id.clone(),
            run.lease.tool_name.clone(),
            run.lease.effect,
            run.input_sha256.clone(),
        );
        if idempotent_work
            .insert(run.lease.idempotency_key.clone(), fingerprint.clone())
            .is_some_and(|stored| stored != fingerprint)
        {
            return Err(NodeRailError::StateCorrupt);
        }

        let decision = stored_decision(connection, &run)?;
        let decision_sequence = run
            .decision_outbound_sequence
            .ok_or(NodeRailError::StateCorrupt)?;
        verify_outbound_evidence(connection, &meta, decision_sequence, &decision)?;
        if let Some(sequence) = run.acceptance_outbound_sequence {
            verify_outbound_evidence(
                connection,
                &meta,
                sequence,
                &HubNodeMessage::RunAccepted {
                    run_id: run.lease.run_id.clone(),
                    attempt: run.lease.attempt,
                },
            )?;
        }
        let approval_decision = stored_approval_decision(connection, &run)?;
        if let Some(sequence) = run.approval_decision_inbound_sequence {
            let decision = approval_decision
                .as_ref()
                .ok_or(NodeRailError::StateCorrupt)?;
            verify_inbound_evidence(
                connection,
                &meta,
                sequence,
                &HubNodeMessage::RunApprovalDecision(decision.clone()),
            )?;
        } else if approval_decision.is_some() {
            return Err(NodeRailError::StateCorrupt);
        }
        let cancel = stored_cancel(connection, &run)?;
        if let Some(sequence) = run.cancel_inbound_sequence {
            verify_inbound_evidence(
                connection,
                &meta,
                sequence,
                cancel.as_ref().ok_or(NodeRailError::StateCorrupt)?,
            )?;
        } else if cancel.is_some() {
            return Err(NodeRailError::StateCorrupt);
        }
        execution_claims::verify_run_claim_state(connection, &run)?;
        verify_run_decision_state(connection, &run, &decision)?;
        let terminal = stored_terminal(connection, &run)?;
        if run.status.is_terminal() != terminal.is_some() {
            return Err(NodeRailError::StateCorrupt);
        }
        if let Some(sequence) = run.terminal_outbound_sequence {
            if sequence < decision_sequence || sequence > meta.last_node_sequence {
                return Err(NodeRailError::StateCorrupt);
            }
            verify_outbound_evidence(
                connection,
                &meta,
                sequence,
                terminal.as_ref().ok_or(NodeRailError::StateCorrupt)?,
            )?;
        } else if terminal.is_some() && run.approval_decision_inbound_sequence.is_none() {
            return Err(NodeRailError::StateCorrupt);
        }
    }
    verify_approval_invariants(connection)?;
    execution_claims::verify_claim_table(connection)?;
    Ok(())
}

fn verify_inbound_evidence(
    connection: &Connection,
    meta: &RailMeta,
    sequence: u64,
    expected: &HubNodeMessage,
) -> Result<(), NodeRailError> {
    if sequence == 0 || sequence > meta.last_hub_sequence {
        return Err(NodeRailError::StateCorrupt);
    }
    let stored = connection
        .query_row(
            "SELECT message_kind, envelope_json, envelope_sha256, applied_at_ms
             FROM node_rail_inbox WHERE sequence = ?1",
            [u64_to_i64(sequence)?],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            },
        )
        .optional()?;
    if sequence <= meta.pruned_hub_sequence {
        if stored.is_some() {
            return Err(NodeRailError::StateCorrupt);
        }
    } else {
        let Some((kind, raw, digest, applied_at_ms)) = stored else {
            return Err(NodeRailError::StateCorrupt);
        };
        let envelope = decode_envelope(&raw, &digest, sequence, &kind)?;
        if applied_at_ms.is_none() || &envelope.message != expected {
            return Err(NodeRailError::StateCorrupt);
        }
    }
    Ok(())
}

fn verify_outbound_evidence(
    connection: &Connection,
    meta: &RailMeta,
    sequence: u64,
    decision: &HubNodeMessage,
) -> Result<(), NodeRailError> {
    if sequence == 0 || sequence > meta.last_node_sequence {
        return Err(NodeRailError::StateCorrupt);
    }
    let outbound = outbound_by_sequence(connection, sequence)?;
    if sequence > meta.acknowledged_node_sequence {
        if outbound.as_ref().map(|envelope| &envelope.message) != Some(decision) {
            return Err(NodeRailError::StateCorrupt);
        }
    } else if outbound.is_some() {
        return Err(NodeRailError::StateCorrupt);
    }
    Ok(())
}

fn verify_run_decision_state(
    connection: &Connection,
    run: &NodeRunRecord,
    decision: &HubNodeMessage,
) -> Result<(), NodeRailError> {
    let approval = connection
        .query_row(
            "SELECT status, request_json FROM node_run_approvals
             WHERE run_id = ?1 AND attempt = ?2",
            params![run.lease.run_id, run.lease.attempt],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .optional()?;
    match decision {
        HubNodeMessage::RunApprovalRequired(request) => {
            let Some((status, request_json)) = approval else {
                return Err(NodeRailError::StateCorrupt);
            };
            let stored_request = serde_json::from_slice::<RunApprovalRequest>(&request_json)
                .map_err(|_| NodeRailError::StateCorrupt)?;
            if stored_request != *request {
                return Err(NodeRailError::StateCorrupt);
            }
            match status.as_str() {
                "pending"
                    if !run.effect_started
                        && run.approval_decision_inbound_sequence.is_none()
                        && run.acceptance_outbound_sequence.is_none()
                        && ((run.status == NodeRunStatus::ApprovalPending
                            && run.cancel_inbound_sequence.is_none()
                            && run.terminal_sha256.is_none())
                            || (run.status == NodeRunStatus::Cancelled
                                && run.cancel_inbound_sequence.is_some()
                                && run.terminal_outbound_sequence.is_some()
                                && run.terminal_sha256.is_some())) => {}
                "approved"
                    if run.approval_decision_inbound_sequence.is_some()
                        && ((run.status == NodeRunStatus::Accepted
                            && run.acceptance_outbound_sequence.is_some()
                            && run.terminal_sha256.is_none())
                            || (matches!(
                                run.status,
                                NodeRunStatus::Running | NodeRunStatus::CancelRequested
                            ) && run.effect_started
                                && run.acceptance_outbound_sequence.is_some()
                                && run.terminal_sha256.is_none())
                            || (matches!(
                                run.status,
                                NodeRunStatus::Succeeded
                                    | NodeRunStatus::Failed
                                    | NodeRunStatus::Cancelled
                                    | NodeRunStatus::Uncertain
                            ) && run.acceptance_outbound_sequence.is_some()
                                && run.terminal_outbound_sequence.is_some()
                                && run.terminal_sha256.is_some())
                            || (run.status == NodeRunStatus::Rejected
                                && !run.effect_started
                                && run.terminal_outbound_sequence.is_some())) => {}
                "denied" | "timed_out"
                    if run.status == NodeRunStatus::Cancelled
                        && !run.effect_started
                        && run.approval_decision_inbound_sequence.is_some()
                        && run.acceptance_outbound_sequence.is_none()
                        && run.terminal_outbound_sequence.is_none()
                        && run.terminal_sha256.is_some() => {}
                _ => return Err(NodeRailError::StateCorrupt),
            }
        }
        HubNodeMessage::RunAccepted { .. } => {
            let rejected_before_effect = run.status == NodeRunStatus::Rejected
                && !run.effect_started
                && run.terminal_outbound_sequence.is_some()
                && run.terminal_sha256.is_some();
            if approval.is_some()
                || run.approval_decision_inbound_sequence.is_some()
                || run.acceptance_outbound_sequence != run.decision_outbound_sequence
                || run.status == NodeRunStatus::ApprovalPending
                || (run.status == NodeRunStatus::Rejected && !rejected_before_effect)
            {
                return Err(NodeRailError::StateCorrupt);
            }
        }
        HubNodeMessage::RunRejected(_) => {
            if approval.is_some()
                || run.status != NodeRunStatus::Rejected
                || run.effect_started
                || run.terminal_outbound_sequence != run.decision_outbound_sequence
                || run.acceptance_outbound_sequence.is_some()
                || run.approval_decision_inbound_sequence.is_some()
            {
                return Err(NodeRailError::StateCorrupt);
            }
        }
        _ => return Err(NodeRailError::StateCorrupt),
    }
    if run.status == NodeRunStatus::CancelRequested {
        if !run.effect_started
            || run.cancel_inbound_sequence.is_none()
            || run.terminal_sha256.is_some()
        {
            return Err(NodeRailError::StateCorrupt);
        }
    } else if run.cancel_inbound_sequence.is_some() && !run.status.is_terminal() {
        return Err(NodeRailError::StateCorrupt);
    }
    Ok(())
}

fn verify_approval_invariants(connection: &Connection) -> Result<(), NodeRailError> {
    let mut statement = connection.prepare(
        "SELECT approval_id, run_id, attempt, action_digest,
                request_json, request_sha256, status,
                decision_json, decision_sha256,
                requested_at_ms, expires_at_ms, decided_at_ms
         FROM node_run_approvals ORDER BY approval_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Vec<u8>>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, Option<Vec<u8>>>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, i64>(9)?,
            row.get::<_, i64>(10)?,
            row.get::<_, Option<i64>>(11)?,
        ))
    })?;
    for row in rows {
        let (
            approval_id,
            run_id,
            attempt,
            action_digest,
            request_json,
            request_sha256,
            status,
            decision_json,
            decision_sha256,
            requested_at_ms,
            expires_at_ms,
            decided_at_ms,
        ) = row?;
        if sha256_hex(&request_json) != request_sha256
            || requested_at_ms <= 0
            || expires_at_ms <= requested_at_ms
        {
            return Err(NodeRailError::StateCorrupt);
        }
        let request =
            serde_json::from_slice::<captain_wire::hub_protocol::RunApprovalRequest>(&request_json)
                .map_err(|_| NodeRailError::StateCorrupt)?;
        request
            .validate()
            .map_err(|_| NodeRailError::StateCorrupt)?;
        if request.approval_id != approval_id
            || request.run_id != run_id
            || i64::from(request.attempt) != attempt
            || request.action_digest != action_digest
            || request.expires_at_ms != expires_at_ms
        {
            return Err(NodeRailError::StateCorrupt);
        }
        match (
            status.as_str(),
            decision_json,
            decision_sha256,
            decided_at_ms,
        ) {
            ("pending", None, None, None) => {}
            ("approved" | "denied" | "timed_out", Some(raw), Some(digest), Some(decided_at))
                if sha256_hex(&raw) == digest =>
            {
                let decision =
                    serde_json::from_slice::<captain_wire::hub_protocol::RunApprovalDecision>(&raw)
                        .map_err(|_| NodeRailError::StateCorrupt)?;
                decision
                    .validate()
                    .map_err(|_| NodeRailError::StateCorrupt)?;
                if decision.approval_id != approval_id
                    || decision.run_id != run_id
                    || i64::from(decision.attempt) != attempt
                    || decision.action_digest != action_digest
                    || decision.decided_at_ms != decided_at
                    || decision.decided_at_ms < requested_at_ms
                    || (decision.decided_at_ms > expires_at_ms
                        && decision.decision != captain_types::approval::ApprovalDecision::TimedOut)
                    || (decision.decision.is_approved()) != (status == "approved")
                    || (decision.decision == captain_types::approval::ApprovalDecision::TimedOut)
                        != (status == "timed_out")
                {
                    return Err(NodeRailError::StateCorrupt);
                }
            }
            _ => return Err(NodeRailError::StateCorrupt),
        }
    }
    Ok(())
}

pub(super) fn pending_inbound_for_sequence(
    transaction: &Transaction<'_>,
    sequence: u64,
) -> Result<NodeInboundRecord, NodeRailError> {
    let oldest = transaction.query_row(
        "SELECT MIN(sequence) FROM node_rail_inbox WHERE applied_at_ms IS NULL",
        [],
        |row| row.get::<_, Option<i64>>(0),
    )?;
    if oldest != Some(u64_to_i64(sequence)?) {
        return Err(NodeRailError::ApplyOrderConflict);
    }
    let stored = transaction
        .query_row(
            "SELECT message_kind, envelope_json, envelope_sha256, received_at_ms
             FROM node_rail_inbox WHERE sequence = ?1 AND applied_at_ms IS NULL",
            [u64_to_i64(sequence)?],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?
        .ok_or(NodeRailError::ApplyOrderConflict)?;
    Ok(NodeInboundRecord {
        envelope: decode_envelope(&stored.1, &stored.2, sequence, &stored.0)?,
        received_at_ms: stored.3,
    })
}

fn decision_message(
    lease: &RunLease,
    disposition: &NodeRunDisposition,
    input_json: &[u8],
    applied_at_ms: i64,
) -> Result<HubNodeMessage, NodeRailError> {
    match disposition {
        NodeRunDisposition::Accept => Ok(HubNodeMessage::RunAccepted {
            run_id: lease.run_id.clone(),
            attempt: lease.attempt,
        }),
        NodeRunDisposition::RequireApproval(request) => {
            request
                .validate()
                .map_err(|_| NodeRailError::RunDecisionConflict)?;
            if request.run_id != lease.run_id
                || request.attempt != lease.attempt
                || request.action_digest != approval_action_digest(&lease.tool_name, input_json)
                || request.expires_at_ms <= applied_at_ms
                || request.expires_at_ms > lease.lease_expires_at_ms
            {
                return Err(NodeRailError::RunDecisionConflict);
            }
            Ok(HubNodeMessage::RunApprovalRequired(request.clone()))
        }
        NodeRunDisposition::Reject(rejection) => {
            rejection
                .validate()
                .map_err(|_| NodeRailError::RunDecisionConflict)?;
            if rejection.run_id != lease.run_id || rejection.attempt != lease.attempt {
                return Err(NodeRailError::RunDecisionConflict);
            }
            Ok(HubNodeMessage::RunRejected(rejection.clone()))
        }
    }
}

pub(super) fn get_run_in_tx(
    connection: &Connection,
    run_id: &str,
    attempt: u32,
) -> Result<Option<NodeRunRecord>, NodeRailError> {
    query_run(connection, run_id, attempt)?
        .map(decode_run)
        .transpose()
}

fn query_run(
    connection: &Connection,
    run_id: &str,
    attempt: u32,
) -> Result<Option<StoredRun>, NodeRailError> {
    connection
        .query_row(
            &format!("{RUN_SELECT} WHERE run_id = ?1 AND attempt = ?2"),
            params![run_id, attempt],
            stored_run_from_row,
        )
        .optional()
        .map_err(Into::into)
}

fn query_run_by_idempotency(
    connection: &Connection,
    idempotency_key: &str,
) -> Result<Option<StoredRun>, NodeRailError> {
    connection
        .query_row(
            &format!("{RUN_SELECT} WHERE idempotency_key = ?1 ORDER BY attempt DESC LIMIT 1"),
            [idempotency_key],
            stored_run_from_row,
        )
        .optional()
        .map_err(Into::into)
}

fn stored_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredRun> {
    Ok(StoredRun {
        run_id: row.get(0)?,
        attempt: row.get(1)?,
        idempotency_key: row.get(2)?,
        workspace_id: row.get(3)?,
        tool_name: row.get(4)?,
        input_json: row.get(5)?,
        input_sha256: row.get(6)?,
        effect: row.get(7)?,
        lease_expires_at_ms: row.get(8)?,
        status: row.get(9)?,
        effect_started: row.get(10)?,
        inbound_sequence: row.get(11)?,
        decision_json: row.get(12)?,
        decision_sha256: row.get(13)?,
        decision_outbound_sequence: row.get(14)?,
        terminal_outbound_sequence: row.get(15)?,
        approval_decision_inbound_sequence: row.get(16)?,
        acceptance_outbound_sequence: row.get(17)?,
        cancel_inbound_sequence: row.get(18)?,
        cancel_json: row.get(19)?,
        cancel_sha256: row.get(20)?,
        execution_claim_id: row.get(21)?,
        execution_claim_started_at_ms: row.get(22)?,
        terminal_json: row.get(23)?,
        terminal_sha256: row.get(24)?,
        created_at_ms: row.get(25)?,
        updated_at_ms: row.get(26)?,
        terminal_at_ms: row.get(27)?,
    })
}

fn decode_run(stored: StoredRun) -> Result<NodeRunRecord, NodeRailError> {
    if sha256_hex(&stored.input_json) != stored.input_sha256
        || sha256_hex(&stored.decision_json) != stored.decision_sha256
        || (stored.cancel_json.is_some() != stored.cancel_sha256.is_some())
        || stored
            .cancel_json
            .as_ref()
            .zip(stored.cancel_sha256.as_ref())
            .is_some_and(|(raw, digest)| sha256_hex(raw) != *digest)
        || (stored.execution_claim_id.is_some() != stored.execution_claim_started_at_ms.is_some())
        || (stored.terminal_json.is_some() != stored.terminal_sha256.is_some())
        || stored
            .terminal_json
            .as_ref()
            .zip(stored.terminal_sha256.as_ref())
            .is_some_and(|(raw, digest)| sha256_hex(raw) != *digest)
    {
        return Err(NodeRailError::StateCorrupt);
    }
    let input =
        serde_json::from_slice(&stored.input_json).map_err(|_| NodeRailError::StateCorrupt)?;
    let lease = RunLease {
        run_id: stored.run_id,
        attempt: u32::try_from(stored.attempt).map_err(|_| NodeRailError::StateCorrupt)?,
        idempotency_key: stored.idempotency_key,
        workspace_id: stored.workspace_id,
        tool_name: stored.tool_name,
        input,
        effect: parse_effect(&stored.effect).ok_or(NodeRailError::StateCorrupt)?,
        lease_expires_at_ms: stored.lease_expires_at_ms,
    };
    lease.validate().map_err(|_| NodeRailError::StateCorrupt)?;
    let status = parse_status(&stored.status).ok_or(NodeRailError::StateCorrupt)?;
    let terminal = status.is_terminal();
    if !matches!(stored.effect_started, 0 | 1)
        || terminal != stored.terminal_at_ms.is_some()
        || stored.updated_at_ms < stored.created_at_ms
    {
        return Err(NodeRailError::StateCorrupt);
    }
    Ok(NodeRunRecord {
        lease,
        input_sha256: stored.input_sha256,
        status,
        effect_started: stored.effect_started == 1,
        inbound_sequence: i64_to_u64(stored.inbound_sequence)?,
        decision_outbound_sequence: stored
            .decision_outbound_sequence
            .map(i64_to_u64)
            .transpose()?,
        approval_decision_inbound_sequence: stored
            .approval_decision_inbound_sequence
            .map(i64_to_u64)
            .transpose()?,
        acceptance_outbound_sequence: stored
            .acceptance_outbound_sequence
            .map(i64_to_u64)
            .transpose()?,
        cancel_inbound_sequence: stored.cancel_inbound_sequence.map(i64_to_u64).transpose()?,
        cancel_sha256: stored.cancel_sha256,
        execution_claim_id: stored.execution_claim_id,
        execution_claim_started_at_ms: stored.execution_claim_started_at_ms,
        terminal_outbound_sequence: stored
            .terminal_outbound_sequence
            .map(i64_to_u64)
            .transpose()?,
        terminal_sha256: stored.terminal_sha256,
        created_at_ms: stored.created_at_ms,
        updated_at_ms: stored.updated_at_ms,
        terminal_at_ms: stored.terminal_at_ms,
    })
}

fn stored_decision(
    connection: &Connection,
    run: &NodeRunRecord,
) -> Result<HubNodeMessage, NodeRailError> {
    let raw = connection.query_row(
        "SELECT decision_json FROM node_runs WHERE run_id = ?1 AND attempt = ?2",
        params![run.lease.run_id, run.lease.attempt],
        |row| row.get::<_, Vec<u8>>(0),
    )?;
    let message =
        serde_json::from_slice::<HubNodeMessage>(&raw).map_err(|_| NodeRailError::StateCorrupt)?;
    let matches_run = match &message {
        HubNodeMessage::RunAccepted { run_id, attempt } => {
            run_id == &run.lease.run_id && *attempt == run.lease.attempt
        }
        HubNodeMessage::RunApprovalRequired(request) => {
            request.validate().is_ok()
                && request.run_id == run.lease.run_id
                && request.attempt == run.lease.attempt
        }
        HubNodeMessage::RunRejected(rejection) => {
            rejection.validate().is_ok()
                && rejection.run_id == run.lease.run_id
                && rejection.attempt == run.lease.attempt
        }
        _ => false,
    };
    if !matches_run {
        return Err(NodeRailError::StateCorrupt);
    }
    Ok(message)
}

fn stored_approval_decision(
    connection: &Connection,
    run: &NodeRunRecord,
) -> Result<Option<captain_wire::hub_protocol::RunApprovalDecision>, NodeRailError> {
    let raw = connection
        .query_row(
            "SELECT decision_json FROM node_run_approvals
             WHERE run_id = ?1 AND attempt = ?2",
            params![run.lease.run_id, run.lease.attempt],
            |row| row.get::<_, Option<Vec<u8>>>(0),
        )
        .optional()?
        .flatten();
    raw.map(|raw| serde_json::from_slice(&raw).map_err(|_| NodeRailError::StateCorrupt))
        .transpose()
}

fn stored_cancel(
    connection: &Connection,
    run: &NodeRunRecord,
) -> Result<Option<HubNodeMessage>, NodeRailError> {
    let raw = connection.query_row(
        "SELECT cancel_json FROM node_runs WHERE run_id = ?1 AND attempt = ?2",
        params![run.lease.run_id, run.lease.attempt],
        |row| row.get::<_, Option<Vec<u8>>>(0),
    )?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let message =
        serde_json::from_slice::<HubNodeMessage>(&raw).map_err(|_| NodeRailError::StateCorrupt)?;
    let contract = HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: "node".to_string(),
        connection_id: "connection".to_string(),
        sequence: 1,
        ack_sequence: None,
        sent_at_ms: 1,
        message: message.clone(),
    };
    if contract.validate().is_err()
        || !matches!(
            &message,
            HubNodeMessage::CancelRun { run_id, attempt, .. }
                if run_id == &run.lease.run_id && *attempt == run.lease.attempt
        )
    {
        return Err(NodeRailError::StateCorrupt);
    }
    Ok(Some(message))
}

fn stored_terminal(
    connection: &Connection,
    run: &NodeRunRecord,
) -> Result<Option<HubNodeMessage>, NodeRailError> {
    let raw = connection.query_row(
        "SELECT terminal_json FROM node_runs WHERE run_id = ?1 AND attempt = ?2",
        params![run.lease.run_id, run.lease.attempt],
        |row| row.get::<_, Option<Vec<u8>>>(0),
    )?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let message =
        serde_json::from_slice::<HubNodeMessage>(&raw).map_err(|_| NodeRailError::StateCorrupt)?;
    let matches_run = match &message {
        HubNodeMessage::RunRejected(rejection) => {
            rejection.validate().is_ok()
                && rejection.run_id == run.lease.run_id
                && rejection.attempt == run.lease.attempt
                && run.status == NodeRunStatus::Rejected
        }
        HubNodeMessage::RunApprovalDecision(decision) => {
            decision.validate().is_ok()
                && decision.run_id == run.lease.run_id
                && decision.attempt == run.lease.attempt
                && !decision.decision.is_approved()
                && run.status == NodeRunStatus::Cancelled
        }
        HubNodeMessage::RunCompleted(completion) => {
            completion.validate().is_ok()
                && completion.run_id == run.lease.run_id
                && completion.attempt == run.lease.attempt
                && terminal_status_matches(run.status, completion.status)
        }
        _ => false,
    };
    if !matches_run {
        return Err(NodeRailError::StateCorrupt);
    }
    Ok(Some(message))
}

fn terminal_status_matches(
    status: NodeRunStatus,
    terminal: captain_wire::hub_protocol::RunTerminalStatus,
) -> bool {
    matches!(
        (status, terminal),
        (
            NodeRunStatus::Succeeded,
            captain_wire::hub_protocol::RunTerminalStatus::Succeeded
        ) | (
            NodeRunStatus::Failed,
            captain_wire::hub_protocol::RunTerminalStatus::Failed
        ) | (
            NodeRunStatus::Cancelled,
            captain_wire::hub_protocol::RunTerminalStatus::Cancelled
        ) | (
            NodeRunStatus::Uncertain,
            captain_wire::hub_protocol::RunTerminalStatus::Uncertain
        )
    )
}

pub(super) fn outbound_by_sequence(
    connection: &Connection,
    sequence: u64,
) -> Result<Option<HubNodeEnvelope>, NodeRailError> {
    connection
        .query_row(
            "SELECT message_kind, envelope_json, envelope_sha256
             FROM node_rail_outbox WHERE sequence = ?1",
            [u64_to_i64(sequence)?],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?
        .map(|stored| decode_envelope(&stored.1, &stored.2, sequence, &stored.0))
        .transpose()
}

fn same_offer(run: &NodeRunRecord, lease: &RunLease, input_sha256: &str) -> bool {
    run.lease.run_id == lease.run_id
        && run.lease.attempt == lease.attempt
        && run.lease.idempotency_key == lease.idempotency_key
        && run.lease.workspace_id == lease.workspace_id
        && run.lease.tool_name == lease.tool_name
        && run.lease.effect == lease.effect
        && run.input_sha256 == input_sha256
}

fn same_idempotent_work(run: &NodeRunRecord, lease: &RunLease, input_sha256: &str) -> bool {
    run.lease.run_id == lease.run_id
        && run.lease.idempotency_key == lease.idempotency_key
        && run.lease.workspace_id == lease.workspace_id
        && run.lease.tool_name == lease.tool_name
        && run.lease.effect == lease.effect
        && run.input_sha256 == input_sha256
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

fn parse_status(value: &str) -> Option<NodeRunStatus> {
    Some(match value {
        "approval_pending" => NodeRunStatus::ApprovalPending,
        "accepted" => NodeRunStatus::Accepted,
        "running" => NodeRunStatus::Running,
        "cancel_requested" => NodeRunStatus::CancelRequested,
        "rejected" => NodeRunStatus::Rejected,
        "succeeded" => NodeRunStatus::Succeeded,
        "failed" => NodeRunStatus::Failed,
        "cancelled" => NodeRunStatus::Cancelled,
        "uncertain" => NodeRunStatus::Uncertain,
        _ => return None,
    })
}
