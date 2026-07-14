//! Append-only event log for session timeline replay (v3.9f).
//!
//! Every meaningful runtime event (tool_use, tool_result, assistant_delta,
//! user_message, pty_output, …) can be appended here with a timestamp.
//! The UI's timeline scrubber reads a range back with [`range`] to replay
//! a session step by step.
//!
//! The log lives in the `sessions_events` table created by migration v9.
//! Writes are single-row INSERTs; reads use the `(session_id, ts)`
//! composite index for efficient windowed queries.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// One row from the `sessions_events` table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionEvent {
    /// Auto-increment row id (also the chronological tie-breaker when two
    /// events share the same `ts`).
    pub id: i64,
    /// Opaque session identifier (matches the runtime's session key).
    pub session_id: String,
    /// Event timestamp as unix milliseconds since the epoch.
    pub ts: i64,
    /// Category tag (e.g. `"tool_use"`, `"tool_result"`, `"assistant_delta"`).
    pub event_type: String,
    /// JSON-encoded event body. Free-form; callers agree on the shape.
    pub payload: serde_json::Value,
}

/// Window query for [`range`].
#[derive(Debug, Clone, Default)]
pub struct RangeQuery {
    /// Session to read from.
    pub session_id: String,
    /// Lower bound on `ts` (inclusive). `None` = start of the log.
    pub from_ts: Option<i64>,
    /// Upper bound on `ts` (inclusive). `None` = end of the log.
    pub to_ts: Option<i64>,
    /// Max rows to return. `None` = no cap. A hard cap of 10_000 is still
    /// applied by [`range`] so dashboards can't accidentally pull huge
    /// windows.
    pub limit: Option<usize>,
}

const HARD_LIMIT: usize = 10_000;

// ---------------------------------------------------------------------------
// API
// ---------------------------------------------------------------------------

/// Append a single event. Returns the new row's `id`.
///
/// Errors propagate from rusqlite; callers at the runtime edge typically
/// log-and-swallow (observability failures must never crash the agent
/// loop — see the hook inside `agent_loop.rs`).
pub fn append(
    conn: &Connection,
    session_id: &str,
    event_type: &str,
    payload: &serde_json::Value,
) -> Result<i64, rusqlite::Error> {
    let payload_str = serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string());
    let ts = now_unix_ms();
    conn.execute(
        "INSERT INTO sessions_events (session_id, ts, event_type, payload)
         VALUES (?1, ?2, ?3, ?4)",
        params![session_id, ts, event_type, payload_str],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Same as [`append`] but with a caller-supplied `ts`. Useful when
/// backfilling events or replaying historical data in tests.
pub fn append_with_ts(
    conn: &Connection,
    session_id: &str,
    event_type: &str,
    payload: &serde_json::Value,
    ts: i64,
) -> Result<i64, rusqlite::Error> {
    let payload_str = serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string());
    conn.execute(
        "INSERT INTO sessions_events (session_id, ts, event_type, payload)
         VALUES (?1, ?2, ?3, ?4)",
        params![session_id, ts, event_type, payload_str],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Read a window of events for a session, ordered by `(ts, id)` ascending.
pub fn range(conn: &Connection, q: &RangeQuery) -> Result<Vec<SessionEvent>, rusqlite::Error> {
    let from_ts = q.from_ts.unwrap_or(i64::MIN);
    let to_ts = q.to_ts.unwrap_or(i64::MAX);
    let limit = q.limit.map(|n| n.min(HARD_LIMIT)).unwrap_or(HARD_LIMIT) as i64;

    let mut stmt = conn.prepare(
        "SELECT id, session_id, ts, event_type, payload
         FROM sessions_events
         WHERE session_id = ?1 AND ts >= ?2 AND ts <= ?3
         ORDER BY ts ASC, id ASC
         LIMIT ?4",
    )?;

    let rows = stmt.query_map(params![q.session_id, from_ts, to_ts, limit], row_to_event)?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Read the newest matching events, returned in chronological order.
pub fn tail(conn: &Connection, q: &RangeQuery) -> Result<Vec<SessionEvent>, rusqlite::Error> {
    let from_ts = q.from_ts.unwrap_or(i64::MIN);
    let to_ts = q.to_ts.unwrap_or(i64::MAX);
    let limit = q.limit.map(|n| n.min(HARD_LIMIT)).unwrap_or(HARD_LIMIT) as i64;

    let mut stmt = conn.prepare(
        "SELECT id, session_id, ts, event_type, payload
         FROM sessions_events
         WHERE session_id = ?1 AND ts >= ?2 AND ts <= ?3
         ORDER BY ts DESC, id DESC
         LIMIT ?4",
    )?;

    let rows = stmt.query_map(params![q.session_id, from_ts, to_ts, limit], row_to_event)?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    out.reverse();
    Ok(out)
}

/// Read the newest events for a session and type, returned in chronological order.
pub fn tail_by_type(
    conn: &Connection,
    q: &RangeQuery,
    event_type: &str,
) -> Result<Vec<SessionEvent>, rusqlite::Error> {
    let from_ts = q.from_ts.unwrap_or(i64::MIN);
    let to_ts = q.to_ts.unwrap_or(i64::MAX);
    let limit = q.limit.map(|n| n.min(HARD_LIMIT)).unwrap_or(HARD_LIMIT) as i64;

    let mut stmt = conn.prepare(
        "SELECT id, session_id, ts, event_type, payload
         FROM sessions_events
         WHERE session_id = ?1 AND event_type = ?2 AND ts >= ?3 AND ts <= ?4
         ORDER BY ts DESC, id DESC
         LIMIT ?5",
    )?;

    let rows = stmt.query_map(
        params![q.session_id, event_type, from_ts, to_ts, limit],
        row_to_event,
    )?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    out.reverse();
    Ok(out)
}

/// Number of rows stored for a session. O(log n) thanks to the index.
pub fn count(conn: &Connection, session_id: &str) -> Result<u64, rusqlite::Error> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sessions_events WHERE session_id = ?1",
        params![session_id],
        |row| row.get(0),
    )?;
    Ok(n as u64)
}

/// Number of rows stored for a session and event type.
pub fn count_by_type(
    conn: &Connection,
    session_id: &str,
    event_type: &str,
) -> Result<u64, rusqlite::Error> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sessions_events WHERE session_id = ?1 AND event_type = ?2",
        params![session_id, event_type],
        |row| row.get(0),
    )?;
    Ok(n as u64)
}

/// Delete every event for a session. Used when a session is explicitly
/// discarded by the user.
pub fn purge(conn: &Connection, session_id: &str) -> Result<u64, rusqlite::Error> {
    let n = conn.execute(
        "DELETE FROM sessions_events WHERE session_id = ?1",
        params![session_id],
    )?;
    Ok(n as u64)
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn row_to_event(row: &rusqlite::Row<'_>) -> Result<SessionEvent, rusqlite::Error> {
    let payload_str: String = row.get(4)?;
    let payload = serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null);
    Ok(SessionEvent {
        id: row.get(0)?,
        session_id: row.get(1)?,
        ts: row.get(2)?,
        event_type: row.get(3)?,
        payload,
    })
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

    #[test]
    fn append_then_range_returns_event() {
        let conn = fresh_db();
        let payload = serde_json::json!({ "tool": "shell_exec", "cmd": "ls" });
        let id = append(&conn, "sess-1", "tool_use", &payload).unwrap();
        assert!(id > 0);

        let rows = range(
            &conn,
            &RangeQuery {
                session_id: "sess-1".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].event_type, "tool_use");
        assert_eq!(rows[0].payload, payload);
    }

    #[test]
    fn range_is_ordered_and_filtered_by_session() {
        let conn = fresh_db();
        append_with_ts(&conn, "a", "x", &serde_json::json!({}), 100).unwrap();
        append_with_ts(&conn, "a", "x", &serde_json::json!({}), 200).unwrap();
        append_with_ts(&conn, "b", "x", &serde_json::json!({}), 150).unwrap();
        append_with_ts(&conn, "a", "x", &serde_json::json!({}), 300).unwrap();

        let rows = range(
            &conn,
            &RangeQuery {
                session_id: "a".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].ts, 100);
        assert_eq!(rows[1].ts, 200);
        assert_eq!(rows[2].ts, 300);
    }

    #[test]
    fn range_respects_ts_window() {
        let conn = fresh_db();
        for ts in [100_i64, 200, 300, 400, 500] {
            append_with_ts(&conn, "s", "x", &serde_json::json!({}), ts).unwrap();
        }
        let rows = range(
            &conn,
            &RangeQuery {
                session_id: "s".into(),
                from_ts: Some(200),
                to_ts: Some(400),
                limit: None,
            },
        )
        .unwrap();
        let ts_seen: Vec<i64> = rows.iter().map(|r| r.ts).collect();
        assert_eq!(ts_seen, vec![200, 300, 400]);
    }

    #[test]
    fn range_respects_limit_and_hard_cap() {
        let conn = fresh_db();
        for ts in 0..50_i64 {
            append_with_ts(&conn, "s", "x", &serde_json::json!({ "i": ts }), ts).unwrap();
        }

        let rows = range(
            &conn,
            &RangeQuery {
                session_id: "s".into(),
                limit: Some(10),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(rows.len(), 10);
        assert_eq!(rows[0].ts, 0);
        assert_eq!(rows[9].ts, 9);
    }

    #[test]
    fn tail_returns_newest_rows_in_chronological_order() {
        let conn = fresh_db();
        for ts in 0..50_i64 {
            append_with_ts(&conn, "s", "x", &serde_json::json!({ "i": ts }), ts).unwrap();
        }

        let rows = tail(
            &conn,
            &RangeQuery {
                session_id: "s".into(),
                limit: Some(10),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(rows.len(), 10);
        assert_eq!(rows[0].ts, 40);
        assert_eq!(rows[9].ts, 49);
    }

    #[test]
    fn tail_respects_ts_window() {
        let conn = fresh_db();
        for ts in 0..10_i64 {
            append_with_ts(&conn, "s", "x", &serde_json::json!({ "i": ts }), ts).unwrap();
        }

        let rows = tail(
            &conn,
            &RangeQuery {
                session_id: "s".into(),
                from_ts: Some(2),
                to_ts: Some(7),
                limit: Some(3),
            },
        )
        .unwrap();
        let ts_seen: Vec<i64> = rows.iter().map(|row| row.ts).collect();

        assert_eq!(ts_seen, vec![5, 6, 7]);
    }

    #[test]
    fn tail_by_type_ignores_newer_other_event_types() {
        let conn = fresh_db();
        for ts in 0..5_i64 {
            append_with_ts(&conn, "s", "project", &serde_json::json!({ "i": ts }), ts).unwrap();
        }
        for ts in 5..10_i64 {
            append_with_ts(&conn, "s", "noise", &serde_json::json!({ "i": ts }), ts).unwrap();
        }

        let rows = tail_by_type(
            &conn,
            &RangeQuery {
                session_id: "s".into(),
                limit: Some(2),
                ..Default::default()
            },
            "project",
        )
        .unwrap();
        let ts_seen: Vec<i64> = rows.iter().map(|row| row.ts).collect();

        assert_eq!(ts_seen, vec![3, 4]);
    }

    #[test]
    fn count_and_purge_behave() {
        let conn = fresh_db();
        for _ in 0..5 {
            append(&conn, "sess", "tick", &serde_json::json!({})).unwrap();
        }
        append(&conn, "sess", "noise", &serde_json::json!({})).unwrap();
        assert_eq!(count(&conn, "sess").unwrap(), 6);
        assert_eq!(count_by_type(&conn, "sess", "tick").unwrap(), 5);
        assert_eq!(count_by_type(&conn, "sess", "noise").unwrap(), 1);

        let removed = purge(&conn, "sess").unwrap();
        assert_eq!(removed, 6);
        assert_eq!(count(&conn, "sess").unwrap(), 0);
    }

    #[test]
    fn payload_with_invalid_json_falls_back_to_null() {
        // Directly insert a malformed payload row to ensure range() does not
        // choke if the store is ever corrupted.
        let conn = fresh_db();
        conn.execute(
            "INSERT INTO sessions_events (session_id, ts, event_type, payload)
             VALUES ('sess', 123, 'bad', '{not json')",
            [],
        )
        .unwrap();

        let rows = range(
            &conn,
            &RangeQuery {
                session_id: "sess".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].payload, serde_json::Value::Null);
    }
}
