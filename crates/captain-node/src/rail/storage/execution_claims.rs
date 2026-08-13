use super::*;

pub(super) const MAX_LOCAL_RUN_CLAIMS: i64 = 16_384;
pub(super) const MAX_CLAIMS_PER_RUN: usize = 32;

pub(super) fn verify_run_claim_state(
    connection: &Connection,
    run: &NodeRunRecord,
) -> Result<(), NodeRailError> {
    let claims = claims_for_run(connection, &run.lease.run_id, run.lease.attempt)?;
    if claims.len() > MAX_CLAIMS_PER_RUN {
        return Err(NodeRailError::StateCorrupt);
    }
    if claims.iter().any(|claim| match claim.status {
        ClaimStatus::InterruptedRetryable | ClaimStatus::InterruptedCancelled => {
            run.lease.effect != RunEffect::ReadOnly
        }
        ClaimStatus::InterruptedUncertain => run.lease.effect == RunEffect::ReadOnly,
        ClaimStatus::Claimed | ClaimStatus::Completed => false,
    }) {
        return Err(NodeRailError::StateCorrupt);
    }
    let active_claims = claims
        .iter()
        .filter(|claim| claim.status == ClaimStatus::Claimed)
        .collect::<Vec<_>>();
    let claim_fields_match =
        run.execution_claim_id.is_some() == run.execution_claim_started_at_ms.is_some();
    if !claim_fields_match {
        return Err(NodeRailError::StateCorrupt);
    }
    if !run.effect_started {
        if run.execution_claim_id.is_some()
            || !active_claims.is_empty()
            || claims
                .iter()
                .any(|claim| claim.status != ClaimStatus::InterruptedRetryable)
            || matches!(
                run.status,
                NodeRunStatus::Running
                    | NodeRunStatus::CancelRequested
                    | NodeRunStatus::Succeeded
                    | NodeRunStatus::Failed
                    | NodeRunStatus::Uncertain
            )
        {
            return Err(NodeRailError::StateCorrupt);
        }
        return Ok(());
    }

    let claim_id = run
        .execution_claim_id
        .as_deref()
        .ok_or(NodeRailError::StateCorrupt)?;
    let claim_started_at_ms = run
        .execution_claim_started_at_ms
        .ok_or(NodeRailError::StateCorrupt)?;
    let current = claims
        .iter()
        .find(|claim| claim.claim_id == claim_id)
        .ok_or(NodeRailError::StateCorrupt)?;
    if current.started_at_ms != claim_started_at_ms
        || claims.iter().any(|claim| {
            claim.claim_id != claim_id && claim.status != ClaimStatus::InterruptedRetryable
        })
    {
        return Err(NodeRailError::StateCorrupt);
    }
    let valid = match run.status {
        NodeRunStatus::Running | NodeRunStatus::CancelRequested => {
            current.status == ClaimStatus::Claimed && active_claims.len() == 1
        }
        NodeRunStatus::Succeeded | NodeRunStatus::Failed => {
            current.status == ClaimStatus::Completed && active_claims.is_empty()
        }
        NodeRunStatus::Cancelled => {
            matches!(
                current.status,
                ClaimStatus::Completed | ClaimStatus::InterruptedCancelled
            ) && active_claims.is_empty()
        }
        NodeRunStatus::Uncertain => {
            matches!(
                current.status,
                ClaimStatus::Completed | ClaimStatus::InterruptedUncertain
            ) && active_claims.is_empty()
        }
        NodeRunStatus::ApprovalPending | NodeRunStatus::Accepted | NodeRunStatus::Rejected => false,
    };
    if valid {
        Ok(())
    } else {
        Err(NodeRailError::StateCorrupt)
    }
}

pub(super) fn verify_claim_table(connection: &Connection) -> Result<(), NodeRailError> {
    let (count, orphaned) = connection.query_row(
        "SELECT COUNT(*), COALESCE(SUM(CASE WHEN runs.run_id IS NULL THEN 1 ELSE 0 END), 0)
         FROM node_run_claims claims
         LEFT JOIN node_runs runs
           ON runs.run_id = claims.run_id AND runs.attempt = claims.attempt",
        [],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    if count > MAX_LOCAL_RUN_CLAIMS || orphaned != 0 {
        return Err(NodeRailError::StateCorrupt);
    }
    Ok(())
}

pub(super) fn stored_claim(
    connection: &Connection,
    claim_id: &str,
) -> Result<StoredClaim, NodeRailError> {
    let claim = connection
        .query_row(
            "SELECT claim_id, run_id, attempt, status, started_at_ms, finished_at_ms
             FROM node_run_claims WHERE claim_id = ?1",
            [claim_id],
            stored_claim_from_row,
        )
        .optional()?
        .ok_or(NodeRailError::RunClaimConflict)?;
    decode_stored_claim(claim)
}

pub(super) fn validate_claim_id(claim_id: &str) -> Result<(), NodeRailError> {
    if uuid::Uuid::parse_str(claim_id)
        .ok()
        .is_none_or(|id| id.hyphenated().to_string() != claim_id)
    {
        return Err(NodeRailError::StateCorrupt);
    }
    Ok(())
}

fn claims_for_run(
    connection: &Connection,
    run_id: &str,
    attempt: u32,
) -> Result<Vec<StoredClaim>, NodeRailError> {
    let mut statement = connection.prepare(
        "SELECT claim_id, run_id, attempt, status, started_at_ms, finished_at_ms
         FROM node_run_claims WHERE run_id = ?1 AND attempt = ?2
         ORDER BY started_at_ms, claim_id LIMIT 33",
    )?;
    let claims = statement
        .query_map(params![run_id, attempt], stored_claim_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    claims.into_iter().map(decode_stored_claim).collect()
}

fn stored_claim_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredClaimRow> {
    Ok(StoredClaimRow {
        claim_id: row.get(0)?,
        run_id: row.get(1)?,
        attempt: row.get(2)?,
        status: row.get(3)?,
        started_at_ms: row.get(4)?,
        finished_at_ms: row.get(5)?,
    })
}

fn decode_stored_claim(stored: StoredClaimRow) -> Result<StoredClaim, NodeRailError> {
    let claim = StoredClaim {
        claim_id: stored.claim_id,
        run_id: stored.run_id,
        attempt: u32::try_from(stored.attempt).map_err(|_| NodeRailError::StateCorrupt)?,
        status: ClaimStatus::parse(&stored.status).ok_or(NodeRailError::StateCorrupt)?,
        started_at_ms: stored.started_at_ms,
        finished_at_ms: stored.finished_at_ms,
    };
    validate_claim_id(&claim.claim_id)?;
    if claim.attempt == 0
        || claim.started_at_ms <= 0
        || claim
            .finished_at_ms
            .is_some_and(|finished| finished < claim.started_at_ms)
        || (claim.status == ClaimStatus::Claimed) != claim.finished_at_ms.is_none()
    {
        return Err(NodeRailError::StateCorrupt);
    }
    Ok(claim)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClaimStatus {
    Claimed,
    Completed,
    InterruptedRetryable,
    InterruptedCancelled,
    InterruptedUncertain,
}

impl ClaimStatus {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "claimed" => Self::Claimed,
            "completed" => Self::Completed,
            "interrupted_retryable" => Self::InterruptedRetryable,
            "interrupted_cancelled" => Self::InterruptedCancelled,
            "interrupted_uncertain" => Self::InterruptedUncertain,
            _ => return None,
        })
    }
}

pub(super) struct StoredClaim {
    pub(super) claim_id: String,
    pub(super) run_id: String,
    pub(super) attempt: u32,
    pub(super) status: ClaimStatus,
    pub(super) started_at_ms: i64,
    finished_at_ms: Option<i64>,
}

struct StoredClaimRow {
    claim_id: String,
    run_id: String,
    attempt: i64,
    status: String,
    started_at_ms: i64,
    finished_at_ms: Option<i64>,
}
