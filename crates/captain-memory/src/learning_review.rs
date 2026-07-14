//! Approval queue for the LearningEngine (v3.12g).
//!
//! When `[learning] mode = "approval"`, the `MemoryCommitter` drops
//! candidates into this queue instead of writing them through. A human
//! then reviews pending items (via the `learning_review_list` tool or
//! Telegram inline keyboards) and calls `learning_review_decide` to
//! approve or deny each one. Approved items are committed through
//! `memory_writer::write_through`; denied items stay in the queue
//! for audit and are GC'd after 30 days.
//!
//! The queue is a plain SQLite table — no MCP, no concurrency magic.
//! Decisions are idempotent per id: the second decide on the same id
//! returns `AlreadyDecided`.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Approved,
    Denied,
}

impl Decision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Denied => "denied",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NewReviewItem {
    pub outcome: String,
    pub agent_id: String,
    pub wing: String,
    pub room: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f32,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewItem {
    pub id: String,
    pub outcome: String,
    pub agent_id: String,
    pub wing: String,
    pub room: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f32,
    pub source: String,
    pub created_at: i64,
    pub decided_at: Option<i64>,
    pub decided_by: Option<String>,
    pub decision: Option<Decision>,
    pub written_write_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReviewError {
    #[error("review item not found: {0}")]
    NotFound(String),
    #[error("review item already decided")]
    AlreadyDecided,
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

pub fn enqueue(conn: &Connection, input: NewReviewItem) -> Result<ReviewItem, ReviewError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_ms();
    conn.execute(
        "INSERT INTO learning_review_queue
         (id, outcome, agent_id, wing, room, subject, predicate, object,
          confidence, source, created_at, decided_at, decided_by, decision,
          written_write_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, NULL, NULL, NULL)",
        params![
            id,
            input.outcome,
            input.agent_id,
            input.wing,
            input.room,
            input.subject,
            input.predicate,
            input.object,
            input.confidence,
            input.source,
            now,
        ],
    )?;
    get(conn, &id)?.ok_or(ReviewError::NotFound(id))
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<ReviewItem>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, outcome, agent_id, wing, room, subject, predicate, object,
                confidence, source, created_at, decided_at, decided_by, decision,
                written_write_id
         FROM learning_review_queue WHERE id = ?1",
    )?;
    stmt.query_row(params![id], row_to_item).optional()
}

/// Pending items (decision IS NULL), oldest first.
pub fn list_pending(conn: &Connection, limit: usize) -> Result<Vec<ReviewItem>, rusqlite::Error> {
    let cap = limit.min(10_000) as i64;
    let mut stmt = conn.prepare(
        "SELECT id, outcome, agent_id, wing, room, subject, predicate, object,
                confidence, source, created_at, decided_at, decided_by, decision,
                written_write_id
         FROM learning_review_queue
         WHERE decision IS NULL
         ORDER BY created_at ASC, id ASC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![cap], row_to_item)?;
    rows.collect()
}

/// Record a decision. Fails with `AlreadyDecided` if the item has
/// already been approved or denied. Caller must pair with
/// `mark_written_write_id` for approved items after the write_through.
pub fn decide(
    conn: &Connection,
    id: &str,
    decision: Decision,
    by: Option<&str>,
) -> Result<ReviewItem, ReviewError> {
    let current = get(conn, id)?.ok_or_else(|| ReviewError::NotFound(id.to_string()))?;
    if current.decision.is_some() {
        return Err(ReviewError::AlreadyDecided);
    }
    let now = now_ms();
    conn.execute(
        "UPDATE learning_review_queue
         SET decision = ?1, decided_at = ?2, decided_by = ?3
         WHERE id = ?4",
        params![decision.as_str(), now, by, id],
    )?;
    get(conn, id)?.ok_or_else(|| ReviewError::NotFound(id.to_string()))
}

pub fn mark_written_write_id(
    conn: &Connection,
    id: &str,
    write_id: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE learning_review_queue SET written_write_id = ?1 WHERE id = ?2",
        params![write_id, id],
    )?;
    Ok(())
}

/// Delete decided rows older than the given threshold (in ms).
pub fn cleanup_decided(conn: &Connection, older_than_ms: i64) -> Result<usize, rusqlite::Error> {
    let cutoff = now_ms() - older_than_ms;
    let n = conn.execute(
        "DELETE FROM learning_review_queue
         WHERE decision IS NOT NULL AND decided_at IS NOT NULL AND decided_at < ?1",
        params![cutoff],
    )?;
    Ok(n)
}

fn row_to_item(row: &rusqlite::Row<'_>) -> Result<ReviewItem, rusqlite::Error> {
    let decision_str: Option<String> = row.get(13)?;
    let decision = decision_str.and_then(|s| match s.as_str() {
        "approved" => Some(Decision::Approved),
        "denied" => Some(Decision::Denied),
        _ => None,
    });
    Ok(ReviewItem {
        id: row.get(0)?,
        outcome: row.get(1)?,
        agent_id: row.get(2)?,
        wing: row.get(3)?,
        room: row.get(4)?,
        subject: row.get(5)?,
        predicate: row.get(6)?,
        object: row.get(7)?,
        confidence: row.get(8)?,
        source: row.get(9)?,
        created_at: row.get(10)?,
        decided_at: row.get(11)?,
        decided_by: row.get(12)?,
        decision,
        written_write_id: row.get(14)?,
    })
}

fn now_ms() -> i64 {
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

    fn sample(subject: &str) -> NewReviewItem {
        NewReviewItem {
            outcome: "user_corrected".into(),
            agent_id: "captain".into(),
            wing: "learnings".into(),
            room: "user_preferences".into(),
            subject: subject.into(),
            predicate: "prefers".into(),
            object: "espresso without sugar in the morning".into(),
            confidence: 0.9,
            source: "learning.user_correction".into(),
        }
    }

    #[test]
    fn enqueue_creates_pending_item() {
        let conn = fresh_db();
        let item = enqueue(&conn, sample("user")).unwrap();
        assert!(item.decision.is_none());
        assert!(item.decided_at.is_none());
        assert!(item.written_write_id.is_none());
        assert_eq!(item.subject, "user");
    }

    #[test]
    fn list_pending_returns_only_undecided_oldest_first() {
        let conn = fresh_db();
        let a = enqueue(&conn, sample("a")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = enqueue(&conn, sample("b")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let c = enqueue(&conn, sample("c")).unwrap();

        decide(&conn, &b.id, Decision::Approved, Some("reviewer")).unwrap();

        let pending = list_pending(&conn, 100).unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].id, a.id);
        assert_eq!(pending[1].id, c.id);
    }

    #[test]
    fn decide_approve_sets_decision_and_timestamp() {
        let conn = fresh_db();
        let item = enqueue(&conn, sample("user")).unwrap();
        let updated = decide(&conn, &item.id, Decision::Approved, Some("reviewer")).unwrap();
        assert_eq!(updated.decision, Some(Decision::Approved));
        assert!(updated.decided_at.is_some());
        assert_eq!(updated.decided_by.as_deref(), Some("reviewer"));
    }

    #[test]
    fn decide_deny_marks_denied() {
        let conn = fresh_db();
        let item = enqueue(&conn, sample("user")).unwrap();
        let updated = decide(&conn, &item.id, Decision::Denied, None).unwrap();
        assert_eq!(updated.decision, Some(Decision::Denied));
    }

    #[test]
    fn decide_twice_returns_already_decided() {
        let conn = fresh_db();
        let item = enqueue(&conn, sample("user")).unwrap();
        decide(&conn, &item.id, Decision::Approved, None).unwrap();
        let err = decide(&conn, &item.id, Decision::Denied, None).unwrap_err();
        assert!(matches!(err, ReviewError::AlreadyDecided));
    }

    #[test]
    fn decide_unknown_id_returns_not_found() {
        let conn = fresh_db();
        let err = decide(&conn, "ghost", Decision::Approved, None).unwrap_err();
        assert!(matches!(err, ReviewError::NotFound(_)));
    }

    #[test]
    fn mark_written_write_id_persists() {
        let conn = fresh_db();
        let item = enqueue(&conn, sample("user")).unwrap();
        decide(&conn, &item.id, Decision::Approved, None).unwrap();
        mark_written_write_id(&conn, &item.id, "write-123").unwrap();
        let got = get(&conn, &item.id).unwrap().unwrap();
        assert_eq!(got.written_write_id.as_deref(), Some("write-123"));
    }

    #[test]
    fn cleanup_decided_removes_old_decided_only() {
        let conn = fresh_db();
        let a = enqueue(&conn, sample("a")).unwrap();
        let b = enqueue(&conn, sample("b")).unwrap();
        let _c = enqueue(&conn, sample("c")).unwrap();

        decide(&conn, &a.id, Decision::Approved, None).unwrap();
        decide(&conn, &b.id, Decision::Denied, None).unwrap();

        // Backdate `a.decided_at` by 40 days.
        let forty_days_ms: i64 = 40 * 24 * 60 * 60 * 1000;
        let old_ts = now_ms() - forty_days_ms;
        conn.execute(
            "UPDATE learning_review_queue SET decided_at = ?1 WHERE id = ?2",
            params![old_ts, a.id],
        )
        .unwrap();

        let thirty_days_ms: i64 = 30 * 24 * 60 * 60 * 1000;
        let removed = cleanup_decided(&conn, thirty_days_ms).unwrap();
        assert_eq!(removed, 1);
        assert!(get(&conn, &a.id).unwrap().is_none());
        assert!(get(&conn, &b.id).unwrap().is_some()); // recent, survives
    }
}
