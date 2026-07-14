//! Per-project Captain configuration discovered at TUI start.
//!
//! A `.captain.toml` placed at the root of a project (or any ancestor of the
//! current working directory) lets Captain auto-bind to the right agent,
//! workspace and tool profile when the user runs `captain` from inside that
//! project — Claude Code style.
//!
//! Only the fields below are recognised. Everything else is ignored so the
//! file format can grow without breaking older binaries.

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Top-level structure of `.captain.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(default)]
    pub captain: WorkspaceCaptain,
}

/// `[captain]` section.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WorkspaceCaptain {
    /// Bind to an existing agent by UUID. Wins over `agent_name`.
    pub agent: Option<String>,
    /// Bind to the first agent matching this name (case-insensitive).
    pub agent_name: Option<String>,
    /// Optional project slug (`project_*` family) for memory-graph wing
    /// binding. Currently informational; kernel uses it lazily.
    pub project_slug: Option<String>,
    /// Default tool profile applied when entering the chat tab.
    pub tool_profile: Option<String>,
    /// Extra workspace paths to surface to the kernel sandbox.
    #[serde(default)]
    pub extra_paths: Vec<PathBuf>,
}

/// Result of a workspace lookup.
#[derive(Debug, Clone)]
pub struct DiscoveredWorkspace {
    /// Absolute path of the `.captain.toml` that matched.
    pub config_path: PathBuf,
    /// Parsed configuration body.
    pub config: WorkspaceConfig,
}

/// Walk up from `start` looking for `.captain.toml`.
///
/// Stops at the user's home directory (`HOME` / `dirs::home_dir()`) so a
/// stray `.captain.toml` higher up — or on a system-shared mount — never
/// silently overrides a project. The home boundary is inclusive: a
/// `.captain.toml` at `$HOME` itself is honoured, but the walk does not
/// continue above it.
///
/// Returns:
/// - `Ok(Some(_))` when a file is found and parses successfully.
/// - `Ok(None)` when no `.captain.toml` exists between `start` and `$HOME`.
/// - `Err(_)` when a file exists but is malformed — the caller should surface
///   the error and fall back to the welcome menu rather than hide it.
pub fn discover(start: &Path) -> Result<Option<DiscoveredWorkspace>, String> {
    let home = dirs::home_dir().and_then(|h| h.canonicalize().ok());
    discover_with_boundary(start, home.as_deref())
}

/// Same as [`discover`], but lets the caller pin the home-boundary path —
/// used by tests to make discovery deterministic on temporary directories
/// that live outside the real `$HOME`.
pub fn discover_with_boundary(
    start: &Path,
    home_boundary: Option<&Path>,
) -> Result<Option<DiscoveredWorkspace>, String> {
    let mut cur = match start.canonicalize() {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    let boundary = home_boundary.map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
    loop {
        let candidate = cur.join(".captain.toml");
        if candidate.is_file() {
            let text = std::fs::read_to_string(&candidate)
                .map_err(|e| format!("read {} failed: {e}", candidate.display()))?;
            let config: WorkspaceConfig = toml::from_str(&text)
                .map_err(|e| format!("parse {} failed: {e}", candidate.display()))?;
            return Ok(Some(DiscoveredWorkspace {
                config_path: candidate,
                config,
            }));
        }
        // Stop the walk at the home boundary so a system-wide `.captain.toml`
        // never contaminates a project that did not opt in. We test the
        // ancestor *before* popping so a config at `$HOME` itself is read.
        if let Some(ref b) = boundary {
            if cur == *b {
                return Ok(None);
            }
        }
        if !cur.pop() {
            return Ok(None);
        }
    }
}

/// Expand a leading `~/` (or bare `~`) using `dirs::home_dir`. Falls back to
/// the raw path when `$HOME` cannot be resolved. Matches the convention
/// every shell uses, so a `.captain.toml` author can write
/// `extra_paths = ["~/.ssh"]` and have it match the same blocklist as the
/// kernel does internally.
fn expand_home(path: &Path) -> PathBuf {
    let s = match path.to_str() {
        Some(s) => s,
        None => return path.to_path_buf(),
    };
    if s == "~" {
        return dirs::home_dir().unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    path.to_path_buf()
}

/// Validate `extra_paths` against the credential blocklist. Returns the
/// canonicalised list on success, or the violating path on rejection. The
/// blocklist is sourced from `captain_kernel::default_blocked_workspace_paths`
/// so the CLI and the kernel always enforce the same protections — adding a
/// new credential file (`~/.aws/credentials`, …) only requires editing the
/// kernel's central list.
pub fn validate_extra_paths(raw: &[PathBuf], captain_home: &Path) -> Result<Vec<PathBuf>, String> {
    let blocked = captain_kernel::default_blocked_workspace_paths(captain_home);
    let blocked_canon: Vec<PathBuf> = blocked
        .iter()
        .map(|b| b.canonicalize().unwrap_or_else(|_| b.clone()))
        .collect();

    let mut accepted = Vec::with_capacity(raw.len());
    for p in raw {
        let expanded = expand_home(p);
        let canon = expanded.canonicalize().unwrap_or_else(|_| expanded.clone());
        for b in &blocked_canon {
            if canon == *b || canon.starts_with(b) {
                return Err(format!(
                    "refused workspace extra_path {}: inside protected zone {}",
                    canon.display(),
                    b.display()
                ));
            }
        }
        accepted.push(canon);
    }
    Ok(accepted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_config(dir: &Path, body: &str) {
        std::fs::write(dir.join(".captain.toml"), body).unwrap();
    }

    #[test]
    fn discover_finds_file_in_current_dir() {
        let tmp = tempfile::tempdir().unwrap();
        write_config(
            tmp.path(),
            "[captain]\nagent_name = \"captain\"\nproject_slug = \"demo\"\n",
        );
        let result = discover(tmp.path()).unwrap().unwrap();
        assert_eq!(result.config.captain.agent_name.as_deref(), Some("captain"));
        assert_eq!(result.config.captain.project_slug.as_deref(), Some("demo"));
    }

    #[test]
    fn discover_walks_up_to_ancestor() {
        let root = tempfile::tempdir().unwrap();
        write_config(
            root.path(),
            "[captain]\nagent = \"00000000-0000-0000-0000-000000000001\"\n",
        );
        let nested = root.path().join("sub").join("deep");
        std::fs::create_dir_all(&nested).unwrap();
        let result = discover(&nested).unwrap().unwrap();
        assert_eq!(
            result.config.captain.agent.as_deref(),
            Some("00000000-0000-0000-0000-000000000001")
        );
    }

    #[test]
    fn discover_returns_none_when_no_config_found() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        // Use the boundary override so this test does not accidentally pick
        // up a `.captain.toml` higher up in the real filesystem.
        let result = discover_with_boundary(&nested, Some(tmp.path())).unwrap();
        assert!(result.is_none(), "unexpected match outside the project");
    }

    #[test]
    fn discover_stops_exactly_at_boundary() {
        // A `.captain.toml` placed *above* the boundary must NOT be picked
        // up. This proves that a stray system-wide config cannot contaminate
        // a project that did not opt in.
        let outer = tempfile::tempdir().unwrap();
        let boundary = outer.path().join("home");
        std::fs::create_dir_all(&boundary).unwrap();
        let project = boundary.join("project");
        std::fs::create_dir_all(&project).unwrap();
        // The would-be poisoner sits one level above the boundary.
        write_config(outer.path(), "[captain]\nagent_name = \"poisoner\"\n");
        let result = discover_with_boundary(&project, Some(&boundary)).unwrap();
        assert!(result.is_none(), "boundary was crossed");
    }

    #[test]
    fn discover_surfaces_parse_errors() {
        let tmp = tempfile::tempdir().unwrap();
        write_config(tmp.path(), "this is not toml === broken\n");
        let err = discover(tmp.path()).unwrap_err();
        assert!(err.contains("parse"));
    }

    #[test]
    fn discover_ignores_unknown_keys() {
        let tmp = tempfile::tempdir().unwrap();
        write_config(
            tmp.path(),
            "[captain]\nagent_name = \"a\"\nfuture_field = 42\n",
        );
        let result = discover(tmp.path()).unwrap().unwrap();
        assert_eq!(result.config.captain.agent_name.as_deref(), Some("a"));
    }

    #[test]
    fn validate_extra_paths_rejects_ssh_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let captain_home = tmp.path();

        // We cannot canonicalize `~/.ssh` reliably across CI hosts, so
        // simulate the credential by creating a fake one inside captain_home
        // and asserting the inclusive-blocklist semantics.
        let fake_secrets = captain_home.join("secrets.env");
        std::fs::write(&fake_secrets, "TOKEN=x\n").unwrap();

        let attempt = vec![fake_secrets.clone()];
        let err = validate_extra_paths(&attempt, captain_home).unwrap_err();
        assert!(err.contains("protected zone"), "got: {err}");
        assert!(err.contains("secrets.env"), "got: {err}");
    }

    #[test]
    fn validate_extra_paths_rejects_tilde_expanded_ssh() {
        // A user writing `~/.ssh/id_rsa` in `.captain.toml` must hit the
        // sandbox even though `canonicalize` cannot resolve the leading
        // tilde. The fallback `expand_home` ensures the rejection.
        if dirs::home_dir().is_none() {
            return;
        }
        let captain_home = tempfile::tempdir().unwrap();
        let attempt = vec![PathBuf::from("~/.ssh")];
        let err = validate_extra_paths(&attempt, captain_home.path()).unwrap_err();
        assert!(err.contains("protected zone"), "got: {err}");
        assert!(err.contains(".ssh"), "got: {err}");
    }

    #[test]
    fn validate_extra_paths_accepts_ordinary_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let captain_home = tmp.path().join("captain_home");
        std::fs::create_dir_all(&captain_home).unwrap();
        let normal = tmp.path().join("project_extra");
        std::fs::create_dir_all(&normal).unwrap();

        let resolved = validate_extra_paths(&[normal.clone()], &captain_home).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], normal.canonicalize().unwrap());
    }

    #[test]
    fn discover_stops_at_home_boundary() {
        // We cannot move `$HOME` reliably from a unit test, so this case is
        // covered by the doc invariant + the live integration tests. Keep
        // a sanity check that `discover` does not panic when `home_dir()` is
        // reachable and the start path is unrelated to it.
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        let _ = discover(&nested).unwrap();
    }
}
