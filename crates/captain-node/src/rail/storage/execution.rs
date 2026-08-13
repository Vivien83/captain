use super::execution_claims::{
    stored_claim, validate_claim_id, ClaimStatus, MAX_CLAIMS_PER_RUN, MAX_LOCAL_RUN_CLAIMS,
};
use super::*;
use captain_wire::hub_protocol::{RunCompletion, RunTerminalStatus};

pub(in crate::rail) fn claim_run(
    connection: &mut Connection,
    run_id: &str,
    attempt: u32,
    claimed_at_ms: i64,
) -> Result<NodeRunClaimOutcome, NodeRailError> {
    validate_timestamp(claimed_at_ms)?;
    if attempt == 0 {
        return Err(NodeRailError::RunClaimConflict);
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let meta = read_meta(&transaction)?;
    let run = runs::get_run_in_tx(&transaction, run_id, attempt)?
        .ok_or(NodeRailError::RunClaimConflict)?;
    let acceptance_sequence = run
        .acceptance_outbound_sequence
        .ok_or(NodeRailError::RunNotReady)?;
    if run.status != NodeRunStatus::Accepted
        || run.effect_started
        || run.execution_claim_id.is_some()
        || run.execution_claim_started_at_ms.is_some()
        || run.cancel_inbound_sequence.is_some()
        || run.terminal_sha256.is_some()
    {
        return Err(NodeRailError::RunClaimConflict);
    }
    if pending_cancellation_exists(&transaction, run_id, attempt)? {
        return Err(NodeRailError::RunCancellationPending);
    }
    if acceptance_sequence > meta.acknowledged_node_sequence
        || run.lease.lease_expires_at_ms <= claimed_at_ms
        || claimed_at_ms < run.created_at_ms
    {
        return Err(NodeRailError::RunNotReady);
    }
    let total_claims =
        transaction.query_row("SELECT COUNT(*) FROM node_run_claims", [], |row| {
            row.get::<_, i64>(0)
        })?;
    let run_claims = transaction.query_row(
        "SELECT COUNT(*) FROM node_run_claims WHERE run_id = ?1 AND attempt = ?2",
        params![run_id, attempt],
        |row| row.get::<_, i64>(0),
    )?;
    if total_claims >= MAX_LOCAL_RUN_CLAIMS
        || run_claims >= i64::try_from(MAX_CLAIMS_PER_RUN).unwrap_or(i64::MAX)
    {
        return Err(NodeRailError::RunClaimConflict);
    }

    let claim_id = uuid::Uuid::new_v4().hyphenated().to_string();
    transaction.execute(
        "INSERT INTO node_run_claims (
             claim_id, run_id, attempt, status, started_at_ms
         ) VALUES (?1, ?2, ?3, 'claimed', ?4)",
        params![claim_id, run_id, attempt, claimed_at_ms],
    )?;
    let changed = transaction.execute(
        "UPDATE node_runs
         SET status = 'running', effect_started = 1,
             execution_claim_id = ?3, execution_claim_started_at_ms = ?4,
             updated_at_ms = MAX(updated_at_ms, ?4)
         WHERE run_id = ?1 AND attempt = ?2 AND status = 'accepted'
           AND effect_started = 0 AND execution_claim_id IS NULL
           AND execution_claim_started_at_ms IS NULL
           AND cancel_inbound_sequence IS NULL AND terminal_json IS NULL",
        params![run_id, attempt, claim_id, claimed_at_ms],
    )?;
    require_exact_transition(changed)?;
    let run =
        runs::get_run_in_tx(&transaction, run_id, attempt)?.ok_or(NodeRailError::StateCorrupt)?;
    transaction.commit()?;
    Ok(NodeRunClaimOutcome {
        run,
        claim_id,
        claimed_at_ms,
    })
}

pub(in crate::rail) fn cancellation_requested(
    connection: &Connection,
    claim_id: &str,
) -> Result<bool, NodeRailError> {
    validate_claim_id(claim_id).map_err(|_| NodeRailError::RunClaimConflict)?;
    let stored = connection
        .query_row(
            "SELECT claims.status, runs.execution_claim_id, runs.status
             FROM node_run_claims claims
             JOIN node_runs runs
               ON runs.run_id = claims.run_id AND runs.attempt = claims.attempt
             WHERE claims.claim_id = ?1",
            [claim_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?
        .ok_or(NodeRailError::RunClaimConflict)?;
    if stored.0 != "claimed" || stored.1.as_deref() != Some(claim_id) {
        return Err(NodeRailError::RunClaimConflict);
    }
    match stored.2.as_str() {
        "running" => Ok(false),
        "cancel_requested" => Ok(true),
        _ => Err(NodeRailError::RunClaimConflict),
    }
}

pub(in crate::rail) fn complete_run(
    connection: &mut Connection,
    claim_id: &str,
    completion: &RunCompletion,
    completed_at_ms: i64,
) -> Result<NodeRunCompletionOutcome, NodeRailError> {
    validate_timestamp(completed_at_ms)?;
    validate_claim_id(claim_id).map_err(|_| NodeRailError::RunClaimConflict)?;
    completion
        .validate()
        .map_err(|_| NodeRailError::InvalidMessage)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut meta = read_meta(&transaction)?;
    let run = runs::get_run_in_tx(&transaction, &completion.run_id, completion.attempt)?
        .ok_or(NodeRailError::RunClaimConflict)?;
    if run.execution_claim_id.as_deref() != Some(claim_id) {
        return Err(NodeRailError::RunClaimConflict);
    }
    let claim = stored_claim(&transaction, claim_id)?;
    let terminal_message = HubNodeMessage::RunCompleted(completion.clone());

    if run.status.is_terminal() {
        if claim.status != ClaimStatus::Completed
            || stored_terminal_message(&transaction, &run)? != Some(terminal_message)
        {
            return Err(NodeRailError::RunClaimConflict);
        }
        let outbound = run
            .terminal_outbound_sequence
            .map(|sequence| runs::outbound_by_sequence(&transaction, sequence))
            .transpose()?
            .flatten();
        transaction.commit()?;
        return Ok(NodeRunCompletionOutcome {
            run,
            outbound,
            replayed: true,
        });
    }
    if claim.status != ClaimStatus::Claimed
        || claim.run_id != completion.run_id
        || claim.attempt != completion.attempt
        || claim.started_at_ms != run.execution_claim_started_at_ms.unwrap_or_default()
        || completed_at_ms < claim.started_at_ms
        || !run.effect_started
        || !matches!(
            run.status,
            NodeRunStatus::Running | NodeRunStatus::CancelRequested
        )
        || run.terminal_sha256.is_some()
    {
        return Err(NodeRailError::RunClaimConflict);
    }

    let terminal_json =
        serde_json::to_vec(&terminal_message).map_err(|_| NodeRailError::InvalidMessage)?;
    let terminal_sha256 = sha256_hex(&terminal_json);
    let outbound = append_next_outbox(&transaction, &mut meta, terminal_message, completed_at_ms)?;
    let changed = transaction.execute(
        "UPDATE node_runs
         SET status = ?3, terminal_outbound_sequence = ?4,
             terminal_json = ?5, terminal_sha256 = ?6,
             terminal_at_ms = ?7, updated_at_ms = MAX(updated_at_ms, ?7)
         WHERE run_id = ?1 AND attempt = ?2
           AND status IN ('running', 'cancel_requested')
           AND effect_started = 1 AND execution_claim_id = ?8
           AND terminal_json IS NULL",
        params![
            completion.run_id,
            completion.attempt,
            terminal_status_str(completion.status),
            u64_to_i64(outbound.sequence)?,
            terminal_json,
            terminal_sha256,
            completed_at_ms,
            claim_id,
        ],
    )?;
    require_exact_transition(changed)?;
    let changed = transaction.execute(
        "UPDATE node_run_claims
         SET status = 'completed', finished_at_ms = ?2
         WHERE claim_id = ?1 AND status = 'claimed' AND finished_at_ms IS NULL",
        params![claim_id, completed_at_ms],
    )?;
    require_exact_transition(changed)?;
    write_meta_cursors(&transaction, &meta, completed_at_ms)?;
    let run = runs::get_run_in_tx(&transaction, &completion.run_id, completion.attempt)?
        .ok_or(NodeRailError::StateCorrupt)?;
    transaction.commit()?;
    Ok(NodeRunCompletionOutcome {
        run,
        outbound: Some(outbound),
        replayed: false,
    })
}

pub(super) fn recover_interrupted_claims(
    connection: &mut Connection,
    recovery_at_ms: i64,
) -> Result<(), NodeRailError> {
    validate_timestamp(recovery_at_ms)?;
    let interrupted = {
        let mut statement = connection.prepare(
            "SELECT run_id, attempt FROM node_runs
             WHERE status IN ('running', 'cancel_requested') AND effect_started = 1
             ORDER BY run_id, attempt LIMIT 257",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    if interrupted.is_empty() {
        return Ok(());
    }
    if interrupted.len() > 256 {
        return Err(NodeRailError::StateCorrupt);
    }

    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut meta = read_meta(&transaction)?;
    let mut persisted_at_ms = recovery_at_ms;
    for (run_id, attempt) in interrupted {
        let attempt = u32::try_from(attempt).map_err(|_| NodeRailError::StateCorrupt)?;
        let run = runs::get_run_in_tx(&transaction, &run_id, attempt)?
            .ok_or(NodeRailError::StateCorrupt)?;
        let claim_id = run
            .execution_claim_id
            .as_deref()
            .ok_or(NodeRailError::StateCorrupt)?;
        let claim = stored_claim(&transaction, claim_id)?;
        if claim.status != ClaimStatus::Claimed
            || claim.run_id != run_id
            || claim.attempt != attempt
        {
            return Err(NodeRailError::StateCorrupt);
        }
        let finished_at_ms = recovery_at_ms.max(claim.started_at_ms);
        persisted_at_ms = persisted_at_ms.max(finished_at_ms);
        match (run.lease.effect, run.status) {
            (RunEffect::ReadOnly, NodeRunStatus::Running) => {
                finish_claim_history(
                    &transaction,
                    claim_id,
                    "interrupted_retryable",
                    finished_at_ms,
                )?;
                let changed = transaction.execute(
                    "UPDATE node_runs
                     SET status = 'accepted', effect_started = 0,
                         execution_claim_id = NULL,
                         execution_claim_started_at_ms = NULL,
                         updated_at_ms = MAX(updated_at_ms, ?3)
                     WHERE run_id = ?1 AND attempt = ?2
                       AND status = 'running' AND effect_started = 1
                       AND execution_claim_id = ?4",
                    params![run_id, attempt, finished_at_ms, claim_id],
                )?;
                require_exact_transition(changed)?;
            }
            (RunEffect::ReadOnly, NodeRunStatus::CancelRequested) => {
                finish_recovered_run(
                    &transaction,
                    &mut meta,
                    &run,
                    claim_id,
                    RecoveryTerminal {
                        status: RunTerminalStatus::Cancelled,
                        content: "Read-only execution was cancelled during restart recovery.",
                        claim_status: "interrupted_cancelled",
                    },
                    finished_at_ms,
                )?;
            }
            (
                RunEffect::LocalMutation | RunEffect::ExternalEffect,
                NodeRunStatus::Running | NodeRunStatus::CancelRequested,
            ) => {
                finish_recovered_run(
                    &transaction,
                    &mut meta,
                    &run,
                    claim_id,
                    RecoveryTerminal {
                        status: RunTerminalStatus::Uncertain,
                        content:
                            "Execution stopped after the local effect claim; the outcome is uncertain.",
                        claim_status: "interrupted_uncertain",
                    },
                    finished_at_ms,
                )?;
            }
            _ => return Err(NodeRailError::StateCorrupt),
        }
    }
    write_meta_cursors(&transaction, &meta, persisted_at_ms)?;
    transaction.commit()?;
    Ok(())
}

fn finish_recovered_run(
    transaction: &Transaction<'_>,
    meta: &mut RailMeta,
    run: &NodeRunRecord,
    claim_id: &str,
    terminal: RecoveryTerminal,
    finished_at_ms: i64,
) -> Result<(), NodeRailError> {
    let completion = recovery_completion(
        &run.lease.run_id,
        run.lease.attempt,
        terminal.status,
        terminal.content,
    );
    let message = HubNodeMessage::RunCompleted(completion);
    let terminal_json = serde_json::to_vec(&message).map_err(|_| NodeRailError::InvalidMessage)?;
    let terminal_sha256 = sha256_hex(&terminal_json);
    let outbound = append_next_outbox(transaction, meta, message, finished_at_ms)?;
    let changed = transaction.execute(
        "UPDATE node_runs
         SET status = ?3, terminal_outbound_sequence = ?4,
             terminal_json = ?5, terminal_sha256 = ?6,
             terminal_at_ms = ?7, updated_at_ms = MAX(updated_at_ms, ?7)
         WHERE run_id = ?1 AND attempt = ?2
           AND status IN ('running', 'cancel_requested')
           AND effect_started = 1 AND execution_claim_id = ?8
           AND terminal_json IS NULL",
        params![
            run.lease.run_id,
            run.lease.attempt,
            terminal_status_str(terminal.status),
            u64_to_i64(outbound.sequence)?,
            terminal_json,
            terminal_sha256,
            finished_at_ms,
            claim_id,
        ],
    )?;
    require_exact_transition(changed)?;
    finish_claim_history(transaction, claim_id, terminal.claim_status, finished_at_ms)
}

fn finish_claim_history(
    transaction: &Transaction<'_>,
    claim_id: &str,
    status: &str,
    finished_at_ms: i64,
) -> Result<(), NodeRailError> {
    let changed = transaction.execute(
        "UPDATE node_run_claims SET status = ?2, finished_at_ms = ?3
         WHERE claim_id = ?1 AND status = 'claimed' AND finished_at_ms IS NULL",
        params![claim_id, status, finished_at_ms],
    )?;
    require_exact_transition(changed)
}

fn recovery_completion(
    run_id: &str,
    attempt: u32,
    status: RunTerminalStatus,
    content: &str,
) -> RunCompletion {
    RunCompletion {
        run_id: run_id.to_string(),
        attempt,
        status,
        result_content: content.to_string(),
        result_sha256: sha256_hex(content.as_bytes()),
        total_output_bytes: content.len() as u64,
        stored_output_bytes: content.len() as u64,
        capped: false,
        redacted: false,
        path_policy_applied: true,
    }
}

fn stored_terminal_message(
    connection: &Connection,
    run: &NodeRunRecord,
) -> Result<Option<HubNodeMessage>, NodeRailError> {
    connection
        .query_row(
            "SELECT terminal_json FROM node_runs WHERE run_id = ?1 AND attempt = ?2",
            params![run.lease.run_id, run.lease.attempt],
            |row| row.get::<_, Option<Vec<u8>>>(0),
        )?
        .map(|raw| serde_json::from_slice(&raw).map_err(|_| NodeRailError::StateCorrupt))
        .transpose()
}

pub(super) fn pending_cancellation_exists(
    connection: &Connection,
    run_id: &str,
    attempt: u32,
) -> Result<bool, NodeRailError> {
    let mut statement = connection.prepare(
        "SELECT sequence, message_kind, envelope_json, envelope_sha256
         FROM node_rail_inbox
         WHERE applied_at_ms IS NULL AND message_kind = 'cancel_run'
         ORDER BY sequence LIMIT 4097",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if rows.len() > 4_096 {
        return Err(NodeRailError::StateCorrupt);
    }
    for (sequence, kind, raw, digest) in rows {
        let envelope = decode_envelope(&raw, &digest, i64_to_u64(sequence)?, &kind)?;
        if matches!(
            envelope.message,
            HubNodeMessage::CancelRun {
                run_id: pending_run_id,
                attempt: pending_attempt,
                ..
            } if pending_run_id == run_id && pending_attempt == attempt
        ) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn terminal_status_str(status: RunTerminalStatus) -> &'static str {
    match status {
        RunTerminalStatus::Succeeded => "succeeded",
        RunTerminalStatus::Failed => "failed",
        RunTerminalStatus::Cancelled => "cancelled",
        RunTerminalStatus::Uncertain => "uncertain",
    }
}

fn require_exact_transition(changed: usize) -> Result<(), NodeRailError> {
    if changed == 1 {
        Ok(())
    } else {
        Err(NodeRailError::RunClaimConflict)
    }
}

struct RecoveryTerminal {
    status: RunTerminalStatus,
    content: &'static str,
    claim_status: &'static str,
}
