//! Minimal `.env` file loader/saver for `~/.captain/.env`.
//!
//! No external crate needed — hand-rolled for simplicity.
//! Format: `KEY=VALUE` lines, `#` comments, optional quotes.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// Get the Captain home directory, respecting CAPTAIN_HOME env var.
fn dotenv_captain_home() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("CAPTAIN_HOME") {
        return Some(PathBuf::from(home));
    }
    dirs::home_dir().map(|h| h.join(".captain"))
}

/// Return the path to `~/.captain/.env`.
pub fn env_file_path() -> Option<PathBuf> {
    dotenv_captain_home().map(|h| h.join(".env"))
}

/// Load `~/.captain/secrets.env` and `~/.captain/.env` into `std::env`.
///
/// System env vars take priority — existing vars are NOT overridden.
/// `secrets.env` is loaded first so canonical secrets take priority over
/// legacy `.env` values (but both yield to system env vars).
/// Silently does nothing if the files don't exist.
pub fn load_dotenv() {
    load_env_file(secrets_env_path());
    load_env_file(env_file_path());
}

/// Return the path to `~/.captain/secrets.env`.
pub fn secrets_env_path() -> Option<PathBuf> {
    dotenv_captain_home().map(|h| h.join("secrets.env"))
}

fn load_env_file(path: Option<PathBuf>) {
    let path = match path {
        Some(p) => p,
        None => return,
    };

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return,
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some((key, value)) = parse_env_line(trimmed) {
            if !is_valid_env_key(&key) {
                continue;
            }
            if std::env::var(&key).is_err() {
                std::env::set_var(&key, &value);
            }
        }
    }
}

/// Upsert a key in `~/.captain/.env`.
///
/// Creates the file if missing. Sets 0600 permissions on Unix.
/// Also sets the key in the current process environment.
pub fn save_env_key(key: &str, value: &str) -> Result<(), String> {
    ensure_valid_env_key(key)?;
    let path = env_file_path().ok_or("Could not determine home directory")?;

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {e}"))?;
    }

    let mut entries = read_env_file(&path);
    entries.insert(key.to_string(), value.to_string());
    write_env_file(&path, &entries)?;

    // Also set in current process
    std::env::set_var(key, value);

    Ok(())
}

/// Upsert a key in `~/.captain/secrets.env`.
///
/// Creates the file if missing. Sets 0600 permissions on Unix. `secrets.env`
/// also stores logical vault-style keys such as `integration:telegram:bot_token`;
/// only shell-safe keys are exported into the current process environment.
pub fn save_secret_key(key: &str, value: &str) -> Result<(), String> {
    ensure_valid_secret_key(key)?;
    ensure_single_line_value(value)?;
    let path = secrets_env_path().ok_or("Could not determine home directory")?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {e}"))?;
    }

    let mut entries = read_env_file(&path);
    entries.insert(key.to_string(), value.to_string());
    write_env_file(&path, &entries)?;

    if is_valid_env_key(key) {
        std::env::set_var(key, value);
    }

    Ok(())
}

/// Remove a key from `~/.captain/.env`.
///
/// Also removes it from the current process environment.
pub fn remove_env_key(key: &str) -> Result<(), String> {
    let path = env_file_path().ok_or("Could not determine home directory")?;

    let mut entries = read_env_file(&path);
    entries.remove(key);
    write_env_file(&path, &entries)?;

    if is_valid_env_key(key) {
        std::env::remove_var(key);
    }

    Ok(())
}

/// Remove a key from `~/.captain/secrets.env`.
///
/// Also removes it from the current process environment.
pub fn remove_secret_key(key: &str) -> Result<(), String> {
    let path = secrets_env_path().ok_or("Could not determine home directory")?;

    let mut entries = read_env_file(&path);
    entries.remove(key);
    write_env_file(&path, &entries)?;

    if is_valid_env_key(key) {
        std::env::remove_var(key);
    }

    Ok(())
}

/// List key names (without values) from `~/.captain/.env`.
#[allow(dead_code)]
pub fn list_env_keys() -> Vec<String> {
    let path = match env_file_path() {
        Some(p) => p,
        None => return Vec::new(),
    };

    read_env_file(&path).into_keys().collect()
}

/// Check if the `.env` file exists.
#[allow(dead_code)]
pub fn env_file_exists() -> bool {
    env_file_path().map(|p| p.exists()).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Parse a single `KEY=VALUE` line. Handles optional quotes.
fn parse_env_line(line: &str) -> Option<(String, String)> {
    let eq_pos = line.find('=')?;
    let key = line[..eq_pos].trim().to_string();
    let mut value = line[eq_pos + 1..].trim().to_string();

    if key.is_empty() {
        return None;
    }

    // Strip matching quotes
    if ((value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\'')))
        && value.len() >= 2
    {
        value = value[1..value.len() - 1].to_string();
    }

    Some((key, value))
}

fn ensure_valid_env_key(key: &str) -> Result<(), String> {
    if is_valid_env_key(key) {
        Ok(())
    } else {
        Err(format!(
            "Invalid environment key `{key}`. Use a shell-safe name like `OPENAI_API_KEY`."
        ))
    }
}

fn ensure_valid_secret_key(key: &str) -> Result<(), String> {
    if key.trim().is_empty()
        || key.trim() != key
        || key.contains('=')
        || key.contains('\n')
        || key.contains('\r')
    {
        Err(format!(
            "Invalid secret key `{key}`. Use a non-empty single-line key without '='."
        ))
    } else {
        Ok(())
    }
}

fn ensure_single_line_value(value: &str) -> Result<(), String> {
    if value.contains('\n') || value.contains('\r') {
        Err("Invalid secret value: newlines are not allowed.".to_string())
    } else {
        Ok(())
    }
}

fn is_valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// Read all key-value pairs from the .env file.
fn read_env_file(path: &PathBuf) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return map,
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = parse_env_line(trimmed) {
            map.insert(key, value);
        }
    }

    map
}

/// Write key-value pairs back to the .env file with a header comment.
fn write_env_file(path: &PathBuf, entries: &BTreeMap<String, String>) -> Result<(), String> {
    let mut content = String::from("# Captain environment — managed by Captain CLI\n");
    content.push_str("# Do not edit while the daemon is running.\n\n");

    for (key, value) in entries {
        // Quote values that contain spaces or special characters
        if value.contains(' ') || value.contains('#') || value.contains('"') {
            content.push_str(&format!("{key}=\"{}\"\n", value.replace('"', "\\\"")));
        } else {
            content.push_str(&format!("{key}={value}\n"));
        }
    }

    captain_types::durable_fs::atomic_write(path, content.as_bytes())
        .map_err(|e| format!("Failed to persist .env file: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_env_line_simple() {
        let (k, v) = parse_env_line("FOO=bar").unwrap();
        assert_eq!(k, "FOO");
        assert_eq!(v, "bar");
    }

    #[test]
    fn test_parse_env_line_quoted() {
        let (k, v) = parse_env_line("KEY=\"hello world\"").unwrap();
        assert_eq!(k, "KEY");
        assert_eq!(v, "hello world");
    }

    #[test]
    fn test_parse_env_line_single_quoted() {
        let (k, v) = parse_env_line("KEY='value'").unwrap();
        assert_eq!(k, "KEY");
        assert_eq!(v, "value");
    }

    #[test]
    fn test_parse_env_line_spaces() {
        let (k, v) = parse_env_line("  KEY  =  value  ").unwrap();
        assert_eq!(k, "KEY");
        assert_eq!(v, "value");
    }

    #[test]
    fn test_parse_env_line_no_value() {
        let (k, v) = parse_env_line("KEY=").unwrap();
        assert_eq!(k, "KEY");
        assert_eq!(v, "");
    }

    #[test]
    fn test_parse_env_line_comment() {
        assert!(
            parse_env_line("# comment").is_none()
                || parse_env_line("# comment").unwrap().0.starts_with('#')
        );
        // Comments are filtered before reaching parse_env_line in production code
    }

    #[test]
    fn test_parse_env_line_no_equals() {
        assert!(parse_env_line("NOEQUALS").is_none());
    }

    #[test]
    fn test_parse_env_line_empty_key() {
        assert!(parse_env_line("=value").is_none());
    }

    #[test]
    fn test_write_env_file_atomic_no_leftover_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.env");
        let mut entries = BTreeMap::new();
        entries.insert("FOO".to_string(), "bar".to_string());

        write_env_file(&path, &entries).unwrap();

        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("FOO=bar"));
        // No temp file should survive the rename.
        assert!(!dir.path().join("secrets.env.tmp").exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_write_env_file_sets_0600_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.env");
        let mut entries = BTreeMap::new();
        entries.insert("SECRET".to_string(), "value".to_string());

        write_env_file(&path, &entries).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn test_write_env_file_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(&path, "STALE=old\n").unwrap();

        let mut entries = BTreeMap::new();
        entries.insert("FRESH".to_string(), "new".to_string());
        write_env_file(&path, &entries).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("FRESH=new"));
        assert!(!content.contains("STALE"));
    }

    #[test]
    fn test_env_key_validation_rejects_shell_unsafe_names() {
        assert!(is_valid_env_key("OPENAI_API_KEY"));
        assert!(is_valid_env_key("_CAPTAIN_TOKEN"));
        assert!(is_valid_env_key("lowercase_ok"));
        assert!(!is_valid_env_key("integration:telegram:bot_token"));
        assert!(!is_valid_env_key("1BAD"));
        assert!(!is_valid_env_key("BAD-NAME"));
        assert!(!is_valid_env_key(""));
    }

    #[test]
    fn test_load_env_file_skips_shell_unsafe_names() {
        let key = format!("CAPTAIN_DOTENV_TEST_{}", std::process::id());
        let path = std::env::temp_dir().join(format!(
            "captain-dotenv-test-{}-{:?}.env",
            std::process::id(),
            std::thread::current().id()
        ));
        std::env::remove_var(&key);
        std::fs::write(
            &path,
            format!("{key}=ok\nintegration:telegram:bot_token=bad\n1BAD=bad\nBAD-NAME=bad\n"),
        )
        .unwrap();

        load_env_file(Some(path.clone()));

        assert_eq!(std::env::var(&key).unwrap(), "ok");
        assert!(std::env::var("integration:telegram:bot_token").is_err());
        assert!(std::env::var("1BAD").is_err());
        assert!(std::env::var("BAD-NAME").is_err());

        std::env::remove_var(&key);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn save_secret_key_allows_logical_names_without_exporting_them() {
        let old_home = std::env::var_os("CAPTAIN_HOME");
        let home = std::env::temp_dir().join(format!(
            "captain-dotenv-secret-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let logical_key = "integration:tts_openai:api_key";
        std::env::set_var("CAPTAIN_HOME", &home);
        std::env::remove_var(logical_key);

        save_secret_key(logical_key, "sk-test-secret-value").unwrap();

        let raw = std::fs::read_to_string(home.join("secrets.env")).unwrap();
        assert!(raw.contains("integration:tts_openai:api_key=sk-test-secret-value"));
        assert!(std::env::var(logical_key).is_err());

        let _ = std::fs::remove_dir_all(&home);
        match old_home {
            Some(value) => std::env::set_var("CAPTAIN_HOME", value),
            None => std::env::remove_var("CAPTAIN_HOME"),
        }
    }
}
