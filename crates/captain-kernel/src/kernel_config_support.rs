use std::path::Path;

/// Keep only the `keep` most recent timestamped config backups (`config.toml.<ts>`).
pub(super) fn rotate_config_backups(dir: &Path, keep: usize) {
    rotate_backups_with_prefix(dir, "config.toml.", keep);
}

pub(super) fn rotate_backups_with_prefix(dir: &Path, prefix: &str, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
        .collect();
    files.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
    for entry in files.into_iter().skip(keep) {
        let _ = std::fs::remove_file(entry.path());
    }
}

pub(super) fn validate_secret_assignment(key: &str, value: &str) -> Result<(), String> {
    if key.trim().is_empty() {
        return Err("Secret key must not be empty".into());
    }
    if key.trim() != key {
        return Err("Secret key must not have leading or trailing whitespace".into());
    }
    if key.contains('=') || key.contains('\n') || key.contains('\r') {
        return Err("Secret key must not contain '=', newline, or carriage return".into());
    }
    if value.contains('\n') || value.contains('\r') {
        return Err("Secret value must be single-line for secrets.env".into());
    }
    if value.trim() != value {
        return Err("Secret value must not have leading or trailing whitespace".into());
    }
    Ok(())
}

pub(super) fn set_secret_file_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms)
            .map_err(|e| format!("Failed to set secrets.env permissions: {e}"))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Get the system hostname as a String.
pub(super) fn gethostname() -> Option<String> {
    #[cfg(unix)]
    {
        std::process::Command::new("hostname")
            .output()
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .map(|s| s.trim().to_string())
    }
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME").ok()
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_rotation_keeps_newest_prefixed_files_only() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "config.toml.2026-05-30T10-00-00-000",
            "config.toml.2026-05-31T10-00-00-000",
            "config.toml.2026-05-29T10-00-00-000",
            "secrets.env.2026-05-31T10-00-00-000",
        ] {
            std::fs::write(dir.path().join(name), "backup").unwrap();
        }

        rotate_config_backups(dir.path(), 2);

        assert!(!dir
            .path()
            .join("config.toml.2026-05-29T10-00-00-000")
            .exists());
        assert!(dir
            .path()
            .join("config.toml.2026-05-30T10-00-00-000")
            .exists());
        assert!(dir
            .path()
            .join("config.toml.2026-05-31T10-00-00-000")
            .exists());
        assert!(dir
            .path()
            .join("secrets.env.2026-05-31T10-00-00-000")
            .exists());
    }

    #[test]
    fn secret_assignment_rejects_env_file_corruption_vectors() {
        assert!(validate_secret_assignment("OPENAI_API_KEY", "sk-test").is_ok());
        assert!(validate_secret_assignment("integration:telegram:bot_token", "123:ABC").is_ok());
        assert!(validate_secret_assignment("", "x").is_err());
        assert!(validate_secret_assignment("BAD=KEY", "x").is_err());
        assert!(validate_secret_assignment(" BAD", "x").is_err());
        assert!(validate_secret_assignment("KEY", "line1\nline2").is_err());
        assert!(validate_secret_assignment("KEY", " value").is_err());
    }
}
