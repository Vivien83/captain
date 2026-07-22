//! Durable operator bindings for refining one exact workflow proposal.

use rusqlite::{params, types::Type, OptionalExtension, Transaction, TransactionBehavior};

use crate::workflow_learning_control::{
    proposal_by_id, WorkflowLearningControlError, WorkflowLearningStore, WorkflowProposalState,
};
pub use crate::workflow_learning_refinement_types::{
    NewWorkflowRefinementRequest, WorkflowRefinementEvent, WorkflowRefinementRecord,
    WorkflowRefinementState,
};
use crate::workflow_learning_validation::{validate_hash, validate_text, validate_token};

const MAX_BINDING_LIFETIME_MS: i64 = 24 * 60 * 60 * 1_000;

impl WorkflowLearningStore {
    pub fn begin_refinement_request(
        &self,
        input: &NewWorkflowRefinementRequest,
    ) -> Result<WorkflowRefinementRecord, WorkflowLearningControlError> {
        validate_new_request(input)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire_due_in_tx(&tx, input.created_at_unix_ms)?;

        if let Some(existing) = refinement_by_idempotency(&tx, &input.idempotency_key)? {
            if request_matches(&existing, input) {
                tx.commit()?;
                return Ok(existing);
            }
            return Err(WorkflowLearningControlError::Conflict(
                "refinement idempotency key was reused with different input".to_string(),
            ));
        }

        if let Some(existing) =
            refinement_by_binding(&tx, &input.surface, &input.conversation_key, &input.actor)?
        {
            if same_active_identity(&existing, input) {
                tx.commit()?;
                return Ok(existing);
            }
            return Err(WorkflowLearningControlError::Conflict(
                "another proposal is already awaiting input in this conversation".to_string(),
            ));
        }
        if let Some(existing) =
            refinement_by_active_revision(&tx, &input.proposal_id, &input.revision_sha256)?
        {
            return Err(WorkflowLearningControlError::Conflict(format!(
                "proposal revision already has active refinement {}",
                existing.id
            )));
        }

        let proposal = proposal_by_id(&tx, &input.proposal_id)?
            .ok_or_else(|| WorkflowLearningControlError::NotFound(input.proposal_id.clone()))?;
        if proposal.state != WorkflowProposalState::Proposed
            || proposal.state_version != input.expected_proposal_version
            || proposal.revision_sha256.as_deref() != Some(input.revision_sha256.as_str())
        {
            return Err(WorkflowLearningControlError::Conflict(
                "refinement requires the exact current proposed revision".to_string(),
            ));
        }

        tx.execute(
            "INSERT INTO workflow_learning_refinements (
                 id, idempotency_key, proposal_id, revision_sha256,
                 expected_proposal_version, actor, surface, conversation_key,
                 source_message_id, language, state, state_version,
                 expires_at, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                       'awaiting_input', 0, ?11, ?12, ?12)",
            params![
                input.id,
                input.idempotency_key,
                input.proposal_id,
                input.revision_sha256,
                input.expected_proposal_version as i64,
                input.actor,
                input.surface,
                input.conversation_key,
                input.source_message_id,
                input.language,
                input.expires_at_unix_ms,
                input.created_at_unix_ms,
            ],
        )?;
        insert_event(
            &tx,
            &input.idempotency_key,
            &input.id,
            None,
            WorkflowRefinementState::AwaitingInput,
            0,
            &input.actor,
            "operator requested proposal refinement",
            input.created_at_unix_ms,
        )?;
        let created = refinement_by_id(&tx, &input.id)?.ok_or_else(|| {
            WorkflowLearningControlError::CorruptData(
                "created refinement request vanished".to_string(),
            )
        })?;
        tx.commit()?;
        Ok(created)
    }

    pub fn get_refinement_request(
        &self,
        request_id: &str,
    ) -> Result<Option<WorkflowRefinementRecord>, WorkflowLearningControlError> {
        validate_token("refinement request id", request_id, 96)?;
        let conn = self.lock_conn()?;
        refinement_by_id(&conn, request_id).map_err(Into::into)
    }

    pub fn pending_refinement_for_binding(
        &self,
        surface: &str,
        conversation_key: &str,
        actor: &str,
        now_unix_ms: i64,
    ) -> Result<Option<WorkflowRefinementRecord>, WorkflowLearningControlError> {
        validate_token("refinement surface", surface, 32)?;
        validate_token("refinement conversation_key", conversation_key, 192)?;
        validate_token("refinement actor", actor, 128)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire_due_in_tx(&tx, now_unix_ms)?;
        let pending = tx
            .query_row(
                &format!(
                    "{REFINEMENT_SELECT}
                     WHERE state = 'awaiting_input' AND surface = ?1
                       AND conversation_key = ?2 AND actor = ?3"
                ),
                params![surface, conversation_key, actor],
                refinement_from_row,
            )
            .optional()?;
        tx.commit()?;
        Ok(pending)
    }

    pub fn refinement_for_captured_message(
        &self,
        surface: &str,
        conversation_key: &str,
        actor: &str,
        captured_message_id: &str,
    ) -> Result<Option<WorkflowRefinementRecord>, WorkflowLearningControlError> {
        validate_token("refinement surface", surface, 32)?;
        validate_token("refinement conversation_key", conversation_key, 192)?;
        validate_token("refinement actor", actor, 128)?;
        validate_token("refinement captured_message_id", captured_message_id, 128)?;
        let conn = self.lock_conn()?;
        conn.query_row(
            &format!(
                "{REFINEMENT_SELECT}
                 WHERE surface = ?1 AND conversation_key = ?2 AND actor = ?3
                   AND captured_message_id = ?4
                   AND state IN ('queued', 'completed', 'failed')"
            ),
            params![surface, conversation_key, actor, captured_message_id],
            refinement_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn refinement_events(
        &self,
        request_id: &str,
    ) -> Result<Vec<WorkflowRefinementEvent>, WorkflowLearningControlError> {
        validate_token("refinement request id", request_id, 96)?;
        let conn = self.lock_conn()?;
        let mut statement = conn.prepare(
            "SELECT sequence, idempotency_key, request_id, from_state, to_state,
                    resulting_version, actor, reason, created_at
             FROM workflow_learning_refinement_events
             WHERE request_id = ?1 ORDER BY sequence",
        )?;
        let rows = statement.query_map(params![request_id], refinement_event_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

pub(crate) fn refinement_by_id(
    conn: &rusqlite::Connection,
    request_id: &str,
) -> rusqlite::Result<Option<WorkflowRefinementRecord>> {
    conn.query_row(
        &format!("{REFINEMENT_SELECT} WHERE id = ?1"),
        params![request_id],
        refinement_from_row,
    )
    .optional()
}

fn refinement_by_idempotency(
    conn: &rusqlite::Connection,
    idempotency_key: &str,
) -> rusqlite::Result<Option<WorkflowRefinementRecord>> {
    conn.query_row(
        &format!("{REFINEMENT_SELECT} WHERE idempotency_key = ?1"),
        params![idempotency_key],
        refinement_from_row,
    )
    .optional()
}

fn refinement_by_binding(
    conn: &rusqlite::Connection,
    surface: &str,
    conversation_key: &str,
    actor: &str,
) -> rusqlite::Result<Option<WorkflowRefinementRecord>> {
    conn.query_row(
        &format!(
            "{REFINEMENT_SELECT}
             WHERE state = 'awaiting_input' AND surface = ?1
               AND conversation_key = ?2 AND actor = ?3"
        ),
        params![surface, conversation_key, actor],
        refinement_from_row,
    )
    .optional()
}

fn refinement_by_active_revision(
    conn: &rusqlite::Connection,
    proposal_id: &str,
    revision_sha256: &str,
) -> rusqlite::Result<Option<WorkflowRefinementRecord>> {
    conn.query_row(
        &format!(
            "{REFINEMENT_SELECT}
             WHERE state IN ('awaiting_input', 'queued')
               AND proposal_id = ?1 AND revision_sha256 = ?2"
        ),
        params![proposal_id, revision_sha256],
        refinement_from_row,
    )
    .optional()
}

pub(crate) fn expire_due_in_tx(
    tx: &Transaction<'_>,
    now_unix_ms: i64,
) -> Result<(), WorkflowLearningControlError> {
    let mut statement = tx.prepare(
        "SELECT id, state_version FROM workflow_learning_refinements
         WHERE state = 'awaiting_input' AND expires_at <= ?1 ORDER BY expires_at, id",
    )?;
    let due = statement
        .query_map(params![now_unix_ms], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?.max(0) as u64,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for (request_id, version) in due {
        let next_version = version.saturating_add(1);
        let changed = tx.execute(
            "UPDATE workflow_learning_refinements
             SET state = 'expired', state_version = ?1, updated_at = ?2
             WHERE id = ?3 AND state = 'awaiting_input' AND state_version = ?4",
            params![next_version as i64, now_unix_ms, request_id, version as i64],
        )?;
        if changed == 1 {
            insert_event(
                tx,
                &format!("refinement-expired:{request_id}"),
                &request_id,
                Some(WorkflowRefinementState::AwaitingInput),
                WorkflowRefinementState::Expired,
                next_version,
                "captain:workflow-refinement",
                "refinement input window expired",
                now_unix_ms,
            )?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_event(
    tx: &Transaction<'_>,
    idempotency_key: &str,
    request_id: &str,
    from_state: Option<WorkflowRefinementState>,
    to_state: WorkflowRefinementState,
    resulting_version: u64,
    actor: &str,
    reason: &str,
    created_at_unix_ms: i64,
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO workflow_learning_refinement_events (
             idempotency_key, request_id, from_state, to_state,
             resulting_version, actor, reason, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            idempotency_key,
            request_id,
            from_state.map(WorkflowRefinementState::as_str),
            to_state.as_str(),
            resulting_version as i64,
            actor,
            reason,
            created_at_unix_ms,
        ],
    )?;
    Ok(())
}

fn validate_new_request(
    input: &NewWorkflowRefinementRequest,
) -> Result<(), WorkflowLearningControlError> {
    validate_token("refinement request id", &input.id, 96)?;
    validate_token(
        "refinement request idempotency_key",
        &input.idempotency_key,
        192,
    )?;
    validate_token("proposal_id", &input.proposal_id, 96)?;
    validate_hash("refinement revision_sha256", &input.revision_sha256)?;
    validate_token("refinement actor", &input.actor, 128)?;
    validate_token("refinement surface", &input.surface, 32)?;
    validate_token("refinement conversation_key", &input.conversation_key, 192)?;
    if let Some(message_id) = &input.source_message_id {
        validate_token("refinement source_message_id", message_id, 128)?;
    }
    validate_text("refinement language", &input.language, 1, 16)?;
    let lifetime = input
        .expires_at_unix_ms
        .checked_sub(input.created_at_unix_ms)
        .ok_or_else(|| {
            WorkflowLearningControlError::InvalidInput(
                "refinement expiry overflows the clock".to_string(),
            )
        })?;
    if !(60_000..=MAX_BINDING_LIFETIME_MS).contains(&lifetime) {
        return Err(WorkflowLearningControlError::InvalidInput(
            "refinement input window must be between one minute and 24 hours".to_string(),
        ));
    }
    Ok(())
}

fn request_matches(
    existing: &WorkflowRefinementRecord,
    input: &NewWorkflowRefinementRequest,
) -> bool {
    existing.id == input.id
        && existing.proposal_id == input.proposal_id
        && existing.revision_sha256 == input.revision_sha256
        && existing.expected_proposal_version == input.expected_proposal_version
        && existing.actor == input.actor
        && existing.surface == input.surface
        && existing.conversation_key == input.conversation_key
        && existing.source_message_id == input.source_message_id
        && existing.language == input.language
        && existing.expires_at_unix_ms == input.expires_at_unix_ms
        && existing.created_at_unix_ms == input.created_at_unix_ms
}

fn same_active_identity(
    existing: &WorkflowRefinementRecord,
    input: &NewWorkflowRefinementRequest,
) -> bool {
    existing.proposal_id == input.proposal_id
        && existing.revision_sha256 == input.revision_sha256
        && existing.expected_proposal_version == input.expected_proposal_version
        && existing.actor == input.actor
        && existing.surface == input.surface
        && existing.conversation_key == input.conversation_key
}

fn refinement_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowRefinementRecord> {
    let state_value: String = row.get(10)?;
    Ok(WorkflowRefinementRecord {
        id: row.get(0)?,
        idempotency_key: row.get(1)?,
        proposal_id: row.get(2)?,
        revision_sha256: row.get(3)?,
        expected_proposal_version: row.get::<_, i64>(4)?.max(0) as u64,
        actor: row.get(5)?,
        surface: row.get(6)?,
        conversation_key: row.get(7)?,
        source_message_id: row.get(8)?,
        language: row.get(9)?,
        state: WorkflowRefinementState::parse(&state_value)
            .ok_or_else(|| corrupt_column(10, format!("unknown refinement state {state_value}")))?,
        state_version: row.get::<_, i64>(11)?.max(0) as u64,
        instruction: row.get(12)?,
        captured_message_id: row.get(13)?,
        child_proposal_id: row.get(14)?,
        draft_job_id: row.get(15)?,
        last_error: row.get(16)?,
        expires_at_unix_ms: row.get(17)?,
        created_at_unix_ms: row.get(18)?,
        updated_at_unix_ms: row.get(19)?,
    })
}

fn refinement_event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowRefinementEvent> {
    let from_value: Option<String> = row.get(3)?;
    let to_value: String = row.get(4)?;
    Ok(WorkflowRefinementEvent {
        sequence: row.get::<_, i64>(0)?.max(0) as u64,
        idempotency_key: row.get(1)?,
        request_id: row.get(2)?,
        from_state: from_value
            .map(|value| {
                WorkflowRefinementState::parse(&value)
                    .ok_or_else(|| corrupt_column(3, format!("unknown refinement state {value}")))
            })
            .transpose()?,
        to_state: WorkflowRefinementState::parse(&to_value)
            .ok_or_else(|| corrupt_column(4, format!("unknown refinement state {to_value}")))?,
        resulting_version: row.get::<_, i64>(5)?.max(0) as u64,
        actor: row.get(6)?,
        reason: row.get(7)?,
        created_at_unix_ms: row.get(8)?,
    })
}

const REFINEMENT_SELECT: &str = "SELECT id, idempotency_key, proposal_id, revision_sha256,
            expected_proposal_version, actor, surface, conversation_key,
            source_message_id, language, state, state_version, instruction,
            captured_message_id, child_proposal_id, draft_job_id, last_error,
            expires_at, created_at, updated_at
     FROM workflow_learning_refinements";

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
