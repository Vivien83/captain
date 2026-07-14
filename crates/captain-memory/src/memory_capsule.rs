//! Compact always-on memory capsule for prompt injection.
//!
//! `memory_writes` remains the audit/replay queue for MemPalace. This module
//! derives a short, deterministic, declarative view from that queue so the
//! runtime can provide Captain-native continuity without dumping raw memory or
//! procedural recipes into every prompt.

use crate::memory_writer::{self, MemoryWrite, SyncStatus};
use rusqlite::Connection;
use std::collections::BTreeSet;

pub const DEFAULT_MAX_ITEMS: usize = 24;
pub const DEFAULT_MAX_CHARS: usize = 3_000;

/// Build a compact, declarative capsule from recent committed/queued facts.
///
/// Returns `None` when there is no useful fact to inject. The output is kept
/// intentionally plain: one bullet per fact, with no imperative wording.
pub fn build_from_writes(
    conn: &Connection,
    max_items: usize,
    max_chars: usize,
) -> Result<Option<String>, rusqlite::Error> {
    let rows = memory_writer::list_recent(conn, None, max_items.saturating_mul(4).max(max_items))?;
    Ok(build_from_rows(&rows, max_items, max_chars))
}

pub fn build_from_rows(rows: &[MemoryWrite], max_items: usize, max_chars: usize) -> Option<String> {
    let mut seen = BTreeSet::new();
    let mut lines = Vec::new();
    let mut used = 0usize;

    for row in rows {
        if lines.len() >= max_items {
            break;
        }
        if !matches!(row.sync_status, SyncStatus::Pending | SyncStatus::Synced) {
            continue;
        }
        if looks_procedural_or_imperative(&row.object) {
            continue;
        }
        let key = format!(
            "{}|{}|{}",
            normalize_key(&row.subject),
            normalize_key(&row.predicate),
            normalize_key(&row.object)
        );
        if key.is_empty() || !seen.insert(key) {
            continue;
        }

        let scope = match (row.wing.as_deref(), row.room.as_deref()) {
            (Some(wing), Some(room)) if !wing.is_empty() && !room.is_empty() => {
                format!("{wing}/{room}")
            }
            (Some(wing), _) if !wing.is_empty() => wing.to_string(),
            _ => "memory".to_string(),
        };
        let line = format!(
            "- [{}] {} {} {}",
            cap(&scope, 64),
            cap(row.subject.trim(), 80),
            cap(row.predicate.trim(), 64),
            cap(row.object.trim(), 220)
        );
        let projected = used + line.len() + 1;
        if projected > max_chars {
            break;
        }
        used = projected;
        lines.push(line);
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn looks_procedural_or_imperative(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    let imperative_prefixes = [
        "always ", "never ", "do not ", "don't ", "run ", "execute ", "call ", "use ", "create ",
        "modify ", "delete ", "add ", "remove ",
    ];
    imperative_prefixes
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

fn normalize_key(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_ascii_lowercase()
}

fn cap(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max_chars).collect();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_writer::{MemoryWrite, SyncStatus};

    fn row(subject: &str, predicate: &str, object: &str) -> MemoryWrite {
        MemoryWrite {
            id: uuid::Uuid::new_v4().to_string(),
            subject: subject.into(),
            predicate: predicate.into(),
            object: object.into(),
            wing: Some("learnings".into()),
            room: Some("user_preferences".into()),
            source: "learning.test".into(),
            sync_status: SyncStatus::Synced,
            sync_attempts: 0,
            created_at: 1,
            synced_at: Some(2),
            last_error: None,
        }
    }

    #[test]
    fn capsule_keeps_declarative_facts() {
        let rows = vec![row(
            "user",
            "prefers_validation_channel",
            "Telegram for learning approvals",
        )];
        let capsule = build_from_rows(&rows, 10, 500).unwrap();
        assert!(capsule.contains("user prefers_validation_channel Telegram"));
        assert!(capsule.contains("learnings/user_preferences"));
    }

    #[test]
    fn capsule_filters_imperative_memory() {
        let rows = vec![row("assistant", "rule", "Always ask before changing files")];
        assert!(build_from_rows(&rows, 10, 500).is_none());
    }

    #[test]
    fn capsule_deduplicates_normalized_rows() {
        let rows = vec![
            row("user", "prefers", "concise technical answers"),
            row(" user ", " prefers ", "concise   technical answers"),
        ];
        let capsule = build_from_rows(&rows, 10, 500).unwrap();
        assert_eq!(capsule.lines().count(), 1);
    }
}
