//! Shared helpers for the Captain `secrets.env` compatibility file.

/// Write or update a key in the secrets.env file.
/// File format: one `KEY=value` per line. Existing keys are overwritten.
pub(crate) fn write_secret_env(
    path: &std::path::Path,
    key: &str,
    value: &str,
) -> Result<(), std::io::Error> {
    validate_secret_env_entry(key, value)?;
    let mut lines: Vec<String> = if path.exists() {
        std::fs::read_to_string(path)?
            .lines()
            .map(|line| line.to_string())
            .collect()
    } else {
        Vec::new()
    };

    lines.retain(|line| !line.starts_with(&format!("{key}=")));
    lines.push(format!("{key}={value}"));

    let serialized = lines.join("\n") + "\n";
    captain_types::durable_fs::atomic_write(path, serialized.as_bytes())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

/// Remove a key from the secrets.env file.
pub(crate) fn remove_secret_env(path: &std::path::Path, key: &str) -> Result<(), std::io::Error> {
    validate_secret_env_key(key)?;
    if !path.exists() {
        return Ok(());
    }

    let lines: Vec<String> = std::fs::read_to_string(path)?
        .lines()
        .filter(|line| !line.starts_with(&format!("{key}=")))
        .map(|line| line.to_string())
        .collect();

    let serialized = lines.join("\n") + "\n";
    captain_types::durable_fs::atomic_write(path, serialized.as_bytes())?;

    Ok(())
}

fn validate_secret_env_entry(key: &str, value: &str) -> Result<(), std::io::Error> {
    validate_secret_env_key(key)?;
    if value.contains('\n') || value.contains('\r') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "secret value must be a single line",
        ));
    }
    Ok(())
}

fn validate_secret_env_key(key: &str) -> Result<(), std::io::Error> {
    if key.is_empty()
        || key.contains('=')
        || key.contains('\n')
        || key.contains('\r')
        || !key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "secret key must be a plain environment variable name",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_secret_env_rejects_line_injection() {
        let path = tempfile::tempdir().unwrap().path().join("secrets.env");

        let err = write_secret_env(&path, "EMAIL_PASSWORD", "secret\nOTHER=value").unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn write_secret_env_rejects_invalid_key() {
        let path = tempfile::tempdir().unwrap().path().join("secrets.env");

        let err = write_secret_env(&path, "EMAIL_PASSWORD=BAD", "secret").unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }
}
