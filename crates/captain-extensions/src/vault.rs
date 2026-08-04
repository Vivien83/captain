//! Credential Vault — AES-256-GCM encrypted secret storage.
//!
//! Stores secrets in `~/.captain/vault.enc`, with the master key sourced from
//! the OS keyring (Windows Credential Manager / macOS Keychain / Linux Secret Service)
//! or the `CAPTAIN_VAULT_KEY` env var for headless/CI environments.

use crate::vault_keyring;
use crate::{ExtensionError, ExtensionResult};
use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::Argon2;
use fs2::FileExt;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tracing::{debug, info};
use zeroize::Zeroizing;

/// Salt length for Argon2.
const SALT_LEN: usize = 16;
/// Nonce length for AES-256-GCM.
const NONCE_LEN: usize = 12;
/// Magic bytes for vault file format versioning.
const VAULT_MAGIC: &[u8; 4] = b"OFV1";
/// A wedged peer must not make a credential mutation hang forever.
const VAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const VAULT_LOCK_RETRY: Duration = Duration::from_millis(25);

/// On-disk vault format (encrypted).
#[derive(Serialize, Deserialize)]
struct VaultFile {
    /// Version marker.
    version: u8,
    /// Argon2 salt (base64).
    salt: String,
    /// AES-256-GCM nonce (base64).
    nonce: String,
    /// Encrypted data (base64).
    ciphertext: String,
}

/// Decrypted vault entries.
#[derive(Default, Serialize, Deserialize)]
struct VaultEntries {
    secrets: HashMap<String, String>,
}

/// AES-256-GCM encrypted credential vault.
pub struct CredentialVault {
    /// Path to vault.enc file.
    path: PathBuf,
    /// Decrypted entries (zeroed on drop via manual clearing).
    entries: HashMap<String, Zeroizing<String>>,
    /// Whether the vault is unlocked.
    unlocked: bool,
    /// Cached master key (zeroed on drop) — avoids re-resolving from env/keyring.
    cached_key: Option<Zeroizing<[u8; 32]>>,
}

impl CredentialVault {
    /// Create a new vault at the given path.
    pub fn new(vault_path: PathBuf) -> Self {
        Self {
            path: vault_path,
            entries: HashMap::new(),
            unlocked: false,
            cached_key: None,
        }
    }

    /// Initialize a new vault. Generates a master key and stores it in the OS keyring.
    pub fn init(&mut self) -> ExtensionResult<()> {
        let _lock = self.acquire_mutation_lock()?;
        if self.path.exists() {
            return Err(ExtensionError::Vault(
                "Vault already exists. Delete it first to re-initialize.".to_string(),
            ));
        }

        let key_bytes = vault_keyring::initialize_master_key()?;

        // Create empty vault file
        self.entries.clear();
        self.unlocked = true;
        if let Err(error) = self.save(&key_bytes) {
            self.entries.clear();
            self.unlocked = false;
            return Err(error);
        }
        self.cached_key = Some(key_bytes);
        info!("Credential vault initialized at {:?}", self.path);
        Ok(())
    }

    /// Unlock the vault by loading and decrypting entries.
    pub fn unlock(&mut self) -> ExtensionResult<()> {
        if self.unlocked {
            return Ok(());
        }
        if !self.path.exists() {
            return Err(ExtensionError::Vault(
                "Vault not initialized. Run `captain vault init`.".to_string(),
            ));
        }

        let master_key = self.resolve_master_key()?;
        self.load(&master_key)?;
        self.unlocked = true;
        self.cached_key = Some(master_key);
        debug!("Vault unlocked with {} entries", self.entries.len());
        Ok(())
    }

    /// Get a secret from the vault.
    pub fn get(&self, key: &str) -> Option<Zeroizing<String>> {
        self.entries.get(key).cloned()
    }

    /// Store a secret in the vault.
    pub fn set(&mut self, key: String, value: Zeroizing<String>) -> ExtensionResult<()> {
        if !self.unlocked {
            return Err(ExtensionError::VaultLocked);
        }
        let master_key = self.resolve_master_key()?;
        let _lock = self.acquire_mutation_lock()?;
        self.load(&master_key)?;
        self.entries.insert(key, value);
        self.persist_mutation(&master_key)
    }

    /// Remove a secret from the vault.
    pub fn remove(&mut self, key: &str) -> ExtensionResult<bool> {
        if !self.unlocked {
            return Err(ExtensionError::VaultLocked);
        }
        let master_key = self.resolve_master_key()?;
        let _lock = self.acquire_mutation_lock()?;
        self.load(&master_key)?;
        let removed = self.entries.remove(key).is_some();
        if removed {
            self.persist_mutation(&master_key)?;
        }
        Ok(removed)
    }

    /// List all keys in the vault (not values).
    pub fn list_keys(&self) -> Vec<&str> {
        self.entries.keys().map(|k| k.as_str()).collect()
    }

    /// Check if the vault file exists.
    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// Check if the vault is unlocked.
    pub fn is_unlocked(&self) -> bool {
        self.unlocked
    }

    /// Initialize a vault with an explicit master key (for testing / programmatic use).
    pub fn init_with_key(&mut self, master_key: Zeroizing<[u8; 32]>) -> ExtensionResult<()> {
        let _lock = self.acquire_mutation_lock()?;
        if self.path.exists() {
            return Err(ExtensionError::Vault(
                "Vault already exists. Delete it first to re-initialize.".to_string(),
            ));
        }
        self.entries.clear();
        self.unlocked = true;
        if let Err(error) = self.save(&master_key) {
            self.entries.clear();
            self.unlocked = false;
            return Err(error);
        }
        self.cached_key = Some(master_key);
        debug!(
            "Credential vault initialized at {:?} (explicit key)",
            self.path
        );
        Ok(())
    }

    /// Unlock the vault with an explicit master key (for testing / programmatic use).
    pub fn unlock_with_key(&mut self, master_key: Zeroizing<[u8; 32]>) -> ExtensionResult<()> {
        if self.unlocked {
            return Ok(());
        }
        if !self.path.exists() {
            return Err(ExtensionError::Vault(
                "Vault not initialized. Run `captain vault init`.".to_string(),
            ));
        }
        self.load(&master_key)?;
        self.unlocked = true;
        self.cached_key = Some(master_key);
        debug!(
            "Vault unlocked with {} entries (explicit key)",
            self.entries.len()
        );
        Ok(())
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the vault is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    // ── Internal ─────────────────────────────────────────────────────────

    /// Serialize the complete reload-mutate-save cycle across processes.
    fn acquire_mutation_lock(&self) -> ExtensionResult<File> {
        let lock_path = vault_lock_path(&self.path);
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let lock_file = open_private_lock_file(&lock_path)?;
        let started = Instant::now();
        loop {
            match lock_file.try_lock_exclusive() {
                Ok(()) => return Ok(lock_file),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if started.elapsed() >= VAULT_LOCK_TIMEOUT {
                        return Err(ExtensionError::Vault(format!(
                            "Credential vault is busy after {} seconds; retry the operation",
                            VAULT_LOCK_TIMEOUT.as_secs()
                        )));
                    }
                    std::thread::sleep(VAULT_LOCK_RETRY);
                }
                Err(error) => {
                    return Err(ExtensionError::Vault(format!(
                        "Credential vault lock failed: {error}"
                    )));
                }
            }
        }
    }

    /// Never expose an in-memory credential state that failed durability.
    fn persist_mutation(&mut self, master_key: &[u8; 32]) -> ExtensionResult<()> {
        match self.save(master_key) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = self.load(master_key);
                Err(error)
            }
        }
    }

    /// Resolve the master key from cache, explicit env, or the OS credential store.
    fn resolve_master_key(&self) -> ExtensionResult<Zeroizing<[u8; 32]>> {
        if let Some(ref cached) = self.cached_key {
            return Ok(cached.clone());
        }

        if let Some(key) = vault_keyring::resolve_master_key()? {
            return Ok(key);
        }

        Err(ExtensionError::VaultLocked)
    }

    /// Save encrypted vault to disk.
    fn save(&self, master_key: &[u8; 32]) -> ExtensionResult<()> {
        // Serialize entries to JSON
        let plain_entries: HashMap<String, String> = self
            .entries
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().to_string()))
            .collect();
        let vault_data = VaultEntries {
            secrets: plain_entries,
        };
        let plaintext = Zeroizing::new(
            serde_json::to_vec(&vault_data)
                .map_err(|e| ExtensionError::Vault(format!("Serialization failed: {e}")))?,
        );

        // Generate salt and nonce
        let mut salt = [0u8; SALT_LEN];
        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut salt);
        OsRng.fill_bytes(&mut nonce_bytes);

        // Derive encryption key from master key + salt using Argon2
        let derived_key = derive_key(master_key, &salt)?;

        // Encrypt with AES-256-GCM
        let cipher = Aes256Gcm::new_from_slice(derived_key.as_ref())
            .map_err(|e| ExtensionError::Vault(format!("Cipher init failed: {e}")))?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_slice())
            .map_err(|e| ExtensionError::Vault(format!("Encryption failed: {e}")))?;

        // Write to file
        let vault_file = VaultFile {
            version: 1,
            salt: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, salt),
            nonce: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, nonce_bytes),
            ciphertext: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &ciphertext,
            ),
        };
        let content = serde_json::to_string_pretty(&vault_file)
            .map_err(|e| ExtensionError::Vault(format!("Vault file serialization failed: {e}")))?;

        // Prepend OFV1 magic bytes for format detection
        let mut output = Vec::with_capacity(VAULT_MAGIC.len() + content.len());
        output.extend_from_slice(VAULT_MAGIC);
        output.extend_from_slice(content.as_bytes());
        captain_types::durable_fs::atomic_write(&self.path, &output)?;
        Ok(())
    }

    /// Load and decrypt vault from disk.
    fn load(&mut self, master_key: &[u8; 32]) -> ExtensionResult<()> {
        let raw = std::fs::read(&self.path)?;

        // Strip OFV1 magic header if present; legacy JSON files start with '{'
        let content = if raw.starts_with(VAULT_MAGIC) {
            std::str::from_utf8(&raw[VAULT_MAGIC.len()..])
                .map_err(|e| ExtensionError::Vault(format!("UTF-8 decode failed: {e}")))?
        } else if raw.first() == Some(&b'{') {
            // Legacy JSON vault (no magic header)
            std::str::from_utf8(&raw)
                .map_err(|e| ExtensionError::Vault(format!("UTF-8 decode failed: {e}")))?
        } else {
            return Err(ExtensionError::Vault(
                "Unrecognized vault file format".to_string(),
            ));
        };

        let vault_file: VaultFile = serde_json::from_str(content)
            .map_err(|e| ExtensionError::Vault(format!("Vault file parse failed: {e}")))?;

        if vault_file.version != 1 {
            return Err(ExtensionError::Vault(format!(
                "Unsupported vault version: {}",
                vault_file.version
            )));
        }

        let salt =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &vault_file.salt)
                .map_err(|e| ExtensionError::Vault(format!("Salt decode failed: {e}")))?;
        let nonce_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &vault_file.nonce,
        )
        .map_err(|e| ExtensionError::Vault(format!("Nonce decode failed: {e}")))?;
        let ciphertext = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &vault_file.ciphertext,
        )
        .map_err(|e| ExtensionError::Vault(format!("Ciphertext decode failed: {e}")))?;

        // Derive key
        let derived_key = derive_key(master_key, &salt)?;

        // Decrypt
        let cipher = Aes256Gcm::new_from_slice(derived_key.as_ref())
            .map_err(|e| ExtensionError::Vault(format!("Cipher init failed: {e}")))?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(nonce, ciphertext.as_slice())
                .map_err(|e| ExtensionError::Vault(format!("Decryption failed: {e}")))?,
        );

        // Parse entries
        let vault_data: VaultEntries = serde_json::from_slice(&plaintext)
            .map_err(|e| ExtensionError::Vault(format!("Vault data parse failed: {e}")))?;

        self.entries.clear();
        for (k, v) in vault_data.secrets {
            self.entries.insert(k, Zeroizing::new(v));
        }
        Ok(())
    }
}

fn vault_lock_path(vault_path: &Path) -> PathBuf {
    let mut path = vault_path.as_os_str().to_os_string();
    path.push(".lock");
    PathBuf::from(path)
}

fn open_private_lock_file(path: &Path) -> ExtensionResult<File> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

impl Drop for CredentialVault {
    fn drop(&mut self) {
        // Zeroizing<String> handles zeroing individual values.
        // Clear the map to ensure all entries are dropped.
        self.entries.clear();
        self.cached_key = None;
        self.unlocked = false;
    }
}

/// Derive a 256-bit key from master key + salt using Argon2id.
fn derive_key(master_key: &[u8; 32], salt: &[u8]) -> ExtensionResult<Zeroizing<[u8; 32]>> {
    let mut derived = Zeroizing::new([0u8; 32]);
    Argon2::default()
        .hash_password_into(master_key, salt, derived.as_mut())
        .map_err(|e| ExtensionError::Vault(format!("Key derivation failed: {e}")))?;
    Ok(derived)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_vault() -> (tempfile::TempDir, CredentialVault) {
        let dir = tempfile::tempdir().unwrap();
        let vault_path = dir.path().join("vault.enc");
        let vault = CredentialVault::new(vault_path);
        (dir, vault)
    }

    /// Generate a random 32-byte master key for tests.
    fn random_key() -> Zeroizing<[u8; 32]> {
        let mut kb = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(kb.as_mut());
        kb
    }

    #[test]
    fn vault_init_and_roundtrip() {
        let (dir, mut vault) = test_vault();
        let key = random_key();

        // Init creates vault file
        vault.init_with_key(key.clone()).unwrap();
        assert!(vault.exists());
        assert!(vault.is_unlocked());
        assert!(vault.is_empty());

        // Store a secret
        vault
            .set(
                "GITHUB_TOKEN".to_string(),
                Zeroizing::new("ghp_test123".to_string()),
            )
            .unwrap();
        assert_eq!(vault.len(), 1);

        // Read it back
        let val = vault.get("GITHUB_TOKEN").unwrap();
        assert_eq!(val.as_str(), "ghp_test123");

        // New vault instance, unlock with same key
        let mut vault2 = CredentialVault::new(dir.path().join("vault.enc"));
        vault2.unlock_with_key(key).unwrap();
        let val2 = vault2.get("GITHUB_TOKEN").unwrap();
        assert_eq!(val2.as_str(), "ghp_test123");

        // Remove
        assert!(vault2.remove("GITHUB_TOKEN").unwrap());
        assert!(vault2.get("GITHUB_TOKEN").is_none());
    }

    #[test]
    fn vault_list_keys() {
        let (_dir, mut vault) = test_vault();
        let key = random_key();

        vault.init_with_key(key).unwrap();
        vault
            .set("A".to_string(), Zeroizing::new("1".to_string()))
            .unwrap();
        vault
            .set("B".to_string(), Zeroizing::new("2".to_string()))
            .unwrap();

        let mut keys = vault.list_keys();
        keys.sort();
        assert_eq!(keys, vec!["A", "B"]);
    }

    #[test]
    fn stale_instances_merge_mutations_under_the_process_lock() {
        let (dir, mut first) = test_vault();
        let key = random_key();
        first.init_with_key(key.clone()).unwrap();

        let mut second = CredentialVault::new(dir.path().join("vault.enc"));
        second.unlock_with_key(key.clone()).unwrap();
        first
            .set("FIRST".to_string(), Zeroizing::new("one".to_string()))
            .unwrap();
        second
            .set("SECOND".to_string(), Zeroizing::new("two".to_string()))
            .unwrap();

        let mut reopened = CredentialVault::new(dir.path().join("vault.enc"));
        reopened.unlock_with_key(key).unwrap();
        assert_eq!(reopened.get("FIRST").unwrap().as_str(), "one");
        assert_eq!(reopened.get("SECOND").unwrap().as_str(), "two");
    }

    #[cfg(unix)]
    #[test]
    fn failed_save_restores_the_last_durable_snapshot() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, mut vault) = test_vault();
        let key = random_key();
        vault.init_with_key(key.clone()).unwrap();
        vault
            .set("DURABLE".to_string(), Zeroizing::new("kept".to_string()))
            .unwrap();

        let original_permissions = std::fs::metadata(dir.path()).unwrap().permissions();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = vault.set(
            "UNSAVED".to_string(),
            Zeroizing::new("discarded".to_string()),
        );
        std::fs::set_permissions(dir.path(), original_permissions).unwrap();

        assert!(result.is_err());
        assert_eq!(vault.get("DURABLE").unwrap().as_str(), "kept");
        assert!(vault.get("UNSAVED").is_none());

        let mut reopened = CredentialVault::new(dir.path().join("vault.enc"));
        reopened.unlock_with_key(key).unwrap();
        assert_eq!(reopened.get("DURABLE").unwrap().as_str(), "kept");
        assert!(reopened.get("UNSAVED").is_none());
    }

    #[test]
    fn vault_wrong_key_fails() {
        let (dir, mut vault) = test_vault();
        let good_key = random_key();

        vault.init_with_key(good_key).unwrap();
        vault
            .set("SECRET".to_string(), Zeroizing::new("value".to_string()))
            .unwrap();

        // Wrong key — should fail to decrypt
        let bad_key = random_key();
        let mut vault2 = CredentialVault::new(dir.path().join("vault.enc"));
        assert!(vault2.unlock_with_key(bad_key).is_err());
    }

    #[test]
    fn derive_key_deterministic() {
        let master = [42u8; 32];
        let salt = [1u8; 16];
        let k1 = derive_key(&master, &salt).unwrap();
        let k2 = derive_key(&master, &salt).unwrap();
        assert_eq!(k1.as_ref(), k2.as_ref());
    }

    #[test]
    fn vault_file_has_magic_header() {
        let (_dir, mut vault) = test_vault();
        let key = random_key();
        vault.init_with_key(key).unwrap();

        let raw = std::fs::read(&vault.path).unwrap();
        assert_eq!(&raw[..4], b"OFV1");
    }

    #[test]
    fn vault_legacy_json_compat() {
        let (dir, mut vault) = test_vault();
        let key = random_key();
        vault.init_with_key(key.clone()).unwrap();
        vault
            .set("KEY".to_string(), Zeroizing::new("val".to_string()))
            .unwrap();

        // Strip the OFV1 magic header to simulate a legacy vault file
        let raw = std::fs::read(&vault.path).unwrap();
        assert_eq!(&raw[..4], b"OFV1");
        std::fs::write(&vault.path, &raw[4..]).unwrap();

        // Should still load (legacy compat)
        let mut vault2 = CredentialVault::new(dir.path().join("vault.enc"));
        vault2.unlock_with_key(key).unwrap();
        assert_eq!(vault2.get("KEY").unwrap().as_str(), "val");
    }

    #[test]
    fn vault_rejects_bad_magic() {
        let (dir, mut vault) = test_vault();
        let key = random_key();
        vault.init_with_key(key.clone()).unwrap();

        // Overwrite with unrecognized binary data
        std::fs::write(&vault.path, b"BAAD not json").unwrap();

        let mut vault2 = CredentialVault::new(dir.path().join("vault.enc"));
        let result = vault2.unlock_with_key(key);
        assert!(result.is_err());
        let msg = format!("{:?}", result.unwrap_err());
        assert!(msg.contains("Unrecognized vault file format"));
    }
}
