//! Durable CAS mirror of workflow-learning installation effects.
//!
//! The filesystem or scheduler journal proves what happened externally. This
//! table links that proof to the exact proposal revision and records every
//! phase change before the proposal can advance to canary or active.

use rusqlite::{params, types::Type, OptionalExtension, Transaction, TransactionBehavior};

use crate::workflow_learning_control::{
    proposal_by_id, WorkflowLearningControlError, WorkflowLearningStore, WorkflowProposalState,
};
pub use crate::workflow_learning_installation_types::{
    NewWorkflowInstallation, WorkflowInstallationEvent, WorkflowInstallationPhase,
    WorkflowInstallationRecord, WorkflowInstallationTransition,
};
use crate::workflow_learning_installation_validation::{
    is_legal_installation_transition, matches_prepared_metadata, require_phase_pair,
    validate_installation_transition, validate_new_installation,
};
use crate::workflow_learning_types::WorkflowArtifactKind;
use crate::workflow_learning_validation::{validate_hash, validate_token};

impl WorkflowLearningStore {
    pub fn record_installation_prepared(
        &self,
        input: &NewWorkflowInstallation,
    ) -> Result<WorkflowInstallationRecord, WorkflowLearningControlError> {
        validate_new_installation(input)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        if let Some(event) = installation_event_by_idempotency(&tx, &input.idempotency_key)? {
            if event.proposal_id == input.proposal_id
                && event.revision_sha256 == input.revision_sha256
                && event.to_phase == WorkflowInstallationPhase::Prepared
            {
                let installation =
                    installation_by_id(&tx, &input.proposal_id, &input.revision_sha256)?
                        .ok_or_else(|| {
                            WorkflowLearningControlError::CorruptData(
                                "installation event exists without its installation".to_string(),
                            )
                        })?;
                if matches_prepared_metadata(&installation, input)
                    && event.actor == input.actor
                    && event.reason == input.reason
                    && event.last_error.is_none()
                {
                    return Ok(installation);
                }
            }
            return Err(WorkflowLearningControlError::Conflict(
                "installation idempotency key was reused".to_string(),
            ));
        }

        let proposal = proposal_by_id(&tx, &input.proposal_id)?
            .ok_or_else(|| WorkflowLearningControlError::NotFound(input.proposal_id.clone()))?;
        if proposal.state != WorkflowProposalState::ApprovedPendingInstall
            || proposal.revision_sha256.as_deref() != Some(&input.revision_sha256)
            || proposal.artifact_sha256.as_deref() != Some(&input.installed_sha256)
            || proposal.kind != Some(input.kind)
        {
            return Err(WorkflowLearningControlError::Conflict(
                "prepared installation does not match the approved proposal revision".to_string(),
            ));
        }

        if let Some(existing) = installation_by_id(&tx, &input.proposal_id, &input.revision_sha256)?
        {
            if existing.phase == WorkflowInstallationPhase::Prepared
                && existing.phase_version == 0
                && matches_prepared_metadata(&existing, input)
            {
                return Err(WorkflowLearningControlError::Conflict(
                    "installation exists without the requested audit event".to_string(),
                ));
            }
            return Err(WorkflowLearningControlError::Conflict(
                "approved revision already has different installation metadata".to_string(),
            ));
        }

        tx.execute(
            "INSERT INTO workflow_learning_installations (
                 proposal_id, revision_sha256, kind, phase, phase_version,
                 target_locator, backup_locator, backup_sha256, installed_sha256,
                 last_error, prepared_at, updated_at
             ) VALUES (?1, ?2, ?3, 'prepared', 0, ?4, ?5, ?6, ?7, NULL, ?8, ?8)",
            params![
                input.proposal_id,
                input.revision_sha256,
                input.kind.as_str(),
                input.target_locator,
                input.backup_locator,
                input.backup_sha256,
                input.installed_sha256,
                input.occurred_at_unix_ms,
            ],
        )?;
        insert_installation_event(
            &tx,
            &input.idempotency_key,
            &input.proposal_id,
            &input.revision_sha256,
            None,
            WorkflowInstallationPhase::Prepared,
            0,
            None,
            &input.actor,
            &input.reason,
            input.occurred_at_unix_ms,
        )?;
        let created = installation_by_id(&tx, &input.proposal_id, &input.revision_sha256)?
            .ok_or_else(|| {
                WorkflowLearningControlError::CorruptData(
                    "prepared installation vanished".to_string(),
                )
            })?;
        tx.commit()?;
        Ok(created)
    }

    pub fn get_installation(
        &self,
        proposal_id: &str,
        revision_sha256: &str,
    ) -> Result<Option<WorkflowInstallationRecord>, WorkflowLearningControlError> {
        validate_token("proposal_id", proposal_id, 96)?;
        validate_hash("installation revision_sha256", revision_sha256)?;
        let conn = self.lock_conn()?;
        installation_by_id(&conn, proposal_id, revision_sha256).map_err(Into::into)
    }

    pub fn installation_events(
        &self,
        proposal_id: &str,
        revision_sha256: &str,
    ) -> Result<Vec<WorkflowInstallationEvent>, WorkflowLearningControlError> {
        validate_token("proposal_id", proposal_id, 96)?;
        validate_hash("installation revision_sha256", revision_sha256)?;
        let conn = self.lock_conn()?;
        let mut statement = conn.prepare(
            "SELECT sequence, idempotency_key, proposal_id, revision_sha256,
                    from_phase, to_phase, resulting_version, last_error,
                    actor, reason, created_at
             FROM workflow_learning_installation_events
             WHERE proposal_id = ?1 AND revision_sha256 = ?2 ORDER BY sequence",
        )?;
        let rows = statement.query_map(
            params![proposal_id, revision_sha256],
            installation_event_from_row,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn record_installation_promoted(
        &self,
        request: &WorkflowInstallationTransition,
    ) -> Result<WorkflowInstallationRecord, WorkflowLearningControlError> {
        require_phase_pair(
            request,
            WorkflowInstallationPhase::Prepared,
            WorkflowInstallationPhase::Promoted,
        )?;
        self.transition_installation(request)
    }

    pub fn record_installation_verified(
        &self,
        request: &WorkflowInstallationTransition,
    ) -> Result<WorkflowInstallationRecord, WorkflowLearningControlError> {
        require_phase_pair(
            request,
            WorkflowInstallationPhase::Promoted,
            WorkflowInstallationPhase::Verified,
        )?;
        self.transition_installation(request)
    }

    pub fn record_installation_rollback_pending(
        &self,
        request: &WorkflowInstallationTransition,
    ) -> Result<WorkflowInstallationRecord, WorkflowLearningControlError> {
        if request.to_phase != WorkflowInstallationPhase::RollbackPending
            || !matches!(
                request.expected_phase,
                WorkflowInstallationPhase::Promoted
                    | WorkflowInstallationPhase::Verified
                    | WorkflowInstallationPhase::Active
                    | WorkflowInstallationPhase::Failed
            )
        {
            return Err(WorkflowLearningControlError::InvalidInput(
                "rollback_pending requires a promoted, verified, active, or failed installation"
                    .to_string(),
            ));
        }
        self.transition_installation(request)
    }

    pub fn record_installation_quarantined(
        &self,
        request: &WorkflowInstallationTransition,
    ) -> Result<WorkflowInstallationRecord, WorkflowLearningControlError> {
        require_phase_pair(
            request,
            WorkflowInstallationPhase::RolledBack,
            WorkflowInstallationPhase::Quarantined,
        )?;
        self.transition_installation(request)
    }

    fn transition_installation(
        &self,
        request: &WorkflowInstallationTransition,
    ) -> Result<WorkflowInstallationRecord, WorkflowLearningControlError> {
        validate_installation_transition(request)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let updated = transition_installation_in_tx(&tx, request)?;
        tx.commit()?;
        Ok(updated)
    }
}

pub(crate) fn transition_installation_in_tx(
    tx: &Transaction<'_>,
    request: &WorkflowInstallationTransition,
) -> Result<WorkflowInstallationRecord, WorkflowLearningControlError> {
    validate_installation_transition(request)?;
    if let Some(event) = installation_event_by_idempotency(tx, &request.idempotency_key)? {
        if event.proposal_id == request.proposal_id
            && event.revision_sha256 == request.revision_sha256
            && event.to_phase == request.to_phase
            && event.from_phase == Some(request.expected_phase)
            && event.resulting_version == request.expected_version + 1
            && event.last_error == request.last_error
            && event.actor == request.actor
            && event.reason == request.reason
        {
            return installation_by_id(tx, &request.proposal_id, &request.revision_sha256)?
                .ok_or_else(|| {
                    WorkflowLearningControlError::CorruptData(
                        "installation event exists without its installation".to_string(),
                    )
                });
        }
        return Err(WorkflowLearningControlError::Conflict(
            "installation transition idempotency key was reused".to_string(),
        ));
    }
    let current = installation_by_id(tx, &request.proposal_id, &request.revision_sha256)?
        .ok_or_else(|| {
            WorkflowLearningControlError::NotFound(format!(
                "{}:{}",
                request.proposal_id, request.revision_sha256
            ))
        })?;
    if current.phase != request.expected_phase || current.phase_version != request.expected_version
    {
        return Err(WorkflowLearningControlError::Conflict(format!(
            "expected installation phase={} version={}, found phase={} version={}",
            request.expected_phase.as_str(),
            request.expected_version,
            current.phase.as_str(),
            current.phase_version
        )));
    }
    if !is_legal_installation_transition(request.expected_phase, request.to_phase) {
        return Err(WorkflowLearningControlError::IllegalTransition {
            from: request.expected_phase.as_str().to_string(),
            to: request.to_phase.as_str().to_string(),
        });
    }

    let next_version = current.phase_version + 1;
    let changed = tx.execute(
        "UPDATE workflow_learning_installations
         SET phase = ?1, phase_version = ?2, last_error = ?3,
             promoted_at = CASE WHEN ?1 = 'promoted' THEN ?4 ELSE promoted_at END,
             verified_at = CASE WHEN ?1 = 'verified' THEN ?4 ELSE verified_at END,
             rolled_back_at = CASE WHEN ?1 IN ('rolled_back', 'quarantined')
                                   THEN ?4 ELSE rolled_back_at END,
             updated_at = ?4
         WHERE proposal_id = ?5 AND revision_sha256 = ?6
           AND phase = ?7 AND phase_version = ?8",
        params![
            request.to_phase.as_str(),
            next_version as i64,
            request.last_error,
            request.occurred_at_unix_ms,
            request.proposal_id,
            request.revision_sha256,
            request.expected_phase.as_str(),
            request.expected_version as i64,
        ],
    )?;
    if changed != 1 {
        return Err(WorkflowLearningControlError::Conflict(
            "installation changed concurrently".to_string(),
        ));
    }
    insert_installation_event(
        tx,
        &request.idempotency_key,
        &request.proposal_id,
        &request.revision_sha256,
        Some(request.expected_phase),
        request.to_phase,
        next_version,
        request.last_error.as_deref(),
        &request.actor,
        &request.reason,
        request.occurred_at_unix_ms,
    )?;
    installation_by_id(tx, &request.proposal_id, &request.revision_sha256)?.ok_or_else(|| {
        WorkflowLearningControlError::CorruptData("transitioned installation vanished".to_string())
    })
}

pub(crate) fn installation_by_id(
    conn: &rusqlite::Connection,
    proposal_id: &str,
    revision_sha256: &str,
) -> rusqlite::Result<Option<WorkflowInstallationRecord>> {
    conn.query_row(
        &format!("{INSTALLATION_SELECT} WHERE proposal_id = ?1 AND revision_sha256 = ?2"),
        params![proposal_id, revision_sha256],
        installation_from_row,
    )
    .optional()
}

const INSTALLATION_SELECT: &str = "SELECT proposal_id, revision_sha256, kind, phase, phase_version,
            target_locator, backup_locator, backup_sha256, installed_sha256,
            last_error, prepared_at, promoted_at, verified_at, rolled_back_at, updated_at
     FROM workflow_learning_installations";

fn installation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowInstallationRecord> {
    let kind_value: String = row.get(2)?;
    let phase_value: String = row.get(3)?;
    Ok(WorkflowInstallationRecord {
        proposal_id: row.get(0)?,
        revision_sha256: row.get(1)?,
        kind: WorkflowArtifactKind::parse(&kind_value)
            .ok_or_else(|| corrupt_column(2, format!("unknown installation kind {kind_value}")))?,
        phase: WorkflowInstallationPhase::parse(&phase_value).ok_or_else(|| {
            corrupt_column(3, format!("unknown installation phase {phase_value}"))
        })?,
        phase_version: row.get::<_, i64>(4)?.max(0) as u64,
        target_locator: row.get(5)?,
        backup_locator: row.get(6)?,
        backup_sha256: row.get(7)?,
        installed_sha256: row.get(8)?,
        last_error: row.get(9)?,
        prepared_at_unix_ms: row.get(10)?,
        promoted_at_unix_ms: row.get(11)?,
        verified_at_unix_ms: row.get(12)?,
        rolled_back_at_unix_ms: row.get(13)?,
        updated_at_unix_ms: row.get(14)?,
    })
}

fn insert_installation_event(
    tx: &Transaction<'_>,
    idempotency_key: &str,
    proposal_id: &str,
    revision_sha256: &str,
    from_phase: Option<WorkflowInstallationPhase>,
    to_phase: WorkflowInstallationPhase,
    resulting_version: u64,
    last_error: Option<&str>,
    actor: &str,
    reason: &str,
    created_at_unix_ms: i64,
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO workflow_learning_installation_events (
             idempotency_key, proposal_id, revision_sha256, from_phase, to_phase,
             resulting_version, last_error, actor, reason, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            idempotency_key,
            proposal_id,
            revision_sha256,
            from_phase.map(WorkflowInstallationPhase::as_str),
            to_phase.as_str(),
            resulting_version as i64,
            last_error,
            actor,
            reason,
            created_at_unix_ms,
        ],
    )?;
    Ok(())
}

fn installation_event_by_idempotency(
    conn: &rusqlite::Connection,
    idempotency_key: &str,
) -> rusqlite::Result<Option<WorkflowInstallationEvent>> {
    conn.query_row(
        "SELECT sequence, idempotency_key, proposal_id, revision_sha256,
                from_phase, to_phase, resulting_version, last_error,
                actor, reason, created_at
         FROM workflow_learning_installation_events WHERE idempotency_key = ?1",
        params![idempotency_key],
        installation_event_from_row,
    )
    .optional()
}

pub(crate) fn installation_event_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<WorkflowInstallationEvent> {
    let from_value: Option<String> = row.get(4)?;
    let to_value: String = row.get(5)?;
    Ok(WorkflowInstallationEvent {
        sequence: row.get::<_, i64>(0)?.max(0) as u64,
        idempotency_key: row.get(1)?,
        proposal_id: row.get(2)?,
        revision_sha256: row.get(3)?,
        from_phase: from_value
            .map(|value| {
                WorkflowInstallationPhase::parse(&value)
                    .ok_or_else(|| corrupt_column(4, format!("unknown installation phase {value}")))
            })
            .transpose()?,
        to_phase: WorkflowInstallationPhase::parse(&to_value)
            .ok_or_else(|| corrupt_column(5, format!("unknown installation phase {to_value}")))?,
        resulting_version: row.get::<_, i64>(6)?.max(0) as u64,
        last_error: row.get(7)?,
        actor: row.get(8)?,
        reason: row.get(9)?,
        created_at_unix_ms: row.get(10)?,
    })
}

fn corrupt_column(column: usize, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message,
        )),
    )
}
