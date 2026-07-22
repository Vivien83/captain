//! Durable workflow episodes used by Skill Learning V2.
//!
//! The store records one owning task/turn and its ordered tool attempts. It
//! deliberately accepts only redacted/normalized input evidence; raw tool
//! payloads and outputs do not belong in these tables.

use captain_types::error::{CaptainError, CaptainResult};
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowEpisodeStatus {
    Succeeded,
    Failed,
    Stopped,
    Uncertain,
}

impl WorkflowEpisodeStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
            Self::Uncertain => "uncertain",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowStepStatus {
    Succeeded,
    Failed,
    Interrupted,
    Uncertain,
}

impl WorkflowStepStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
            Self::Uncertain => "uncertain",
        }
    }

    fn is_success(self) -> bool {
        self == Self::Succeeded
    }
}

#[derive(Debug, Clone)]
pub struct NewWorkflowEpisode {
    pub id: String,
    pub session_id: String,
    pub turn_id: String,
    pub agent_id: String,
    pub origin_channel: Option<String>,
    pub project_id: Option<String>,
    pub workspace_scope: Option<String>,
    pub intent_redacted: String,
    pub intent_fingerprint: String,
    pub secret_detected: bool,
    pub explicit_reuse_request: bool,
    pub started_at_unix_ms: i64,
}

#[derive(Debug, Clone)]
pub struct NewWorkflowEpisodeStep {
    pub episode_id: String,
    pub tool_use_id: String,
    pub ordinal: u32,
    pub tool_name: String,
    pub dependency_ids_json: String,
    pub input_shape_json: String,
    pub input_fingerprint: String,
    pub effect_class: String,
    pub secret_detected: bool,
    pub started_at_unix_ms: i64,
}

#[derive(Debug, Clone)]
pub struct WorkflowStepOutcome {
    pub status: WorkflowStepStatus,
    pub output_class: Option<String>,
    pub verification_marker: Option<String>,
    pub retry_count: u32,
    pub completed_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowEpisodeRecord {
    pub id: String,
    pub session_id: String,
    pub turn_id: String,
    pub agent_id: String,
    pub origin_channel: Option<String>,
    pub project_id: Option<String>,
    pub workspace_scope: Option<String>,
    pub intent_redacted: String,
    pub intent_fingerprint: String,
    pub status: String,
    pub explicit_reuse_request: bool,
    pub tool_attempt_count: u32,
    pub success_count: u32,
    pub failure_count: u32,
    pub has_secret_input: bool,
    pub has_unverified_mutation: bool,
    pub failure_reason: Option<String>,
    pub started_at_unix_ms: i64,
    pub completed_at_unix_ms: Option<i64>,
    pub analysis_status: String,
    pub analysis_result_json: Option<String>,
    pub analysis_proposal_id: Option<String>,
    pub analysis_updated_at_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowAnalysisOutcomeStatus {
    Processed,
    Rejected,
}

impl WorkflowAnalysisOutcomeStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Processed => "processed",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkflowAnalysisOutcome {
    pub episode_ids: Vec<String>,
    pub status: WorkflowAnalysisOutcomeStatus,
    pub result_json: String,
    pub proposal_id: Option<String>,
    pub recorded_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowEpisodeStepRecord {
    pub episode_id: String,
    pub tool_use_id: String,
    pub ordinal: u32,
    pub tool_name: String,
    pub dependency_ids_json: String,
    pub input_shape_json: String,
    pub input_fingerprint: String,
    pub effect_class: String,
    pub status: String,
    pub retry_count: u32,
    pub output_class: Option<String>,
    pub verification_marker: Option<String>,
    pub secret_detected: bool,
    pub started_at_unix_ms: i64,
    pub completed_at_unix_ms: Option<i64>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowEpisodeEvidence {
    pub episode: WorkflowEpisodeRecord,
    pub steps: Vec<WorkflowEpisodeStepRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowRecoverySummary {
    pub episodes_reconciled: usize,
    pub steps_interrupted: usize,
    pub analysis_claims_released: usize,
}

#[derive(Clone)]
pub struct WorkflowEpisodeStore {
    conn: Arc<Mutex<Connection>>,
}

impl WorkflowEpisodeStore {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Begin an episode, returning the authoritative id. Retrying the same
    /// agent/session/turn tuple is idempotent and returns its existing id.
    pub fn begin_episode(&self, episode: &NewWorkflowEpisode) -> CaptainResult<String> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO workflow_episodes (
                 id, session_id, turn_id, agent_id, origin_channel, project_id,
                 workspace_scope, intent_redacted, intent_fingerprint,
                 has_secret_input, explicit_reuse_request, started_at,
                 created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12, ?12)
             ON CONFLICT(agent_id, session_id, turn_id) DO NOTHING",
            params![
                episode.id,
                episode.session_id,
                episode.turn_id,
                episode.agent_id,
                episode.origin_channel,
                episode.project_id,
                episode.workspace_scope,
                episode.intent_redacted,
                episode.intent_fingerprint,
                episode.secret_detected,
                episode.explicit_reuse_request,
                episode.started_at_unix_ms,
            ],
        )
        .map_err(memory_error)?;

        conn.query_row(
            "SELECT id FROM workflow_episodes
             WHERE agent_id = ?1 AND session_id = ?2 AND turn_id = ?3",
            params![episode.agent_id, episode.session_id, episode.turn_id],
            |row| row.get(0),
        )
        .map_err(memory_error)
    }

    /// Record a tool attempt exactly once. Returns `true` only when a new row
    /// was inserted, so retries cannot inflate episode counters.
    pub fn begin_step(&self, step: &NewWorkflowEpisodeStep) -> CaptainResult<bool> {
        validate_json(&step.dependency_ids_json, true)?;
        validate_json(&step.input_shape_json, false)?;
        validate_effect(&step.effect_class)?;

        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(memory_error)?;
        let inserted = tx
            .execute(
                "INSERT INTO workflow_episode_steps (
                     episode_id, tool_use_id, ordinal, tool_name,
                     dependency_ids_json, input_shape_json, input_fingerprint,
                     effect_class, secret_detected, started_at
                 )
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10
                 FROM workflow_episodes
                 WHERE id = ?1 AND status = 'running'
                 ON CONFLICT(episode_id, tool_use_id) DO NOTHING",
                params![
                    step.episode_id,
                    step.tool_use_id,
                    step.ordinal,
                    step.tool_name,
                    step.dependency_ids_json,
                    step.input_shape_json,
                    step.input_fingerprint,
                    step.effect_class,
                    step.secret_detected,
                    step.started_at_unix_ms,
                ],
            )
            .map_err(memory_error)?;

        if inserted == 1 {
            tx.execute(
                "UPDATE workflow_episodes
                 SET tool_attempt_count = tool_attempt_count + 1,
                     has_secret_input = MAX(has_secret_input, ?2),
                     updated_at = ?3
                 WHERE id = ?1",
                params![
                    step.episode_id,
                    step.secret_detected,
                    step.started_at_unix_ms
                ],
            )
            .map_err(memory_error)?;
        } else if !step_exists(&tx, &step.episode_id, &step.tool_use_id)? {
            return Err(CaptainError::Memory(format!(
                "cannot record tool step {}: episode {} is missing or no longer running",
                step.tool_use_id, step.episode_id
            )));
        }
        tx.commit().map_err(memory_error)?;
        Ok(inserted == 1)
    }

    /// Finish a step exactly once. Returns `false` for an idempotent replay of
    /// the same terminal outcome and rejects conflicting terminal outcomes.
    pub fn finish_step(
        &self,
        episode_id: &str,
        tool_use_id: &str,
        outcome: &WorkflowStepOutcome,
    ) -> CaptainResult<bool> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(memory_error)?;
        let verification = outcome.verification_marker.as_deref();
        let updated = tx
            .execute(
                "UPDATE workflow_episode_steps
                 SET status = ?3, retry_count = ?4, output_class = ?5,
                     verification_marker = ?6, completed_at = ?7,
                     duration_ms = MAX(0, ?7 - started_at)
                 WHERE episode_id = ?1 AND tool_use_id = ?2 AND status = 'running'",
                params![
                    episode_id,
                    tool_use_id,
                    outcome.status.as_str(),
                    outcome.retry_count,
                    outcome.output_class,
                    verification,
                    outcome.completed_at_unix_ms,
                ],
            )
            .map_err(memory_error)?;

        if updated == 1 {
            let mutating_unverified = tx
                .query_row(
                    "SELECT effect_class <> 'read' AND ?3 IS NULL
                     FROM workflow_episode_steps
                     WHERE episode_id = ?1 AND tool_use_id = ?2",
                    params![episode_id, tool_use_id, verification],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(memory_error)?;
            tx.execute(
                "UPDATE workflow_episodes
                 SET success_count = success_count + ?2,
                     failure_count = failure_count + ?3,
                     has_unverified_mutation = MAX(has_unverified_mutation, ?4),
                     updated_at = ?5
                 WHERE id = ?1",
                params![
                    episode_id,
                    outcome.status.is_success(),
                    !outcome.status.is_success(),
                    mutating_unverified,
                    outcome.completed_at_unix_ms,
                ],
            )
            .map_err(memory_error)?;
        } else {
            let existing: Option<String> = tx
                .query_row(
                    "SELECT status FROM workflow_episode_steps
                     WHERE episode_id = ?1 AND tool_use_id = ?2",
                    params![episode_id, tool_use_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(memory_error)?;
            match existing.as_deref() {
                Some(status) if status == outcome.status.as_str() => {}
                Some(status) => {
                    return Err(CaptainError::Memory(format!(
                        "tool step {tool_use_id} is already terminal as {status}"
                    )));
                }
                None => {
                    return Err(CaptainError::Memory(format!(
                        "tool step {tool_use_id} does not exist in episode {episode_id}"
                    )));
                }
            }
        }
        tx.commit().map_err(memory_error)?;
        Ok(updated == 1)
    }

    /// Close an owning turn/task. A successful episode cannot hide a tool
    /// attempt that is still running.
    pub fn finish_episode(
        &self,
        episode_id: &str,
        status: WorkflowEpisodeStatus,
        reason: Option<&str>,
        completed_at_unix_ms: i64,
    ) -> CaptainResult<bool> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(memory_error)?;
        let running_steps: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM workflow_episode_steps
                 WHERE episode_id = ?1 AND status = 'running'",
                params![episode_id],
                |row| row.get(0),
            )
            .map_err(memory_error)?;
        if status == WorkflowEpisodeStatus::Succeeded && running_steps > 0 {
            return Err(CaptainError::Memory(format!(
                "cannot mark episode {episode_id} succeeded with {running_steps} running step(s)"
            )));
        }

        if running_steps > 0 {
            tx.execute(
                "UPDATE workflow_episode_steps
                 SET status = 'interrupted', output_class = 'owner_terminated',
                     completed_at = ?2, duration_ms = MAX(0, ?2 - started_at)
                 WHERE episode_id = ?1 AND status = 'running'",
                params![episode_id, completed_at_unix_ms],
            )
            .map_err(memory_error)?;
            tx.execute(
                "UPDATE workflow_episodes
                 SET failure_count = failure_count + ?2,
                     has_unverified_mutation = MAX(
                         has_unverified_mutation,
                         EXISTS(
                             SELECT 1 FROM workflow_episode_steps
                             WHERE episode_id = ?1 AND status = 'interrupted'
                               AND effect_class <> 'read'
                         )
                     )
                 WHERE id = ?1",
                params![episode_id, running_steps],
            )
            .map_err(memory_error)?;
        }

        let updated = tx
            .execute(
                "UPDATE workflow_episodes
                 SET status = ?2, failure_reason = ?3, completed_at = ?4,
                     updated_at = ?4
                 WHERE id = ?1 AND status = 'running'",
                params![episode_id, status.as_str(), reason, completed_at_unix_ms],
            )
            .map_err(memory_error)?;
        if updated == 0 {
            let existing: Option<String> = tx
                .query_row(
                    "SELECT status FROM workflow_episodes WHERE id = ?1",
                    params![episode_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(memory_error)?;
            match existing.as_deref() {
                Some(value) if value == status.as_str() => {}
                Some(value) => {
                    return Err(CaptainError::Memory(format!(
                        "episode {episode_id} is already terminal as {value}"
                    )));
                }
                None => {
                    return Err(CaptainError::Memory(format!(
                        "workflow episode {episode_id} does not exist"
                    )));
                }
            }
        }
        tx.commit().map_err(memory_error)?;
        Ok(updated == 1)
    }

    /// Reconcile crash signatures before accepting new learning work.
    pub fn reconcile_incomplete(&self) -> CaptainResult<WorkflowRecoverySummary> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(memory_error)?;
        let steps_interrupted: usize = tx
            .query_row(
                "SELECT COUNT(*) FROM workflow_episode_steps WHERE status = 'running'",
                [],
                |row| row.get(0),
            )
            .map_err(memory_error)?;
        let episodes_reconciled: usize = tx
            .query_row(
                "SELECT COUNT(*) FROM workflow_episodes WHERE status = 'running'",
                [],
                |row| row.get(0),
            )
            .map_err(memory_error)?;
        let analysis_claims_released: usize = tx
            .query_row(
                "SELECT COUNT(*) FROM workflow_episodes WHERE analysis_status = 'claimed'",
                [],
                |row| row.get(0),
            )
            .map_err(memory_error)?;

        tx.execute(
            "UPDATE workflow_episodes
             SET failure_count = failure_count + (
                     SELECT COUNT(*) FROM workflow_episode_steps s
                     WHERE s.episode_id = workflow_episodes.id
                       AND s.status = 'running'
                 ),
                 has_unverified_mutation = MAX(
                     has_unverified_mutation,
                     EXISTS(
                         SELECT 1 FROM workflow_episode_steps s
                         WHERE s.episode_id = workflow_episodes.id
                           AND s.status = 'running' AND s.effect_class <> 'read'
                     )
                 ),
                 status = 'uncertain',
                 failure_reason = 'Captain stopped before the workflow episode closed',
                 completed_at = ?1,
                 updated_at = ?1
             WHERE status = 'running'",
            params![now],
        )
        .map_err(memory_error)?;
        tx.execute(
            "UPDATE workflow_episode_steps
             SET status = 'interrupted', output_class = 'captain_restart',
                 completed_at = ?1, duration_ms = MAX(0, ?1 - started_at)
             WHERE status = 'running'",
            params![now],
        )
        .map_err(memory_error)?;
        tx.execute(
            "UPDATE workflow_episodes
             SET analysis_status = 'pending', analysis_result_json = NULL,
                 analysis_proposal_id = NULL, analysis_updated_at = ?1,
                 updated_at = MAX(updated_at, ?1)
             WHERE analysis_status = 'claimed'",
            params![now],
        )
        .map_err(memory_error)?;
        tx.commit().map_err(memory_error)?;

        Ok(WorkflowRecoverySummary {
            episodes_reconciled,
            steps_interrupted,
            analysis_claims_released,
        })
    }

    pub fn get_episode(&self, episode_id: &str) -> CaptainResult<Option<WorkflowEpisodeRecord>> {
        let conn = self.lock_conn()?;
        conn.query_row(
            "SELECT id, session_id, turn_id, agent_id, origin_channel,
                    project_id, workspace_scope, intent_redacted,
                    intent_fingerprint, status, explicit_reuse_request,
                    tool_attempt_count, success_count, failure_count,
                    has_secret_input, has_unverified_mutation, failure_reason,
                    started_at, completed_at, analysis_status,
                    analysis_result_json, analysis_proposal_id, analysis_updated_at
             FROM workflow_episodes WHERE id = ?1",
            params![episode_id],
            episode_from_row,
        )
        .optional()
        .map_err(memory_error)
    }

    pub fn list_steps(&self, episode_id: &str) -> CaptainResult<Vec<WorkflowEpisodeStepRecord>> {
        let conn = self.lock_conn()?;
        list_steps_for_conn(&conn, episode_id)
    }

    /// Return a bounded, deterministic snapshot for the V2 analyzer. Running
    /// work and already claimed/processed rows never enter a new batch.
    pub fn list_pending_evidence(
        &self,
        limit: usize,
    ) -> CaptainResult<Vec<WorkflowEpisodeEvidence>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit.min(1_000)).unwrap_or(1_000);
        let conn = self.lock_conn()?;
        let episodes = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, turn_id, agent_id, origin_channel,
                            project_id, workspace_scope, intent_redacted,
                            intent_fingerprint, status, explicit_reuse_request,
                            tool_attempt_count, success_count, failure_count,
                            has_secret_input, has_unverified_mutation, failure_reason,
                            started_at, completed_at, analysis_status,
                            analysis_result_json, analysis_proposal_id, analysis_updated_at
                     FROM workflow_episodes
                     WHERE analysis_status = 'pending' AND completed_at IS NOT NULL
                       AND status <> 'running'
                     ORDER BY completed_at, id
                     LIMIT ?1",
                )
                .map_err(memory_error)?;
            let rows = stmt
                .query_map(params![limit], episode_from_row)
                .map_err(memory_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(memory_error)?
        };

        episodes
            .into_iter()
            .map(|episode| {
                let steps = list_steps_for_conn(&conn, &episode.id)?;
                Ok(WorkflowEpisodeEvidence { episode, steps })
            })
            .collect()
    }

    /// Record one deterministic analyzer decision for a bounded set of
    /// episodes. Replaying the exact decision is safe; changing it is a
    /// conflict so a restart cannot silently relabel evidence.
    pub fn record_analysis_outcome(&self, outcome: &WorkflowAnalysisOutcome) -> CaptainResult<()> {
        if outcome.episode_ids.is_empty() || outcome.episode_ids.len() > 1_000 {
            return Err(CaptainError::Memory(
                "analysis outcome must contain 1..=1000 episode ids".to_string(),
            ));
        }
        validate_json(&outcome.result_json, false)?;
        if outcome.result_json.len() > 64 * 1024 {
            return Err(CaptainError::Memory(
                "analysis result exceeds 64 KiB".to_string(),
            ));
        }
        match (outcome.status, outcome.proposal_id.as_deref()) {
            (WorkflowAnalysisOutcomeStatus::Processed, Some(id))
                if valid_analysis_identifier(id) => {}
            (WorkflowAnalysisOutcomeStatus::Rejected, None) => {}
            _ => return Err(CaptainError::Memory(
                "processed analysis requires one safe proposal id; rejected analysis forbids it"
                    .to_string(),
            )),
        }
        let mut ids = outcome.episode_ids.clone();
        ids.sort();
        ids.dedup();
        if ids.len() != outcome.episode_ids.len()
            || ids.iter().any(|id| !valid_analysis_identifier(id))
        {
            return Err(CaptainError::Memory(
                "analysis episode ids must be unique safe identifiers".to_string(),
            ));
        }

        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(memory_error)?;
        for id in ids {
            let existing = tx
                .query_row(
                    "SELECT status, completed_at, analysis_status,
                            analysis_result_json, analysis_proposal_id
                     FROM workflow_episodes WHERE id = ?1",
                    params![id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<i64>>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(memory_error)?
                .ok_or_else(|| CaptainError::Memory(format!("workflow episode {id} not found")))?;
            if existing.2 == outcome.status.as_str()
                && existing.3.as_deref() == Some(outcome.result_json.as_str())
                && existing.4 == outcome.proposal_id
            {
                continue;
            }
            if existing.2 != "pending" {
                return Err(CaptainError::Memory(format!(
                    "workflow episode {id} analysis is already {}",
                    existing.2
                )));
            }
            if existing.0 == "running" || existing.1.is_none() {
                return Err(CaptainError::Memory(format!(
                    "workflow episode {id} is not terminal"
                )));
            }
            let changed = tx
                .execute(
                    "UPDATE workflow_episodes
                     SET analysis_status = ?1, analysis_result_json = ?2,
                         analysis_proposal_id = ?3, analysis_updated_at = ?4,
                         updated_at = MAX(updated_at, ?4)
                     WHERE id = ?5 AND analysis_status = 'pending'",
                    params![
                        outcome.status.as_str(),
                        outcome.result_json,
                        outcome.proposal_id,
                        outcome.recorded_at_unix_ms,
                        id,
                    ],
                )
                .map_err(memory_error)?;
            if changed != 1 {
                return Err(CaptainError::Memory(format!(
                    "workflow episode {id} analysis changed concurrently"
                )));
            }
        }
        tx.commit().map_err(memory_error)
    }

    fn lock_conn(&self) -> CaptainResult<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|error| CaptainError::Internal(error.to_string()))
    }
}

fn list_steps_for_conn(
    conn: &Connection,
    episode_id: &str,
) -> CaptainResult<Vec<WorkflowEpisodeStepRecord>> {
    let mut stmt = conn
        .prepare(
            "SELECT episode_id, tool_use_id, ordinal, tool_name,
                        dependency_ids_json, input_shape_json, input_fingerprint,
                        effect_class, status, retry_count, output_class,
                        verification_marker, secret_detected, started_at,
                        completed_at, duration_ms
                 FROM workflow_episode_steps WHERE episode_id = ?1
                 ORDER BY ordinal, started_at, tool_use_id",
        )
        .map_err(memory_error)?;
    let rows = stmt
        .query_map(params![episode_id], step_from_row)
        .map_err(memory_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(memory_error)
}

fn validate_json(value: &str, require_array: bool) -> CaptainResult<()> {
    let parsed: serde_json::Value = serde_json::from_str(value)
        .map_err(|error| CaptainError::Memory(format!("invalid normalized JSON: {error}")))?;
    if require_array && !parsed.is_array() {
        return Err(CaptainError::Memory(
            "dependency_ids_json must be a JSON array".to_string(),
        ));
    }
    Ok(())
}

fn validate_effect(effect: &str) -> CaptainResult<()> {
    if matches!(
        effect,
        "read" | "write" | "external" | "destructive" | "unknown"
    ) {
        Ok(())
    } else {
        Err(CaptainError::Memory(format!(
            "invalid workflow effect class: {effect}"
        )))
    }
}

fn step_exists(conn: &Connection, episode_id: &str, tool_use_id: &str) -> CaptainResult<bool> {
    conn.query_row(
        "SELECT 1 FROM workflow_episode_steps WHERE episode_id = ?1 AND tool_use_id = ?2",
        params![episode_id, tool_use_id],
        |_| Ok(()),
    )
    .optional()
    .map(|value| value.is_some())
    .map_err(memory_error)
}

fn episode_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowEpisodeRecord> {
    Ok(WorkflowEpisodeRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        turn_id: row.get(2)?,
        agent_id: row.get(3)?,
        origin_channel: row.get(4)?,
        project_id: row.get(5)?,
        workspace_scope: row.get(6)?,
        intent_redacted: row.get(7)?,
        intent_fingerprint: row.get(8)?,
        status: row.get(9)?,
        explicit_reuse_request: row.get(10)?,
        tool_attempt_count: row.get(11)?,
        success_count: row.get(12)?,
        failure_count: row.get(13)?,
        has_secret_input: row.get(14)?,
        has_unverified_mutation: row.get(15)?,
        failure_reason: row.get(16)?,
        started_at_unix_ms: row.get(17)?,
        completed_at_unix_ms: row.get(18)?,
        analysis_status: row.get(19)?,
        analysis_result_json: row.get(20)?,
        analysis_proposal_id: row.get(21)?,
        analysis_updated_at_unix_ms: row.get(22)?,
    })
}

fn step_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowEpisodeStepRecord> {
    Ok(WorkflowEpisodeStepRecord {
        episode_id: row.get(0)?,
        tool_use_id: row.get(1)?,
        ordinal: row.get(2)?,
        tool_name: row.get(3)?,
        dependency_ids_json: row.get(4)?,
        input_shape_json: row.get(5)?,
        input_fingerprint: row.get(6)?,
        effect_class: row.get(7)?,
        status: row.get(8)?,
        retry_count: row.get(9)?,
        output_class: row.get(10)?,
        verification_marker: row.get(11)?,
        secret_detected: row.get(12)?,
        started_at_unix_ms: row.get(13)?,
        completed_at_unix_ms: row.get(14)?,
        duration_ms: row.get(15)?,
    })
}

fn memory_error(error: rusqlite::Error) -> CaptainError {
    CaptainError::Memory(error.to_string())
}

fn valid_analysis_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn setup() -> WorkflowEpisodeStore {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        WorkflowEpisodeStore::new(Arc::new(Mutex::new(conn)))
    }

    fn episode(id: &str, turn_id: &str) -> NewWorkflowEpisode {
        NewWorkflowEpisode {
            id: id.to_string(),
            session_id: "session-a".to_string(),
            turn_id: turn_id.to_string(),
            agent_id: "captain".to_string(),
            origin_channel: Some("telegram".to_string()),
            project_id: None,
            workspace_scope: Some("workspace:abc".to_string()),
            intent_redacted: "inspect <host>".to_string(),
            intent_fingerprint: "intent-hash".to_string(),
            secret_detected: false,
            explicit_reuse_request: false,
            started_at_unix_ms: 100,
        }
    }

    fn step(episode_id: &str, tool_use_id: &str, effect: &str) -> NewWorkflowEpisodeStep {
        NewWorkflowEpisodeStep {
            episode_id: episode_id.to_string(),
            tool_use_id: tool_use_id.to_string(),
            ordinal: 0,
            tool_name: "ssh_health_check".to_string(),
            dependency_ids_json: "[]".to_string(),
            input_shape_json: r#"{"host":"<id>"}"#.to_string(),
            input_fingerprint: "input-hash".to_string(),
            effect_class: effect.to_string(),
            secret_detected: false,
            started_at_unix_ms: 110,
        }
    }

    #[test]
    fn records_and_closes_an_episode() {
        let store = setup();
        assert_eq!(
            store.begin_episode(&episode("ep-1", "turn-1")).unwrap(),
            "ep-1"
        );
        assert!(store.begin_step(&step("ep-1", "tool-1", "read")).unwrap());
        assert!(store
            .finish_step(
                "ep-1",
                "tool-1",
                &WorkflowStepOutcome {
                    status: WorkflowStepStatus::Succeeded,
                    output_class: Some("health_report".to_string()),
                    verification_marker: Some("exit_zero".to_string()),
                    retry_count: 0,
                    completed_at_unix_ms: 130,
                },
            )
            .unwrap());
        assert!(store
            .finish_episode("ep-1", WorkflowEpisodeStatus::Succeeded, None, 140)
            .unwrap());

        let record = store.get_episode("ep-1").unwrap().unwrap();
        assert_eq!(record.status, "succeeded");
        assert_eq!(record.tool_attempt_count, 1);
        assert_eq!(record.success_count, 1);
        assert_eq!(record.failure_count, 0);
        assert_eq!(store.list_steps("ep-1").unwrap()[0].status, "succeeded");
    }

    #[test]
    fn retries_do_not_double_count_steps_or_terminal_updates() {
        let store = setup();
        store.begin_episode(&episode("ep-1", "turn-1")).unwrap();
        let new_step = step("ep-1", "tool-1", "read");
        assert!(store.begin_step(&new_step).unwrap());
        assert!(!store.begin_step(&new_step).unwrap());
        let outcome = WorkflowStepOutcome {
            status: WorkflowStepStatus::Succeeded,
            output_class: None,
            verification_marker: Some("observed".to_string()),
            retry_count: 0,
            completed_at_unix_ms: 120,
        };
        assert!(store.finish_step("ep-1", "tool-1", &outcome).unwrap());
        assert!(!store.finish_step("ep-1", "tool-1", &outcome).unwrap());
        let record = store.get_episode("ep-1").unwrap().unwrap();
        assert_eq!(record.tool_attempt_count, 1);
        assert_eq!(record.success_count, 1);
    }

    #[test]
    fn duplicate_turn_returns_the_authoritative_episode_id() {
        let store = setup();
        store.begin_episode(&episode("ep-first", "turn-1")).unwrap();
        assert_eq!(
            store.begin_episode(&episode("ep-retry", "turn-1")).unwrap(),
            "ep-first"
        );
    }

    #[test]
    fn pending_evidence_is_bounded_to_unclaimed_terminal_episodes() {
        let store = setup();
        store
            .begin_episode(&episode("ep-ready", "turn-ready"))
            .unwrap();
        store
            .begin_step(&step("ep-ready", "tool-ready", "read"))
            .unwrap();
        store
            .finish_step(
                "ep-ready",
                "tool-ready",
                &WorkflowStepOutcome {
                    status: WorkflowStepStatus::Succeeded,
                    output_class: Some("tool_success".to_string()),
                    verification_marker: Some("result_received".to_string()),
                    retry_count: 0,
                    completed_at_unix_ms: 120,
                },
            )
            .unwrap();
        store
            .finish_episode("ep-ready", WorkflowEpisodeStatus::Succeeded, None, 130)
            .unwrap();

        store
            .begin_episode(&episode("ep-claimed", "turn-claimed"))
            .unwrap();
        store
            .finish_episode(
                "ep-claimed",
                WorkflowEpisodeStatus::Failed,
                Some("test_failure"),
                140,
            )
            .unwrap();
        store
            .lock_conn()
            .unwrap()
            .execute(
                "UPDATE workflow_episodes SET analysis_status = 'claimed' WHERE id = ?1",
                params!["ep-claimed"],
            )
            .unwrap();
        store
            .begin_episode(&episode("ep-running", "turn-running"))
            .unwrap();

        let evidence = store.list_pending_evidence(10).unwrap();
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].episode.id, "ep-ready");
        assert_eq!(evidence[0].steps.len(), 1);
        assert_eq!(evidence[0].steps[0].tool_use_id, "tool-ready");
        assert!(store.list_pending_evidence(0).unwrap().is_empty());

        let summary = store.reconcile_incomplete().unwrap();
        assert_eq!(summary.analysis_claims_released, 1);
        assert_eq!(
            store
                .get_episode("ep-claimed")
                .unwrap()
                .unwrap()
                .analysis_status,
            "pending"
        );
    }

    #[test]
    fn successful_close_refuses_a_running_step() {
        let store = setup();
        store.begin_episode(&episode("ep-1", "turn-1")).unwrap();
        store
            .begin_step(&step("ep-1", "tool-1", "external"))
            .unwrap();
        let error = store
            .finish_episode("ep-1", WorkflowEpisodeStatus::Succeeded, None, 130)
            .unwrap_err();
        assert!(error.to_string().contains("running step"));
    }

    #[test]
    fn analysis_outcome_is_exact_idempotent_and_audited() {
        let store = setup();
        for (id, turn) in [("ep-a", "turn-a"), ("ep-b", "turn-b")] {
            store.begin_episode(&episode(id, turn)).unwrap();
            store
                .finish_episode(id, WorkflowEpisodeStatus::Succeeded, None, 150)
                .unwrap();
        }
        let outcome = WorkflowAnalysisOutcome {
            episode_ids: vec!["ep-b".to_string(), "ep-a".to_string()],
            status: WorkflowAnalysisOutcomeStatus::Processed,
            result_json: r#"{"classification":"skill"}"#.to_string(),
            proposal_id: Some("proposal-analysis".to_string()),
            recorded_at_unix_ms: 200,
        };
        store.record_analysis_outcome(&outcome).unwrap();
        store.record_analysis_outcome(&outcome).unwrap();
        let record = store.get_episode("ep-a").unwrap().unwrap();
        assert_eq!(record.analysis_status, "processed");
        assert_eq!(
            record.analysis_proposal_id.as_deref(),
            Some("proposal-analysis")
        );
        assert_eq!(
            record.analysis_result_json,
            Some(outcome.result_json.clone())
        );
        assert_eq!(record.analysis_updated_at_unix_ms, Some(200));
        let mut conflicting = outcome;
        conflicting.result_json = r#"{"classification":"capspec"}"#.to_string();
        assert!(store.record_analysis_outcome(&conflicting).is_err());
        assert!(store.list_pending_evidence(10).unwrap().is_empty());
    }

    #[test]
    fn restart_reconciles_running_work_as_uncertain() {
        let store = setup();
        store.begin_episode(&episode("ep-1", "turn-1")).unwrap();
        store.begin_step(&step("ep-1", "tool-1", "write")).unwrap();

        let summary = store.reconcile_incomplete().unwrap();
        assert_eq!(summary.episodes_reconciled, 1);
        assert_eq!(summary.steps_interrupted, 1);
        assert_eq!(summary.analysis_claims_released, 0);
        let record = store.get_episode("ep-1").unwrap().unwrap();
        assert_eq!(record.status, "uncertain");
        assert_eq!(record.failure_count, 1);
        assert!(record.has_unverified_mutation);
        assert_eq!(store.list_steps("ep-1").unwrap()[0].status, "interrupted");

        assert_eq!(
            store.reconcile_incomplete().unwrap(),
            WorkflowRecoverySummary {
                episodes_reconciled: 0,
                steps_interrupted: 0,
                analysis_claims_released: 0,
            }
        );
    }
}
