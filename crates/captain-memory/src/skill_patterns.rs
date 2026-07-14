//! Recurring tool-sequence patterns (v3.13a).
//!
//! The `pattern_detector` records every tool the agent runs in a
//! rolling window. When a window's worth of tools forms a stable
//! ordered sequence (hash) it is upserted here. The same hash for
//! the same agent increments `count`; once `count` crosses the
//! configured threshold inside `pattern_window_days` the row is
//! handed to the LLM-based `SkillProposer` which decides whether the
//! pattern deserves to become a Captain skill.
//!
//! `proposed_at` is stamped after the first proposal so a popular
//! pattern is not re-proposed every increment.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillPattern {
    pub hash: String,
    pub agent_id: String,
    pub tool_sequence: Vec<String>,
    pub first_seen: i64,
    pub last_seen: i64,
    pub count: u32,
    pub proposed_at: Option<i64>,
}

/// Insert the row at count=1, or bump `count` and refresh `last_seen`
/// if the (hash, agent_id) pair already exists. Returns the resulting
/// row.
pub fn incr_or_insert(
    conn: &Connection,
    hash: &str,
    agent_id: &str,
    tool_sequence: &[String],
) -> Result<SkillPattern, rusqlite::Error> {
    let now = now_ms();
    let seq_json = serde_json::to_string(tool_sequence).unwrap_or_else(|_| "[]".to_string());
    let existing = get(conn, hash)?;
    match existing {
        Some(row) if row.agent_id == agent_id => {
            conn.execute(
                "UPDATE skill_patterns
                 SET count = count + 1, last_seen = ?1
                 WHERE hash = ?2",
                params![now, hash],
            )?;
        }
        _ => {
            conn.execute(
                "INSERT OR REPLACE INTO skill_patterns
                 (hash, agent_id, tool_sequence_json, first_seen, last_seen, count, proposed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1, NULL)",
                params![hash, agent_id, seq_json, now, now],
            )?;
        }
    }
    get(conn, hash)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

pub fn get(conn: &Connection, hash: &str) -> Result<Option<SkillPattern>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT hash, agent_id, tool_sequence_json, first_seen, last_seen, count, proposed_at
         FROM skill_patterns WHERE hash = ?1",
    )?;
    stmt.query_row(params![hash], row_to_pattern).optional()
}

/// List patterns whose `count >= threshold`, `last_seen` within
/// `window_days`, and not yet proposed. Newest activity first.
pub fn list_ready(
    conn: &Connection,
    threshold: u32,
    window_days: u32,
    limit: usize,
) -> Result<Vec<SkillPattern>, rusqlite::Error> {
    let cap = limit.min(10_000) as i64;
    let cutoff_ms = now_ms() - (window_days as i64) * 24 * 60 * 60 * 1000;
    let mut stmt = conn.prepare(
        "SELECT hash, agent_id, tool_sequence_json, first_seen, last_seen, count, proposed_at
         FROM skill_patterns
         WHERE count >= ?1
           AND last_seen >= ?2
           AND proposed_at IS NULL
         ORDER BY last_seen DESC, count DESC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![threshold, cutoff_ms, cap], row_to_pattern)?;
    rows.collect()
}

pub fn mark_proposed(conn: &Connection, hash: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE skill_patterns SET proposed_at = ?1 WHERE hash = ?2",
        params![now_ms(), hash],
    )?;
    Ok(())
}

/// GC patterns older than `older_than_days` whose `count` never crossed
/// `threshold`. Keeps the table small over long runtimes.
pub fn cleanup_stale(
    conn: &Connection,
    older_than_days: u32,
    threshold: u32,
) -> Result<usize, rusqlite::Error> {
    let cutoff_ms = now_ms() - (older_than_days as i64) * 24 * 60 * 60 * 1000;
    let n = conn.execute(
        "DELETE FROM skill_patterns
         WHERE last_seen < ?1 AND count < ?2 AND proposed_at IS NULL",
        params![cutoff_ms, threshold],
    )?;
    Ok(n)
}

fn row_to_pattern(row: &rusqlite::Row<'_>) -> Result<SkillPattern, rusqlite::Error> {
    let seq_json: String = row.get(2)?;
    let tool_sequence: Vec<String> = serde_json::from_str(&seq_json).unwrap_or_default();
    Ok(SkillPattern {
        hash: row.get(0)?,
        agent_id: row.get(1)?,
        tool_sequence,
        first_seen: row.get(3)?,
        last_seen: row.get(4)?,
        count: row.get::<_, i64>(5)? as u32,
        proposed_at: row.get(6)?,
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

    fn seq() -> Vec<String> {
        vec!["file_read".into(), "shell_exec".into(), "file_write".into()]
    }

    #[test]
    fn first_insert_creates_row_with_count_1() {
        let conn = fresh_db();
        let row = incr_or_insert(&conn, "h1", "captain", &seq()).unwrap();
        assert_eq!(row.count, 1);
        assert_eq!(row.tool_sequence, seq());
        assert!(row.proposed_at.is_none());
    }

    #[test]
    fn second_insert_same_hash_increments_count() {
        let conn = fresh_db();
        incr_or_insert(&conn, "h1", "captain", &seq()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let row = incr_or_insert(&conn, "h1", "captain", &seq()).unwrap();
        assert_eq!(row.count, 2);
        assert!(row.last_seen > row.first_seen);
    }

    #[test]
    fn list_ready_filters_by_threshold() {
        let conn = fresh_db();
        for _ in 0..3 {
            incr_or_insert(&conn, "h1", "captain", &seq()).unwrap();
        }
        incr_or_insert(&conn, "h2", "captain", &seq()).unwrap();
        let ready = list_ready(&conn, 3, 7, 10).unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].hash, "h1");
    }

    #[test]
    fn list_ready_respects_window() {
        let conn = fresh_db();
        for _ in 0..5 {
            incr_or_insert(&conn, "h1", "captain", &seq()).unwrap();
        }
        // Backdate last_seen to 30d ago.
        let thirty_days_ms: i64 = 30 * 24 * 60 * 60 * 1000;
        let old_ts = now_ms() - thirty_days_ms;
        conn.execute(
            "UPDATE skill_patterns SET last_seen = ?1 WHERE hash = 'h1'",
            params![old_ts],
        )
        .unwrap();
        // 7-day window — pattern excluded.
        assert!(list_ready(&conn, 3, 7, 10).unwrap().is_empty());
        // 60-day window — pattern back.
        assert_eq!(list_ready(&conn, 3, 60, 10).unwrap().len(), 1);
    }

    #[test]
    fn list_ready_excludes_already_proposed() {
        let conn = fresh_db();
        for _ in 0..5 {
            incr_or_insert(&conn, "h1", "captain", &seq()).unwrap();
        }
        mark_proposed(&conn, "h1").unwrap();
        assert!(list_ready(&conn, 3, 7, 10).unwrap().is_empty());
    }

    #[test]
    fn mark_proposed_sets_timestamp() {
        let conn = fresh_db();
        incr_or_insert(&conn, "h1", "captain", &seq()).unwrap();
        mark_proposed(&conn, "h1").unwrap();
        let row = get(&conn, "h1").unwrap().unwrap();
        assert!(row.proposed_at.is_some());
    }

    #[test]
    fn list_ready_orders_by_last_seen_desc() {
        let conn = fresh_db();
        for _ in 0..5 {
            incr_or_insert(&conn, "old", "a", &seq()).unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        for _ in 0..5 {
            incr_or_insert(&conn, "new", "a", &seq()).unwrap();
        }
        let ready = list_ready(&conn, 3, 7, 10).unwrap();
        assert_eq!(ready[0].hash, "new");
        assert_eq!(ready[1].hash, "old");
    }

    #[test]
    fn cleanup_stale_removes_old_low_count_unproposed() {
        let conn = fresh_db();
        // Pattern that crossed threshold — survives
        for _ in 0..5 {
            incr_or_insert(&conn, "popular", "a", &seq()).unwrap();
        }
        // Pattern that never crossed threshold + old — purged
        incr_or_insert(&conn, "stale", "a", &seq()).unwrap();
        let thirty_days_ms: i64 = 30 * 24 * 60 * 60 * 1000;
        conn.execute(
            "UPDATE skill_patterns SET last_seen = ?1 WHERE hash = 'stale'",
            params![now_ms() - thirty_days_ms],
        )
        .unwrap();

        let removed = cleanup_stale(&conn, 7, 3).unwrap();
        assert_eq!(removed, 1);
        assert!(get(&conn, "popular").unwrap().is_some());
        assert!(get(&conn, "stale").unwrap().is_none());
    }

    #[test]
    fn cleanup_stale_keeps_proposed_rows_even_if_low_count() {
        let conn = fresh_db();
        incr_or_insert(&conn, "h1", "a", &seq()).unwrap();
        mark_proposed(&conn, "h1").unwrap();
        let thirty_days_ms: i64 = 30 * 24 * 60 * 60 * 1000;
        conn.execute(
            "UPDATE skill_patterns SET last_seen = ?1 WHERE hash = 'h1'",
            params![now_ms() - thirty_days_ms],
        )
        .unwrap();
        let removed = cleanup_stale(&conn, 7, 5).unwrap();
        assert_eq!(removed, 0);
    }

    #[test]
    fn list_ready_caps_at_limit() {
        let conn = fresh_db();
        for h in 0..5 {
            for _ in 0..3 {
                incr_or_insert(&conn, &format!("h{h}"), "a", &seq()).unwrap();
            }
        }
        assert_eq!(list_ready(&conn, 3, 7, 2).unwrap().len(), 2);
    }

    #[test]
    fn get_returns_none_for_unknown_hash() {
        let conn = fresh_db();
        assert!(get(&conn, "ghost").unwrap().is_none());
    }

    #[test]
    fn tool_sequence_roundtrips_through_json() {
        let conn = fresh_db();
        let s = vec!["a".into(), "b with spaces".into(), "c".into()];
        let row = incr_or_insert(&conn, "h", "agent", &s).unwrap();
        assert_eq!(row.tool_sequence, s);
    }

    #[test]
    fn empty_sequence_serializes_as_empty_array() {
        let conn = fresh_db();
        let row = incr_or_insert(&conn, "h", "agent", &[]).unwrap();
        assert!(row.tool_sequence.is_empty());
    }
}
