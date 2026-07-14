use crate::{cli_captain_home, ui};

use super::DoctorReport;

struct MempalaceStatus {
    db_path: std::path::PathBuf,
    db_size: u64,
    total: u64,
    synced: u64,
    pending: u64,
    error: u64,
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
            report.push(serde_json::json!({
                "check": "mempalace",
                "status": "ok",
                "total": st.total,
                "synced": st.synced,
                "pending": st.pending,
                "error": st.error,
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
        ui::check_ok(&format!(
            "Writes: {} total ({} synced, {} pending, {} error)",
            st.total, st.synced, st.pending, st.error
        ));
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

    let total = count("");
    let synced = count("sync_status = 'synced'");
    let pending = count("sync_status = 'pending'");
    let error = count("sync_status = 'error'");
    let (last_write_iso, last_source) = last_memory_write(&conn);

    Some(MempalaceStatus {
        db_path,
        db_size: meta.len(),
        total,
        synced,
        pending,
        error,
        last_write_iso,
        last_source,
        process_pid: mempalace_process_pid(),
    })
}

fn last_memory_write(conn: &rusqlite::Connection) -> (Option<String>, Option<String>) {
    conn.query_row(
        "SELECT created_at, source FROM memory_writes ORDER BY created_at DESC LIMIT 1",
        [],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
    )
    .map(|(epoch_raw, src)| {
        let epoch_secs = if epoch_raw > 10_000_000_000 {
            epoch_raw / 1000
        } else {
            epoch_raw
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(epoch_secs);
        let delta = (now - epoch_secs).max(0) as u64;
        let rel = if delta < 60 {
            format!("{delta}s ago")
        } else if delta < 3600 {
            format!("{}m ago", delta / 60)
        } else if delta < 86400 {
            format!("{}h ago", delta / 3600)
        } else {
            format!("{}j ago", delta / 86400)
        };
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
