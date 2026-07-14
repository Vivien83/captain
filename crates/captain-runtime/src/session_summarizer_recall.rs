use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde::Serialize;
use tracing::debug;

const SESSION_INDEX_DB: &str = ".captain-session-checkpoints.fts.sqlite";

/// Result entry returned by `recall_checkpoints` — one matched session.
#[derive(Debug, Clone, Serialize)]
pub struct RecallHit {
    pub session_id: String,
    pub agent_key: String,
    pub updated_at: u64,
    pub snippet: String,
}

/// Insensitive multi-word AND match: every token in `query` (split on
/// whitespace) must appear somewhere in `haystack` (also lowercased).
/// Empty query → matches everything.
pub fn matches_query(haystack: &str, query: &str) -> bool {
    let lower = haystack.to_lowercase();
    query
        .split_whitespace()
        .all(|tok| lower.contains(&tok.to_lowercase()))
}

pub fn session_index_path(sessions_root: &Path) -> PathBuf {
    sessions_root.join(SESSION_INDEX_DB)
}

fn open_session_index(sessions_root: &Path) -> Result<Connection, rusqlite::Error> {
    let path = session_index_path(sessions_root);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
    ensure_session_index(&conn)?;
    Ok(conn)
}

fn ensure_session_index(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE VIRTUAL TABLE IF NOT EXISTS session_checkpoint_fts
        USING fts5(
            session_id UNINDEXED,
            agent_key UNINDEXED,
            updated_at UNINDEXED,
            body,
            tokenize = 'unicode61'
        );
        ",
    )
}

pub fn index_checkpoint(
    sessions_root: &Path,
    session_id: &str,
    agent_key: &str,
    updated_at: u64,
    body: &str,
) -> Result<(), rusqlite::Error> {
    let conn = open_session_index(sessions_root)?;
    conn.execute(
        "DELETE FROM session_checkpoint_fts WHERE session_id = ?1 AND agent_key = ?2",
        params![session_id, agent_key],
    )?;
    conn.execute(
        "INSERT INTO session_checkpoint_fts (session_id, agent_key, updated_at, body)
         VALUES (?1, ?2, ?3, ?4)",
        params![session_id, agent_key, updated_at.to_string(), body],
    )?;
    Ok(())
}

pub(crate) fn sessions_root_for_json(json_path: &Path) -> Option<PathBuf> {
    json_path.parent()?.parent().map(Path::to_path_buf)
}

fn backfill_session_index(
    sessions_root: &Path,
    agent_filter: Option<&str>,
) -> Result<usize, rusqlite::Error> {
    let mut count = 0usize;
    for item in scan_checkpoint_files(sessions_root, agent_filter) {
        let (session_id, agent_key, updated_at, body) = item;
        index_checkpoint(sessions_root, &session_id, &agent_key, updated_at, &body)?;
        count += 1;
    }
    Ok(count)
}

fn fts_query(query: &str) -> String {
    query
        .split(|ch: char| !ch.is_alphanumeric())
        .map(str::trim)
        .filter(|tok| !tok.is_empty())
        .map(|tok| format!("\"{}\"", tok.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn recall_checkpoints_fts(
    sessions_root: &Path,
    query: &str,
    max_results: usize,
    agent_filter: Option<&str>,
) -> Result<Vec<RecallHit>, rusqlite::Error> {
    let conn = open_session_index(sessions_root)?;
    let cap = max_results.clamp(1, 20) as i64;
    let mut hits = Vec::new();

    if query.trim().is_empty() {
        let sql = if agent_filter.is_some() {
            "SELECT session_id, agent_key, updated_at, body
             FROM session_checkpoint_fts
             WHERE agent_key = ?1
             ORDER BY CAST(updated_at AS INTEGER) DESC
             LIMIT ?2"
        } else {
            "SELECT session_id, agent_key, updated_at, body
             FROM session_checkpoint_fts
             ORDER BY CAST(updated_at AS INTEGER) DESC
             LIMIT ?1"
        };
        let mut stmt = conn.prepare(sql)?;
        if let Some(agent) = agent_filter {
            let rows = stmt.query_map(params![agent, cap], row_to_recall_hit)?;
            for row in rows {
                hits.push(row?);
            }
        } else {
            let rows = stmt.query_map(params![cap], row_to_recall_hit)?;
            for row in rows {
                hits.push(row?);
            }
        }
        return Ok(hits);
    }

    let query = fts_query(query);
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let sql = if agent_filter.is_some() {
        "SELECT session_id, agent_key, updated_at, body
         FROM session_checkpoint_fts
         WHERE session_checkpoint_fts MATCH ?1 AND agent_key = ?2
         ORDER BY bm25(session_checkpoint_fts), CAST(updated_at AS INTEGER) DESC
         LIMIT ?3"
    } else {
        "SELECT session_id, agent_key, updated_at, body
         FROM session_checkpoint_fts
         WHERE session_checkpoint_fts MATCH ?1
         ORDER BY bm25(session_checkpoint_fts), CAST(updated_at AS INTEGER) DESC
         LIMIT ?2"
    };
    let mut stmt = conn.prepare(sql)?;
    if let Some(agent) = agent_filter {
        let rows = stmt.query_map(params![query, agent, cap], row_to_recall_hit)?;
        for row in rows {
            hits.push(row?);
        }
    } else {
        let rows = stmt.query_map(params![query, cap], row_to_recall_hit)?;
        for row in rows {
            hits.push(row?);
        }
    }
    Ok(hits)
}

fn row_to_recall_hit(row: &rusqlite::Row<'_>) -> Result<RecallHit, rusqlite::Error> {
    let session_id: String = row.get(0)?;
    let agent_key: String = row.get(1)?;
    let updated_at_str: String = row.get(2)?;
    let body: String = row.get(3)?;
    Ok(RecallHit {
        session_id,
        agent_key,
        updated_at: updated_at_str.parse().unwrap_or(0),
        snippet: trim_to(&body, 800),
    })
}

/// Best-effort scan of every checkpoint markdown under `sessions_root`,
/// returning hits whose body matches `query`. Sorted by updated_at desc
/// (freshest first), capped at `max_results`. `agent_filter` (when set)
/// restricts the walk to one agent's directory.
///
/// Used by the `session_recall` tool. Never panics — corrupt or
/// unreadable files are skipped silently.
pub fn recall_checkpoints(
    sessions_root: &Path,
    query: &str,
    max_results: usize,
    agent_filter: Option<&str>,
) -> Vec<RecallHit> {
    let index_path = session_index_path(sessions_root);
    if !index_path.exists() {
        match backfill_session_index(sessions_root, agent_filter) {
            Ok(n) if n > 0 => debug!(count = n, "session checkpoint FTS index backfilled"),
            Ok(_) => {}
            Err(e) => debug!(error = %e, "session checkpoint FTS backfill failed"),
        }
    }
    match recall_checkpoints_fts(sessions_root, query, max_results, agent_filter) {
        Ok(hits) if !hits.is_empty() || query.trim().is_empty() => return hits,
        Ok(_) => {}
        Err(e) => debug!(error = %e, "session checkpoint FTS recall failed; falling back to scan"),
    }
    scan_checkpoints(sessions_root, query, max_results, agent_filter)
}

fn scan_checkpoints(
    sessions_root: &Path,
    query: &str,
    max_results: usize,
    agent_filter: Option<&str>,
) -> Vec<RecallHit> {
    let mut hits: Vec<(u64, RecallHit)> = Vec::new();
    for (session_id, agent_key, updated_at, body) in
        scan_checkpoint_files(sessions_root, agent_filter)
    {
        if !matches_query(&body, query) {
            continue;
        }
        let snippet = trim_to(&body, 800);
        hits.push((
            updated_at,
            RecallHit {
                session_id,
                agent_key,
                updated_at,
                snippet,
            },
        ));
    }
    hits.sort_by_key(|h| std::cmp::Reverse(h.0));
    hits.truncate(max_results);
    hits.into_iter().map(|(_, h)| h).collect()
}

fn scan_checkpoint_files(
    sessions_root: &Path,
    agent_filter: Option<&str>,
) -> Vec<(String, String, u64, String)> {
    let mut out = Vec::new();
    let agent_dirs = match std::fs::read_dir(sessions_root) {
        Ok(d) => d,
        Err(_) => return out,
    };
    for ad in agent_dirs.flatten() {
        if !ad.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let agent_key = ad.file_name().to_string_lossy().into_owned();
        if let Some(f) = agent_filter {
            if agent_key != f {
                continue;
            }
        }
        let files = match std::fs::read_dir(ad.path()) {
            Ok(f) => f,
            Err(_) => continue,
        };
        for f in files.flatten() {
            let path = f.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if !name.ends_with(".checkpoint.md") {
                continue;
            }
            let body = match std::fs::read_to_string(&path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let updated_at = read_checkpoint_field(&body, "updated_at")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            let session_id = read_checkpoint_field(&body, "session_id").unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("?")
                    .to_string()
            });
            out.push((session_id, agent_key.clone(), updated_at, body));
        }
    }
    out
}

/// Pull a `key: value` field out of the YAML-ish front matter at the
/// top of a checkpoint markdown. Returns the trimmed value or None.
fn read_checkpoint_field(body: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    body.lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .map(|v| v.trim().to_string())
}

/// Truncate `s` to at most `n` chars (UTF-8 safe via char iteration),
/// adding an ellipsis marker so the LLM knows the snippet is partial.
fn trim_to(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n).collect();
    out.push('…');
    out
}
