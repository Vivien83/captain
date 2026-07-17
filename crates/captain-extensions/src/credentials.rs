//! Credential resolution chain — resolves secrets from multiple sources.
//!
//! Resolution order:
//! 1. Canonical secrets file (`~/.captain/secrets.env`)
//! 2. Encrypted vault (`~/.captain/vault.enc`)
//! 3. Dotenv file (`~/.captain/.env`)
//! 4. Process environment variable
//! 5. Interactive prompt (CLI only, when `interactive` is true)

use crate::vault::CredentialVault;
use crate::{ExtensionError, ExtensionResult};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::debug;
use zeroize::Zeroizing;

/// Credential resolver — tries multiple sources in priority order.
pub struct CredentialResolver {
    /// Reference to the credential vault.
    vault: Option<CredentialVault>,
    /// Canonical agent-facing secrets file (`~/.captain/secrets.env`).
    ///
    /// This is read live on each lookup so `secret_write` becomes visible to
    /// MCP/integration resolution without a daemon restart.
    secrets_path: Option<PathBuf>,
    /// Dotenv entries (loaded from `~/.captain/.env`).
    dotenv: HashMap<String, String>,
    /// Whether to prompt interactively as a last resort.
    interactive: bool,
}

impl CredentialResolver {
    /// Create a resolver with optional vault and dotenv path.
    pub fn new(vault: Option<CredentialVault>, dotenv_path: Option<&Path>) -> Self {
        Self::new_with_secrets(vault, None, dotenv_path)
    }

    /// Create a resolver with optional vault, canonical secrets, and legacy dotenv paths.
    pub fn new_with_secrets(
        vault: Option<CredentialVault>,
        secrets_path: Option<&Path>,
        dotenv_path: Option<&Path>,
    ) -> Self {
        let dotenv = if let Some(path) = dotenv_path {
            load_dotenv(path).unwrap_or_default()
        } else {
            HashMap::new()
        };
        Self {
            vault,
            secrets_path: secrets_path.map(Path::to_path_buf),
            dotenv,
            interactive: false,
        }
    }

    /// Enable interactive prompting as a last-resort source.
    pub fn with_interactive(mut self, interactive: bool) -> Self {
        self.interactive = interactive;
        self
    }

    /// Resolve a credential by key, trying all sources in order.
    pub fn resolve(&self, key: &str) -> Option<Zeroizing<String>> {
        // 1. Canonical secrets.env
        if let Some(val) = self.resolve_from_secrets_file(key) {
            debug!("Credential '{}' resolved from secrets.env", key);
            return Some(val);
        }

        // 2. Vault
        if let Some(ref vault) = self.vault {
            if vault.is_unlocked() {
                if let Some(val) = vault.get(key) {
                    debug!("Credential '{}' resolved from vault", key);
                    return Some(val);
                }
            }
        }

        // 3. Dotenv file
        if let Some(val) = self.dotenv.get(key) {
            debug!("Credential '{}' resolved from .env", key);
            return Some(Zeroizing::new(val.clone()));
        }

        // 4. Environment variable
        if let Ok(val) = std::env::var(key) {
            debug!("Credential '{}' resolved from env var", key);
            return Some(Zeroizing::new(val));
        }

        // 5. Interactive prompt (CLI only)
        if self.interactive {
            if let Some(val) = prompt_secret(key) {
                debug!("Credential '{}' resolved from interactive prompt", key);
                return Some(val);
            }
        }

        None
    }

    /// Check if a credential is available (without prompting).
    pub fn has_credential(&self, key: &str) -> bool {
        // Check canonical secrets.env first so freshly written secrets win.
        if self.resolve_from_secrets_file(key).is_some() {
            return true;
        }
        // Check vault
        if let Some(ref vault) = self.vault {
            if vault.is_unlocked() && vault.get(key).is_some() {
                return true;
            }
        }
        // Check dotenv
        if self.dotenv.contains_key(key) {
            return true;
        }
        // Check env
        std::env::var(key).is_ok()
    }

    /// Resolve all required credentials for an integration.
    /// Returns a map of env_var_name -> value for all resolved credentials.
    pub fn resolve_all(&self, keys: &[&str]) -> HashMap<String, Zeroizing<String>> {
        let mut result = HashMap::new();
        for key in keys {
            if let Some(val) = self.resolve(key) {
                result.insert(key.to_string(), val);
            }
        }
        result
    }

    /// Check which credentials are missing.
    pub fn missing_credentials(&self, keys: &[&str]) -> Vec<String> {
        keys.iter()
            .filter(|k| !self.has_credential(k))
            .map(|k| k.to_string())
            .collect()
    }

    /// Store a credential in the canonical secrets file, falling back to vault.
    pub fn store_credential(&mut self, key: &str, value: &str) -> ExtensionResult<()> {
        let secrets_result = self.store_in_secrets_file(key, value);
        if secrets_result.is_ok() {
            if let Err(e) = self.store_in_vault(key, Zeroizing::new(value.to_string())) {
                debug!("Vault mirror skipped for '{}': {}", key, e);
            }
            return Ok(());
        }

        match self.store_in_vault(key, Zeroizing::new(value.to_string())) {
            Ok(()) => Ok(()),
            Err(vault_err) => match secrets_result {
                Err(secrets_err) => Err(ExtensionError::Vault(format!(
                    "Could not persist credential in secrets.env ({secrets_err}) or vault ({vault_err})"
                ))),
                Ok(()) => Ok(()),
            },
        }
    }

    /// Store a credential in the vault (if available).
    pub fn store_in_vault(&mut self, key: &str, value: Zeroizing<String>) -> ExtensionResult<()> {
        if let Some(ref mut vault) = self.vault {
            vault.set(key.to_string(), value)?;
            Ok(())
        } else {
            Err(crate::ExtensionError::Vault(
                "No vault configured".to_string(),
            ))
        }
    }

    /// Remove a credential from the vault (if available).
    pub fn remove_from_vault(&mut self, key: &str) -> ExtensionResult<bool> {
        if let Some(ref mut vault) = self.vault {
            vault.remove(key)
        } else {
            Err(crate::ExtensionError::Vault(
                "No vault configured".to_string(),
            ))
        }
    }

    fn resolve_from_secrets_file(&self, key: &str) -> Option<Zeroizing<String>> {
        let path = self.secrets_path.as_deref()?;
        match load_dotenv(path) {
            Ok(map) => map.get(key).cloned().map(Zeroizing::new),
            Err(e) => {
                debug!("Could not read secrets.env for credential '{}': {}", key, e);
                None
            }
        }
    }

    fn store_in_secrets_file(&self, key: &str, value: &str) -> ExtensionResult<()> {
        validate_secret_assignment(key, value)?;
        let path = self
            .secrets_path
            .as_deref()
            .ok_or_else(|| ExtensionError::Vault("No secrets.env configured".to_string()))?;
        write_dotenv_value(path, key, value)?;
        Ok(())
    }
}

/// Load a dotenv file into a HashMap.
fn load_dotenv(path: &Path) -> Result<HashMap<String, String>, std::io::Error> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let content = std::fs::read_to_string(path)?;
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let mut value = value.trim().to_string();
            // Strip surrounding quotes
            if (value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\''))
            {
                value = value[1..value.len() - 1].to_string();
            }
            map.insert(key.to_string(), value);
        }
    }
    Ok(map)
}

fn validate_secret_assignment(key: &str, value: &str) -> ExtensionResult<()> {
    let key_ok = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if !key_ok {
        return Err(ExtensionError::Vault(
            "Secret key must use env-var format: A-Z, 0-9, underscore".to_string(),
        ));
    }
    if value.contains('\n') || value.contains('\r') {
        return Err(ExtensionError::Vault(
            "Secret value must be single-line for secrets.env".to_string(),
        ));
    }
    if value.trim() != value {
        return Err(ExtensionError::Vault(
            "Secret value must not have leading/trailing whitespace".to_string(),
        ));
    }
    Ok(())
}

fn write_dotenv_value(path: &Path, key: &str, value: &str) -> Result<(), std::io::Error> {
    let original = if path.exists() {
        std::fs::read_to_string(path)?
    } else {
        String::new()
    };
    let mut lines: Vec<String> = original.lines().map(str::to_string).collect();

    let mut found = false;
    for line in &mut lines {
        if let Some((existing_key, _)) = line.split_once('=') {
            if existing_key.trim() == key {
                *line = format!("{key}={value}");
                found = true;
                break;
            }
        }
    }
    if !found {
        lines.push(format!("{key}={value}"));
    }

    let serialized = lines.join("\n") + "\n";
    captain_types::durable_fs::atomic_write(path, serialized.as_bytes())?;
    set_secret_file_permissions(path)?;
    Ok(())
}

fn set_secret_file_permissions(path: &Path) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Prompt the user interactively for a secret value.
fn prompt_secret(key: &str) -> Option<Zeroizing<String>> {
    use std::io::{self, Write};

    eprint!("Enter value for {}: ", key);
    io::stderr().flush().ok()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input).ok()?;
    let trimmed = input.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(Zeroizing::new(trimmed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_dotenv_basic() {
        let dir = tempfile::tempdir().unwrap();
        let env_path = dir.path().join(".env");
        std::fs::write(
            &env_path,
            r#"
# Comment
GITHUB_TOKEN=ghp_test123
SLACK_TOKEN="xoxb-quoted"
EMPTY=
SINGLE_QUOTED='single'
"#,
        )
        .unwrap();

        let map = load_dotenv(&env_path).unwrap();
        assert_eq!(map.get("GITHUB_TOKEN").unwrap(), "ghp_test123");
        assert_eq!(map.get("SLACK_TOKEN").unwrap(), "xoxb-quoted");
        assert_eq!(map.get("EMPTY").unwrap(), "");
        assert_eq!(map.get("SINGLE_QUOTED").unwrap(), "single");
    }

    #[test]
    fn load_dotenv_nonexistent() {
        let map = load_dotenv(Path::new("/nonexistent/.env")).unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn resolver_env_var() {
        std::env::set_var("TEST_CRED_RESOLVE_123", "from_env");
        let resolver = CredentialResolver::new(None, None);
        let val = resolver.resolve("TEST_CRED_RESOLVE_123").unwrap();
        assert_eq!(val.as_str(), "from_env");
        assert!(resolver.has_credential("TEST_CRED_RESOLVE_123"));
        std::env::remove_var("TEST_CRED_RESOLVE_123");
    }

    #[test]
    fn resolver_dotenv_overrides_env() {
        let dir = tempfile::tempdir().unwrap();
        let env_path = dir.path().join(".env");
        std::fs::write(&env_path, "TEST_CRED_DOT_456=from_dotenv\n").unwrap();

        std::env::set_var("TEST_CRED_DOT_456", "from_env");

        let resolver = CredentialResolver::new(None, Some(&env_path));
        let val = resolver.resolve("TEST_CRED_DOT_456").unwrap();
        assert_eq!(val.as_str(), "from_dotenv"); // dotenv takes priority

        std::env::remove_var("TEST_CRED_DOT_456");
    }

    #[test]
    fn resolver_secrets_env_overrides_dotenv_and_env() {
        let dir = tempfile::tempdir().unwrap();
        let secrets_path = dir.path().join("secrets.env");
        let env_path = dir.path().join(".env");
        std::fs::write(&secrets_path, "TEST_CRED_SECRET_456=from_secrets\n").unwrap();
        std::fs::write(&env_path, "TEST_CRED_SECRET_456=from_dotenv\n").unwrap();

        std::env::set_var("TEST_CRED_SECRET_456", "from_env");

        let resolver =
            CredentialResolver::new_with_secrets(None, Some(&secrets_path), Some(&env_path));
        let val = resolver.resolve("TEST_CRED_SECRET_456").unwrap();
        assert_eq!(val.as_str(), "from_secrets");
        assert!(resolver.has_credential("TEST_CRED_SECRET_456"));

        std::env::remove_var("TEST_CRED_SECRET_456");
    }

    #[test]
    fn resolver_reads_secrets_env_live() {
        let dir = tempfile::tempdir().unwrap();
        let secrets_path = dir.path().join("secrets.env");
        let resolver = CredentialResolver::new_with_secrets(None, Some(&secrets_path), None);

        assert!(resolver.resolve("TEST_CRED_LIVE_789").is_none());
        std::fs::write(&secrets_path, "TEST_CRED_LIVE_789=now_visible\n").unwrap();

        let val = resolver.resolve("TEST_CRED_LIVE_789").unwrap();
        assert_eq!(val.as_str(), "now_visible");
    }

    #[test]
    fn resolver_store_credential_writes_secrets_env() {
        let dir = tempfile::tempdir().unwrap();
        let secrets_path = dir.path().join("secrets.env");
        let mut resolver = CredentialResolver::new_with_secrets(None, Some(&secrets_path), None);

        resolver
            .store_credential("TEST_CRED_STORE_123", "stored_value")
            .unwrap();

        let val = resolver.resolve("TEST_CRED_STORE_123").unwrap();
        assert_eq!(val.as_str(), "stored_value");
    }

    #[test]
    fn resolver_missing_credentials() {
        let resolver = CredentialResolver::new(None, None);
        let missing = resolver.missing_credentials(&["DEFINITELY_NOT_SET_XYZ_789"]);
        assert_eq!(missing, vec!["DEFINITELY_NOT_SET_XYZ_789"]);
    }

    #[test]
    fn resolver_resolve_all() {
        std::env::set_var("TEST_MULTI_A", "a_val");
        std::env::set_var("TEST_MULTI_B", "b_val");

        let resolver = CredentialResolver::new(None, None);
        let resolved = resolver.resolve_all(&["TEST_MULTI_A", "TEST_MULTI_B", "TEST_MULTI_C"]);
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved["TEST_MULTI_A"].as_str(), "a_val");
        assert_eq!(resolved["TEST_MULTI_B"].as_str(), "b_val");

        std::env::remove_var("TEST_MULTI_A");
        std::env::remove_var("TEST_MULTI_B");
    }
}
