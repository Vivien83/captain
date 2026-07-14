use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::time::Duration;

/// `agent_id` is sanitized to keep only `[A-Za-z0-9_-]` — anything else
/// is collapsed to `_` so a hostile / oddly-named id cannot escape the
/// directory via path traversal.
pub(crate) fn user_data_dir_for_agent(agent_id: &str) -> Result<PathBuf, String> {
    let dir = user_data_dir_path_for_agent(agent_id)?;
    std::fs::create_dir_all(&dir).map_err(|e| {
        format!(
            "Failed to create per-agent browser profile dir {}: {e}",
            dir.display()
        )
    })?;
    Ok(dir)
}

pub(super) fn user_data_dir_path_for_agent(agent_id: &str) -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("HOME not resolvable")?;
    user_data_dir_path_for_agent_under(&home.join(".captain/browser-profiles"), agent_id)
}

fn user_data_dir_path_for_agent_under(
    profile_root: &Path,
    agent_id: &str,
) -> Result<PathBuf, String> {
    let safe: String = agent_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if safe.is_empty() {
        return Err("agent_id must not be empty for user-data-dir resolution".into());
    }
    Ok(profile_root.join(&safe))
}

#[cfg(unix)]
pub(super) async fn reclaim_captain_profile_lock(user_data_dir: &Path) -> Result<(), String> {
    let Some(pid) = chrome_singleton_lock_pid(user_data_dir) else {
        return Ok(());
    };
    if !process_exists(pid) {
        remove_chrome_singleton_files(user_data_dir);
        return Ok(());
    }
    if !process_uses_user_data_dir(pid, user_data_dir) {
        return Err(format!(
            "Browser profile {} is locked by pid {pid}, but that process does not look like Captain's Chrome. Close it or run browser_close before retrying.",
            user_data_dir.display()
        ));
    }

    tracing::warn!(
        pid,
        profile = %user_data_dir.display(),
        "Terminating stale Captain browser process holding profile lock"
    );
    let _ = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status();
    for _ in 0..20 {
        if !process_exists(pid) {
            remove_chrome_singleton_files(user_data_dir);
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(format!(
        "Browser profile {} is still locked by pid {pid} after termination request",
        user_data_dir.display()
    ))
}

#[cfg(not(unix))]
pub(super) async fn reclaim_captain_profile_lock(_user_data_dir: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn chrome_singleton_lock_pid(user_data_dir: &Path) -> Option<u32> {
    let target = std::fs::read_link(user_data_dir.join("SingletonLock")).ok()?;
    parse_chrome_singleton_lock_pid(&target.to_string_lossy())
}

#[cfg(unix)]
fn parse_chrome_singleton_lock_pid(target: &str) -> Option<u32> {
    target.rsplit('-').next()?.parse().ok()
}

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(unix)]
fn process_uses_user_data_dir(pid: u32, user_data_dir: &Path) -> bool {
    let Ok(output) = std::process::Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-o")
        .arg("args=")
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let args = String::from_utf8_lossy(&output.stdout);
    let profile = user_data_dir.to_string_lossy();
    args.contains("Chrome")
        && (args.contains(&format!("--user-data-dir={profile}"))
            || args.contains(&format!("--user-data-dir=\"{profile}\"")))
}

#[cfg(unix)]
fn remove_chrome_singleton_files(user_data_dir: &Path) {
    for name in ["SingletonLock", "SingletonSocket", "SingletonCookie"] {
        let _ = std::fs::remove_file(user_data_dir.join(name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B.7 — two agents must land on two different `--user-data-dir`s
    /// so cookies / localStorage / saved logins do not bleed across.
    #[test]
    fn user_data_dir_isolates_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("browser-profiles");
        let a = user_data_dir_path_for_agent_under(&root, "agent-aaa").unwrap();
        let b = user_data_dir_path_for_agent_under(&root, "agent-bbb").unwrap();
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        assert_ne!(a, b);
        assert!(a.is_dir());
        assert!(b.is_dir());
    }

    /// B.7 — sanitisation collapses unsafe characters to `_` so a
    /// crafted agent_id cannot escape the profile root via `..`.
    #[test]
    fn user_data_dir_sanitises_path_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("browser-profiles");
        let resolved = user_data_dir_path_for_agent_under(&root, "../../etc/passwd").unwrap();
        let path_str = resolved.display().to_string();
        assert!(
            resolved.starts_with(&root),
            "sanitization escape: {path_str}"
        );
        assert!(
            !path_str.contains("/../") && !path_str.contains("/etc/passwd"),
            "sanitization left traversal in: {path_str}"
        );
    }

    /// B.7 — empty agent_id is refused outright; we don't want a
    /// shared root profile across "anonymous" callers.
    #[test]
    fn user_data_dir_rejects_empty_id() {
        assert!(user_data_dir_for_agent("").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn chrome_singleton_lock_pid_parses_macos_lock_target() {
        assert_eq!(
            parse_chrome_singleton_lock_pid("Mac-mini-de-Alex.local-44818"),
            Some(44818)
        );
        assert_eq!(parse_chrome_singleton_lock_pid("invalid"), None);
    }
}
