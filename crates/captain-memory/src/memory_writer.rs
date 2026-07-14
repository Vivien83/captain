//! Write-through persistence buffer for MemPalace (v3.12a).
//!
//! Every call that writes a semantic triple to MemPalace (from the
//! `memory_store` tool, from `mirror_to_mempalace`, or from the future
//! `LearningCommitter`) is first captured here in SQLite. A background
//! worker then attempts the actual MemPalace `kg_add` / `drawer_add`
//! call and flips the row's `sync_status` when it succeeds.
//!
//! The key invariant: **local SQLite is never the source of truth.**
//! MemPalace remains canonical. This table is the crash-resistant queue
//! so that a momentary MCP outage never loses a write.
//!
//! States:
//! - `pending` — not yet acknowledged by MemPalace (or retry in flight)
//! - `synced`  — MemPalace confirmed the write (permanent audit trail)
//! - `error`   — exceeded `MAX_SYNC_ATTEMPTS` consecutive failures;
//!   kept for 30 days then GC'd by `cleanup_errors`.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Cap on consecutive MemPalace failures before the row is considered dead.
pub const MAX_SYNC_ATTEMPTS: i32 = 3;

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
}

/// Insert a new row in `pending` state. Local persistence always succeeds
/// (modulo IO) — never fails because MemPalace is down.
pub fn append(conn: &Connection, input: NewMemoryWrite) -> Result<MemoryWrite, rusqlite::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_ms();
    conn.execute(
        "INSERT INTO memory_writes
         (id, subject, predicate, object, wing, room, source,
          sync_status, sync_attempts, created_at, synced_at, last_error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', 0, ?8, NULL, NULL)",
        params![
            id,
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
        "SELECT id, subject, predicate, object, wing, room, source,
                sync_status, sync_attempts, created_at, synced_at, last_error
         FROM memory_writes WHERE id = ?1",
    )?;
    stmt.query_row(params![id], row_to_memory_write).optional()
}

/// Mark a row as successfully synced to MemPalace.
pub fn mark_synced(conn: &Connection, id: &str) -> Result<(), rusqlite::Error> {
    let now = now_ms();
    conn.execute(
        "UPDATE memory_writes
         SET sync_status = 'synced', synced_at = ?1, last_error = NULL
         WHERE id = ?2",
        params![now, id],
    )?;
    Ok(())
}

/// Record a sync failure. Returns the new status: `Pending` if retries
/// remain, `Error` once the attempt count reaches `MAX_SYNC_ATTEMPTS`.
pub fn mark_error(conn: &Connection, id: &str, error: &str) -> Result<SyncStatus, rusqlite::Error> {
    let current = get(conn, id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    let new_attempts = current.sync_attempts + 1;
    let new_status = if new_attempts >= MAX_SYNC_ATTEMPTS {
        SyncStatus::Error
    } else {
        SyncStatus::Pending
    };
    conn.execute(
        "UPDATE memory_writes
         SET sync_status = ?1, sync_attempts = ?2, last_error = ?3
         WHERE id = ?4",
        params![new_status.as_str(), new_attempts, error, id],
    )?;
    Ok(new_status)
}

/// List rows with `sync_status='pending'`, oldest first — what the
/// resync worker iterates.
pub fn list_pending(conn: &Connection, limit: usize) -> Result<Vec<MemoryWrite>, rusqlite::Error> {
    let cap = limit.min(10_000) as i64;
    let mut stmt = conn.prepare(
        "SELECT id, subject, predicate, object, wing, room, source,
                sync_status, sync_attempts, created_at, synced_at, last_error
         FROM memory_writes
         WHERE sync_status = 'pending'
         ORDER BY created_at ASC, id ASC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![cap], row_to_memory_write)?;
    rows.collect()
}

/// Delete `error` rows older than `older_than_ms` milliseconds. Synced rows
/// are kept indefinitely for the audit trail.
pub fn cleanup_errors(conn: &Connection, older_than_ms: i64) -> Result<usize, rusqlite::Error> {
    let cutoff = now_ms() - older_than_ms;
    let rows = conn.execute(
        "DELETE FROM memory_writes
         WHERE sync_status = 'error' AND created_at < ?1",
        params![cutoff],
    )?;
    Ok(rows)
}

/// Delete every `memory_writes` row whose `subject` / `predicate` /
/// `object` matches the supplied LIKE patterns. Each filter is
/// optional but at least one must be `Some` (the caller enforces this
/// and the function returns 0 deletes when all three are `None`,
/// never wiping the whole table by accident).
///
/// Returns the number of rows actually removed. Powers the
/// `memory_forget` tool — the LLM-facing way to retract a fact it
/// stored incorrectly without leaving a stale triple in MemPalace.
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
            "SELECT id, subject, predicate, object, wing, room, source,
                    sync_status, sync_attempts, created_at, synced_at, last_error
             FROM memory_writes
             WHERE source LIKE ?1
             ORDER BY created_at DESC, id DESC
             LIMIT ?2",
            format!("{p}%"),
        ),
        None => (
            "SELECT id, subject, predicate, object, wing, room, source,
                    sync_status, sync_attempts, created_at, synced_at, last_error
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

fn row_to_memory_write(row: &rusqlite::Row<'_>) -> Result<MemoryWrite, rusqlite::Error> {
    let status_str: String = row.get(7)?;
    let sync_status = SyncStatus::parse(&status_str).unwrap_or(SyncStatus::Pending);
    Ok(MemoryWrite {
        id: row.get(0)?,
        subject: row.get(1)?,
        predicate: row.get(2)?,
        object: row.get(3)?,
        wing: row.get(4)?,
        room: row.get(5)?,
        source: row.get(6)?,
        sync_status,
        sync_attempts: row.get(8)?,
        created_at: row.get(9)?,
        synced_at: row.get(10)?,
        last_error: row.get(11)?,
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
    fn cleanup_errors_removes_old_error_rows_only() {
        let conn = fresh_db();
        // Synced row — must survive
        let synced = append(&conn, sample("s")).unwrap();
        mark_synced(&conn, &synced.id).unwrap();

        // Error row (backdated > 30 days)
        let old_err = append(&conn, sample("e")).unwrap();
        for _ in 0..3 {
            mark_error(&conn, &old_err.id, "x").unwrap();
        }
        // Backdate created_at to 40 days ago
        let forty_days_ms = 40 * 24 * 60 * 60 * 1000;
        let old_ts = now_ms() - forty_days_ms;
        conn.execute(
            "UPDATE memory_writes SET created_at = ?1 WHERE id = ?2",
            params![old_ts, old_err.id],
        )
        .unwrap();

        // Recent error row (must survive)
        let recent_err = append(&conn, sample("r")).unwrap();
        for _ in 0..3 {
            mark_error(&conn, &recent_err.id, "x").unwrap();
        }

        let thirty_days_ms = 30 * 24 * 60 * 60 * 1000;
        let removed = cleanup_errors(&conn, thirty_days_ms).unwrap();
        assert_eq!(removed, 1);
        assert!(get(&conn, &synced.id).unwrap().is_some());
        assert!(get(&conn, &old_err.id).unwrap().is_none());
        assert!(get(&conn, &recent_err.id).unwrap().is_some());
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
