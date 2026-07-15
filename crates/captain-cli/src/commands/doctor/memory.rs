use crate::{cli_captain_home, ui};

use super::DoctorReport;

struct MempalaceStatus {
    db_path: std::path::PathBuf,
    db_size: u64,
    total: u64,
    synced: u64,
    pending: u64,
    error: u64,
    retracted: u64,
    oldest_unsynced_age: Option<String>,
    next_retry_in: Option<String>,
    max_sync_attempts: u64,
    last_sync_error: Option<String>,
    last_write_iso: Option<String>,
    last_source: Option<String>,
    process_pid: Option<u32>,
}

pub(super) fn check_mempalace(report: &mut DoctorReport) {
    let captain_dir = cli_captain_home();
    if !report.json {
        println!("\n  MemPalace Memory:");
    }
    match mempalace_status(&captain_dir) {
        Some(st) => {
            if !report.json {
                print_mempalace_status(&st);
            }
            let status = if st.error > 0 || st.pending > 0 {
                "warn"
            } else {
                "ok"
            };
            report.push(serde_json::json!({
                "check": "mempalace",
                "status": status,
                "total": st.total,
                "synced": st.synced,
                "pending": st.pending,
                "error": st.error,
                "retracted": st.retracted,
                "oldest_unsynced_age": st.oldest_unsynced_age,
                "next_retry_in": st.next_retry_in,
                "max_sync_attempts": st.max_sync_attempts,
                "last_sync_error": st.last_sync_error,
                "continuity": "local_journal_available",
                "recovery": if st.error > 0 || st.pending > 0 { "automatic_retry_active" } else { "in_sync" },
                "process_running": st.process_pid.is_some(),
                "db_size_bytes": st.db_size,
                "last_write": st.last_write_iso,
                "last_source": st.last_source,
            }));
        }
        None => {
            if !report.json {
                ui::check_warn("Cannot read captain.db (will be created on first daemon boot)");
            }
            report.push(serde_json::json!({"check": "mempalace", "status": "warn"}));
        }
    }
}

fn print_mempalace_status(st: &MempalaceStatus) {
    let size_mb = st.db_size as f64 / 1_048_576.0;
    ui::check_ok(&format!(
        "Database: {} ({:.1} MB)",
        st.db_path.display(),
        size_mb
    ));
    match st.process_pid {
        Some(pid) => ui::check_ok(&format!("Process: PID {pid} (mempalace.mcp_server)")),
        None => ui::check_warn("Process: not running (will start when daemon boots)"),
    }
    if st.total > 0 {
        let writes = format!(
            "Writes: {} total ({} synced, {} pending, {} error)",
            st.total, st.synced, st.pending, st.error
        );
        if st.error > 0 || st.pending > 0 {
            ui::check_warn(&writes);
        } else {
            ui::check_ok(&writes);
        }
        if st.error > 0 {
            ui::check_warn(&format!(
                "Semantic index degraded; local journal remains available and automatic retry is active (oldest: {}, attempts max: {}, next retry: {})",
                st.oldest_unsynced_age.as_deref().unwrap_or("unknown"),
                st.max_sync_attempts,
                st.next_retry_in.as_deref().unwrap_or("on next worker tick"),
            ));
            if let Some(error) = &st.last_sync_error {
                ui::check_warn(&format!("Last sync error: {error}"));
            }
        } else if st.pending > 0 {
            ui::check_warn(&format!(
                "Queued safely in local journal; automatic retry {}",
                st.next_retry_in.as_deref().unwrap_or("on next worker tick")
            ));
        }
        if st.retracted > 0 {
            ui::check_ok(&format!(
                "Retractions: {} preserved in the audit journal",
                st.retracted
            ));
        }
        if let (Some(t), Some(s)) = (&st.last_write_iso, &st.last_source) {
            ui::check_ok(&format!("Last write: {t} (source: {s})"));
        }
    } else {
        ui::check_warn("Writes: 0 (memory is empty - agent hasn't stored anything yet)");
    }
}

fn mempalace_status(captain_dir: &std::path::Path) -> Option<MempalaceStatus> {
    let db_path = captain_dir.join("data").join("captain.db");
    let meta = std::fs::metadata(&db_path).ok()?;
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;

    let count = |where_clause: &str| -> u64 {
        let sql = if where_clause.is_empty() {
            "SELECT count(*) FROM memory_writes".to_string()
        } else {
            format!("SELECT count(*) FROM memory_writes WHERE {where_clause}")
        };
        conn.query_row(&sql, [], |r| r.get::<_, i64>(0))
            .map(|n| n as u64)
            .unwrap_or(0)
    };

    let journal = captain_memory::memory_writer::journal_health(&conn).ok();
    let (total, synced, pending, error, retracted) = match &journal {
        Some(health) => (
            health.total.max(0) as u64,
            health.synced.max(0) as u64,
            health.pending.max(0) as u64,
            health.error.max(0) as u64,
            health.retracted.max(0) as u64,
        ),
        None => (
            count(""),
            count("sync_status = 'synced'"),
            count("sync_status = 'pending'"),
            count("sync_status = 'error'"),
            0,
        ),
    };
    let retry = journal
        .map(|health| MemoryRetryStatus {
            oldest_unsynced_at: health.oldest_unsynced_at,
            next_retry_at: health.next_retry_at,
            max_sync_attempts: health.max_sync_attempts.max(0) as u64,
            last_sync_error: health.last_sync_error,
        })
        .unwrap_or_else(|| memory_retry_status(&conn));
    let (last_write_iso, last_source) = last_memory_write(&conn);

    Some(MempalaceStatus {
        db_path,
        db_size: meta.len(),
        total,
        synced,
        pending,
        error,
        retracted,
        oldest_unsynced_age: retry.oldest_unsynced_at.map(relative_age),
        next_retry_in: retry.next_retry_at.map(relative_retry),
        max_sync_attempts: retry.max_sync_attempts,
        last_sync_error: retry.last_sync_error,
        last_write_iso,
        last_source,
        process_pid: mempalace_process_pid(),
    })
}

#[derive(Debug, Default, PartialEq, Eq)]
struct MemoryRetryStatus {
    oldest_unsynced_at: Option<i64>,
    next_retry_at: Option<i64>,
    max_sync_attempts: u64,
    last_sync_error: Option<String>,
}

fn memory_retry_status(conn: &rusqlite::Connection) -> MemoryRetryStatus {
    let has_retry_metadata = column_exists(conn, "memory_writes", "next_retry_at");
    let aggregate_sql = if has_retry_metadata {
        "SELECT MIN(created_at), MIN(next_retry_at), MAX(sync_attempts)
         FROM memory_writes WHERE sync_status IN ('pending', 'error')"
    } else {
        "SELECT MIN(created_at), NULL, MAX(sync_attempts)
         FROM memory_writes WHERE sync_status IN ('pending', 'error')"
    };
    let (oldest_unsynced_at, next_retry_at, max_sync_attempts) = conn
        .query_row(aggregate_sql, [], |row| {
            Ok((
                row.get::<_, Option<i64>>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<i64>>(2)?.unwrap_or(0).max(0) as u64,
            ))
        })
        .unwrap_or((None, None, 0));
    let last_sync_error = conn
        .query_row(
            "SELECT last_error FROM memory_writes
             WHERE sync_status IN ('pending', 'error') AND last_error IS NOT NULL
             ORDER BY COALESCE(last_attempt_at, created_at) DESC, id DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .or_else(|_| {
            conn.query_row(
                "SELECT last_error FROM memory_writes
                 WHERE sync_status IN ('pending', 'error') AND last_error IS NOT NULL
                 ORDER BY created_at DESC, id DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
        })
        .ok()
        .map(|error| error.chars().take(500).collect());
    MemoryRetryStatus {
        oldest_unsynced_at,
        next_retry_at,
        max_sync_attempts,
        last_sync_error,
    }
}

fn column_exists(conn: &rusqlite::Connection, table: &str, column: &str) -> bool {
    let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({table})")) else {
        return false;
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(1)) else {
        return false;
    };
    let exists = rows.filter_map(Result::ok).any(|name| name == column);
    exists
}

fn relative_age(epoch_raw: i64) -> String {
    let epoch_secs = epoch_seconds(epoch_raw);
    let now = unix_now_seconds();
    human_duration(now.saturating_sub(epoch_secs) as u64, "ago")
}

fn relative_retry(epoch_raw: i64) -> String {
    let retry_at = epoch_seconds(epoch_raw);
    let now = unix_now_seconds();
    if retry_at <= now {
        "due now".into()
    } else {
        human_duration((retry_at - now) as u64, "from now")
    }
}

fn human_duration(seconds: u64, suffix: &str) -> String {
    let value = if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h", seconds / 3_600)
    } else {
        format!("{}d", seconds / 86_400)
    };
    format!("{value} {suffix}")
}

fn epoch_seconds(epoch_raw: i64) -> i64 {
    if epoch_raw > 10_000_000_000 {
        epoch_raw / 1_000
    } else {
        epoch_raw
    }
}

fn unix_now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn last_memory_write(conn: &rusqlite::Connection) -> (Option<String>, Option<String>) {
    conn.query_row(
        "SELECT created_at, source FROM memory_writes ORDER BY created_at DESC LIMIT 1",
        [],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
    )
    .map(|(epoch_raw, src)| {
        let epoch_secs = epoch_seconds(epoch_raw);
        let now = unix_now_seconds().max(epoch_secs);
        let rel = human_duration((now - epoch_secs) as u64, "ago");
        (Some(rel), Some(src))
    })
    .unwrap_or((None, None))
}

#[cfg(unix)]
fn mempalace_process_pid() -> Option<u32> {
    let out = std::process::Command::new("pgrep")
        .args(["-f", "mempalace.mcp_server"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
}

#[cfg(not(unix))]
fn mempalace_process_pid() -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_status_surfaces_durable_degraded_row() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        captain_memory::migration::run_migrations(&conn).unwrap();
        let row = captain_memory::memory_writer::append(
            &conn,
            captain_memory::memory_writer::NewMemoryWrite {
                subject: "user".into(),
                predicate: "prefers".into(),
                object: "concise answers".into(),
                wing: None,
                room: None,
                source: "test".into(),
            },
        )
        .unwrap();
        for _ in 0..3 {
            captain_memory::memory_writer::mark_error(&conn, &row.id, "backend unavailable")
                .unwrap();
        }

        let status = memory_retry_status(&conn);
        assert!(status.oldest_unsynced_at.is_some());
        assert!(status.next_retry_at.is_some());
        assert_eq!(status.max_sync_attempts, 3);
        assert_eq!(
            status.last_sync_error.as_deref(),
            Some("backend unavailable")
        );
    }

    #[test]
    fn retry_status_remains_compatible_with_pre_retry_schema() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE memory_writes (
                id TEXT PRIMARY KEY,
                sync_status TEXT NOT NULL,
                sync_attempts INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                last_error TEXT
            );
            INSERT INTO memory_writes VALUES ('old', 'pending', 2, 1000, 'offline');",
        )
        .unwrap();
        let status = memory_retry_status(&conn);
        assert_eq!(status.oldest_unsynced_at, Some(1000));
        assert_eq!(status.next_retry_at, None);
        assert_eq!(status.max_sync_attempts, 2);
        assert_eq!(status.last_sync_error.as_deref(), Some("offline"));
    }
}
