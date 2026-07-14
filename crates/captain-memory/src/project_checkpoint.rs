//! Project handoff checkpoints (v3.11g).
//!
//! A checkpoint is a compact snapshot of "where I left off" on a
//! project. It's written opportunistically at end-of-session (or on
//! explicit save) and read on `project_resume(slug)` so Captain can
//! reconstruct context without a full scroll-back.
//!
//! Shape:
//! - `summary`    — prose narration of the session's progress
//! - `state_json` — free-form structured payload (open tasks,
//!   pending decisions, peer agents, next action)
//! - `session_id` — optional link back to the originating chat
//!
//! `latest(project_id)` gives the most recent checkpoint; the table
//! keeps history so future sub-phases can diff across sessions.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Checkpoint {
    pub id: String,
    pub project_id: String,
    pub session_id: Option<String>,
    pub summary: String,
    pub state: serde_json::Value,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewCheckpoint {
    pub project_id: String,
    pub session_id: Option<String>,
    pub summary: String,
    pub state: serde_json::Value,
}

/// Write a new checkpoint. Returns the inserted row with its id.
pub fn append(conn: &Connection, input: NewCheckpoint) -> Result<Checkpoint, rusqlite::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_unix_ms();
    let state_str = serde_json::to_string(&input.state).unwrap_or_else(|_| "{}".to_string());
    conn.execute(
        "INSERT INTO project_checkpoints (id, project_id, session_id, summary, state_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, input.project_id, input.session_id, input.summary, state_str, now],
    )?;
    get(conn, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<Checkpoint>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, project_id, session_id, summary, state_json, created_at
         FROM project_checkpoints WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_checkpoint(row)?))
    } else {
        Ok(None)
    }
}

/// Return the most recent checkpoint for a project, or `None` when the
/// project has never been checkpointed.
pub fn latest(conn: &Connection, project_id: &str) -> Result<Option<Checkpoint>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, project_id, session_id, summary, state_json, created_at
         FROM project_checkpoints WHERE project_id = ?1
         ORDER BY created_at DESC, id DESC
         LIMIT 1",
    )?;
    let mut rows = stmt.query(params![project_id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_checkpoint(row)?))
    } else {
        Ok(None)
    }
}

/// Paginated history, newest first. `limit` is capped at 100 — deep
/// history is expected to live in MemPalace diaries, not here.
pub fn history(
    conn: &Connection,
    project_id: &str,
    limit: usize,
) -> Result<Vec<Checkpoint>, rusqlite::Error> {
    let cap = limit.min(100) as i64;
    let mut stmt = conn.prepare(
        "SELECT id, project_id, session_id, summary, state_json, created_at
         FROM project_checkpoints WHERE project_id = ?1
         ORDER BY created_at DESC, id DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![project_id, cap], row_to_checkpoint)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn row_to_checkpoint(row: &rusqlite::Row<'_>) -> Result<Checkpoint, rusqlite::Error> {
    let state_str: String = row.get(4)?;
    let state =
        serde_json::from_str(&state_str).unwrap_or(serde_json::Value::Object(Default::default()));
    Ok(Checkpoint {
        id: row.get(0)?,
        project_id: row.get(1)?,
        session_id: row.get(2)?,
        summary: row.get(3)?,
        state,
        created_at: row.get(5)?,
    })
}

fn now_unix_ms() -> i64 {
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
    use crate::project::{create as create_project, NewProject};

    fn fresh_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    fn make_project(conn: &Connection, slug: &str) -> String {
        create_project(
            conn,
            NewProject {
                name: format!("p-{slug}"),
                slug: slug.to_string(),
                goal: String::new(),
                deadline: None,
            },
        )
        .unwrap()
        .id
    }

    #[test]
    fn append_then_latest_returns_it() {
        let conn = fresh_db();
        let pid = make_project(&conn, "alpha");
        let cp = append(
            &conn,
            NewCheckpoint {
                project_id: pid.clone(),
                session_id: Some("sess-1".into()),
                summary: "Landed the v1 spec".into(),
                state: serde_json::json!({ "next": "review" }),
            },
        )
        .unwrap();
        assert_eq!(cp.session_id.as_deref(), Some("sess-1"));
        assert_eq!(cp.state["next"], "review");

        let got = latest(&conn, &pid).unwrap().unwrap();
        assert_eq!(got.id, cp.id);
    }

    #[test]
    fn latest_returns_most_recent_across_multiple() {
        let conn = fresh_db();
        let pid = make_project(&conn, "alpha");
        for i in 0..3 {
            append(
                &conn,
                NewCheckpoint {
                    project_id: pid.clone(),
                    session_id: None,
                    summary: format!("step {i}"),
                    state: serde_json::Value::Null,
                },
            )
            .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let last = latest(&conn, &pid).unwrap().unwrap();
        assert_eq!(last.summary, "step 2");
    }

    #[test]
    fn latest_is_none_when_project_has_no_checkpoints() {
        let conn = fresh_db();
        let pid = make_project(&conn, "empty");
        assert!(latest(&conn, &pid).unwrap().is_none());
    }

    #[test]
    fn history_respects_limit_and_order() {
        let conn = fresh_db();
        let pid = make_project(&conn, "alpha");
        for i in 0..5 {
            append(
                &conn,
                NewCheckpoint {
                    project_id: pid.clone(),
                    session_id: None,
                    summary: format!("s{i}"),
                    state: serde_json::Value::Null,
                },
            )
            .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let h = history(&conn, &pid, 3).unwrap();
        assert_eq!(h.len(), 3);
        assert_eq!(h[0].summary, "s4"); // newest first
        assert_eq!(h[2].summary, "s2");
    }

    #[test]
    fn history_caps_at_100_even_when_limit_is_larger() {
        let conn = fresh_db();
        let pid = make_project(&conn, "alpha");
        append(
            &conn,
            NewCheckpoint {
                project_id: pid.clone(),
                session_id: None,
                summary: "one".into(),
                state: serde_json::Value::Null,
            },
        )
        .unwrap();
        let h = history(&conn, &pid, 10_000).unwrap();
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn project_cascade_removes_checkpoints() {
        let conn = fresh_db();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        let pid = make_project(&conn, "alpha");
        append(
            &conn,
            NewCheckpoint {
                project_id: pid.clone(),
                session_id: None,
                summary: "seed".into(),
                state: serde_json::Value::Null,
            },
        )
        .unwrap();
        conn.execute("DELETE FROM projects WHERE id = ?1", params![pid])
            .unwrap();
        assert!(latest(&conn, "alpha").unwrap().is_none());
    }
}
