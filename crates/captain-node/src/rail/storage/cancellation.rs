use super::*;
use captain_wire::hub_protocol::{RunCompletion, RunTerminalStatus};

pub(in crate::rail) fn apply_cancel_run(
    connection: &mut Connection,
    sequence: u64,
    applied_at_ms: i64,
) -> Result<NodeRunCancelOutcome, NodeRailError> {
    validate_timestamp(applied_at_ms)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut meta = read_meta(&transaction)?;
    let inbound = runs::pending_inbound_for_sequence(&transaction, sequence)?;
    let HubNodeMessage::CancelRun {
        run_id, attempt, ..
    } = &inbound.envelope.message
    else {
        return Err(NodeRailError::RunDecisionConflict);
    };
    let run =
        runs::get_run_in_tx(&transaction, run_id, *attempt)?.ok_or(NodeRailError::RunConflict)?;
    let cancel_json =
        serde_json::to_vec(&inbound.envelope.message).map_err(|_| NodeRailError::InvalidMessage)?;
    let cancel_sha256 = sha256_hex(&cancel_json);

    if let Some(existing) = stored_cancel(&transaction, run_id, *attempt)? {
        if existing != cancel_json {
            return Err(NodeRailError::RunConflict);
        }
        let changed = transaction.execute(
            "UPDATE node_runs
             SET cancel_inbound_sequence = ?3,
                 updated_at_ms = MAX(updated_at_ms, ?4)
             WHERE run_id = ?1 AND attempt = ?2",
            params![run_id, attempt, u64_to_i64(sequence)?, applied_at_ms],
        )?;
        require_run_transition(changed)?;
        mark_inbound_applied_in_tx(&transaction, &mut meta, sequence, applied_at_ms)?;
        write_meta_cursors(&transaction, &meta, applied_at_ms)?;
        let run = runs::get_run_in_tx(&transaction, run_id, *attempt)?
            .ok_or(NodeRailError::StateCorrupt)?;
        let outbound = terminal_outbound(&transaction, &run)?;
        let signal_runner = run.status == NodeRunStatus::CancelRequested;
        transaction.commit()?;
        return Ok(NodeRunCancelOutcome {
            run,
            outbound,
            signal_runner,
            replayed: true,
        });
    }

    let (outbound, signal_runner) = if run.status.is_terminal() {
        let changed = transaction.execute(
            "UPDATE node_runs
             SET cancel_inbound_sequence = ?3, cancel_json = ?4,
                 cancel_sha256 = ?5, updated_at_ms = MAX(updated_at_ms, ?6)
             WHERE run_id = ?1 AND attempt = ?2
               AND cancel_inbound_sequence IS NULL",
            params![
                run_id,
                attempt,
                u64_to_i64(sequence)?,
                cancel_json,
                cancel_sha256,
                applied_at_ms,
            ],
        )?;
        require_run_transition(changed)?;
        (terminal_outbound(&transaction, &run)?, false)
    } else if !run.effect_started
        && matches!(
            run.status,
            NodeRunStatus::ApprovalPending | NodeRunStatus::Accepted
        )
    {
        let completion = cancelled_before_effect(run_id, *attempt);
        let terminal = HubNodeMessage::RunCompleted(completion);
        let terminal_json =
            serde_json::to_vec(&terminal).map_err(|_| NodeRailError::InvalidMessage)?;
        let terminal_sha256 = sha256_hex(&terminal_json);
        let outbound = append_next_outbox(&transaction, &mut meta, terminal, applied_at_ms)?;
        let changed = transaction.execute(
            "UPDATE node_runs
             SET status = 'cancelled', cancel_inbound_sequence = ?3,
                 cancel_json = ?4, cancel_sha256 = ?5,
                 terminal_outbound_sequence = ?6, terminal_json = ?7,
                 terminal_sha256 = ?8, terminal_at_ms = ?9,
                 updated_at_ms = MAX(updated_at_ms, ?9)
             WHERE run_id = ?1 AND attempt = ?2
               AND status IN ('approval_pending', 'accepted')
               AND effect_started = 0 AND cancel_inbound_sequence IS NULL",
            params![
                run_id,
                attempt,
                u64_to_i64(sequence)?,
                cancel_json,
                cancel_sha256,
                u64_to_i64(outbound.sequence)?,
                terminal_json,
                terminal_sha256,
                applied_at_ms,
            ],
        )?;
        require_run_transition(changed)?;
        (Some(outbound), false)
    } else if run.effect_started
        && matches!(run.status, NodeRunStatus::Running | NodeRunStatus::Accepted)
    {
        let changed = transaction.execute(
            "UPDATE node_runs
             SET status = 'cancel_requested', cancel_inbound_sequence = ?3,
                 cancel_json = ?4, cancel_sha256 = ?5,
                 updated_at_ms = MAX(updated_at_ms, ?6)
             WHERE run_id = ?1 AND attempt = ?2
               AND status IN ('accepted', 'running')
               AND effect_started = 1 AND cancel_inbound_sequence IS NULL",
            params![
                run_id,
                attempt,
                u64_to_i64(sequence)?,
                cancel_json,
                cancel_sha256,
                applied_at_ms,
            ],
        )?;
        require_run_transition(changed)?;
        (None, true)
    } else {
        return Err(NodeRailError::RunDecisionConflict);
    };

    mark_inbound_applied_in_tx(&transaction, &mut meta, sequence, applied_at_ms)?;
    write_meta_cursors(&transaction, &meta, applied_at_ms)?;
    let run =
        runs::get_run_in_tx(&transaction, run_id, *attempt)?.ok_or(NodeRailError::StateCorrupt)?;
    transaction.commit()?;
    Ok(NodeRunCancelOutcome {
        run,
        outbound,
        signal_runner,
        replayed: false,
    })
}

fn stored_cancel(
    connection: &Connection,
    run_id: &str,
    attempt: u32,
) -> Result<Option<Vec<u8>>, NodeRailError> {
    connection
        .query_row(
            "SELECT cancel_json FROM node_runs WHERE run_id = ?1 AND attempt = ?2",
            params![run_id, attempt],
            |row| row.get::<_, Option<Vec<u8>>>(0),
        )
        .optional()
        .map(|value| value.flatten())
        .map_err(Into::into)
}

fn terminal_outbound(
    connection: &Connection,
    run: &NodeRunRecord,
) -> Result<Option<HubNodeEnvelope>, NodeRailError> {
    run.terminal_outbound_sequence
        .map(|sequence| runs::outbound_by_sequence(connection, sequence))
        .transpose()
        .map(Option::flatten)
}

fn cancelled_before_effect(run_id: &str, attempt: u32) -> RunCompletion {
    let result_content = "Cancelled before local execution.".to_string();
    let stored_output_bytes = result_content.len() as u64;
    RunCompletion {
        run_id: run_id.to_string(),
        attempt,
        status: RunTerminalStatus::Cancelled,
        result_sha256: sha256_hex(result_content.as_bytes()),
        result_content,
        total_output_bytes: stored_output_bytes,
        stored_output_bytes,
        capped: false,
        redacted: false,
        path_policy_applied: true,
    }
}

fn require_run_transition(changed: usize) -> Result<(), NodeRailError> {
    if changed == 1 {
        Ok(())
    } else {
        Err(NodeRailError::RunDecisionConflict)
    }
}
