//! Active project tracking for `/project <slug>` slash commands (v3.11d).
//!
//! Tracks which project an agent is currently focused on. Persisted to
//! `~/.captain/active_project.json` keyed by agent id so the choice
//! survives daemon restarts. Reads are lock-free and cheap — every
//! chat turn consults the file to enrich the prompt context.
//!
//! The module is intentionally decoupled from the `Project` entity
//! itself: it only stores the slug. Resolution (slug → Project row)
//! happens at the caller's layer so a stale file doesn't crash the
//! agent loop if a project has since been archived.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

const FILE_NAME: &str = "active_project.json";

/// In-memory registry mapping `agent_id` → active project slug.
/// Mirrored to disk on every write; loaded lazily on first read.
pub struct ActiveProjectRegistry {
    inner: DashMap<String, String>,
    path: PathBuf,
}

impl ActiveProjectRegistry {
    pub fn open(home_dir: &Path) -> Self {
        let path = home_dir.join(FILE_NAME);
        let inner = load_from_disk(&path).unwrap_or_default();
        Self { inner, path }
    }

    /// Lookup the active slug for an agent. Returns `None` when the
    /// agent hasn't picked a project.
    pub fn get(&self, agent_id: &str) -> Option<String> {
        self.inner.get(agent_id).map(|v| v.clone())
    }

    /// Set the active slug for an agent. Flushes to disk — a failure
    /// is logged at `warn` but does not propagate, since the live
    /// in-memory state is still correct for the running daemon.
    pub fn set(&self, agent_id: String, slug: String) {
        self.inner.insert(agent_id, slug);
        self.flush();
    }

    /// Clear the active project for an agent. No-op if absent.
    pub fn clear(&self, agent_id: &str) -> bool {
        let removed = self.inner.remove(agent_id).is_some();
        if removed {
            self.flush();
        }
        removed
    }

    /// Snapshot of all active projects. Used by diagnostics + the
    /// `/project list` slash command to flag which is current.
    pub fn snapshot(&self) -> std::collections::HashMap<String, String> {
        self.inner
            .iter()
            .map(|r| (r.key().clone(), r.value().clone()))
            .collect()
    }

    fn flush(&self) {
        let map: std::collections::HashMap<String, String> = self.snapshot();
        if let Err(err) = save_to_disk(&self.path, &map) {
            tracing::warn!("active_project flush failed: {err}");
        }
    }
}

static GLOBAL_REGISTRY: OnceLock<Arc<ActiveProjectRegistry>> = OnceLock::new();

/// Install a process-wide registry. Called once at kernel boot.
pub fn install(home_dir: &Path) -> Arc<ActiveProjectRegistry> {
    let reg = Arc::new(ActiveProjectRegistry::open(home_dir));
    let _ = GLOBAL_REGISTRY.set(Arc::clone(&reg));
    reg
}

/// Access the installed registry, if any. Returns `None` before
/// `install` has been called (e.g. inside unit tests).
pub fn global() -> Option<Arc<ActiveProjectRegistry>> {
    GLOBAL_REGISTRY.get().cloned()
}

// ---------------------------------------------------------------------------
// Slash command parser
// ---------------------------------------------------------------------------

/// Parsed form of a `/project …` line typed by the user.
#[derive(Debug, Clone, PartialEq)]
pub enum SlashCommand {
    /// `/project <slug>` — switch context to this slug.
    Switch(String),
    /// `/project list` — show available projects + current marker.
    List,
    /// `/project clear` — drop the current context.
    Clear,
    /// Not a `/project` command.
    None,
}

/// Parse the first line of a user message. Whitespace-tolerant;
/// returns [`SlashCommand::None`] when the line doesn't start with
/// `/project`.
pub fn parse_slash(line: &str) -> SlashCommand {
    let trimmed = line.trim();
    let rest = match trimmed.strip_prefix("/project") {
        Some(r) => r.trim(),
        None => return SlashCommand::None,
    };
    if rest.is_empty() {
        return SlashCommand::List;
    }
    match rest {
        "list" | "ls" => SlashCommand::List,
        "clear" | "none" | "off" => SlashCommand::Clear,
        other => SlashCommand::Switch(other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Disk I/O
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Default)]
struct Persistent {
    version: u32,
    entries: std::collections::HashMap<String, String>,
}

fn load_from_disk(path: &Path) -> Option<DashMap<String, String>> {
    let raw = std::fs::read_to_string(path).ok()?;
    let parsed: Persistent = serde_json::from_str(&raw).ok()?;
    let map = DashMap::new();
    for (k, v) in parsed.entries {
        map.insert(k, v);
    }
    Some(map)
}

fn save_to_disk(
    path: &Path,
    entries: &std::collections::HashMap<String, String>,
) -> std::io::Result<()> {
    let payload = Persistent {
        version: 1,
        entries: entries.clone(),
    };
    let raw = serde_json::to_string_pretty(&payload)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    captain_types::durable_fs::atomic_write(path, raw.as_bytes())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn set_then_get_roundtrip() {
        let dir = temp_home();
        let reg = ActiveProjectRegistry::open(dir.path());
        reg.set("agent-1".into(), "alpha".into());
        assert_eq!(reg.get("agent-1").as_deref(), Some("alpha"));
    }

    #[test]
    fn persistence_across_reopen() {
        let dir = temp_home();
        {
            let reg = ActiveProjectRegistry::open(dir.path());
            reg.set("captain".into(), "mlx-finetune".into());
        }
        let reg2 = ActiveProjectRegistry::open(dir.path());
        assert_eq!(reg2.get("captain").as_deref(), Some("mlx-finetune"));
    }

    #[test]
    fn clear_removes_entry_and_flushes() {
        let dir = temp_home();
        let reg = ActiveProjectRegistry::open(dir.path());
        reg.set("a".into(), "p1".into());
        assert!(reg.clear("a"));
        assert!(reg.get("a").is_none());

        let reg2 = ActiveProjectRegistry::open(dir.path());
        assert!(reg2.get("a").is_none());
    }

    #[test]
    fn clear_missing_returns_false() {
        let dir = temp_home();
        let reg = ActiveProjectRegistry::open(dir.path());
        assert!(!reg.clear("ghost"));
    }

    #[test]
    fn snapshot_reflects_entries() {
        let dir = temp_home();
        let reg = ActiveProjectRegistry::open(dir.path());
        reg.set("a".into(), "p1".into());
        reg.set("b".into(), "p2".into());
        let s = reg.snapshot();
        assert_eq!(s.len(), 2);
        assert_eq!(s.get("a").map(|v| v.as_str()), Some("p1"));
    }

    // --- Slash parser ---

    #[test]
    fn parse_switch_recognises_slug() {
        assert_eq!(
            parse_slash("/project alpha"),
            SlashCommand::Switch("alpha".into())
        );
        assert_eq!(
            parse_slash("   /project  beta  "),
            SlashCommand::Switch("beta".into())
        );
    }

    #[test]
    fn parse_list_and_clear_variants() {
        assert_eq!(parse_slash("/project"), SlashCommand::List);
        assert_eq!(parse_slash("/project list"), SlashCommand::List);
        assert_eq!(parse_slash("/project ls"), SlashCommand::List);
        assert_eq!(parse_slash("/project clear"), SlashCommand::Clear);
        assert_eq!(parse_slash("/project none"), SlashCommand::Clear);
        assert_eq!(parse_slash("/project off"), SlashCommand::Clear);
    }

    #[test]
    fn parse_returns_none_for_non_project_lines() {
        assert_eq!(parse_slash("hello world"), SlashCommand::None);
        assert_eq!(parse_slash("/compact"), SlashCommand::None);
        assert_eq!(parse_slash(""), SlashCommand::None);
    }
}
