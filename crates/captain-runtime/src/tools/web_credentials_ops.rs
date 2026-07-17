use crate::kernel_handle::KernelHandle;
use crate::tools::require_kernel;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug)]
struct WebCredentialsUpdate {
    username: Option<String>,
    password_hash: Option<String>,
    generated_password: Option<String>,
    session_ttl_hours: Option<i64>,
}

pub(crate) async fn tool_web_credentials_update(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let home = kh
        .home_dir()
        .ok_or("Kernel did not expose a home_dir; cannot locate config.toml")?;

    let update = parse_web_credentials_update(input)?;
    let backup_path = write_web_credentials_config(
        &home,
        update.username.as_deref(),
        update.password_hash.as_deref(),
        update.session_ttl_hours,
    )?;
    kh.publish_integration_configured("config");

    render_web_credentials_update_response(&update, &backup_path)
}

fn parse_web_credentials_update(input: &serde_json::Value) -> Result<WebCredentialsUpdate, String> {
    let username = input
        .get("username")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    if let Some(username) = &username {
        validate_web_username(username)?;
    }

    let generate_password = input
        .get("generate_password")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let provided_password = input
        .get("password")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    if provided_password.is_some() && generate_password {
        return Err("Provide either password or generate_password=true, not both.".into());
    }
    let generated_password = if provided_password.is_none() && generate_password {
        Some(generate_web_password())
    } else {
        None
    };
    let password = provided_password.or_else(|| generated_password.clone());
    if let Some(password) = &password {
        validate_web_password(password, username.as_deref())?;
    }
    let password_hash = password.as_deref().map(hash_web_password);

    let session_ttl_hours = match input.get("session_ttl_hours").and_then(|v| v.as_i64()) {
        Some(ttl) if (1..=8760).contains(&ttl) => Some(ttl),
        Some(_) => {
            return Err("session_ttl_hours must be between 1 and 8760 hours when provided.".into());
        }
        None => None,
    };

    if username.is_none() && password_hash.is_none() && session_ttl_hours.is_none() {
        return Err(
            "Nothing to update. Provide username, password, generate_password=true, or session_ttl_hours."
                .into(),
        );
    }

    Ok(WebCredentialsUpdate {
        username,
        password_hash,
        generated_password,
        session_ttl_hours,
    })
}

fn render_web_credentials_update_response(
    update: &WebCredentialsUpdate,
    backup_path: &Path,
) -> Result<String, String> {
    let mut out = serde_json::json!({
        "status": "ok",
        "auth_enabled": true,
        "username_changed": update.username.is_some(),
        "password_changed": update.password_hash.is_some(),
        "session_ttl_hours_changed": update.session_ttl_hours.is_some(),
        "backup_path": backup_path.display().to_string(),
        "message": "Captain web credentials updated in config.toml. New logins use them immediately; existing sessions may need to sign in again."
    });
    if let Some(password) = &update.generated_password {
        out["generated_password"] = serde_json::Value::String(password.clone());
        out["generated_password_note"] = serde_json::Value::String(
            "Show this generated password to the user once; do not store it in memory.".to_string(),
        );
    }
    serde_json::to_string_pretty(&out).map_err(|e| format!("Serialize error: {e}"))
}

fn validate_web_username(username: &str) -> Result<(), String> {
    if username.len() < 2 || username.len() > 64 {
        return Err("username must be between 2 and 64 characters.".into());
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err("username may contain only ASCII letters, digits, '.', '_' and '-'.".into());
    }
    Ok(())
}

fn validate_web_password(password: &str, username: Option<&str>) -> Result<(), String> {
    if password.len() < 12 || password.len() > 256 {
        return Err("password must be between 12 and 256 characters.".into());
    }
    if password.contains('\n') || password.contains('\r') {
        return Err("password must be single-line.".into());
    }
    if let Some(username) = username {
        if password.eq_ignore_ascii_case(username) {
            return Err("password must not be identical to username.".into());
        }
    }
    Ok(())
}

fn generate_web_password() -> String {
    format!(
        "captain-{}-{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

pub(crate) fn hash_web_password(password: &str) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(password.as_bytes()))
}

pub(crate) fn write_web_credentials_config(
    home: &Path,
    username: Option<&str>,
    password_hash: Option<&str>,
    session_ttl_hours: Option<i64>,
) -> Result<PathBuf, String> {
    let config_path = web_credentials_config_path(home)?;
    let raw = read_web_credentials_config(&config_path)?;
    let old_size = raw.len();
    let mut doc = parse_web_credentials_config(&raw)?;
    let old_top_keys = document_top_keys(&doc);
    let backup_path = create_web_credentials_backup(home, &config_path)?;

    apply_web_credentials_auth_table(&mut doc, username, password_hash, session_ttl_hours)?;
    ensure_no_top_level_keys_lost(&old_top_keys, &document_top_keys(&doc), &backup_path)?;
    let serialized = doc.to_string();
    ensure_serialized_size_not_suspicious(old_size, serialized.len(), &backup_path)?;
    write_web_credentials_config_atomically(&config_path, &serialized)?;
    validate_web_credentials_roundtrip(&config_path, &backup_path, &old_top_keys)?;

    Ok(backup_path)
}

fn web_credentials_config_path(home: &Path) -> Result<PathBuf, String> {
    let config_path = home.join("config.toml");
    if config_path.exists() {
        Ok(config_path)
    } else {
        Err(format!(
            "config.toml not found at {}; run Captain setup first.",
            config_path.display()
        ))
    }
}

fn read_web_credentials_config(config_path: &Path) -> Result<String, String> {
    std::fs::read_to_string(config_path).map_err(|e| format!("Failed to read config.toml: {e}"))
}

fn parse_web_credentials_config(raw: &str) -> Result<toml_edit::DocumentMut, String> {
    raw.parse()
        .map_err(|e| format!("Failed to parse config.toml: {e}"))
}

fn document_top_keys(doc: &toml_edit::DocumentMut) -> BTreeSet<String> {
    doc.as_table().iter().map(|(k, _)| k.to_string()).collect()
}

fn create_web_credentials_backup(home: &Path, config_path: &Path) -> Result<PathBuf, String> {
    let backup_dir = home.join("config-backups");
    captain_types::durable_fs::create_dir_all(&backup_dir)
        .map_err(|e| format!("Failed to create config-backups dir: {e}"))?;
    let ts = chrono::Utc::now()
        .format("%Y-%m-%dT%H-%M-%S-%3f")
        .to_string();
    let backup_path = backup_dir.join(format!("config.toml.web-auth.{ts}"));
    captain_types::durable_fs::atomic_copy(config_path, &backup_path)
        .map_err(|e| format!("Config pre-write backup failed: {e}"))?;
    Ok(backup_path)
}

fn apply_web_credentials_auth_table(
    doc: &mut toml_edit::DocumentMut,
    username: Option<&str>,
    password_hash: Option<&str>,
    session_ttl_hours: Option<i64>,
) -> Result<(), String> {
    let root = doc.as_table_mut();
    if !root.contains_key("auth") {
        let mut table = toml_edit::Table::new();
        table.set_implicit(false);
        root.insert("auth", toml_edit::Item::Table(table));
    }
    let auth = root
        .get_mut("auth")
        .and_then(|item| item.as_table_mut())
        .ok_or("[auth] exists but is not a TOML table")?;
    auth.insert("enabled", toml_edit::value(true));
    if let Some(username) = username {
        auth.insert("username", toml_edit::value(username));
    }
    if let Some(password_hash) = password_hash {
        auth.insert("password_hash", toml_edit::value(password_hash));
    }
    if let Some(ttl) = session_ttl_hours {
        auth.insert("session_ttl_hours", toml_edit::value(ttl));
    }
    Ok(())
}

fn ensure_no_top_level_keys_lost(
    old_keys: &BTreeSet<String>,
    new_keys: &BTreeSet<String>,
    backup_path: &Path,
) -> Result<(), String> {
    let lost: Vec<&String> = old_keys.difference(new_keys).collect();
    if lost.is_empty() {
        return Ok(());
    }
    Err(format!(
        "Refusing to write: top-level keys would be lost: {lost:?}. Backup at {}",
        backup_path.display()
    ))
}

fn ensure_serialized_size_not_suspicious(
    old_size: usize,
    new_size: usize,
    backup_path: &Path,
) -> Result<(), String> {
    if old_size > 100 && new_size < (old_size * 7 / 10) {
        return Err(format!(
            "Refusing to write: serialized config shrank suspiciously ({} -> {} bytes). Backup at {}",
            old_size,
            new_size,
            backup_path.display()
        ));
    }
    Ok(())
}

fn write_web_credentials_config_atomically(
    config_path: &Path,
    serialized: &str,
) -> Result<(), String> {
    captain_types::durable_fs::atomic_write(config_path, serialized.as_bytes())
        .map_err(|e| format!("Failed to persist config.toml: {e}"))?;
    set_private_file_permissions(config_path)
}

fn validate_web_credentials_roundtrip(
    config_path: &Path,
    backup_path: &Path,
    old_top_keys: &BTreeSet<String>,
) -> Result<(), String> {
    let reparsed: toml::Value = std::fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to re-read config.toml: {e}"))?
        .parse()
        .map_err(|e| {
            let _ = captain_types::durable_fs::atomic_copy(backup_path, config_path);
            format!(
                "Roundtrip re-parse failed ({e}). Rolled back from {}",
                backup_path.display()
            )
        })?;
    let rt_keys = toml_value_top_keys(&reparsed);
    let lost: Vec<&String> = old_top_keys.difference(&rt_keys).collect();
    if lost.is_empty() {
        return Ok(());
    }
    let _ = captain_types::durable_fs::atomic_copy(backup_path, config_path);
    Err(format!(
        "Roundtrip validation failed, keys lost after write: {lost:?}. Rolled back from {}",
        backup_path.display()
    ))
}

fn toml_value_top_keys(value: &toml::Value) -> BTreeSet<String> {
    value
        .as_table()
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default()
}

fn set_private_file_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms)
            .map_err(|e| format!("Failed to set private file permissions: {e}"))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_web_credentials_update_hashes_password_and_ttl() {
        let update = parse_web_credentials_update(&json!({
            "username": " owner ",
            "password": "new-password-123",
            "session_ttl_hours": 24
        }))
        .expect("update should parse");

        assert_eq!(update.username.as_deref(), Some("owner"));
        assert_eq!(
            update.password_hash.as_deref(),
            Some(hash_web_password("new-password-123").as_str())
        );
        assert_eq!(update.generated_password, None);
        assert_eq!(update.session_ttl_hours, Some(24));
    }

    #[test]
    fn parse_web_credentials_update_rejects_empty_update() {
        let err = parse_web_credentials_update(&json!({})).expect_err("empty update must fail");
        assert!(err.contains("Nothing to update"));
    }

    #[test]
    fn parse_web_credentials_update_rejects_conflicting_password_modes() {
        let err = parse_web_credentials_update(&json!({
            "password": "new-password-123",
            "generate_password": true
        }))
        .expect_err("conflicting password modes must fail");
        assert!(err.contains("either password or generate_password"));
    }

    #[test]
    fn serialized_size_guard_rejects_suspicious_shrink() {
        let backup = Path::new("/tmp/backup.toml");
        let err = ensure_serialized_size_not_suspicious(200, 100, backup)
            .expect_err("large shrink should fail");
        assert!(err.contains("shrank suspiciously"));
    }

    #[test]
    fn top_level_key_guard_reports_lost_keys() {
        let old = BTreeSet::from(["auth".to_string(), "default_model".to_string()]);
        let new = BTreeSet::from(["auth".to_string()]);
        let err = ensure_no_top_level_keys_lost(&old, &new, Path::new("/tmp/backup.toml"))
            .expect_err("lost key should fail");
        assert!(err.contains("default_model"));
    }
}
