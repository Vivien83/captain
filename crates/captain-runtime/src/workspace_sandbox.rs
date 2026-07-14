//! Workspace filesystem sandboxing.
//!
//! Confines agent file operations to their workspace directory.
//! Prevents path traversal, symlink escapes, and access outside the sandbox.

use std::path::{Path, PathBuf};

/// Resolve a user-supplied path within a workspace sandbox.
///
/// - Rejects `..` components outright.
/// - Relative paths are joined with `workspace_root`.
/// - Absolute paths are checked against the workspace root after canonicalization.
/// - For new files: canonicalizes the parent directory and appends the filename.
/// - The final canonical path must start with the canonical workspace root.
pub fn resolve_sandbox_path(user_path: &str, workspace_root: &Path) -> Result<PathBuf, String> {
    let path = Path::new(user_path);

    // Reject any `..` components
    for component in path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err("Path traversal denied: '..' components are forbidden".to_string());
        }
    }

    // Build the candidate path
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };

    // Canonicalize the workspace root
    let canon_root = workspace_root
        .canonicalize()
        .map_err(|e| format!("Failed to resolve workspace root: {e}"))?;

    // Canonicalize the candidate (or its parent for new files)
    let canon_candidate = if candidate.exists() {
        candidate
            .canonicalize()
            .map_err(|e| format!("Failed to resolve path: {e}"))?
    } else {
        // For new files: canonicalize the parent and append the filename
        let parent = candidate
            .parent()
            .ok_or_else(|| "Invalid path: no parent directory".to_string())?;
        let filename = candidate
            .file_name()
            .ok_or_else(|| "Invalid path: no filename".to_string())?;
        let canon_parent = parent
            .canonicalize()
            .map_err(|e| format!("Failed to resolve parent directory: {e}"))?;
        canon_parent.join(filename)
    };

    // Verify the canonical path is inside the workspace
    if !canon_candidate.starts_with(&canon_root) {
        return Err(format!(
            "Access denied: path '{}' resolves outside workspace. \
             Do not retry the same call. To work on that directory, extend \
             the sandbox with workspace_add({{\"path\": \"<dir>\"}}) (main \
             Captain agent only), or read it via shell_exec (cat/find/grep). \
             mcp_filesystem_* tools also work if an MCP filesystem server is \
             configured.",
            user_path
        ));
    }

    Ok(canon_candidate)
}

/// Resolve a path against multiple allowed roots, with a hard blocklist
/// that wins over every allow rule.
///
/// Used by Captain (the principal agent) to grant access beyond its own
/// workspace — the kernel hands in `~/.captain/` and any user-declared
/// `workspace_add` paths. Any subagent / hand keeps the legacy single-
/// root `resolve_sandbox_path` because they have no business roaming.
///
/// Semantics:
/// - The path must canonicalize *inside* one of the `allowed_roots`.
/// - The path must NOT canonicalize inside any of the `blocked_paths`.
///   Blocklist trumps allowlist (e.g. `~/.captain/` is allowed but
///   `~/.captain/secrets.env` stays denied for direct file access).
/// - Empty `allowed_roots` rejects everything (no implicit access).
pub fn resolve_sandbox_path_multi(
    user_path: &str,
    allowed_roots: &[&Path],
    blocked_paths: &[&Path],
) -> Result<PathBuf, String> {
    if allowed_roots.is_empty() {
        return Err("Access denied: no workspace root configured".to_string());
    }

    let mut last_err = String::from("Access denied: path resolves outside every allowed root");
    let mut usable_roots = 0usize;
    for root in allowed_roots {
        // A root that no longer exists on disk cannot grant access; skip it
        // instead of letting its canonicalization failure overwrite the
        // actionable "Access denied" message. Seen live: stale extra_paths
        // in config.toml turned every denial into "Failed to resolve
        // workspace root: No such file or directory", so the agent never
        // saw the workspace_add/shell_exec guidance and retried.
        if root.canonicalize().is_err() {
            continue;
        }
        usable_roots += 1;
        match resolve_sandbox_path(user_path, root) {
            Ok(canon) => {
                for blocked in blocked_paths {
                    if let Some(canon_blocked) = canonicalize_existing_or_parent(blocked) {
                        if canon.starts_with(&canon_blocked) {
                            return Err(format!(
                                "Access denied: path '{}' is in a protected zone ({})",
                                user_path,
                                blocked.display()
                            ));
                        }
                    }
                }
                return Ok(canon);
            }
            Err(e) => last_err = e,
        }
    }
    if usable_roots == 0 {
        return Err(
            "Access denied: no usable workspace root (all configured roots are missing on disk). \
             Fix [workspace] extra_paths in config.toml or use shell_exec."
                .to_string(),
        );
    }
    Err(last_err)
}

fn canonicalize_existing_or_parent(path: &Path) -> Option<PathBuf> {
    if let Ok(canon) = path.canonicalize() {
        return Some(canon);
    }
    let parent = path.parent()?;
    let filename = path.file_name()?;
    parent.canonicalize().ok().map(|p| p.join(filename))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_relative_path_inside_workspace() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(data_dir.join("test.txt"), "hello").unwrap();

        let result = resolve_sandbox_path("data/test.txt", dir.path());
        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(resolved.starts_with(dir.path().canonicalize().unwrap()));
    }

    #[test]
    fn test_absolute_path_inside_workspace() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("file.txt"), "ok").unwrap();
        let abs_path = dir.path().join("file.txt");

        let result = resolve_sandbox_path(abs_path.to_str().unwrap(), dir.path());
        assert!(result.is_ok());
    }

    #[test]
    fn test_absolute_path_outside_workspace_blocked() {
        let dir = TempDir::new().unwrap();
        let outside = std::env::temp_dir().join("outside_test.txt");
        std::fs::write(&outside, "nope").unwrap();

        let result = resolve_sandbox_path(outside.to_str().unwrap(), dir.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Access denied"));
        // The denial must point to the actual ways out, so agents don't
        // retry the same refused call (seen 3x in a row live).
        assert!(err.contains("workspace_add"));
        assert!(err.contains("shell_exec"));

        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn test_dotdot_component_blocked() {
        let dir = TempDir::new().unwrap();
        let result = resolve_sandbox_path("../../../etc/passwd", dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Path traversal denied"));
    }

    #[test]
    fn test_nonexistent_file_with_valid_parent() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();

        let result = resolve_sandbox_path("data/new_file.txt", dir.path());
        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(resolved.starts_with(dir.path().canonicalize().unwrap()));
        assert!(resolved.ends_with("new_file.txt"));
    }

    #[test]
    fn multi_resolves_when_absolute_path_lives_in_secondary_root() {
        // Absolute paths inside the *secondary* root are accepted because
        // multi tries each allowed root in turn — the principal use case
        // for Captain reaching into ~/.captain/ from its own workspace.
        let primary = TempDir::new().unwrap();
        let secondary = TempDir::new().unwrap();
        let target = secondary.path().join("note.md");
        std::fs::write(&target, "hi").unwrap();

        let roots: &[&Path] = &[primary.path(), secondary.path()];
        let resolved = resolve_sandbox_path_multi(target.to_str().unwrap(), roots, &[]).unwrap();
        assert!(resolved.starts_with(secondary.path().canonicalize().unwrap()));
    }

    #[test]
    fn multi_rejects_when_outside_every_root() {
        let primary = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let abs = outside.path().join("note.md");
        std::fs::write(&abs, "x").unwrap();

        let roots: &[&Path] = &[primary.path()];
        let res = resolve_sandbox_path_multi(abs.to_str().unwrap(), roots, &[]);
        assert!(res.is_err());
    }

    #[test]
    fn multi_blocklist_trumps_allowlist() {
        let allow = TempDir::new().unwrap();
        let secret_dir = allow.path().join("secret_zone");
        std::fs::create_dir_all(&secret_dir).unwrap();
        let secret_file = secret_dir.join("token.txt");
        std::fs::write(&secret_file, "shh").unwrap();

        let allowed: &[&Path] = &[allow.path()];
        let blocked: &[&Path] = &[secret_dir.as_path()];

        let allowed_path = allow.path().join("ok.txt");
        std::fs::write(&allowed_path, "ok").unwrap();
        assert!(
            resolve_sandbox_path_multi(allowed_path.to_str().unwrap(), allowed, blocked).is_ok()
        );

        let res = resolve_sandbox_path_multi(secret_file.to_str().unwrap(), allowed, blocked);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("protected zone"));
    }

    #[test]
    fn multi_blocklist_rejects_nonexistent_protected_leaf() {
        let allow = TempDir::new().unwrap();
        let secret_file = allow.path().join("secrets.env");

        let allowed: &[&Path] = &[allow.path()];
        let blocked: &[&Path] = &[secret_file.as_path()];

        let res = resolve_sandbox_path_multi(secret_file.to_str().unwrap(), allowed, blocked);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("protected zone"));
    }

    #[test]
    fn multi_rejects_when_no_allowed_roots() {
        let res = resolve_sandbox_path_multi("anything", &[], &[]);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("no workspace root"));
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_escape_blocked() {
        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();

        // Create a symlink inside the workspace pointing outside
        let link_path = dir.path().join("escape");
        std::os::unix::fs::symlink(outside.path(), &link_path).unwrap();

        let result = resolve_sandbox_path("escape/secret.txt", dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Access denied"));
    }

    /// Live bug: stale extra_paths (deleted /tmp dirs) made every denial
    /// read "Failed to resolve workspace root" instead of the actionable
    /// Access denied message.
    #[test]
    fn test_stale_root_does_not_mask_access_denied_message() {
        let dir = TempDir::new().unwrap();
        let stale = dir.path().join("deleted-root");
        let outside = std::env::temp_dir().join("multi_outside_test.txt");
        std::fs::write(&outside, "nope").unwrap();

        let roots: Vec<&Path> = vec![dir.path(), stale.as_path()];
        let res = resolve_sandbox_path_multi(outside.to_str().unwrap(), &roots, &[]);

        let err = res.unwrap_err();
        assert!(err.contains("Access denied"));
        assert!(err.contains("workspace_add"));
        assert!(!err.contains("Failed to resolve workspace root"));

        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn test_valid_root_still_grants_access_despite_stale_sibling() {
        let dir = TempDir::new().unwrap();
        let stale = dir.path().join("deleted-root");
        let inside = dir.path().join("ok.txt");
        std::fs::write(&inside, "ok").unwrap();

        let roots: Vec<&Path> = vec![stale.as_path(), dir.path()];
        let res = resolve_sandbox_path_multi(inside.to_str().unwrap(), &roots, &[]);

        assert!(res.is_ok());
    }

    #[test]
    fn test_all_roots_stale_yields_dedicated_message() {
        let dir = TempDir::new().unwrap();
        let stale1 = dir.path().join("gone-1");
        let stale2 = dir.path().join("gone-2");

        let roots: Vec<&Path> = vec![stale1.as_path(), stale2.as_path()];
        let res = resolve_sandbox_path_multi("anything.txt", &roots, &[]);

        let err = res.unwrap_err();
        assert!(err.contains("no usable workspace root"));
    }
}
