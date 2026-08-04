//! Durable, public-safe Gmail account registry.

use captain_types::email::{
    GmailAccessProfile, GmailAccountAlias, GmailAccountStatus, GmailAccountSummary,
};
use captain_types::error::{CaptainError, CaptainResult};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

const ACCOUNT_SELECT: &str = "SELECT alias, email_address, access_profile, granted_scopes_json,
            token_vault_key, client_vault_key, history_id, status,
            enabled, is_default, last_sync_at, last_error_code,
            created_at, updated_at
     FROM gmail_accounts";

/// Internal registry row. Secret values remain in the encrypted vault; this
/// type only carries opaque vault entry names and is intentionally not serializable.
#[derive(Clone, PartialEq, Eq)]
pub struct GmailAccountRecord {
    pub summary: GmailAccountSummary,
    pub token_vault_key: String,
    pub client_vault_key: String,
}

/// Validated data produced by a completed OAuth flow.
pub struct NewGmailAccount {
    pub alias: GmailAccountAlias,
    pub email_address: String,
    pub access_profile: GmailAccessProfile,
    pub granted_scopes: Vec<String>,
    pub token_vault_key: String,
    pub client_vault_key: String,
    pub history_id: Option<String>,
    pub make_default: bool,
}

/// SQLite-backed Gmail account metadata.
#[derive(Clone)]
pub struct GmailAccountStore {
    conn: Arc<Mutex<Connection>>,
}

impl GmailAccountStore {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Insert or replace one alias after enforcing identity and scope invariants.
    pub fn upsert(&self, input: NewGmailAccount) -> CaptainResult<GmailAccountRecord> {
        validate_new_account(&input)?;
        let email_address = normalize_email(&input.email_address)?;
        let scopes_json = serde_json::to_string(&canonical_scopes(&input.granted_scopes))
            .map_err(|error| CaptainError::Serialization(error.to_string()))?;
        let now = Utc::now().timestamp_millis();
        let mut conn = self
            .conn
            .lock()
            .map_err(|error| CaptainError::Internal(error.to_string()))?;
        let transaction = conn
            .transaction()
            .map_err(|error| CaptainError::Memory(error.to_string()))?;

        let alias_for_email: Option<String> = transaction
            .query_row(
                "SELECT alias FROM gmail_accounts
                 WHERE email_address = ?1 COLLATE NOCASE AND alias <> ?2",
                rusqlite::params![email_address, input.alias.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        if let Some(alias) = alias_for_email {
            return Err(CaptainError::Memory(format!(
                "Gmail address is already connected as '{alias}'"
            )));
        }

        let existing_default: Option<bool> = transaction
            .query_row(
                "SELECT is_default FROM gmail_accounts WHERE alias = ?1",
                [input.alias.as_str()],
                |row| row.get::<_, i64>(0).map(|value| value != 0),
            )
            .optional()
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        let account_count: i64 = transaction
            .query_row("SELECT COUNT(*) FROM gmail_accounts", [], |row| row.get(0))
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        let is_default =
            input.make_default || account_count == 0 || existing_default.is_some_and(|value| value);
        if is_default {
            transaction
                .execute("UPDATE gmail_accounts SET is_default = 0", [])
                .map_err(|error| CaptainError::Memory(error.to_string()))?;
        }

        transaction
            .execute(
                "INSERT INTO gmail_accounts (
                     alias, email_address, access_profile, granted_scopes_json,
                     token_vault_key, client_vault_key, history_id, status,
                     enabled, is_default, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'ready', 1, ?8, ?9, ?9)
                 ON CONFLICT(alias) DO UPDATE SET
                     email_address = excluded.email_address,
                     access_profile = excluded.access_profile,
                     granted_scopes_json = excluded.granted_scopes_json,
                     token_vault_key = excluded.token_vault_key,
                     client_vault_key = excluded.client_vault_key,
                     history_id = excluded.history_id,
                     status = 'ready',
                     enabled = 1,
                     is_default = excluded.is_default,
                     last_error_code = NULL,
                     updated_at = excluded.updated_at",
                rusqlite::params![
                    input.alias.as_str(),
                    email_address,
                    input.access_profile.to_string(),
                    scopes_json,
                    input.token_vault_key,
                    input.client_vault_key,
                    input.history_id,
                    i64::from(is_default),
                    now,
                ],
            )
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        transaction
            .commit()
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        drop(conn);

        self.get(&input.alias)?.ok_or_else(|| {
            CaptainError::Memory("Gmail account disappeared after commit".to_string())
        })
    }

    pub fn get(&self, alias: &GmailAccountAlias) -> CaptainResult<Option<GmailAccountRecord>> {
        let conn = self
            .conn
            .lock()
            .map_err(|error| CaptainError::Internal(error.to_string()))?;
        conn.query_row(
            &format!("{ACCOUNT_SELECT} WHERE alias = ?1"),
            [alias.as_str()],
            row_to_record,
        )
        .optional()
        .map_err(|error| CaptainError::Memory(error.to_string()))
    }

    pub fn list(&self) -> CaptainResult<Vec<GmailAccountRecord>> {
        let conn = self
            .conn
            .lock()
            .map_err(|error| CaptainError::Internal(error.to_string()))?;
        let mut statement = conn
            .prepare(&format!(
                "{ACCOUNT_SELECT} ORDER BY is_default DESC, alias ASC"
            ))
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        let rows = statement
            .query_map([], row_to_record)
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| CaptainError::Memory(error.to_string()))
    }

    pub fn set_default(&self, alias: &GmailAccountAlias) -> CaptainResult<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|error| CaptainError::Internal(error.to_string()))?;
        let transaction = conn
            .transaction()
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM gmail_accounts WHERE alias = ?1)",
                [alias.as_str()],
                |row| row.get(0),
            )
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        if !exists {
            return Err(CaptainError::Memory(format!(
                "Gmail account '{}' was not found",
                alias
            )));
        }
        transaction
            .execute("UPDATE gmail_accounts SET is_default = 0", [])
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        transaction
            .execute(
                "UPDATE gmail_accounts
                 SET is_default = 1, updated_at = ?2 WHERE alias = ?1",
                rusqlite::params![alias.as_str(), Utc::now().timestamp_millis()],
            )
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        transaction
            .commit()
            .map_err(|error| CaptainError::Memory(error.to_string()))
    }

    pub fn set_status(
        &self,
        alias: &GmailAccountAlias,
        status: GmailAccountStatus,
        error_code: Option<&str>,
    ) -> CaptainResult<()> {
        validate_error_code(error_code)?;
        let conn = self
            .conn
            .lock()
            .map_err(|error| CaptainError::Internal(error.to_string()))?;
        let changed = conn
            .execute(
                "UPDATE gmail_accounts
                 SET status = ?2, enabled = ?3, last_error_code = ?4,
                     updated_at = ?5
                 WHERE alias = ?1",
                rusqlite::params![
                    alias.as_str(),
                    status.to_string(),
                    i64::from(status != GmailAccountStatus::Disabled),
                    error_code,
                    Utc::now().timestamp_millis(),
                ],
            )
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        require_changed(changed, alias)
    }

    /// Record a successful mailbox synchronization. This is the only account
    /// operation allowed to advance the durable automation cursor.
    pub fn record_sync_success(
        &self,
        alias: &GmailAccountAlias,
        history_id: Option<&str>,
    ) -> CaptainResult<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|error| CaptainError::Internal(error.to_string()))?;
        let transaction = conn
            .transaction()
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        let profile: Option<String> = transaction
            .query_row(
                "SELECT access_profile FROM gmail_accounts WHERE alias = ?1",
                [alias.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        let Some(profile) = profile else {
            return Err(CaptainError::Memory(format!(
                "Gmail account '{}' was not found",
                alias
            )));
        };
        let profile = GmailAccessProfile::from_str(&profile).map_err(CaptainError::Memory)?;
        if profile.can_read() && history_id.is_none_or(str::is_empty) {
            return Err(CaptainError::Memory(
                "Readable Gmail accounts require a live history_id".to_string(),
            ));
        }
        let now = Utc::now().timestamp_millis();
        transaction
            .execute(
                "UPDATE gmail_accounts
                 SET history_id = COALESCE(?2, history_id), status = 'ready',
                     enabled = 1, last_sync_at = ?3, last_error_code = NULL,
                     updated_at = ?3
                 WHERE alias = ?1",
                rusqlite::params![alias.as_str(), history_id, now],
            )
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        transaction
            .commit()
            .map_err(|error| CaptainError::Memory(error.to_string()))
    }

    /// Clear a transient live-check error without changing the mailbox cursor
    /// or the timestamp of the last completed automation synchronization.
    pub fn record_check_success(&self, alias: &GmailAccountAlias) -> CaptainResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|error| CaptainError::Internal(error.to_string()))?;
        let changed = conn
            .execute(
                "UPDATE gmail_accounts
                 SET last_error_code = NULL, updated_at = ?2
                 WHERE alias = ?1 AND status = 'ready' AND enabled = 1",
                rusqlite::params![alias.as_str(), Utc::now().timestamp_millis()],
            )
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        require_changed(changed, alias)
    }

    /// Preserve the current readiness state while publishing a bounded code
    /// for a failed live check. Transient network failures must not force an
    /// otherwise valid account into reauthorization.
    pub fn record_check_failure(
        &self,
        alias: &GmailAccountAlias,
        error_code: &str,
    ) -> CaptainResult<()> {
        validate_error_code(Some(error_code))?;
        let conn = self
            .conn
            .lock()
            .map_err(|error| CaptainError::Internal(error.to_string()))?;
        let changed = conn
            .execute(
                "UPDATE gmail_accounts
                 SET last_error_code = ?2, updated_at = ?3
                 WHERE alias = ?1",
                rusqlite::params![alias.as_str(), error_code, Utc::now().timestamp_millis()],
            )
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        require_changed(changed, alias)
    }

    pub fn delete(&self, alias: &GmailAccountAlias) -> CaptainResult<Option<GmailAccountRecord>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|error| CaptainError::Internal(error.to_string()))?;
        let transaction = conn
            .transaction()
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        let existing = transaction
            .query_row(
                &format!("{ACCOUNT_SELECT} WHERE alias = ?1"),
                [alias.as_str()],
                row_to_record,
            )
            .optional()
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        let Some(existing) = existing else {
            return Ok(None);
        };
        transaction
            .execute(
                "DELETE FROM gmail_accounts WHERE alias = ?1",
                [alias.as_str()],
            )
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        let default_count: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM gmail_accounts WHERE is_default = 1",
                [],
                |row| row.get(0),
            )
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        if default_count == 0 {
            transaction
                .execute(
                    "UPDATE gmail_accounts SET is_default = 1, updated_at = ?1
                     WHERE alias = (
                         SELECT alias FROM gmail_accounts ORDER BY alias ASC LIMIT 1
                     )",
                    [Utc::now().timestamp_millis()],
                )
                .map_err(|error| CaptainError::Memory(error.to_string()))?;
        }
        transaction
            .commit()
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        Ok(Some(existing))
    }
}

fn validate_new_account(input: &NewGmailAccount) -> CaptainResult<()> {
    normalize_email(&input.email_address)?;
    validate_vault_key(&input.token_vault_key)?;
    validate_vault_key(&input.client_vault_key)?;
    if input.token_vault_key == input.client_vault_key {
        return Err(CaptainError::Memory(
            "Gmail token and client vault references must differ".to_string(),
        ));
    }
    if !input
        .granted_scopes
        .iter()
        .any(|scope| scope == input.access_profile.required_gmail_scope())
    {
        return Err(CaptainError::Memory(format!(
            "Gmail OAuth grant is missing required scope {}",
            input.access_profile.required_gmail_scope()
        )));
    }
    if input.access_profile.can_read() && input.history_id.as_deref().is_none_or(str::is_empty) {
        return Err(CaptainError::Memory(
            "Readable Gmail accounts require an initial history_id".to_string(),
        ));
    }
    Ok(())
}

fn normalize_email(value: &str) -> CaptainResult<String> {
    let email = value.trim().to_ascii_lowercase();
    let valid = email.len() <= 320
        && !email.contains(['\n', '\r', ' '])
        && email
            .split_once('@')
            .is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.'));
    if !valid {
        return Err(CaptainError::Memory(
            "Google returned an invalid Gmail address".to_string(),
        ));
    }
    Ok(email)
}

fn validate_vault_key(value: &str) -> CaptainResult<()> {
    let valid = (1..=128).contains(&value.len())
        && value.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        });
    if valid {
        Ok(())
    } else {
        Err(CaptainError::Memory(
            "Gmail vault references must use env-var format".to_string(),
        ))
    }
}

fn validate_error_code(value: Option<&str>) -> CaptainResult<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let valid = !value.is_empty()
        && value.len() <= 64
        && value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '-' | '.')
        });
    if valid {
        Ok(())
    } else {
        Err(CaptainError::Memory(
            "Gmail error code must be a bounded machine-readable identifier".to_string(),
        ))
    }
}

fn canonical_scopes(scopes: &[String]) -> Vec<String> {
    let mut scopes = scopes.to_vec();
    scopes.sort();
    scopes.dedup();
    scopes
}

/// A live identity check may supply Gmail's current cursor, but only the sync
/// worker may advance an already persisted automation cursor.
pub fn preserved_sync_cursor(
    persisted: Option<String>,
    live_identity: Option<String>,
) -> Option<String> {
    persisted.or(live_identity)
}

fn row_to_record(row: &Row<'_>) -> rusqlite::Result<GmailAccountRecord> {
    let alias_text: String = row.get(0)?;
    let profile_text: String = row.get(2)?;
    let scopes_json: String = row.get(3)?;
    let status_text: String = row.get(7)?;
    let last_sync_at: Option<i64> = row.get(10)?;
    let created_at: i64 = row.get(12)?;
    let updated_at: i64 = row.get(13)?;
    Ok(GmailAccountRecord {
        summary: GmailAccountSummary {
            alias: GmailAccountAlias::parse(&alias_text)
                .map_err(|error| conversion_error(0, error))?,
            email_address: row.get(1)?,
            access_profile: GmailAccessProfile::from_str(&profile_text)
                .map_err(|error| conversion_error(2, error))?,
            granted_scopes: serde_json::from_str(&scopes_json)
                .map_err(|error| conversion_error(3, error))?,
            history_id: row.get(6)?,
            status: GmailAccountStatus::from_str(&status_text)
                .map_err(|error| conversion_error(7, error))?,
            enabled: row.get::<_, i64>(8)? != 0,
            is_default: row.get::<_, i64>(9)? != 0,
            last_sync_at: last_sync_at
                .map(timestamp_from_millis)
                .transpose()
                .map_err(|error| conversion_error(10, error))?,
            last_error_code: row.get(11)?,
            created_at: timestamp_from_millis(created_at)
                .map_err(|error| conversion_error(12, error))?,
            updated_at: timestamp_from_millis(updated_at)
                .map_err(|error| conversion_error(13, error))?,
        },
        token_vault_key: row.get(4)?,
        client_vault_key: row.get(5)?,
    })
}

fn timestamp_from_millis(value: i64) -> Result<DateTime<Utc>, String> {
    DateTime::<Utc>::from_timestamp_millis(value)
        .ok_or_else(|| format!("invalid timestamp {value}"))
}

fn conversion_error(column: usize, error: impl std::fmt::Display) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        )),
    )
}

fn require_changed(changed: usize, alias: &GmailAccountAlias) -> CaptainResult<()> {
    if changed == 0 {
        Err(CaptainError::Memory(format!(
            "Gmail account '{}' was not found",
            alias
        )))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn store() -> GmailAccountStore {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        GmailAccountStore::new(Arc::new(Mutex::new(conn)))
    }

    fn account(alias: &str, email: &str, profile: GmailAccessProfile) -> NewGmailAccount {
        NewGmailAccount {
            alias: GmailAccountAlias::parse(alias).unwrap(),
            email_address: email.to_string(),
            access_profile: profile,
            granted_scopes: profile
                .required_scopes()
                .iter()
                .map(ToString::to_string)
                .collect(),
            token_vault_key: format!("CAPTAIN_GMAIL_{}_TOKEN", alias.to_ascii_uppercase()),
            client_vault_key: format!("CAPTAIN_GMAIL_{}_CLIENT", alias.to_ascii_uppercase()),
            history_id: profile.can_read().then(|| "12345".to_string()),
            make_default: false,
        }
    }

    #[test]
    fn first_account_becomes_default_and_multiple_accounts_are_stable() {
        let store = store();
        let personal = store
            .upsert(account(
                "personal",
                "Person@Gmail.com",
                GmailAccessProfile::Assistant,
            ))
            .unwrap();
        let work = store
            .upsert(account(
                "work",
                "work@example.com",
                GmailAccessProfile::Send,
            ))
            .unwrap();

        assert!(personal.summary.is_default);
        assert!(!work.summary.is_default);
        assert_eq!(personal.summary.email_address, "person@gmail.com");
        assert_eq!(store.list().unwrap().len(), 2);
    }

    #[test]
    fn default_switch_and_delete_preserve_exactly_one_default() {
        let store = store();
        let personal = GmailAccountAlias::parse("personal").unwrap();
        let work = GmailAccountAlias::parse("work").unwrap();
        store
            .upsert(account(
                personal.as_str(),
                "person@gmail.com",
                GmailAccessProfile::Assistant,
            ))
            .unwrap();
        store
            .upsert(account(
                work.as_str(),
                "work@example.com",
                GmailAccessProfile::Read,
            ))
            .unwrap();

        store.set_default(&work).unwrap();
        assert!(store.get(&work).unwrap().unwrap().summary.is_default);
        store.delete(&work).unwrap();
        assert!(store.get(&personal).unwrap().unwrap().summary.is_default);
    }

    #[test]
    fn duplicate_email_and_insufficient_scope_fail_closed() {
        let store = store();
        store
            .upsert(account(
                "first",
                "same@gmail.com",
                GmailAccessProfile::Assistant,
            ))
            .unwrap();
        assert!(store
            .upsert(account(
                "second",
                "SAME@gmail.com",
                GmailAccessProfile::Assistant,
            ))
            .is_err());

        let mut insufficient = account("third", "third@gmail.com", GmailAccessProfile::Assistant);
        insufficient.granted_scopes = vec!["openid".to_string()];
        assert!(store.upsert(insufficient).is_err());
    }

    #[test]
    fn public_summary_never_serializes_vault_references() {
        let record = store()
            .upsert(account(
                "personal",
                "person@gmail.com",
                GmailAccessProfile::Assistant,
            ))
            .unwrap();
        let json = serde_json::to_string(&record.summary).unwrap();
        assert!(!json.contains("vault"));
        assert!(!json.contains("CAPTAIN_GMAIL"));
        assert!(!json.contains("token_vault_key"));
    }

    #[test]
    fn live_check_success_preserves_the_automation_cursor() {
        let store = store();
        let alias = GmailAccountAlias::parse("personal").unwrap();
        store
            .upsert(account(
                alias.as_str(),
                "person@gmail.com",
                GmailAccessProfile::Assistant,
            ))
            .unwrap();

        store
            .record_check_failure(&alias, "verification_failed")
            .unwrap();
        let failed = store.get(&alias).unwrap().unwrap().summary;
        assert_eq!(failed.status, GmailAccountStatus::Ready);
        assert_eq!(
            failed.last_error_code.as_deref(),
            Some("verification_failed")
        );
        assert!(failed.last_sync_at.is_none());

        store.record_check_success(&alias).unwrap();
        let checked = store.get(&alias).unwrap().unwrap().summary;
        assert_eq!(checked.history_id.as_deref(), Some("12345"));
        assert!(checked.last_error_code.is_none());
        assert!(checked.last_sync_at.is_none());

        store.record_sync_success(&alias, Some("456")).unwrap();
        let recovered = store.get(&alias).unwrap().unwrap().summary;
        assert_eq!(recovered.history_id.as_deref(), Some("456"));
        assert!(recovered.last_error_code.is_none());
        assert!(recovered.last_sync_at.is_some());
    }

    #[test]
    fn live_identity_cursor_only_initializes_an_absent_cursor() {
        assert_eq!(
            preserved_sync_cursor(Some("123".to_string()), Some("999".to_string())).as_deref(),
            Some("123")
        );
        assert_eq!(
            preserved_sync_cursor(None, Some("999".to_string())).as_deref(),
            Some("999")
        );
    }
}
