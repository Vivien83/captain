//! Write-through persistence buffer for MemPalace (v3.12a).
//!
//! Every call that writes a semantic triple to MemPalace (from the
//! `memory_store` tool, from `mirror_to_mempalace`, or from the future
//! `LearningCommitter`) is first captured here in SQLite. A background
//! worker then attempts the actual MemPalace `kg_add` / `drawer_add`
//! call and flips the row's `sync_status` when it succeeds.
//!
//! The key invariant: local SQLite is Captain's durable continuity journal.
//! MemPalace is the semantic index fed from that journal. A backend outage,
//! exhausted retry budget, process crash, or restart must never make a locally
//! accepted fact disappear from recall or become permanently unreplayable.
//!
//! States:
//! - `pending` — not yet acknowledged by MemPalace (or retry in flight)
//! - `synced`  — MemPalace confirmed the write (permanent audit trail)
//! - `error`   — exceeded `MAX_SYNC_ATTEMPTS` consecutive failures and is in
//!   durable backoff; it remains visible and eligible for later recovery.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

/// Consecutive failures before a row is surfaced as degraded. `Error` is an
/// observable backoff state, not a terminal/deleted state.
pub const MAX_SYNC_ATTEMPTS: i32 = 3;
pub const INITIAL_RETRY_DELAY_MS: i64 = 30_000;
pub const MAX_RETRY_DELAY_MS: i64 = 3_600_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryOperation {
    Add,
    Invalidate,
}

impl MemoryOperation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Invalidate => "invalidate",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "add" => Some(Self::Add),
            "invalidate" => Some(Self::Invalidate),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncStatus {
    Pending,
    Synced,
    Error,
}

impl SyncStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SyncStatus::Pending => "pending",
            SyncStatus::Synced => "synced",
            SyncStatus::Error => "error",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "synced" => Some(Self::Synced),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NewMemoryWrite {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub wing: Option<String>,
    pub room: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryWrite {
    pub id: String,
    pub operation: MemoryOperation,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub wing: Option<String>,
    pub room: Option<String>,
    pub source: String,
    pub sync_status: SyncStatus,
    pub sync_attempts: i32,
    pub created_at: i64,
    pub synced_at: Option<i64>,
    pub last_error: Option<String>,
    pub last_attempt_at: Option<i64>,
    pub next_retry_at: Option<i64>,
    pub retracted_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RetractionBatch {
    pub retracted: Vec<MemoryWrite>,
    pub invalidations: Vec<MemoryWrite>,
}

/// Insert a new row in `pending` state. Local persistence always succeeds
/// (modulo IO) — never fails because MemPalace is down.
pub fn append(conn: &Connection, input: NewMemoryWrite) -> Result<MemoryWrite, rusqlite::Error> {
    append_operation(conn, input, MemoryOperation::Add)
}

pub fn append_invalidation(
    conn: &Connection,
    input: NewMemoryWrite,
) -> Result<MemoryWrite, rusqlite::Error> {
    append_operation(conn, input, MemoryOperation::Invalidate)
}

/// Return the latest already-journaled invalidation for an exact triple, or
/// append one when no such terminal operation exists. This supports forgetting
/// legacy MemPalace facts that predate Captain's local journal without
/// generating duplicates on repeated requests.
pub fn ensure_exact_invalidation(
    conn: &Connection,
    input: NewMemoryWrite,
) -> Result<(MemoryWrite, bool), rusqlite::Error> {
    let latest = {
        let mut stmt = conn.prepare(
            "SELECT id, operation, subject, predicate, object, wing, room, source,
                    sync_status, sync_attempts, created_at, synced_at, last_error,
                    last_attempt_at, next_retry_at, retracted_at
             FROM memory_writes
             WHERE subject = ?1 AND predicate = ?2 AND object = ?3
             ORDER BY created_at DESC,
                      CASE operation WHEN 'invalidate' THEN 1 ELSE 0 END DESC,
                      id DESC
             LIMIT 1",
        )?;
        stmt.query_row(
            params![&input.subject, &input.predicate, &input.object],
            row_to_memory_write,
        )
        .optional()?
    };
    if let Some(row) = latest.filter(|row| row.operation == MemoryOperation::Invalidate) {
        return Ok((row, false));
    }
    append_invalidation(conn, input).map(|row| (row, true))
}

fn append_operation(
    conn: &Connection,
    input: NewMemoryWrite,
    operation: MemoryOperation,
) -> Result<MemoryWrite, rusqlite::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_ms();
    conn.execute(
        "INSERT INTO memory_writes
         (id, operation, subject, predicate, object, wing, room, source,
          sync_status, sync_attempts, created_at, synced_at, last_error,
          last_attempt_at, next_retry_at, retracted_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', 0, ?9,
                 NULL, NULL, NULL, NULL, NULL)",
        params![
            id,
            operation.as_str(),
            input.subject,
            input.predicate,
            input.object,
            input.wing,
            input.room,
            input.source,
            now,
        ],
    )?;
    get(conn, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<MemoryWrite>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, operation, subject, predicate, object, wing, room, source,
                sync_status, sync_attempts, created_at, synced_at, last_error,
                last_attempt_at, next_retry_at, retracted_at
         FROM memory_writes WHERE id = ?1",
    )?;
    stmt.query_row(params![id], row_to_memory_write).optional()
}

/// Mark a row as successfully synced to MemPalace.
pub fn mark_synced(conn: &Connection, id: &str) -> Result<(), rusqlite::Error> {
    let now = now_ms();
    conn.execute(
        "UPDATE memory_writes
         SET sync_status = 'synced', synced_at = ?1, last_error = NULL,
             last_attempt_at = ?1, next_retry_at = NULL
         WHERE id = ?2",
        params![now, id],
    )?;
    Ok(())
}

/// Record a sync failure with durable exponential backoff. Returns `Error`
/// once the alert threshold is reached, but the row remains retryable.
pub fn mark_error(conn: &Connection, id: &str, error: &str) -> Result<SyncStatus, rusqlite::Error> {
    let current = get(conn, id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    let new_attempts = current.sync_attempts + 1;
    let new_status = if new_attempts >= MAX_SYNC_ATTEMPTS {
        SyncStatus::Error
    } else {
        SyncStatus::Pending
    };
    let attempted_at = now_ms();
    let next_retry_at = attempted_at.saturating_add(retry_delay_ms(new_attempts));
    let bounded_error: String = error.chars().take(1_000).collect();
    conn.execute(
        "UPDATE memory_writes
         SET sync_status = ?1, sync_attempts = ?2, last_error = ?3,
             last_attempt_at = ?4, next_retry_at = ?5
         WHERE id = ?6",
        params![
            new_status.as_str(),
            new_attempts,
            bounded_error,
            attempted_at,
            next_retry_at,
            id
        ],
    )?;
    Ok(new_status)
}

/// Keep a row queued when no sender exists without consuming a real backend
/// attempt. This is common during startup before the MCP connection is ready.
pub fn mark_deferred(conn: &Connection, id: &str, reason: &str) -> Result<(), rusqlite::Error> {
    let now = now_ms();
    let next_retry_at = now.saturating_add(INITIAL_RETRY_DELAY_MS);
    let bounded_reason: String = reason.chars().take(1_000).collect();
    conn.execute(
        "UPDATE memory_writes
             SET sync_status = CASE
                 WHEN sync_status = 'error' THEN 'error'
                 ELSE 'pending'
             END,
             last_error = CASE
                 WHEN sync_status = 'error' THEN last_error
                 ELSE ?1
             END,
             next_retry_at = CASE
                 WHEN sync_status = 'error' THEN next_retry_at
                 ELSE ?2
             END
         WHERE id = ?3",
        params![bounded_reason, next_retry_at, id],
    )?;
    Ok(())
}

pub fn retry_delay_ms(attempts: i32) -> i64 {
    let exponent = attempts.saturating_sub(1).clamp(0, 16) as u32;
    INITIAL_RETRY_DELAY_MS
        .saturating_mul(1_i64.checked_shl(exponent).unwrap_or(i64::MAX))
        .min(MAX_RETRY_DELAY_MS)
}

/// List rows with `sync_status='pending'`, oldest first — what the
/// resync worker iterates.
pub fn list_pending(conn: &Connection, limit: usize) -> Result<Vec<MemoryWrite>, rusqlite::Error> {
    let cap = limit.min(10_000) as i64;
    let mut stmt = conn.prepare(
        "SELECT id, operation, subject, predicate, object, wing, room, source,
                sync_status, sync_attempts, created_at, synced_at, last_error,
                last_attempt_at, next_retry_at, retracted_at
         FROM memory_writes
         WHERE sync_status = 'pending' AND retracted_at IS NULL
         ORDER BY created_at ASC, id ASC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![cap], row_to_memory_write)?;
    rows.collect()
}

/// List every active unsynced operation whose durable backoff has elapsed.
/// Error rows deliberately remain eligible: they are degraded, never dead.
pub fn list_retryable(
    conn: &Connection,
    retry_at_ms: i64,
    limit: usize,
) -> Result<Vec<MemoryWrite>, rusqlite::Error> {
    let cap = limit.min(10_000) as i64;
    let mut stmt = conn.prepare(
        "SELECT id, operation, subject, predicate, object, wing, room, source,
                sync_status, sync_attempts, created_at, synced_at, last_error,
                last_attempt_at, next_retry_at, retracted_at
         FROM memory_writes
         WHERE sync_status IN ('pending', 'error')
           AND retracted_at IS NULL
           AND COALESCE(next_retry_at, 0) <= ?1
         ORDER BY created_at ASC, id ASC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![retry_at_ms, cap], row_to_memory_write)?;
    rows.collect()
}

/// Retract matching active add rows and atomically enqueue one MemPalace
/// invalidation per row. The original rows remain as an immutable audit trail;
/// active recall and resync exclude them immediately.
pub fn retract_by_match(
    conn: &Connection,
    subject_like: Option<&str>,
    predicate_like: Option<&str>,
    object_like: Option<&str>,
    source: &str,
) -> Result<RetractionBatch, rusqlite::Error> {
    if subject_like.is_none() && predicate_like.is_none() && object_like.is_none() {
        return Ok(RetractionBatch::default());
    }
    let mut sql = String::from(
        "SELECT id, operation, subject, predicate, object, wing, room, source,
                sync_status, sync_attempts, created_at, synced_at, last_error,
                last_attempt_at, next_retry_at, retracted_at
         FROM memory_writes
         WHERE operation = 'add' AND retracted_at IS NULL",
    );
    let mut args: Vec<String> = Vec::new();
    if let Some(pattern) = subject_like {
        sql.push_str(" AND subject LIKE ?");
        args.push(pattern.to_string());
    }
    if let Some(pattern) = predicate_like {
        sql.push_str(" AND predicate LIKE ?");
        args.push(pattern.to_string());
    }
    if let Some(pattern) = object_like {
        sql.push_str(" AND object LIKE ?");
        args.push(pattern.to_string());
    }
    sql.push_str(" ORDER BY created_at ASC, id ASC");
    let arg_refs: Vec<&dyn rusqlite::ToSql> = args
        .iter()
        .map(|value| value as &dyn rusqlite::ToSql)
        .collect();
    let matched = {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(arg_refs.as_slice(), row_to_memory_write)?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    let now = now_ms();
    let tx = conn.unchecked_transaction()?;
    let mut batch = RetractionBatch::default();
    for mut row in matched {
        let changed = tx.execute(
            "UPDATE memory_writes
             SET retracted_at = ?1
             WHERE id = ?2 AND retracted_at IS NULL",
            params![now, row.id],
        )?;
        if changed == 0 {
            continue;
        }
        row.retracted_at = Some(now);
        let invalidation = append_operation(
            &tx,
            NewMemoryWrite {
                subject: row.subject.clone(),
                predicate: row.predicate.clone(),
                object: row.object.clone(),
                wing: row.wing.clone(),
                room: row.room.clone(),
                source: format!("{source}:{}", row.id),
            },
            MemoryOperation::Invalidate,
        )?;
        batch.retracted.push(row);
        batch.invalidations.push(invalidation);
    }
    tx.commit()?;
    Ok(batch)
}

/// Low-level destructive maintenance helper retained for compatibility.
/// User-facing correction paths must use `retract_by_match` so audit history
/// and remote invalidations survive crashes.
pub fn delete_by_match(
    conn: &Connection,
    subject_like: Option<&str>,
    predicate_like: Option<&str>,
    object_like: Option<&str>,
) -> Result<usize, rusqlite::Error> {
    if subject_like.is_none() && predicate_like.is_none() && object_like.is_none() {
        return Ok(0);
    }
    let mut sql = String::from("DELETE FROM memory_writes WHERE 1=1");
    let mut args: Vec<String> = Vec::new();
    if let Some(p) = subject_like {
        sql.push_str(" AND subject LIKE ?");
        args.push(p.to_string());
    }
    if let Some(p) = predicate_like {
        sql.push_str(" AND predicate LIKE ?");
        args.push(p.to_string());
    }
    if let Some(p) = object_like {
        sql.push_str(" AND object LIKE ?");
        args.push(p.to_string());
    }
    let arg_refs: Vec<&dyn rusqlite::ToSql> =
        args.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    conn.execute(&sql, arg_refs.as_slice())
}

/// Count rows in a given sync state (for metrics / dashboard).
pub fn count_by_status(conn: &Connection, status: SyncStatus) -> Result<i64, rusqlite::Error> {
    let mut stmt = conn.prepare("SELECT COUNT(*) FROM memory_writes WHERE sync_status = ?1")?;
    stmt.query_row(params![status.as_str()], |row| row.get(0))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct JournalHealth {
    pub total: i64,
    pub synced: i64,
    pub pending: i64,
    pub error: i64,
    pub retracted: i64,
    pub oldest_unsynced_at: Option<i64>,
    pub next_retry_at: Option<i64>,
    pub max_sync_attempts: i64,
    pub last_sync_error: Option<String>,
}

pub fn journal_health(conn: &Connection) -> Result<JournalHealth, rusqlite::Error> {
    let (total, synced, pending, error, retracted) = conn.query_row(
        "SELECT
            COUNT(*),
            SUM(CASE WHEN sync_status = 'synced' THEN 1 ELSE 0 END),
            SUM(CASE WHEN sync_status = 'pending' AND retracted_at IS NULL THEN 1 ELSE 0 END),
            SUM(CASE WHEN sync_status = 'error' AND retracted_at IS NULL THEN 1 ELSE 0 END),
            SUM(CASE WHEN retracted_at IS NOT NULL THEN 1 ELSE 0 END)
         FROM memory_writes",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                row.get::<_, Option<i64>>(4)?.unwrap_or(0),
            ))
        },
    )?;
    let (oldest_unsynced_at, next_retry_at, max_sync_attempts) = conn.query_row(
        "SELECT MIN(created_at), MIN(next_retry_at), MAX(sync_attempts)
         FROM memory_writes
         WHERE sync_status IN ('pending', 'error') AND retracted_at IS NULL",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get::<_, Option<i64>>(2)?.unwrap_or(0),
            ))
        },
    )?;
    let last_sync_error = conn
        .query_row(
            "SELECT last_error FROM memory_writes
             WHERE sync_status IN ('pending', 'error')
               AND retracted_at IS NULL
               AND last_error IS NOT NULL
             ORDER BY COALESCE(last_attempt_at, created_at) DESC, id DESC
             LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|error| error.chars().take(500).collect());
    Ok(JournalHealth {
        total,
        synced,
        pending,
        error,
        retracted,
        oldest_unsynced_at,
        next_retry_at,
        max_sync_attempts,
        last_sync_error,
    })
}

/// List recent rows whose source matches an optional prefix, newest
/// first. Used by the /learning dashboard to tail the audit log.
pub fn list_recent(
    conn: &Connection,
    source_prefix: Option<&str>,
    limit: usize,
) -> Result<Vec<MemoryWrite>, rusqlite::Error> {
    let cap = limit.min(10_000) as i64;
    let (sql, prefix_owned) = match source_prefix {
        Some(p) => (
            "SELECT id, operation, subject, predicate, object, wing, room, source,
                    sync_status, sync_attempts, created_at, synced_at, last_error,
                    last_attempt_at, next_retry_at, retracted_at
             FROM memory_writes
             WHERE source LIKE ?1
             ORDER BY created_at DESC, id DESC
             LIMIT ?2",
            format!("{p}%"),
        ),
        None => (
            "SELECT id, operation, subject, predicate, object, wing, room, source,
                    sync_status, sync_attempts, created_at, synced_at, last_error,
                    last_attempt_at, next_retry_at, retracted_at
             FROM memory_writes
             ORDER BY created_at DESC, id DESC
             LIMIT ?2",
            String::new(),
        ),
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = if source_prefix.is_some() {
        stmt.query_map(params![prefix_owned, cap], row_to_memory_write)?
            .collect::<Result<Vec<_>, _>>()?
    } else {
        stmt.query_map(params![0_i64, cap], row_to_memory_write)?
            .collect::<Result<Vec<_>, _>>()?
    };
    Ok(rows)
}

/// List recent active facts for prompt injection and local recall. Retracted
/// rows remain available through `list_recent` for audit, but never re-enter
/// active context.
pub fn list_recent_active(
    conn: &Connection,
    limit: usize,
) -> Result<Vec<MemoryWrite>, rusqlite::Error> {
    let cap = limit.min(10_000) as i64;
    let mut stmt = conn.prepare(
        "SELECT id, operation, subject, predicate, object, wing, room, source,
                sync_status, sync_attempts, created_at, synced_at, last_error,
                last_attempt_at, next_retry_at, retracted_at
         FROM memory_writes
         WHERE retracted_at IS NULL AND operation = 'add'
         ORDER BY created_at DESC, id DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![cap], row_to_memory_write)?;
    rows.collect()
}

/// Search the complete active journal for facts containing any supplied term.
/// Results are ranked by lexical coverage, then recency, so an older precise
/// fact cannot disappear behind a busy stream of unrelated recent writes.
pub fn search_active_by_terms(
    conn: &Connection,
    terms: &[String],
    limit: usize,
) -> Result<Vec<MemoryWrite>, rusqlite::Error> {
    const MAX_SEARCH_TERMS: usize = 12;
    let mut seen = HashSet::new();
    let mut terms = terms
        .iter()
        .map(|term| term.trim().to_lowercase())
        .filter(|term| !term.is_empty() && seen.insert(term.clone()))
        .collect::<Vec<_>>();
    terms.sort_by(|left, right| {
        term_is_precise_anchor(right)
            .cmp(&term_is_precise_anchor(left))
            .then_with(|| right.chars().count().cmp(&left.chars().count()))
    });
    terms.truncate(MAX_SEARCH_TERMS);
    if terms.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }

    let haystack = "lower(subject || ' ' || predicate || ' ' || object)";
    let matches = (1..=terms.len())
        .map(|index| format!("instr({haystack}, ?{index}) > 0"))
        .collect::<Vec<_>>();
    let score = (1..=terms.len())
        .map(|index| format!("CASE WHEN instr({haystack}, ?{index}) > 0 THEN 1 ELSE 0 END"))
        .collect::<Vec<_>>();
    let limit_index = terms.len() + 1;
    let sql = format!(
        "SELECT id, operation, subject, predicate, object, wing, room, source,
                sync_status, sync_attempts, created_at, synced_at, last_error,
                last_attempt_at, next_retry_at, retracted_at
         FROM memory_writes
         WHERE retracted_at IS NULL
           AND operation = 'add'
           AND ({})
         ORDER BY ({}) DESC, created_at DESC, id DESC
         LIMIT ?{limit_index}",
        matches.join(" OR "),
        score.join(" + "),
    );
    let mut args = terms
        .into_iter()
        .map(rusqlite::types::Value::Text)
        .collect::<Vec<_>>();
    args.push(rusqlite::types::Value::Integer(limit.min(10_000) as i64));
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), row_to_memory_write)?;
    rows.collect()
}

fn term_is_precise_anchor(term: &str) -> bool {
    term.chars().any(|character| character.is_ascii_digit())
        && term.chars().any(char::is_alphabetic)
}

/// List retracted add rows newest first so active-context guards can be
/// reconstructed after a crash even if the auxiliary KV snapshot was not yet
/// written.
pub fn list_recent_retracted(
    conn: &Connection,
    limit: usize,
) -> Result<Vec<MemoryWrite>, rusqlite::Error> {
    let cap = limit.min(10_000) as i64;
    let mut stmt = conn.prepare(
        "SELECT id, operation, subject, predicate, object, wing, room, source,
                sync_status, sync_attempts, created_at, synced_at, last_error,
                last_attempt_at, next_retry_at, retracted_at
         FROM memory_writes
         WHERE retracted_at IS NOT NULL AND operation = 'add'
         ORDER BY retracted_at DESC, created_at DESC, id DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![cap], row_to_memory_write)?;
    rows.collect()
}

fn row_to_memory_write(row: &rusqlite::Row<'_>) -> Result<MemoryWrite, rusqlite::Error> {
    let operation_str: String = row.get(1)?;
    let operation = MemoryOperation::parse(&operation_str).unwrap_or(MemoryOperation::Add);
    let status_str: String = row.get(8)?;
    let sync_status = SyncStatus::parse(&status_str).unwrap_or(SyncStatus::Pending);
    Ok(MemoryWrite {
        id: row.get(0)?,
        operation,
        subject: row.get(2)?,
        predicate: row.get(3)?,
        object: row.get(4)?,
        wing: row.get(5)?,
        room: row.get(6)?,
        source: row.get(7)?,
        sync_status,
        sync_attempts: row.get(9)?,
        created_at: row.get(10)?,
        synced_at: row.get(11)?,
        last_error: row.get(12)?,
        last_attempt_at: row.get(13)?,
        next_retry_at: row.get(14)?,
        retracted_at: row.get(15)?,
    })
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn fresh_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    fn sample(source: &str) -> NewMemoryWrite {
        NewMemoryWrite {
            subject: "alice".into(),
            predicate: "likes".into(),
            object: "coffee".into(),
            wing: None,
            room: None,
            source: source.into(),
        }
    }

    #[test]
    fn append_creates_pending_row_with_zero_attempts() {
        let conn = fresh_db();
        let row = append(&conn, sample("memory_store")).unwrap();
        assert_eq!(row.operation, MemoryOperation::Add);
        assert_eq!(row.sync_status, SyncStatus::Pending);
        assert_eq!(row.sync_attempts, 0);
        assert!(row.synced_at.is_none());
        assert!(row.last_error.is_none());
        assert_eq!(row.source, "memory_store");
    }

    #[test]
    fn get_returns_none_for_unknown_id() {
        let conn = fresh_db();
        assert!(get(&conn, "does-not-exist").unwrap().is_none());
    }

    #[test]
    fn mark_synced_sets_status_and_synced_at() {
        let conn = fresh_db();
        let row = append(&conn, sample("learning.success")).unwrap();
        mark_synced(&conn, &row.id).unwrap();
        let after = get(&conn, &row.id).unwrap().unwrap();
        assert_eq!(after.sync_status, SyncStatus::Synced);
        assert!(after.synced_at.is_some());
        assert!(after.last_error.is_none());
    }

    #[test]
    fn mark_error_keeps_pending_until_cap() {
        let conn = fresh_db();
        let row = append(&conn, sample("mirror")).unwrap();

        // 1st failure — still pending
        let s1 = mark_error(&conn, &row.id, "conn refused").unwrap();
        assert_eq!(s1, SyncStatus::Pending);
        let r1 = get(&conn, &row.id).unwrap().unwrap();
        assert_eq!(r1.sync_attempts, 1);
        assert_eq!(r1.last_error.as_deref(), Some("conn refused"));

        // 2nd failure — still pending
        let s2 = mark_error(&conn, &row.id, "timeout").unwrap();
        assert_eq!(s2, SyncStatus::Pending);
        assert_eq!(get(&conn, &row.id).unwrap().unwrap().sync_attempts, 2);

        // 3rd failure — transitions to Error
        let s3 = mark_error(&conn, &row.id, "dns fail").unwrap();
        assert_eq!(s3, SyncStatus::Error);
        let r3 = get(&conn, &row.id).unwrap().unwrap();
        assert_eq!(r3.sync_attempts, 3);
        assert_eq!(r3.sync_status, SyncStatus::Error);
        assert!(r3.last_attempt_at.is_some());
        assert!(r3.next_retry_at > r3.last_attempt_at);
    }

    #[test]
    fn mark_error_unknown_id_returns_no_rows() {
        let conn = fresh_db();
        let err = mark_error(&conn, "nope", "oops").unwrap_err();
        matches!(err, rusqlite::Error::QueryReturnedNoRows);
    }

    #[test]
    fn list_pending_oldest_first_and_excludes_synced() {
        let conn = fresh_db();
        let a = append(&conn, sample("a")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = append(&conn, sample("b")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let c = append(&conn, sample("c")).unwrap();

        // b transitions to synced — should disappear from pending
        mark_synced(&conn, &b.id).unwrap();

        let pending = list_pending(&conn, 100).unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].id, a.id); // oldest first
        assert_eq!(pending[1].id, c.id);
    }

    #[test]
    fn list_pending_respects_limit() {
        let conn = fresh_db();
        for _ in 0..5 {
            append(&conn, sample("bulk")).unwrap();
        }
        assert_eq!(list_pending(&conn, 3).unwrap().len(), 3);
        assert_eq!(list_pending(&conn, 10).unwrap().len(), 5);
    }

    #[test]
    fn list_pending_excludes_error_rows() {
        let conn = fresh_db();
        let row = append(&conn, sample("dead")).unwrap();
        for _ in 0..3 {
            mark_error(&conn, &row.id, "x").unwrap();
        }
        assert!(list_pending(&conn, 100).unwrap().is_empty());
    }

    #[test]
    fn error_rows_remain_durable_and_retryable_after_backoff() {
        let conn = fresh_db();
        let old_err = append(&conn, sample("e")).unwrap();
        for _ in 0..3 {
            mark_error(&conn, &old_err.id, "x").unwrap();
        }
        let forty_days_ms = 40 * 24 * 60 * 60 * 1000;
        let old_ts = now_ms() - forty_days_ms;
        conn.execute(
            "UPDATE memory_writes
             SET created_at = ?1, next_retry_at = 0
             WHERE id = ?2",
            params![old_ts, old_err.id],
        )
        .unwrap();

        let retryable = list_retryable(&conn, now_ms(), 100).unwrap();
        assert_eq!(retryable.len(), 1);
        assert_eq!(retryable[0].id, old_err.id);
        assert_eq!(retryable[0].sync_status, SyncStatus::Error);
        assert!(get(&conn, &old_err.id).unwrap().is_some());
    }

    #[test]
    fn retry_backoff_is_exponential_and_bounded() {
        assert_eq!(retry_delay_ms(1), 30_000);
        assert_eq!(retry_delay_ms(2), 60_000);
        assert_eq!(retry_delay_ms(3), 120_000);
        assert_eq!(retry_delay_ms(8), 3_600_000);
        assert_eq!(retry_delay_ms(i32::MAX), 3_600_000);
    }

    #[test]
    fn mark_deferred_does_not_consume_attempt_budget() {
        let conn = fresh_db();
        let row = append(&conn, sample("startup")).unwrap();
        mark_deferred(&conn, &row.id, "sender not ready").unwrap();
        let row = get(&conn, &row.id).unwrap().unwrap();
        assert_eq!(row.sync_attempts, 0);
        assert_eq!(row.sync_status, SyncStatus::Pending);
        assert_eq!(row.last_error.as_deref(), Some("sender not ready"));
        assert!(row.next_retry_at.is_some());
    }

    #[test]
    fn mark_deferred_does_not_hide_existing_degradation() {
        let conn = fresh_db();
        let row = append(&conn, sample("startup")).unwrap();
        for _ in 0..3 {
            mark_error(&conn, &row.id, "backend offline").unwrap();
        }
        mark_deferred(&conn, &row.id, "sender not ready").unwrap();
        let row = get(&conn, &row.id).unwrap().unwrap();
        assert_eq!(row.sync_status, SyncStatus::Error);
        assert_eq!(row.sync_attempts, 3);
        assert_eq!(row.last_error.as_deref(), Some("backend offline"));
    }

    #[test]
    fn count_by_status_matches_reality() {
        let conn = fresh_db();
        let a = append(&conn, sample("a")).unwrap();
        let b = append(&conn, sample("b")).unwrap();
        let c = append(&conn, sample("c")).unwrap();
        mark_synced(&conn, &a.id).unwrap();
        for _ in 0..3 {
            mark_error(&conn, &c.id, "x").unwrap();
        }
        let _ = b; // leave pending

        assert_eq!(count_by_status(&conn, SyncStatus::Synced).unwrap(), 1);
        assert_eq!(count_by_status(&conn, SyncStatus::Pending).unwrap(), 1);
        assert_eq!(count_by_status(&conn, SyncStatus::Error).unwrap(), 1);
    }

    #[test]
    fn journal_health_exposes_recovery_backlog() {
        let conn = fresh_db();
        let synced = append(&conn, sample("synced")).unwrap();
        mark_synced(&conn, &synced.id).unwrap();
        let degraded = append(&conn, sample("degraded")).unwrap();
        for _ in 0..3 {
            mark_error(&conn, &degraded.id, "backend offline").unwrap();
        }
        let health = journal_health(&conn).unwrap();
        assert_eq!(health.total, 2);
        assert_eq!(health.synced, 1);
        assert_eq!(health.error, 1);
        assert_eq!(health.max_sync_attempts, 3);
        assert!(health.oldest_unsynced_at.is_some());
        assert!(health.next_retry_at.is_some());
        assert_eq!(health.last_sync_error.as_deref(), Some("backend offline"));
    }

    #[test]
    fn sync_status_parse_roundtrip() {
        for s in [SyncStatus::Pending, SyncStatus::Synced, SyncStatus::Error] {
            assert_eq!(SyncStatus::parse(s.as_str()).unwrap(), s);
        }
        assert!(SyncStatus::parse("bogus").is_none());
    }

    #[test]
    fn list_recent_filters_by_source_prefix_and_orders_desc() {
        let conn = fresh_db();
        let mut a = sample("learning.success");
        a.source = "learning.success".into();
        append(&conn, a).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let mut b = sample("mirror.fact");
        b.source = "mirror.fact".into();
        append(&conn, b).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let mut c = sample("learning.failure");
        c.source = "learning.failure".into();
        append(&conn, c).unwrap();

        let all = list_recent(&conn, None, 10).unwrap();
        assert_eq!(all.len(), 3);
        // Newest first
        assert_eq!(all[0].source, "learning.failure");

        let only_learning = list_recent(&conn, Some("learning."), 10).unwrap();
        assert_eq!(only_learning.len(), 2);
        assert!(only_learning
            .iter()
            .all(|r| r.source.starts_with("learning.")));
    }

    #[test]
    fn delete_by_match_returns_zero_when_no_filter_supplied() {
        let conn = fresh_db();
        append(&conn, sample("a")).unwrap();
        let n = delete_by_match(&conn, None, None, None).unwrap();
        assert_eq!(n, 0);
        assert_eq!(count_by_status(&conn, SyncStatus::Pending).unwrap(), 1);
    }

    #[test]
    fn delete_by_match_subject_only() {
        let conn = fresh_db();
        let mut a = sample("ok");
        a.subject = "user".into();
        append(&conn, a).unwrap();
        let mut b = sample("ok");
        b.subject = "project".into();
        append(&conn, b).unwrap();
        let n = delete_by_match(&conn, Some("user"), None, None).unwrap();
        assert_eq!(n, 1);
        assert_eq!(count_by_status(&conn, SyncStatus::Pending).unwrap(), 1);
    }

    #[test]
    fn delete_by_match_supports_like_wildcards_on_object() {
        let conn = fresh_db();
        let mut a = sample("ok");
        a.object = "ancienne valeur utilisateur".into();
        append(&conn, a).unwrap();
        let mut b = sample("ok");
        b.object = "café noir".into();
        append(&conn, b).unwrap();
        let n = delete_by_match(&conn, None, None, Some("%ancienne%")).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn delete_by_match_combines_filters_with_and() {
        let conn = fresh_db();
        let mut a = sample("ok");
        a.subject = "user".into();
        a.predicate = "prefers".into();
        a.object = "ancienne valeur".into();
        append(&conn, a).unwrap();
        let mut b = sample("ok");
        b.subject = "user".into();
        b.predicate = "timezone".into();
        b.object = "Europe/Paris".into();
        append(&conn, b).unwrap();
        let n = delete_by_match(&conn, Some("user"), Some("prefers"), None).unwrap();
        assert_eq!(n, 1);
        assert_eq!(count_by_status(&conn, SyncStatus::Pending).unwrap(), 1);
    }

    #[test]
    fn retract_preserves_audit_and_queues_remote_invalidation() {
        let conn = fresh_db();
        let row = append(&conn, sample("memory_save:info")).unwrap();
        mark_synced(&conn, &row.id).unwrap();

        let batch = retract_by_match(
            &conn,
            Some("alice"),
            Some("likes"),
            Some("coffee"),
            "memory_forget",
        )
        .unwrap();
        assert_eq!(batch.retracted.len(), 1);
        assert_eq!(batch.invalidations.len(), 1);
        assert_eq!(
            batch.invalidations[0].operation,
            MemoryOperation::Invalidate
        );
        assert_eq!(batch.invalidations[0].sync_status, SyncStatus::Pending);

        let original = get(&conn, &row.id).unwrap().unwrap();
        assert!(original.retracted_at.is_some());
        assert!(list_recent_active(&conn, 10).unwrap().is_empty());
        assert_eq!(list_recent(&conn, None, 10).unwrap().len(), 2);

        let retryable = list_retryable(&conn, now_ms(), 10).unwrap();
        assert_eq!(retryable.len(), 1);
        assert_eq!(retryable[0].operation, MemoryOperation::Invalidate);
        let health = journal_health(&conn).unwrap();
        assert_eq!(health.pending, 1);
        assert_eq!(health.retracted, 1);
        let retracted = list_recent_retracted(&conn, 10).unwrap();
        assert_eq!(retracted.len(), 1);
        assert_eq!(retracted[0].id, row.id);
    }

    #[test]
    fn search_active_by_terms_finds_precise_fact_beyond_recent_noise() {
        let conn = fresh_db();
        let durable = append(
            &conn,
            NewMemoryWrite {
                subject: "user:vivien".into(),
                predicate: "prefers_certification_label".into(),
                object: "PUBLIC2 stages are called jalons ambre".into(),
                wing: Some("preferences".into()),
                room: Some("naming".into()),
                source: "memory_save:preference".into(),
            },
        )
        .unwrap();
        for index in 0..200 {
            let mut noise = sample("mirror");
            noise.object = format!("unrelated recent event {index}");
            append(&conn, noise).unwrap();
        }

        let rows = search_active_by_terms(
            &conn,
            &[
                "relire".into(),
                "autre".into(),
                "session".into(),
                "utiliser".into(),
                "historique".into(),
                "visible".into(),
                "demander".into(),
                "indice".into(),
                "quelle".into(),
                "préférence".into(),
                "durable".into(),
                "actuelle".into(),
                "nommer".into(),
                "étapes".into(),
                "certification".into(),
                "public2".into(),
                "ambre".into(),
            ],
            5,
        )
        .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, durable.id);
    }

    #[test]
    fn search_term_anchor_requires_letters_and_digits() {
        assert!(term_is_precise_anchor("public2"));
        assert!(term_is_precise_anchor("0715b"));
        assert!(!term_is_precise_anchor("2026"));
    }

    #[test]
    fn search_active_by_terms_excludes_retracted_and_respects_limit() {
        let conn = fresh_db();
        for object in ["jalons ambre", "jalons cuivre", "jalons argent"] {
            append(
                &conn,
                NewMemoryWrite {
                    subject: "user:vivien".into(),
                    predicate: "prefers_label".into(),
                    object: object.into(),
                    wing: None,
                    room: None,
                    source: "test".into(),
                },
            )
            .unwrap();
        }
        retract_by_match(
            &conn,
            Some("user:vivien"),
            Some("prefers_label"),
            Some("jalons cuivre"),
            "memory_forget",
        )
        .unwrap();

        let rows = search_active_by_terms(&conn, &["jalons".into()], 1).unwrap();

        assert_eq!(rows.len(), 1);
        assert_ne!(rows[0].object, "jalons cuivre");
    }

    #[test]
    fn retract_is_idempotent_and_never_queues_duplicate_invalidation() {
        let conn = fresh_db();
        append(&conn, sample("memory_save:info")).unwrap();
        let first =
            retract_by_match(&conn, Some("alice"), Some("likes"), None, "memory_forget").unwrap();
        let second =
            retract_by_match(&conn, Some("alice"), Some("likes"), None, "memory_forget").unwrap();
        assert_eq!(first.invalidations.len(), 1);
        assert!(second.retracted.is_empty());
        assert!(second.invalidations.is_empty());
        assert_eq!(list_recent(&conn, None, 10).unwrap().len(), 2);
    }

    #[test]
    fn exact_legacy_invalidation_reuses_latest_terminal_operation() {
        let conn = fresh_db();
        append(&conn, sample("memory_save:info")).unwrap();
        let batch = retract_by_match(
            &conn,
            Some("alice"),
            Some("likes"),
            Some("coffee"),
            "memory_forget",
        )
        .unwrap();
        let existing_id = batch.invalidations[0].id.clone();
        let (invalidation, created) = ensure_exact_invalidation(
            &conn,
            NewMemoryWrite {
                source: "memory_forget:legacy".into(),
                ..sample("unused")
            },
        )
        .unwrap();
        assert!(!created);
        assert_eq!(invalidation.id, existing_id);
        assert_eq!(list_recent(&conn, None, 10).unwrap().len(), 2);
    }

    #[test]
    fn append_preserves_wing_and_room() {
        let conn = fresh_db();
        let input = NewMemoryWrite {
            subject: "x".into(),
            predicate: "y".into(),
            object: "z".into(),
            wing: Some("learnings".into()),
            room: Some("failures".into()),
            source: "reflector".into(),
        };
        let row = append(&conn, input).unwrap();
        assert_eq!(row.wing.as_deref(), Some("learnings"));
        assert_eq!(row.room.as_deref(), Some("failures"));
    }
}
