use crate::{ExtensionError, ExtensionResult};
use aes_gcm::aead::OsRng;
use rand::RngCore;
use sha2::{Digest as _, Sha256};
use std::path::{Path, PathBuf};
use tracing::{info, warn};
use zeroize::Zeroizing;

const KEYRING_SERVICE: &str = "captain-vault";
const KEYRING_USER: &str = "master-key";
const VAULT_KEY_ENV: &str = "CAPTAIN_VAULT_KEY";

enum StoredMasterKey {
    Missing,
    Found(Zeroizing<String>),
}

trait MasterKeyStore {
    fn load(&self) -> Result<StoredMasterKey, String>;
    fn store(&self, key_b64: &str) -> Result<(), String>;
}

struct OsMasterKeyStore;

impl MasterKeyStore for OsMasterKeyStore {
    fn load(&self) -> Result<StoredMasterKey, String> {
        load_os_keyring_key()
    }

    fn store(&self, key_b64: &str) -> Result<(), String> {
        store_os_keyring_key(key_b64)
    }
}

pub(crate) fn initialize_master_key() -> ExtensionResult<Zeroizing<[u8; 32]>> {
    if let Some(encoded) = read_env_master_key()? {
        info!("Using existing vault key from {}", VAULT_KEY_ENV);
        return decode_master_key(&encoded);
    }

    let store = OsMasterKeyStore;
    let legacy_path = legacy_keyring_path()?;
    let fingerprint = machine_fingerprint();
    if let Some(existing) = load_or_migrate_master_key(&store, &legacy_path, &fingerprint)? {
        info!("Using existing vault key from OS credential store");
        return Ok(existing);
    }

    let mut generated = Zeroizing::new([0u8; 32]);
    OsRng.fill_bytes(generated.as_mut());
    persist_master_key(&store, &generated)?;
    info!("Vault master key stored in OS credential store");
    Ok(generated)
}

pub(crate) fn resolve_master_key() -> ExtensionResult<Option<Zeroizing<[u8; 32]>>> {
    if let Some(encoded) = read_env_master_key()? {
        return decode_master_key(&encoded).map(Some);
    }

    let store = OsMasterKeyStore;
    let legacy_path = legacy_keyring_path()?;
    load_or_migrate_master_key(&store, &legacy_path, &machine_fingerprint())
}

fn read_env_master_key() -> ExtensionResult<Option<Zeroizing<String>>> {
    match std::env::var(VAULT_KEY_ENV) {
        Ok(value) => Ok(Some(Zeroizing::new(value))),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(ExtensionError::Vault(format!(
            "{VAULT_KEY_ENV} is not valid UTF-8"
        ))),
    }
}

fn persist_master_key(store: &impl MasterKeyStore, key: &[u8; 32]) -> ExtensionResult<()> {
    let encoded = Zeroizing::new(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        key,
    ));
    store.store(&encoded).map_err(key_store_error)?;
    match store.load().map_err(key_store_error)? {
        StoredMasterKey::Found(stored) if stored.as_bytes() == encoded.as_bytes() => Ok(()),
        StoredMasterKey::Found(_) => Err(ExtensionError::Vault(
            "OS credential store verification returned a different master key".to_string(),
        )),
        StoredMasterKey::Missing => Err(ExtensionError::Vault(
            "OS credential store did not retain the master key".to_string(),
        )),
    }
}

fn load_or_migrate_master_key(
    store: &impl MasterKeyStore,
    legacy_path: &Path,
    fingerprint: &[u8],
) -> ExtensionResult<Option<Zeroizing<[u8; 32]>>> {
    match store.load().map_err(key_store_error)? {
        StoredMasterKey::Found(encoded) => {
            let key = decode_master_key(&encoded)?;
            cleanup_legacy_keyring_copy(legacy_path, fingerprint, &key)?;
            Ok(Some(key))
        }
        StoredMasterKey::Missing if legacy_path.exists() => {
            let encoded = load_legacy_keyring_key(legacy_path, fingerprint)?;
            let key = decode_master_key(&encoded)?;
            persist_master_key(store, &key)?;
            std::fs::remove_file(legacy_path).map_err(|error| {
                ExtensionError::Vault(format!(
                    "Master key migrated but obsolete legacy key file could not be removed at {}: {error}",
                    legacy_path.display()
                ))
            })?;
            info!(path = %legacy_path.display(), "Migrated legacy vault key into OS credential store");
            Ok(Some(key))
        }
        StoredMasterKey::Missing => Ok(None),
    }
}

fn cleanup_legacy_keyring_copy(
    legacy_path: &Path,
    fingerprint: &[u8],
    native_key: &[u8; 32],
) -> ExtensionResult<()> {
    if !legacy_path.exists() {
        return Ok(());
    }
    match load_legacy_keyring_key(legacy_path, fingerprint)
        .and_then(|encoded| decode_master_key(&encoded))
    {
        Ok(legacy_key) if legacy_key.as_ref() == native_key => {
            std::fs::remove_file(legacy_path).map_err(|error| {
                ExtensionError::Vault(format!(
                    "OS credential store is ready but obsolete legacy key file could not be removed at {}: {error}",
                    legacy_path.display()
                ))
            })?;
            info!(path = %legacy_path.display(), "Removed obsolete legacy vault key file");
        }
        Ok(_) => {
            return Err(ExtensionError::Vault(format!(
                "Legacy vault key at {} conflicts with the OS credential store; refusing to delete either copy",
                legacy_path.display()
            )))
        }
        Err(error) => {
            warn!(
                path = %legacy_path.display(),
                %error,
                "Could not validate obsolete legacy vault key file"
            );
            return Err(ExtensionError::Vault(format!(
                "OS credential store is ready but obsolete legacy key file at {} could not be validated; refusing to ignore or delete it",
                legacy_path.display()
            )));
        }
    }
    Ok(())
}

fn key_store_error(error: String) -> ExtensionError {
    ExtensionError::Vault(format!(
        "OS credential store unavailable: {error}. Unlock the system credential store or set {VAULT_KEY_ENV} explicitly for headless/CI"
    ))
}

fn legacy_keyring_path() -> ExtensionResult<PathBuf> {
    dirs::data_local_dir()
        .map(|path| path.join("captain").join(".keyring"))
        .ok_or_else(|| {
            ExtensionError::Vault(
                "Could not determine the legacy keyring path for migration".to_string(),
            )
        })
}

fn load_legacy_keyring_key(path: &Path, fingerprint: &[u8]) -> ExtensionResult<Zeroizing<String>> {
    let encoded = std::fs::read_to_string(path).map_err(|error| {
        ExtensionError::Vault(format!(
            "Could not read legacy vault key at {}: {error}",
            path.display()
        ))
    })?;
    decode_legacy_keyring(&encoded, fingerprint)
}

fn decode_legacy_keyring(encoded: &str, fingerprint: &[u8]) -> ExtensionResult<Zeroizing<String>> {
    let obfuscated =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded.trim())
            .map_err(|error| ExtensionError::Vault(format!("Legacy key decode failed: {error}")))?;
    let mask = legacy_keyring_mask(fingerprint);
    let key_bytes = Zeroizing::new(
        obfuscated
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % mask.len()])
            .collect::<Vec<_>>(),
    );
    let key = String::from_utf8(key_bytes.to_vec())
        .map_err(|error| ExtensionError::Vault(format!("Legacy key UTF-8 failed: {error}")))?;
    Ok(Zeroizing::new(key))
}

fn decode_master_key(key_b64: &str) -> ExtensionResult<Zeroizing<[u8; 32]>> {
    let bytes = Zeroizing::new(
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, key_b64)
            .map_err(|error| ExtensionError::Vault(format!("Key decode failed: {error}")))?,
    );
    if bytes.len() != 32 {
        return Err(ExtensionError::Vault(format!(
            "Invalid key length: expected 32, got {}",
            bytes.len()
        )));
    }
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&bytes);
    Ok(key)
}

fn legacy_keyring_mask(fingerprint: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(fingerprint);
    hasher.update(KEYRING_SERVICE.as_bytes());
    hasher.finalize().to_vec()
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn os_keyring_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|error| format!("credential entry: {error}"))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn store_os_keyring_key(key_b64: &str) -> Result<(), String> {
    os_keyring_entry()?
        .set_password(key_b64)
        .map_err(|error| format!("store: {error}"))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn load_os_keyring_key() -> Result<StoredMasterKey, String> {
    match os_keyring_entry()?.get_password() {
        Ok(value) => Ok(StoredMasterKey::Found(Zeroizing::new(value))),
        Err(keyring::Error::NoEntry) => Ok(StoredMasterKey::Missing),
        Err(error) => Err(format!("read: {error}")),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn store_os_keyring_key(_key_b64: &str) -> Result<(), String> {
    Err("native credential storage is unsupported on this platform".to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn load_os_keyring_key() -> Result<StoredMasterKey, String> {
    Err("native credential storage is unsupported on this platform".to_string())
}

fn machine_fingerprint() -> Vec<u8> {
    let mut hasher = Sha256::new();
    if let Ok(user) = std::env::var("USERNAME").or_else(|_| std::env::var("USER")) {
        hasher.update(user.as_bytes());
    }
    if let Ok(host) = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")) {
        hasher.update(host.as_bytes());
    }
    hasher.update(b"captain-vault-v1");
    hasher.finalize().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct TestKeyStore {
        value: Mutex<Option<String>>,
        fail_store: bool,
        corrupt_readback: bool,
    }

    impl MasterKeyStore for TestKeyStore {
        fn load(&self) -> Result<StoredMasterKey, String> {
            let value = self.value.lock().unwrap().clone();
            match value {
                Some(mut value) => {
                    if self.corrupt_readback {
                        value = base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            [99u8; 32],
                        );
                    }
                    Ok(StoredMasterKey::Found(Zeroizing::new(value)))
                }
                None => Ok(StoredMasterKey::Missing),
            }
        }

        fn store(&self, key_b64: &str) -> Result<(), String> {
            if self.fail_store {
                return Err("test store unavailable".to_string());
            }
            *self.value.lock().unwrap() = Some(key_b64.to_string());
            Ok(())
        }
    }

    fn write_legacy_key(path: &Path, fingerprint: &[u8], key: &[u8; 32]) {
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key);
        let mask = legacy_keyring_mask(fingerprint);
        let obfuscated: Vec<u8> = encoded
            .as_bytes()
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % mask.len()])
            .collect();
        std::fs::write(
            path,
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, obfuscated),
        )
        .unwrap();
    }

    #[test]
    fn legacy_key_is_migrated_verified_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let legacy_path = dir.path().join(".keyring");
        let fingerprint = b"stable-test-machine";
        let key = [42u8; 32];
        write_legacy_key(&legacy_path, fingerprint, &key);
        let store = TestKeyStore::default();

        let migrated = load_or_migrate_master_key(&store, &legacy_path, fingerprint)
            .unwrap()
            .unwrap();

        assert_eq!(migrated.as_ref(), &key);
        assert!(!legacy_path.exists());
        assert!(matches!(store.load().unwrap(), StoredMasterKey::Found(_)));
    }

    #[test]
    fn legacy_key_remains_when_native_storage_fails() {
        let dir = tempfile::tempdir().unwrap();
        let legacy_path = dir.path().join(".keyring");
        let fingerprint = b"stable-test-machine";
        write_legacy_key(&legacy_path, fingerprint, &[42u8; 32]);
        let store = TestKeyStore {
            fail_store: true,
            ..TestKeyStore::default()
        };

        let error = load_or_migrate_master_key(&store, &legacy_path, fingerprint)
            .unwrap_err()
            .to_string();

        assert!(error.contains("OS credential store unavailable"));
        assert!(error.contains(VAULT_KEY_ENV));
        assert!(legacy_path.exists());
    }

    #[test]
    fn native_store_is_verified_before_success() {
        let store = TestKeyStore {
            corrupt_readback: true,
            ..TestKeyStore::default()
        };

        let error = persist_master_key(&store, &[7u8; 32])
            .unwrap_err()
            .to_string();

        assert!(error.contains("different master key"));
    }

    #[test]
    fn conflicting_legacy_copy_is_never_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let legacy_path = dir.path().join(".keyring");
        let fingerprint = b"stable-test-machine";
        write_legacy_key(&legacy_path, fingerprint, &[7u8; 32]);
        let native = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, [8u8; 32]);
        let store = TestKeyStore {
            value: Mutex::new(Some(native)),
            ..TestKeyStore::default()
        };

        let error = load_or_migrate_master_key(&store, &legacy_path, fingerprint)
            .unwrap_err()
            .to_string();

        assert!(error.contains("conflicts"));
        assert!(legacy_path.exists());
    }

    #[test]
    fn malformed_legacy_copy_is_not_silently_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let legacy_path = dir.path().join(".keyring");
        std::fs::write(&legacy_path, "not-base64").unwrap();
        let native = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, [8u8; 32]);
        let store = TestKeyStore {
            value: Mutex::new(Some(native)),
            ..TestKeyStore::default()
        };

        let error = load_or_migrate_master_key(&store, &legacy_path, b"stable-test-machine")
            .unwrap_err()
            .to_string();

        assert!(error.contains("could not be validated"));
        assert!(legacy_path.exists());
    }
}
