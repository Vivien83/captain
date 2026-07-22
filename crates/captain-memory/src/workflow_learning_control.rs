//! Durable authority for Skill Learning V2 proposals.
//!
//! SQLite owns the state machine. Every mutation is a compare-and-set against
//! the current state, version, and immutable artifact revision, then appends an
//! audit event in the same transaction.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::workflow_learning_installation::{
    installation_by_id, WorkflowInstallationEvent, WorkflowInstallationRecord,
};
pub use crate::workflow_learning_types::{
    NewWorkflowProposal, PublishValidatedDraft, WorkflowArtifactKind, WorkflowIsolatedTestRecord,
    WorkflowIsolatedTestStatus, WorkflowLearningControlError, WorkflowProposalEvent,
    WorkflowProposalRecord, WorkflowProposalState, WorkflowProposalTransition,
};
use crate::workflow_learning_validation::{
    validate_hash, validate_json, validate_text, validate_token,
};
use rusqlite::{
    params, types::Type, Connection, OptionalExtension, Transaction, TransactionBehavior,
};

#[derive(Clone)]
pub struct WorkflowLearningStore {
    pub(crate) conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowProposalSnapshot {
    pub proposal: WorkflowProposalRecord,
    pub proposal_events: Vec<WorkflowProposalEvent>,
    pub installation: Option<WorkflowInstallationRecord>,
    pub installation_events: Vec<WorkflowInstallationEvent>,
}

impl WorkflowLearningStore {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn create_observed(
        &self,
        input: &NewWorkflowProposal,
    ) -> Result<WorkflowProposalRecord, WorkflowLearningControlError> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let created = create_observed_in_tx(&tx, input)?;
        tx.commit()?;
        Ok(created)
    }

    pub fn get(
        &self,
        proposal_id: &str,
    ) -> Result<Option<WorkflowProposalRecord>, WorkflowLearningControlError> {
        validate_token("proposal_id", proposal_id, 96)?;
        let conn = self.lock_conn()?;
        proposal_by_id(&conn, proposal_id).map_err(Into::into)
    }

    pub fn get_by_operator_token(
        &self,
        operator_token: &str,
    ) -> Result<Option<WorkflowProposalRecord>, WorkflowLearningControlError> {
        validate_operator_token(operator_token)?;
        let normalized = operator_token.to_ascii_lowercase();
        let conn = self.lock_conn()?;
        proposal_by_operator_token(&conn, &normalized).map_err(Into::into)
    }

    pub fn list(
        &self,
        state: Option<WorkflowProposalState>,
        limit: usize,
    ) -> Result<Vec<WorkflowProposalRecord>, WorkflowLearningControlError> {
        let conn = self.lock_conn()?;
        let limit = limit.clamp(1, 1_000) as i64;
        let sql = if state.is_some() {
            format!(
                "{PROPOSAL_SELECT} WHERE p.state = ?1 ORDER BY p.updated_at DESC, p.id LIMIT ?2"
            )
        } else {
            format!("{PROPOSAL_SELECT} ORDER BY p.updated_at DESC, p.id LIMIT ?1")
        };
        let mut statement = conn.prepare(&sql)?;
        let rows = if let Some(state) = state {
            statement.query_map(params![state.as_str(), limit], proposal_from_row)?
        } else {
            statement.query_map(params![limit], proposal_from_row)?
        };
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Read every component needed by an operator view from one SQLite snapshot.
    pub fn list_snapshots(
        &self,
        limit: usize,
    ) -> Result<Vec<WorkflowProposalSnapshot>, WorkflowLearningControlError> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let proposals = {
            let mut statement = tx.prepare(&format!(
                "{PROPOSAL_SELECT} ORDER BY p.updated_at DESC, p.id LIMIT ?1"
            ))?;
            let rows =
                statement.query_map(params![limit.clamp(1, 1_000) as i64], proposal_from_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let mut snapshots = Vec::with_capacity(proposals.len());
        for proposal in proposals {
            let proposal_events = {
                let mut statement = tx.prepare(
                    "SELECT sequence, idempotency_key, proposal_id, from_state, to_state,
                            resulting_version, revision_sha256, actor, reason, created_at
                     FROM workflow_learning_proposal_events
                     WHERE proposal_id = ?1 ORDER BY sequence",
                )?;
                let rows = statement.query_map(params![proposal.id], event_from_row)?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            let (installation, installation_events) =
                if let Some(revision) = proposal.revision_sha256.as_deref() {
                    let installation = installation_by_id(&tx, &proposal.id, revision)?;
                    let events = {
                        let mut statement = tx.prepare(
                            "SELECT sequence, idempotency_key, proposal_id, revision_sha256,
                                    from_phase, to_phase, resulting_version, last_error,
                                    actor, reason, created_at
                             FROM workflow_learning_installation_events
                             WHERE proposal_id = ?1 AND revision_sha256 = ?2 ORDER BY sequence",
                        )?;
                        let rows = statement.query_map(
                            params![proposal.id, revision],
                            crate::workflow_learning_installation::installation_event_from_row,
                        )?;
                        rows.collect::<Result<Vec<_>, _>>()?
                    };
                    (installation, events)
                } else {
                    (None, Vec::new())
                };
            snapshots.push(WorkflowProposalSnapshot {
                proposal,
                proposal_events,
                installation,
                installation_events,
            });
        }
        tx.commit()?;
        Ok(snapshots)
    }

    pub fn list_due_snoozed(
        &self,
        now_unix_ms: i64,
        limit: usize,
    ) -> Result<Vec<WorkflowProposalRecord>, WorkflowLearningControlError> {
        let conn = self.lock_conn()?;
        let mut statement = conn.prepare(&format!(
            "{PROPOSAL_SELECT}
             WHERE p.state = 'snoozed' AND p.snoozed_until IS NOT NULL
               AND p.snoozed_until <= ?1
             ORDER BY p.snoozed_until, p.updated_at, p.id LIMIT ?2"
        ))?;
        let rows = statement.query_map(
            params![now_unix_ms, limit.clamp(1, 1_000) as i64],
            proposal_from_row,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn transition(
        &self,
        request: &WorkflowProposalTransition,
    ) -> Result<WorkflowProposalRecord, WorkflowLearningControlError> {
        validate_transition(request)?;
        if requires_atomic_effect_transition(request.expected_state, request.to_state) {
            return Err(WorkflowLearningControlError::InvalidInput(format!(
                "transition to {} requires its dedicated atomic control-plane operation",
                request.to_state.as_str()
            )));
        }
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let updated = transition_in_tx(&tx, request)?;
        tx.commit()?;
        Ok(updated)
    }

    pub fn publish_validated_draft(
        &self,
        request: &PublishValidatedDraft,
    ) -> Result<WorkflowProposalRecord, WorkflowLearningControlError> {
        validate_publish(request)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let published = publish_validated_draft_in_tx(&tx, request)?;
        tx.commit()?;
        Ok(published)
    }

    pub fn events(
        &self,
        proposal_id: &str,
    ) -> Result<Vec<WorkflowProposalEvent>, WorkflowLearningControlError> {
        validate_token("proposal_id", proposal_id, 96)?;
        let conn = self.lock_conn()?;
        let mut statement = conn.prepare(
            "SELECT sequence, idempotency_key, proposal_id, from_state, to_state,
                    resulting_version, revision_sha256, actor, reason, created_at
             FROM workflow_learning_proposal_events
             WHERE proposal_id = ?1 ORDER BY sequence",
        )?;
        let rows = statement.query_map(params![proposal_id], event_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub(crate) fn lock_conn(
        &self,
    ) -> Result<MutexGuard<'_, Connection>, WorkflowLearningControlError> {
        self.conn.lock().map_err(|error| {
            WorkflowLearningControlError::CorruptData(format!("database lock poisoned: {error}"))
        })
    }
}

pub(crate) fn create_observed_in_tx(
    tx: &Transaction<'_>,
    input: &NewWorkflowProposal,
) -> Result<WorkflowProposalRecord, WorkflowLearningControlError> {
    validate_new_proposal(input)?;
    if let Some(existing) = proposal_by_idempotency(tx, &input.idempotency_key)? {
        if existing.id == input.id
            && existing.workflow_signature == input.workflow_signature
            && existing.source_agent_id == input.source_agent_id
            && existing.origin_channel == input.origin_channel
            && existing.evidence_json == input.evidence_json
        {
            return Ok(existing);
        }
        return Err(WorkflowLearningControlError::Conflict(
            "proposal idempotency key was reused with different input".to_string(),
        ));
    }

    tx.execute(
        "INSERT INTO workflow_learning_proposals (
             id, idempotency_key, workflow_signature, state, state_version,
             source_agent_id, origin_channel, evidence_json, created_at, updated_at
         ) VALUES (?1, ?2, ?3, 'observed', 0, ?4, ?5, ?6, ?7, ?7)",
        params![
            input.id,
            input.idempotency_key,
            input.workflow_signature,
            input.source_agent_id,
            input.origin_channel,
            input.evidence_json,
            input.created_at_unix_ms,
        ],
    )?;
    insert_event(
        tx,
        &input.idempotency_key,
        &input.id,
        None,
        WorkflowProposalState::Observed,
        0,
        None,
        "captain:workflow-learning",
        "candidate observed",
        input.created_at_unix_ms,
    )?;
    proposal_by_id(tx, &input.id)?
        .ok_or_else(|| WorkflowLearningControlError::NotFound(input.id.clone()))
}

pub(crate) fn transition_in_tx(
    tx: &Transaction<'_>,
    request: &WorkflowProposalTransition,
) -> Result<WorkflowProposalRecord, WorkflowLearningControlError> {
    if let Some(event) = event_by_idempotency(tx, &request.idempotency_key)? {
        if event.proposal_id == request.proposal_id
            && event.from_state == Some(request.expected_state)
            && event.to_state == request.to_state
            && event.resulting_version == request.expected_version.saturating_add(1)
            && event.revision_sha256 == request.expected_revision_sha256
            && event.actor == request.actor
            && event.reason == request.reason
        {
            return proposal_by_id(tx, &request.proposal_id)?.ok_or_else(|| {
                WorkflowLearningControlError::NotFound(request.proposal_id.clone())
            });
        }
        return Err(WorkflowLearningControlError::Conflict(
            "transition idempotency key was reused".to_string(),
        ));
    }

    let current = proposal_by_id(tx, &request.proposal_id)?
        .ok_or_else(|| WorkflowLearningControlError::NotFound(request.proposal_id.clone()))?;
    if current.state != request.expected_state
        || current.state_version != request.expected_version
        || current.revision_sha256 != request.expected_revision_sha256
    {
        return Err(WorkflowLearningControlError::Conflict(format!(
            "expected state={} version={} revision={:?}, found state={} version={} revision={:?}",
            request.expected_state.as_str(),
            request.expected_version,
            request.expected_revision_sha256,
            current.state.as_str(),
            current.state_version,
            current.revision_sha256
        )));
    }
    if !is_legal_transition(request.expected_state, request.to_state) {
        return Err(WorkflowLearningControlError::IllegalTransition {
            from: request.expected_state.as_str().to_string(),
            to: request.to_state.as_str().to_string(),
        });
    }
    if current.state == WorkflowProposalState::Proposed
        && request.to_state != WorkflowProposalState::Superseded
    {
        if let Some(revision) = current.revision_sha256.as_deref() {
            let blocked: i64 = tx.query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM workflow_learning_refinements
                     WHERE proposal_id = ?1 AND revision_sha256 = ?2
                       AND (state = 'queued'
                            OR (state = 'awaiting_input' AND expires_at > ?3))
                 )",
                params![current.id, revision, request.occurred_at_unix_ms],
                |row| row.get(0),
            )?;
            if blocked != 0 {
                return Err(WorkflowLearningControlError::Conflict(
                    "proposal has an active refinement and cannot change concurrently".to_string(),
                ));
            }
        }
    }
    if requires_revision(request.to_state) && current.revision_sha256.is_none() {
        return Err(WorkflowLearningControlError::Conflict(
            "activation transition requires an immutable staged revision".to_string(),
        ));
    }

    let next_version = current.state_version + 1;
    let changed = tx.execute(
        "UPDATE workflow_learning_proposals
         SET state = ?1, state_version = ?2, snoozed_until = ?3, updated_at = ?4
         WHERE id = ?5 AND state = ?6 AND state_version = ?7
           AND revision_sha256 IS ?8",
        params![
            request.to_state.as_str(),
            next_version as i64,
            request.snoozed_until_unix_ms,
            request.occurred_at_unix_ms,
            request.proposal_id,
            request.expected_state.as_str(),
            request.expected_version as i64,
            request.expected_revision_sha256,
        ],
    )?;
    if changed != 1 {
        return Err(WorkflowLearningControlError::Conflict(
            "proposal changed concurrently".to_string(),
        ));
    }
    insert_event(
        tx,
        &request.idempotency_key,
        &request.proposal_id,
        Some(request.expected_state),
        request.to_state,
        next_version,
        current.revision_sha256.as_deref(),
        &request.actor,
        &request.reason,
        request.occurred_at_unix_ms,
    )?;
    proposal_by_id(tx, &request.proposal_id)?
        .ok_or_else(|| WorkflowLearningControlError::NotFound(request.proposal_id.clone()))
}

pub(crate) fn publish_validated_draft_in_tx(
    tx: &Transaction<'_>,
    request: &PublishValidatedDraft,
) -> Result<WorkflowProposalRecord, WorkflowLearningControlError> {
    let operator_token = workflow_operator_token(&request.revision_sha256)?;
    if let Some(event) = event_by_idempotency(tx, &request.idempotency_key)? {
        let proposal = proposal_by_id(tx, &request.proposal_id)?
            .ok_or_else(|| WorkflowLearningControlError::NotFound(request.proposal_id.clone()))?;
        if event.proposal_id == request.proposal_id
            && event.from_state == Some(WorkflowProposalState::Validating)
            && event.to_state == WorkflowProposalState::Proposed
            && event.resulting_version == request.expected_version.saturating_add(1)
            && event.revision_sha256.as_deref() == Some(request.revision_sha256.as_str())
            && event.actor == request.actor
            && event.reason == request.reason
            && proposal.operator_token.as_deref() == Some(operator_token.as_str())
            && proposal.staging_job_id.as_deref() == Some(request.staging_job_id.as_str())
            && proposal.artifact_sha256.as_deref() == Some(request.artifact_sha256.as_str())
            && proposal.kind == Some(request.kind)
            && proposal.name.as_deref() == Some(request.name.as_str())
            && proposal.validation_json.as_deref() == Some(request.validation_json.as_str())
        {
            return Ok(proposal);
        }
        return Err(WorkflowLearningControlError::Conflict(
            "publish idempotency key was reused".to_string(),
        ));
    }

    let current = proposal_by_id(tx, &request.proposal_id)?
        .ok_or_else(|| WorkflowLearningControlError::NotFound(request.proposal_id.clone()))?;
    if current.state != WorkflowProposalState::Validating
        || current.state_version != request.expected_version
        || current.revision_sha256.is_some()
    {
        return Err(WorkflowLearningControlError::Conflict(
            "proposal is not the expected unrevisioned validating candidate".to_string(),
        ));
    }
    if let Some(existing) = proposal_by_operator_token(tx, &operator_token)? {
        return Err(WorkflowLearningControlError::Conflict(format!(
            "operator token collision with proposal {}",
            existing.id
        )));
    }

    let next_version = current.state_version + 1;
    let changed = tx.execute(
        "UPDATE workflow_learning_proposals
         SET state = 'proposed', state_version = ?1, revision_sha256 = ?2,
             operator_token = ?3, artifact_sha256 = ?4, staging_job_id = ?5,
             kind = ?6, name = ?7, validation_json = ?8,
             snoozed_until = NULL, updated_at = ?9
         WHERE id = ?10 AND state = 'validating' AND state_version = ?11
           AND revision_sha256 IS NULL",
        params![
            next_version as i64,
            request.revision_sha256,
            operator_token,
            request.artifact_sha256,
            request.staging_job_id,
            request.kind.as_str(),
            request.name,
            request.validation_json,
            request.occurred_at_unix_ms,
            request.proposal_id,
            request.expected_version as i64,
        ],
    )?;
    if changed != 1 {
        return Err(WorkflowLearningControlError::Conflict(
            "proposal changed concurrently while publishing".to_string(),
        ));
    }
    insert_event(
        tx,
        &request.idempotency_key,
        &request.proposal_id,
        Some(WorkflowProposalState::Validating),
        WorkflowProposalState::Proposed,
        next_version,
        Some(&request.revision_sha256),
        &request.actor,
        &request.reason,
        request.occurred_at_unix_ms,
    )?;
    proposal_by_id(tx, &request.proposal_id)?
        .ok_or_else(|| WorkflowLearningControlError::NotFound(request.proposal_id.clone()))
}

pub fn is_legal_transition(from: WorkflowProposalState, to: WorkflowProposalState) -> bool {
    use WorkflowProposalState::*;
    matches!(
        (from, to),
        (Observed, Eligible | Dismissed | Superseded)
            | (Eligible, Drafting | Dismissed | Superseded)
            | (Drafting, Validating | Rejected | Superseded)
            | (Validating, Rejected | Superseded)
            | (
                Proposed,
                ApprovedPendingInstall | Dismissed | Snoozed | Superseded | Rejected
            )
            | (Snoozed, Proposed | Dismissed | Superseded | Rejected)
            | (
                ApprovedPendingInstall,
                Proposed | ActiveCanary | InstallFailed | Rejected
            )
            | (ActiveCanary, Active | InstallFailed | RolledBack)
            | (Active, RolledBack | Superseded)
            | (
                InstallFailed,
                ApprovedPendingInstall | Rejected | RolledBack
            )
            | (RolledBack, ApprovedPendingInstall | Superseded)
    )
}

fn requires_revision(state: WorkflowProposalState) -> bool {
    matches!(
        state,
        WorkflowProposalState::ApprovedPendingInstall
            | WorkflowProposalState::ActiveCanary
            | WorkflowProposalState::Active
            | WorkflowProposalState::InstallFailed
            | WorkflowProposalState::RolledBack
    )
}

fn requires_atomic_effect_transition(
    from: WorkflowProposalState,
    to: WorkflowProposalState,
) -> bool {
    (from == WorkflowProposalState::ApprovedPendingInstall && to == WorkflowProposalState::Proposed)
        || matches!(
            to,
            WorkflowProposalState::ApprovedPendingInstall
                | WorkflowProposalState::ActiveCanary
                | WorkflowProposalState::Active
                | WorkflowProposalState::InstallFailed
                | WorkflowProposalState::RolledBack
        )
}

const PROPOSAL_SELECT: &str =
    "SELECT p.id, p.idempotency_key, p.workflow_signature, p.state, p.state_version,
            p.revision_sha256, p.operator_token, p.artifact_sha256, p.staging_job_id,
            p.kind, p.name, p.source_agent_id, p.origin_channel, p.evidence_json,
            p.validation_json, p.snoozed_until, p.last_error_code, p.last_error_message,
            p.created_at, p.updated_at,
            t.id, t.idempotency_key, t.proposal_id, t.revision_sha256, t.job_id,
            t.status, t.requested_by, t.result_json, t.requested_at, t.completed_at,
            t.updated_at
     FROM workflow_learning_proposals p
     LEFT JOIN workflow_learning_tests t ON t.id = (
         SELECT latest.id FROM workflow_learning_tests latest
         WHERE latest.proposal_id = p.id
           AND latest.revision_sha256 = p.revision_sha256
         ORDER BY latest.requested_at DESC, latest.sequence DESC LIMIT 1
     )";

pub(crate) fn proposal_by_id(
    conn: &Connection,
    proposal_id: &str,
) -> rusqlite::Result<Option<WorkflowProposalRecord>> {
    conn.query_row(
        &format!("{PROPOSAL_SELECT} WHERE p.id = ?1"),
        params![proposal_id],
        proposal_from_row,
    )
    .optional()
}

fn proposal_by_operator_token(
    conn: &Connection,
    operator_token: &str,
) -> rusqlite::Result<Option<WorkflowProposalRecord>> {
    conn.query_row(
        &format!("{PROPOSAL_SELECT} WHERE p.operator_token = ?1"),
        params![operator_token],
        proposal_from_row,
    )
    .optional()
}

fn proposal_by_idempotency(
    conn: &Connection,
    idempotency_key: &str,
) -> rusqlite::Result<Option<WorkflowProposalRecord>> {
    conn.query_row(
        &format!("{PROPOSAL_SELECT} WHERE p.idempotency_key = ?1"),
        params![idempotency_key],
        proposal_from_row,
    )
    .optional()
}

fn proposal_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowProposalRecord> {
    let state_value: String = row.get(3)?;
    let state = parse_state_column(&state_value, 3)?;
    let kind_value: Option<String> = row.get(9)?;
    let kind =
        match kind_value {
            Some(value) => Some(WorkflowArtifactKind::parse(&value).ok_or_else(|| {
                corrupt_column(9, format!("unknown workflow artifact kind {value}"))
            })?),
            None => None,
        };
    let test_id: Option<String> = row.get(20)?;
    let isolated_test = match test_id {
        Some(id) => {
            let status_value: String = row.get(25)?;
            let status = WorkflowIsolatedTestStatus::parse(&status_value).ok_or_else(|| {
                corrupt_column(25, format!("unknown isolated test status {status_value}"))
            })?;
            let result_json: Option<String> = row.get(27)?;
            let completed_at_unix_ms: Option<i64> = row.get(29)?;
            if (status == WorkflowIsolatedTestStatus::Queued
                && (result_json.is_some() || completed_at_unix_ms.is_some()))
                || (status != WorkflowIsolatedTestStatus::Queued
                    && (result_json.is_none() || completed_at_unix_ms.is_none()))
            {
                return Err(corrupt_column(
                    25,
                    "isolated test status and completion evidence disagree".to_string(),
                ));
            }
            Some(WorkflowIsolatedTestRecord {
                id,
                idempotency_key: row.get(21)?,
                proposal_id: row.get(22)?,
                revision_sha256: row.get(23)?,
                job_id: row.get(24)?,
                status,
                requested_by: row.get(26)?,
                result_json,
                requested_at_unix_ms: row.get(28)?,
                completed_at_unix_ms,
                updated_at_unix_ms: row.get(30)?,
            })
        }
        None => None,
    };
    Ok(WorkflowProposalRecord {
        id: row.get(0)?,
        idempotency_key: row.get(1)?,
        workflow_signature: row.get(2)?,
        state,
        state_version: row.get::<_, i64>(4)?.max(0) as u64,
        revision_sha256: row.get(5)?,
        operator_token: row.get(6)?,
        artifact_sha256: row.get(7)?,
        staging_job_id: row.get(8)?,
        kind,
        name: row.get(10)?,
        source_agent_id: row.get(11)?,
        origin_channel: row.get(12)?,
        evidence_json: row.get(13)?,
        validation_json: row.get(14)?,
        isolated_test,
        snoozed_until_unix_ms: row.get(15)?,
        last_error_code: row.get(16)?,
        last_error_message: row.get(17)?,
        created_at_unix_ms: row.get(18)?,
        updated_at_unix_ms: row.get(19)?,
    })
}

pub fn workflow_operator_token(
    revision_sha256: &str,
) -> Result<String, WorkflowLearningControlError> {
    validate_hash("revision_sha256", revision_sha256)?;
    Ok(revision_sha256[..20].to_ascii_lowercase())
}

fn validate_operator_token(value: &str) -> Result<(), WorkflowLearningControlError> {
    if value.len() == 20 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(WorkflowLearningControlError::InvalidInput(
            "operator_token must be a 20-character hex token".to_string(),
        ))
    }
}

fn insert_event(
    tx: &Transaction<'_>,
    idempotency_key: &str,
    proposal_id: &str,
    from_state: Option<WorkflowProposalState>,
    to_state: WorkflowProposalState,
    resulting_version: u64,
    revision_sha256: Option<&str>,
    actor: &str,
    reason: &str,
    created_at_unix_ms: i64,
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO workflow_learning_proposal_events (
             idempotency_key, proposal_id, from_state, to_state,
             resulting_version, revision_sha256, actor, reason, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            idempotency_key,
            proposal_id,
            from_state.map(WorkflowProposalState::as_str),
            to_state.as_str(),
            resulting_version as i64,
            revision_sha256,
            actor,
            reason,
            created_at_unix_ms,
        ],
    )?;
    Ok(())
}

fn event_by_idempotency(
    conn: &Connection,
    idempotency_key: &str,
) -> rusqlite::Result<Option<WorkflowProposalEvent>> {
    conn.query_row(
        "SELECT sequence, idempotency_key, proposal_id, from_state, to_state,
                resulting_version, revision_sha256, actor, reason, created_at
         FROM workflow_learning_proposal_events WHERE idempotency_key = ?1",
        params![idempotency_key],
        event_from_row,
    )
    .optional()
}

pub(crate) fn event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowProposalEvent> {
    let from_value: Option<String> = row.get(3)?;
    let from_state = match from_value {
        Some(value) => Some(parse_state_column(&value, 3)?),
        None => None,
    };
    let to_value: String = row.get(4)?;
    Ok(WorkflowProposalEvent {
        sequence: row.get::<_, i64>(0)?.max(0) as u64,
        idempotency_key: row.get(1)?,
        proposal_id: row.get(2)?,
        from_state,
        to_state: parse_state_column(&to_value, 4)?,
        resulting_version: row.get::<_, i64>(5)?.max(0) as u64,
        revision_sha256: row.get(6)?,
        actor: row.get(7)?,
        reason: row.get(8)?,
        created_at_unix_ms: row.get(9)?,
    })
}

fn parse_state_column(value: &str, column: usize) -> rusqlite::Result<WorkflowProposalState> {
    WorkflowProposalState::parse(value)
        .ok_or_else(|| corrupt_column(column, format!("unknown proposal state {value}")))
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

fn validate_new_proposal(input: &NewWorkflowProposal) -> Result<(), WorkflowLearningControlError> {
    validate_token("proposal_id", &input.id, 96)?;
    validate_token("idempotency_key", &input.idempotency_key, 192)?;
    validate_hash("workflow_signature", &input.workflow_signature)?;
    validate_token("source_agent_id", &input.source_agent_id, 96)?;
    if let Some(channel) = &input.origin_channel {
        validate_token("origin_channel", channel, 64)?;
    }
    validate_json("evidence_json", &input.evidence_json, 64 * 1024)
}

pub(crate) fn validate_transition(
    request: &WorkflowProposalTransition,
) -> Result<(), WorkflowLearningControlError> {
    validate_token("proposal_id", &request.proposal_id, 96)?;
    validate_token("transition idempotency_key", &request.idempotency_key, 192)?;
    validate_token("actor", &request.actor, 128)?;
    validate_text("reason", &request.reason, 1, 2_048)?;
    if let Some(revision) = &request.expected_revision_sha256 {
        validate_hash("revision_sha256", revision)?;
    }
    if request.to_state == WorkflowProposalState::Snoozed {
        if request.snoozed_until_unix_ms.is_none() {
            return Err(WorkflowLearningControlError::InvalidInput(
                "snoozed transition requires snoozed_until".to_string(),
            ));
        }
    } else if request.snoozed_until_unix_ms.is_some() {
        return Err(WorkflowLearningControlError::InvalidInput(
            "snoozed_until is only valid for snoozed state".to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_publish(
    request: &PublishValidatedDraft,
) -> Result<(), WorkflowLearningControlError> {
    validate_token("proposal_id", &request.proposal_id, 96)?;
    validate_token("staging_job_id", &request.staging_job_id, 96)?;
    validate_hash("revision_sha256", &request.revision_sha256)?;
    validate_hash("artifact_sha256", &request.artifact_sha256)?;
    validate_token("artifact name", &request.name, 96)?;
    validate_json("validation_json", &request.validation_json, 64 * 1024)?;
    validate_token("actor", &request.actor, 128)?;
    validate_text("reason", &request.reason, 1, 2_048)?;
    validate_token("publish idempotency_key", &request.idempotency_key, 192)
}
