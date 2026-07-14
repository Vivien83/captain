//! Skill proposals review queue (v3.13c).
//!
//! Drafted by `SkillProposer` (v3.13b), held here pending human review.
//! Approved proposals are turned into `.md` files by the SkillWriter
//! (v3.13d); the resulting path is stored in `written_path` for audit.
//!
//! Mirrors the `learning_review` table pattern from v3.12g — same
//! decide / mark / cleanup verbs.

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
pub struct NewProposal {
    pub pattern_hash: String,
    pub name: String,
    pub description: String,
    pub trigger_hint: String,
    pub tool_sequence: Vec<String>,
    pub arg_schema_hint: String,
    pub confidence: f32,
    pub family: String,
    pub source_agent_id: String,
    pub origin_channel: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Proposal {
    pub id: String,
    pub pattern_hash: String,
    pub name: String,
    pub description: String,
    pub trigger_hint: String,
    pub tool_sequence: Vec<String>,
    pub arg_schema_hint: String,
    pub confidence: f32,
    pub family: String,
    pub source_agent_id: String,
    pub origin_channel: Option<String>,
    pub status: Option<Decision>,
    pub created_at: i64,
    pub decided_at: Option<i64>,
    pub decided_by: Option<String>,
    pub written_path: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProposalError {
    #[error("proposal not found: {0}")]
    NotFound(String),
    #[error("proposal already decided")]
    AlreadyDecided,
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

pub fn enqueue(conn: &Connection, input: NewProposal) -> Result<Proposal, ProposalError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_ms();
    let seq_json = serde_json::to_string(&input.tool_sequence).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT INTO skill_proposals
         (id, pattern_hash, name, description, trigger_hint, tool_sequence_json,
          arg_schema_hint, confidence, family, source_agent_id, origin_channel, status,
          created_at, decided_at, decided_by, written_path)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, ?12, NULL, NULL, NULL)",
        params![
            id,
            input.pattern_hash,
            input.name,
            input.description,
            input.trigger_hint,
            seq_json,
            input.arg_schema_hint,
            input.confidence,
            input.family,
            input.source_agent_id,
            input.origin_channel,
            now,
        ],
    )?;
    get(conn, &id)?.ok_or(ProposalError::NotFound(id))
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<Proposal>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, pattern_hash, name, description, trigger_hint, tool_sequence_json,
                arg_schema_hint, confidence, family, source_agent_id, origin_channel, status, created_at,
                decided_at, decided_by, written_path
         FROM skill_proposals WHERE id = ?1",
    )?;
    stmt.query_row(params![id], row_to_proposal).optional()
}

/// Pending items (status IS NULL), oldest first.
pub fn list_pending(conn: &Connection, limit: usize) -> Result<Vec<Proposal>, rusqlite::Error> {
    let cap = limit.min(10_000) as i64;
    let mut stmt = conn.prepare(
        "SELECT id, pattern_hash, name, description, trigger_hint, tool_sequence_json,
                arg_schema_hint, confidence, family, source_agent_id, origin_channel, status, created_at,
                decided_at, decided_by, written_path
         FROM skill_proposals
         WHERE status IS NULL
         ORDER BY created_at ASC, id ASC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![cap], row_to_proposal)?;
    rows.collect()
}

pub fn decide(
    conn: &Connection,
    id: &str,
    decision: Decision,
    by: Option<&str>,
) -> Result<Proposal, ProposalError> {
    let current = get(conn, id)?.ok_or_else(|| ProposalError::NotFound(id.to_string()))?;
    if current.status.is_some() {
        return Err(ProposalError::AlreadyDecided);
    }
    let now = now_ms();
    conn.execute(
        "UPDATE skill_proposals
         SET status = ?1, decided_at = ?2, decided_by = ?3
         WHERE id = ?4",
        params![decision.as_str(), now, by, id],
    )?;
    get(conn, id)?.ok_or_else(|| ProposalError::NotFound(id.to_string()))
}

pub fn mark_written(conn: &Connection, id: &str, path: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE skill_proposals SET written_path = ?1 WHERE id = ?2",
        params![path, id],
    )?;
    Ok(())
}

pub fn cleanup_decided(conn: &Connection, older_than_ms: i64) -> Result<usize, rusqlite::Error> {
    let cutoff = now_ms() - older_than_ms;
    let n = conn.execute(
        "DELETE FROM skill_proposals
         WHERE status IS NOT NULL AND decided_at IS NOT NULL AND decided_at < ?1",
        params![cutoff],
    )?;
    Ok(n)
}

fn row_to_proposal(row: &rusqlite::Row<'_>) -> Result<Proposal, rusqlite::Error> {
    let seq_json: String = row.get(5)?;
    let tool_sequence: Vec<String> = serde_json::from_str(&seq_json).unwrap_or_default();
    let status_str: Option<String> = row.get(11)?;
    let status = status_str.and_then(|s| match s.as_str() {
        "approved" => Some(Decision::Approved),
        "denied" => Some(Decision::Denied),
        _ => None,
    });
    Ok(Proposal {
        id: row.get(0)?,
        pattern_hash: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        trigger_hint: row.get(4)?,
        tool_sequence,
        arg_schema_hint: row.get(6)?,
        confidence: row.get(7)?,
        family: row.get(8)?,
        source_agent_id: row.get(9)?,
        origin_channel: row.get(10)?,
        status,
        created_at: row.get(12)?,
        decided_at: row.get(13)?,
        decided_by: row.get(14)?,
        written_path: row.get(15)?,
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

    fn sample(name: &str) -> NewProposal {
        NewProposal {
            pattern_hash: "h123".into(),
            name: name.into(),
            description: "Searches the web then writes a markdown summary".into(),
            trigger_hint: "user asks to research a topic".into(),
            tool_sequence: vec!["web_search".into(), "web_fetch".into(), "file_write".into()],
            arg_schema_hint: "query: string".into(),
            confidence: 0.9,
            family: "general-automation".into(),
            source_agent_id: "captain".into(),
            origin_channel: Some("telegram".into()),
        }
    }

    #[test]
    fn enqueue_creates_pending_item() {
        let conn = fresh_db();
        let p = enqueue(&conn, sample("research-log")).unwrap();
        assert!(p.status.is_none());
        assert!(p.decided_at.is_none());
        assert!(p.written_path.is_none());
        assert_eq!(p.name, "research-log");
        assert_eq!(p.tool_sequence.len(), 3);
        assert_eq!(p.family, "general-automation");
        assert_eq!(p.origin_channel.as_deref(), Some("telegram"));
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
    fn decide_approve_sets_status() {
        let conn = fresh_db();
        let p = enqueue(&conn, sample("x")).unwrap();
        let updated = decide(&conn, &p.id, Decision::Approved, Some("reviewer")).unwrap();
        assert_eq!(updated.status, Some(Decision::Approved));
        assert!(updated.decided_at.is_some());
        assert_eq!(updated.decided_by.as_deref(), Some("reviewer"));
    }

    #[test]
    fn decide_deny_marks_denied() {
        let conn = fresh_db();
        let p = enqueue(&conn, sample("x")).unwrap();
        let updated = decide(&conn, &p.id, Decision::Denied, None).unwrap();
        assert_eq!(updated.status, Some(Decision::Denied));
    }

    #[test]
    fn decide_twice_returns_already_decided() {
        let conn = fresh_db();
        let p = enqueue(&conn, sample("x")).unwrap();
        decide(&conn, &p.id, Decision::Approved, None).unwrap();
        let err = decide(&conn, &p.id, Decision::Denied, None).unwrap_err();
        assert!(matches!(err, ProposalError::AlreadyDecided));
    }

    #[test]
    fn decide_unknown_id_returns_not_found() {
        let conn = fresh_db();
        let err = decide(&conn, "ghost", Decision::Approved, None).unwrap_err();
        assert!(matches!(err, ProposalError::NotFound(_)));
    }

    #[test]
    fn mark_written_persists_path() {
        let conn = fresh_db();
        let p = enqueue(&conn, sample("x")).unwrap();
        decide(&conn, &p.id, Decision::Approved, None).unwrap();
        mark_written(&conn, &p.id, "/tmp/skills/x.md").unwrap();
        let got = get(&conn, &p.id).unwrap().unwrap();
        assert_eq!(got.written_path.as_deref(), Some("/tmp/skills/x.md"));
    }

    #[test]
    fn cleanup_decided_removes_old_decided_only() {
        let conn = fresh_db();
        let a = enqueue(&conn, sample("a")).unwrap();
        let _b = enqueue(&conn, sample("b")).unwrap();
        decide(&conn, &a.id, Decision::Approved, None).unwrap();

        let forty_days_ms: i64 = 40 * 24 * 60 * 60 * 1000;
        let old_ts = now_ms() - forty_days_ms;
        conn.execute(
            "UPDATE skill_proposals SET decided_at = ?1 WHERE id = ?2",
            params![old_ts, a.id],
        )
        .unwrap();

        let thirty_days_ms: i64 = 30 * 24 * 60 * 60 * 1000;
        let removed = cleanup_decided(&conn, thirty_days_ms).unwrap();
        assert_eq!(removed, 1);
        assert!(get(&conn, &a.id).unwrap().is_none());
    }

    #[test]
    fn tool_sequence_roundtrips_through_json() {
        let conn = fresh_db();
        let mut s = sample("x");
        s.tool_sequence = vec!["a b".into(), "c".into(), "d e f".into()];
        let p = enqueue(&conn, s.clone()).unwrap();
        assert_eq!(p.tool_sequence, s.tool_sequence);
    }
}
