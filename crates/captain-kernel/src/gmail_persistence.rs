//! Crash-safe coordination between Gmail metadata and vault credentials.

use captain_extensions::vault::CredentialVault;
use captain_memory::gmail_accounts::{GmailAccountRecord, GmailAccountStore, NewGmailAccount};
use captain_types::email::GmailAccountAlias;
use captain_types::error::{CaptainError, CaptainResult};
use fs2::FileExt;
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::time::{Duration, Instant};
use uuid::Uuid;
use zeroize::Zeroizing;

const TOKEN_KEY_PREFIX: &str = "CAPTAIN_GMAIL_TOKEN_";
const CLIENT_KEY_PREFIX: &str = "CAPTAIN_GMAIL_CLIENT_";
const PERSISTENCE_LOCK_NAME: &str = "gmail-accounts.lock";
const PERSISTENCE_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const PERSISTENCE_LOCK_RETRY: Duration = Duration::from_millis(25);

/// Marker proving that one process owns the Gmail vault/SQLite lifecycle.
pub struct GmailPersistenceLock {
    _file: File,
}

pub struct GmailPersistenceOutcome {
    pub record: GmailAccountRecord,
    pub cleanup_warnings: Vec<String>,
}

pub struct GmailDeleteOutcome {
    pub deleted: Option<GmailAccountRecord>,
    pub cleanup_warnings: Vec<String>,
}

/// Acquire the bounded lock shared by CLI, tools and background sync.
pub fn acquire_gmail_persistence_lock(home: &Path) -> CaptainResult<GmailPersistenceLock> {
    std::fs::create_dir_all(home)?;
    let path = home.join(PERSISTENCE_LOCK_NAME);
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }

    let started = Instant::now();
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(GmailPersistenceLock { _file: file }),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if started.elapsed() >= PERSISTENCE_LOCK_TIMEOUT {
                    return Err(CaptainError::Internal(
                        "Gmail account state is busy after 10 seconds; retry the operation"
                            .to_string(),
                    ));
                }
                std::thread::sleep(PERSISTENCE_LOCK_RETRY);
            }
            Err(error) => {
                return Err(CaptainError::Io(error));
            }
        }
    }
}

pub fn new_gmail_token_vault_key() -> String {
    versioned_vault_key(TOKEN_KEY_PREFIX)
}

pub fn new_gmail_client_vault_key() -> String {
    versioned_vault_key(CLIENT_KEY_PREFIX)
}

/// Persist secrets first, then publish their references in one SQLite commit.
/// A later sweep removes pre-commit or replaced versions after a crash.
pub fn persist_gmail_grant(
    _lock: &GmailPersistenceLock,
    vault: &mut CredentialVault,
    store: &GmailAccountStore,
    account: NewGmailAccount,
    token_secret: Zeroizing<String>,
    new_client_secret: Option<Zeroizing<String>>,
) -> CaptainResult<GmailPersistenceOutcome> {
    validate_entry_kinds(&account)?;
    let records = store.list()?;
    reject_token_collision(vault, &records, &account.token_vault_key)?;
    validate_client_reference(
        vault,
        &records,
        &account.client_vault_key,
        new_client_secret.is_some(),
    )?;

    if let Some(client_secret) = new_client_secret {
        vault
            .set(account.client_vault_key.clone(), client_secret)
            .map_err(vault_write_error)?;
    }
    if let Err(error) = vault.set(account.token_vault_key.clone(), token_secret) {
        let cleanup = sweep_orphaned_gmail_secrets(_lock, vault, store);
        return Err(with_rollback_status(vault_write_error(error), cleanup));
    }

    let record = match store.upsert(account) {
        Ok(record) => record,
        Err(error) => {
            let cleanup = sweep_orphaned_gmail_secrets(_lock, vault, store);
            return Err(with_rollback_status(error, cleanup));
        }
    };
    let cleanup_warnings = sweep_orphaned_gmail_secrets(_lock, vault, store);
    Ok(GmailPersistenceOutcome {
        record,
        cleanup_warnings,
    })
}

/// Delete metadata first. A crash before vault cleanup leaves only an orphan
/// that the next locked operation removes; it never resurrects an account.
pub fn delete_gmail_account(
    lock: &GmailPersistenceLock,
    vault: &mut CredentialVault,
    store: &GmailAccountStore,
    alias: &GmailAccountAlias,
) -> CaptainResult<GmailDeleteOutcome> {
    let deleted = store.delete(alias)?;
    let cleanup_warnings = sweep_orphaned_gmail_secrets(lock, vault, store);
    Ok(GmailDeleteOutcome {
        deleted,
        cleanup_warnings,
    })
}

/// Remove only versioned Gmail entries that no committed account references.
pub fn sweep_orphaned_gmail_secrets(
    _lock: &GmailPersistenceLock,
    vault: &mut CredentialVault,
    store: &GmailAccountStore,
) -> Vec<String> {
    let records = match store.list() {
        Ok(records) => records,
        Err(_) => return vec!["metadata_unavailable".to_string()],
    };
    let referenced = referenced_vault_keys(&records);
    let candidates = vault
        .list_keys()
        .into_iter()
        .filter(|key| is_managed_gmail_vault_key(key) && !referenced.contains(*key))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let mut warnings = Vec::new();
    for key in candidates {
        if vault.remove(&key).is_err() {
            warnings.push("vault_cleanup_failed".to_string());
        }
    }
    warnings.sort();
    warnings.dedup();
    warnings
}

fn validate_entry_kinds(account: &NewGmailAccount) -> CaptainResult<()> {
    if !account.token_vault_key.starts_with(TOKEN_KEY_PREFIX)
        || !account.client_vault_key.starts_with(CLIENT_KEY_PREFIX)
    {
        return Err(CaptainError::Config(
            "Gmail credential references use an unsupported namespace".to_string(),
        ));
    }
    Ok(())
}

fn reject_token_collision(
    vault: &CredentialVault,
    records: &[GmailAccountRecord],
    key: &str,
) -> CaptainResult<()> {
    let referenced = records.iter().any(|record| record.token_vault_key == key);
    if referenced || vault.get(key).is_some() {
        return Err(CaptainError::Config(
            "Generated Gmail token reference collided; retry the operation".to_string(),
        ));
    }
    Ok(())
}

fn validate_client_reference(
    vault: &CredentialVault,
    records: &[GmailAccountRecord],
    key: &str,
    has_new_secret: bool,
) -> CaptainResult<()> {
    let referenced = records.iter().any(|record| record.client_vault_key == key);
    let exists = vault.get(key).is_some();
    match (has_new_secret, referenced, exists) {
        (true, false, false) | (false, true, true) => Ok(()),
        (true, _, _) => Err(CaptainError::Config(
            "Generated Gmail client reference collided; retry the operation".to_string(),
        )),
        (false, _, _) => Err(CaptainError::Config(
            "Stored Gmail OAuth client is unavailable; reconnect with --client-json".to_string(),
        )),
    }
}

fn referenced_vault_keys(records: &[GmailAccountRecord]) -> HashSet<&str> {
    records
        .iter()
        .flat_map(|record| {
            [
                record.token_vault_key.as_str(),
                record.client_vault_key.as_str(),
            ]
        })
        .collect()
}

fn versioned_vault_key(prefix: &str) -> String {
    format!(
        "{prefix}{}",
        Uuid::new_v4().simple().to_string().to_uppercase()
    )
}

/// Whether a vault entry belongs to Captain's managed Gmail namespace.
/// Generic vault commands must not mutate these entries independently of the
/// metadata transaction.
pub fn is_managed_gmail_vault_key(key: &str) -> bool {
    key.starts_with(TOKEN_KEY_PREFIX) || key.starts_with(CLIENT_KEY_PREFIX)
}

fn vault_write_error(error: captain_extensions::ExtensionError) -> CaptainError {
    CaptainError::Config(format!("Gmail credential persistence failed: {error}"))
}

fn with_rollback_status(error: CaptainError, cleanup: Vec<String>) -> CaptainError {
    if cleanup.is_empty() {
        error
    } else {
        CaptainError::Internal(format!(
            "{error}; credential cleanup is pending and will retry on the next email operation"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_memory::MemorySubstrate;
    use captain_types::email::{GmailAccessProfile, GmailAccountAlias};

    fn account(alias: &str, email: &str, token_key: String, client_key: String) -> NewGmailAccount {
        NewGmailAccount {
            alias: GmailAccountAlias::parse(alias).unwrap(),
            email_address: email.to_string(),
            access_profile: GmailAccessProfile::Assistant,
            granted_scopes: GmailAccessProfile::Assistant
                .required_scopes()
                .iter()
                .map(|scope| (*scope).to_string())
                .collect(),
            token_vault_key: token_key,
            client_vault_key: client_key,
            history_id: Some("123".to_string()),
            make_default: false,
        }
    }

    fn secret(value: &str) -> Zeroizing<String> {
        Zeroizing::new(value.to_string())
    }

    #[test]
    fn failed_metadata_commit_rolls_back_staged_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let memory = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = memory.gmail_accounts();
        let mut vault = CredentialVault::new(dir.path().join("vault.enc"));
        vault.init_with_key(Zeroizing::new([7_u8; 32])).unwrap();
        let lock = acquire_gmail_persistence_lock(dir.path()).unwrap();

        let first_client = new_gmail_client_vault_key();
        persist_gmail_grant(
            &lock,
            &mut vault,
            store,
            account(
                "first",
                "same@example.com",
                new_gmail_token_vault_key(),
                first_client.clone(),
            ),
            secret("token-one"),
            Some(secret("client-one")),
        )
        .unwrap();
        let staged_token = new_gmail_token_vault_key();
        let staged_client = new_gmail_client_vault_key();
        let result = persist_gmail_grant(
            &lock,
            &mut vault,
            store,
            account(
                "second",
                "same@example.com",
                staged_token.clone(),
                staged_client.clone(),
            ),
            secret("token-two"),
            Some(secret("client-two")),
        );

        assert!(result.is_err());
        assert!(vault.get(&staged_token).is_none());
        assert!(vault.get(&staged_client).is_none());
        assert!(vault.get(&first_client).is_some());
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn replacement_cleans_old_token_but_preserves_shared_client() {
        let dir = tempfile::tempdir().unwrap();
        let memory = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = memory.gmail_accounts();
        let mut vault = CredentialVault::new(dir.path().join("vault.enc"));
        vault.init_with_key(Zeroizing::new([8_u8; 32])).unwrap();
        let lock = acquire_gmail_persistence_lock(dir.path()).unwrap();
        let shared_client = new_gmail_client_vault_key();
        let old_token = new_gmail_token_vault_key();

        persist_gmail_grant(
            &lock,
            &mut vault,
            store,
            account(
                "one",
                "one@example.com",
                old_token.clone(),
                shared_client.clone(),
            ),
            secret("old-token"),
            Some(secret("shared-client")),
        )
        .unwrap();
        persist_gmail_grant(
            &lock,
            &mut vault,
            store,
            account(
                "two",
                "two@example.com",
                new_gmail_token_vault_key(),
                shared_client.clone(),
            ),
            secret("token-two"),
            None,
        )
        .unwrap();
        let new_token = new_gmail_token_vault_key();
        persist_gmail_grant(
            &lock,
            &mut vault,
            store,
            account(
                "one",
                "one@example.com",
                new_token.clone(),
                shared_client.clone(),
            ),
            secret("new-token"),
            None,
        )
        .unwrap();

        assert!(vault.get(&old_token).is_none());
        assert!(vault.get(&new_token).is_some());
        assert!(vault.get(&shared_client).is_some());
        assert_eq!(store.list().unwrap().len(), 2);
    }

    #[test]
    fn orphan_sweep_never_touches_non_gmail_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let memory = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = memory.gmail_accounts();
        let mut vault = CredentialVault::new(dir.path().join("vault.enc"));
        vault.init_with_key(Zeroizing::new([9_u8; 32])).unwrap();
        let lock = acquire_gmail_persistence_lock(dir.path()).unwrap();
        let orphan = new_gmail_token_vault_key();
        vault.set(orphan.clone(), secret("orphan")).unwrap();
        vault
            .set("UNRELATED_SECRET".to_string(), secret("keep"))
            .unwrap();

        assert!(sweep_orphaned_gmail_secrets(&lock, &mut vault, store).is_empty());
        assert!(vault.get(&orphan).is_none());
        assert_eq!(vault.get("UNRELATED_SECRET").unwrap().as_str(), "keep");
    }
}
