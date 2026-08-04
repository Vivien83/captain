//! Runtime credential lifecycle for native Gmail operations.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use captain_extensions::gmail_oauth::{
    refresh_google_tokens, verify_google_tokens, GmailTokenSet, GoogleDesktopClient,
};
use captain_extensions::vault::CredentialVault;
use captain_memory::gmail_accounts::{
    preserved_sync_cursor, GmailAccountRecord, GmailAccountStore, NewGmailAccount,
};
use captain_types::email::{GmailAccessProfile, GmailAccountAlias, GmailAccountStatus};
use tracing::warn;

use crate::gmail_persistence::{
    acquire_gmail_persistence_lock, new_gmail_token_vault_key, persist_gmail_grant,
    sweep_orphaned_gmail_secrets,
};

const MAX_REFRESH_RETRIES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GmailRequiredAccess {
    Read,
    Send,
    Modify,
}

impl GmailRequiredAccess {
    fn is_allowed_by(self, profile: GmailAccessProfile) -> bool {
        match self {
            Self::Read => profile.can_read(),
            Self::Send => profile.can_send(),
            Self::Modify => profile.can_modify(),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Send => "send",
            Self::Modify => "modify",
        }
    }
}

pub(crate) struct GmailCredentialContext {
    pub record: GmailAccountRecord,
    pub tokens: GmailTokenSet,
}

#[derive(Clone)]
pub(crate) struct GmailCredentialManager {
    home: PathBuf,
    store: GmailAccountStore,
}

impl GmailCredentialManager {
    pub(crate) fn new(home: PathBuf, store: GmailAccountStore) -> Self {
        Self { home, store }
    }

    /// Resolve one ready account and refresh its token without holding the
    /// cross-process Gmail lock over network I/O.
    pub(crate) async fn authorize(
        &self,
        alias: Option<GmailAccountAlias>,
        required: GmailRequiredAccess,
    ) -> Result<GmailCredentialContext, String> {
        for _ in 0..MAX_REFRESH_RETRIES {
            let home = self.home.clone();
            let store = self.store.clone();
            let requested_alias = alias.clone();
            let snapshot = tokio::task::spawn_blocking(move || {
                load_snapshot(&home, &store, requested_alias.as_ref(), required)
            })
            .await
            .map_err(|_| "Gmail credential worker stopped unexpectedly".to_string())??;

            if !snapshot.tokens.needs_refresh(now_unix_seconds()?) {
                return Ok(GmailCredentialContext {
                    record: snapshot.record,
                    tokens: snapshot.tokens,
                });
            }

            let refreshed = match refresh_google_tokens(
                &snapshot.client,
                &snapshot.tokens,
                snapshot.record.summary.access_profile,
            )
            .await
            {
                Ok(tokens) => tokens,
                Err(error) => {
                    self.record_failure_if_unchanged(&snapshot.record, "token_refresh_failed")
                        .await;
                    return Err(format!(
                        "Gmail token refresh failed for '{}': {error}",
                        snapshot.record.summary.alias
                    ));
                }
            };

            let identity = match verify_google_tokens(
                &refreshed,
                snapshot.record.summary.access_profile,
            )
            .await
            {
                Ok(identity) => identity,
                Err(error) => {
                    self.record_failure_if_unchanged(&snapshot.record, "live_check_failed")
                        .await;
                    return Err(format!(
                        "Gmail identity verification failed for '{}': {error}",
                        snapshot.record.summary.alias
                    ));
                }
            };
            if !identity
                .email_address
                .eq_ignore_ascii_case(&snapshot.record.summary.email_address)
            {
                self.mark_reauth_if_unchanged(&snapshot.record, "identity_mismatch")
                    .await;
                return Err(format!(
                    "Google returned a different identity for Gmail account '{}'; reconnect it",
                    snapshot.record.summary.alias
                ));
            }

            let secret = refreshed
                .to_secret_json()
                .map_err(|error| format!("Could not encode refreshed Gmail tokens: {error}"))?;
            let home = self.home.clone();
            let store = self.store.clone();
            let original = snapshot.record;
            let scopes = refreshed.granted_scopes().to_vec();
            let history_id = identity.history_id;
            let persisted = tokio::task::spawn_blocking(move || {
                persist_refreshed_tokens(&home, &store, &original, scopes, history_id, secret)
            })
            .await
            .map_err(|_| "Gmail persistence worker stopped unexpectedly".to_string())??;

            match persisted {
                PersistRefreshOutcome::Persisted(record) => {
                    return Ok(GmailCredentialContext {
                        record,
                        tokens: refreshed,
                    });
                }
                PersistRefreshOutcome::ConcurrentChange => continue,
            }
        }

        Err(
            "Gmail account changed repeatedly during token refresh; retry the operation"
                .to_string(),
        )
    }

    pub(crate) async fn record_success(&self, record: &GmailAccountRecord) {
        let home = self.home.clone();
        let store = self.store.clone();
        let snapshot = record.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _lock = acquire_gmail_persistence_lock(&home).map_err(|error| error.to_string())?;
            let Some(latest) = store
                .get(&snapshot.summary.alias)
                .map_err(|error| error.to_string())?
            else {
                return Ok(());
            };
            if account_matches_snapshot(&latest, &snapshot) {
                store
                    .record_check_success(&snapshot.summary.alias)
                    .map_err(|error| error.to_string())?;
            }
            Ok::<_, String>(())
        })
        .await;
        if !matches!(result, Ok(Ok(()))) {
            warn!(alias = %record.summary.alias, "Gmail success bookkeeping failed");
        }
    }

    pub(crate) async fn record_api_failure(
        &self,
        record: &GmailAccountRecord,
        error: &captain_extensions::gmail_api::GmailApiError,
    ) {
        if error.requires_reauthorization() {
            self.mark_reauth_if_unchanged(record, error.code()).await;
        } else {
            self.record_failure_if_unchanged(record, error.code()).await;
        }
    }

    async fn mark_reauth_if_unchanged(&self, record: &GmailAccountRecord, code: &'static str) {
        let home = self.home.clone();
        let store = self.store.clone();
        let snapshot = record.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _lock = acquire_gmail_persistence_lock(&home).map_err(|error| error.to_string())?;
            mark_reauth_if_current(&store, &snapshot, code)
        })
        .await;
        if !matches!(result, Ok(Ok(()))) {
            warn!(alias = %record.summary.alias, code, "Gmail reauthorization status update failed");
        }
    }

    async fn record_failure_if_unchanged(&self, record: &GmailAccountRecord, code: &'static str) {
        let home = self.home.clone();
        let store = self.store.clone();
        let snapshot = record.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _lock = acquire_gmail_persistence_lock(&home).map_err(|error| error.to_string())?;
            let Some(latest) = store
                .get(&snapshot.summary.alias)
                .map_err(|error| error.to_string())?
            else {
                return Ok(());
            };
            if account_matches_snapshot(&latest, &snapshot) {
                store
                    .record_check_failure(&snapshot.summary.alias, code)
                    .map_err(|error| error.to_string())?;
            }
            Ok::<_, String>(())
        })
        .await;
        if !matches!(result, Ok(Ok(()))) {
            warn!(alias = %record.summary.alias, code, "Gmail failure bookkeeping failed");
        }
    }
}

struct CredentialSnapshot {
    record: GmailAccountRecord,
    client: GoogleDesktopClient,
    tokens: GmailTokenSet,
}

enum PersistRefreshOutcome {
    Persisted(GmailAccountRecord),
    ConcurrentChange,
}

fn load_snapshot(
    home: &Path,
    store: &GmailAccountStore,
    alias: Option<&GmailAccountAlias>,
    required: GmailRequiredAccess,
) -> Result<CredentialSnapshot, String> {
    let _lock = acquire_gmail_persistence_lock(home).map_err(|error| error.to_string())?;
    let record = select_account(store, alias)?;
    require_ready_access(&record, required)?;

    if !home.join("vault.enc").exists() {
        mark_reauth_if_current(store, &record, "credential_invalid")?;
        return Err(format!(
            "Captain credential vault is missing for Gmail account '{}'; reconnect it",
            record.summary.alias
        ));
    }
    let mut vault = open_vault(home)?;
    let warnings = sweep_orphaned_gmail_secrets(&_lock, &mut vault, store);
    if !warnings.is_empty() {
        warn!(alias = %record.summary.alias, ?warnings, "Gmail credential cleanup remains pending");
    }
    let client = match vault
        .get(&record.client_vault_key)
        .ok_or_else(|| "stored OAuth client is missing".to_string())
        .and_then(|secret| {
            GoogleDesktopClient::from_secret_json(&secret).map_err(|error| error.to_string())
        }) {
        Ok(client) => client,
        Err(reason) => {
            mark_reauth_if_current(store, &record, "credential_invalid")?;
            return Err(format!(
                "Gmail credentials for '{}' are invalid ({reason}); reconnect the account",
                record.summary.alias
            ));
        }
    };
    let tokens = match vault
        .get(&record.token_vault_key)
        .ok_or_else(|| "stored OAuth tokens are missing".to_string())
        .and_then(|secret| {
            GmailTokenSet::from_secret_json(&secret).map_err(|error| error.to_string())
        }) {
        Ok(tokens) => tokens,
        Err(reason) => {
            mark_reauth_if_current(store, &record, "credential_invalid")?;
            return Err(format!(
                "Gmail credentials for '{}' are invalid ({reason}); reconnect the account",
                record.summary.alias
            ));
        }
    };
    let required_scope = record.summary.access_profile.required_gmail_scope();
    if !record
        .summary
        .granted_scopes
        .iter()
        .any(|scope| scope == required_scope)
        || !tokens
            .granted_scopes()
            .iter()
            .any(|scope| scope == required_scope)
    {
        mark_reauth_if_current(store, &record, "scope_mismatch")?;
        return Err(format!(
            "Gmail account '{}' is missing its required OAuth scope; reconnect it",
            record.summary.alias
        ));
    }

    Ok(CredentialSnapshot {
        record,
        client,
        tokens,
    })
}

fn persist_refreshed_tokens(
    home: &Path,
    store: &GmailAccountStore,
    original: &GmailAccountRecord,
    granted_scopes: Vec<String>,
    history_id: Option<String>,
    token_secret: zeroize::Zeroizing<String>,
) -> Result<PersistRefreshOutcome, String> {
    let lock = acquire_gmail_persistence_lock(home).map_err(|error| error.to_string())?;
    let Some(latest) = store
        .get(&original.summary.alias)
        .map_err(|error| error.to_string())?
    else {
        return Ok(PersistRefreshOutcome::ConcurrentChange);
    };
    if !account_matches_snapshot(&latest, original) {
        return Ok(PersistRefreshOutcome::ConcurrentChange);
    }

    let mut vault = open_vault(home)?;
    let account = NewGmailAccount {
        alias: latest.summary.alias.clone(),
        email_address: latest.summary.email_address.clone(),
        access_profile: latest.summary.access_profile,
        granted_scopes,
        token_vault_key: new_gmail_token_vault_key(),
        client_vault_key: latest.client_vault_key.clone(),
        history_id: preserved_sync_cursor(latest.summary.history_id.clone(), history_id),
        make_default: latest.summary.is_default,
    };
    let outcome = persist_gmail_grant(&lock, &mut vault, store, account, token_secret, None)
        .map_err(|error| error.to_string())?;
    if !outcome.cleanup_warnings.is_empty() {
        warn!(alias = %outcome.record.summary.alias, warnings = ?outcome.cleanup_warnings, "Gmail credential cleanup remains pending");
    }
    Ok(PersistRefreshOutcome::Persisted(outcome.record))
}

fn select_account(
    store: &GmailAccountStore,
    alias: Option<&GmailAccountAlias>,
) -> Result<GmailAccountRecord, String> {
    if let Some(alias) = alias {
        return store
            .get(alias)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Gmail account '{alias}' was not found"));
    }
    store
        .list()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|record| record.summary.is_default)
        .ok_or_else(|| {
            "No default Gmail account is connected; run `captain email connect gmail`".to_string()
        })
}

fn require_ready_access(
    record: &GmailAccountRecord,
    required: GmailRequiredAccess,
) -> Result<(), String> {
    match record.summary.status {
        GmailAccountStatus::Ready if record.summary.enabled => {}
        GmailAccountStatus::ReauthRequired => {
            return Err(format!(
                "Gmail account '{}' requires reauthorization; run `captain email connect gmail --alias {}`",
                record.summary.alias, record.summary.alias
            ));
        }
        _ => {
            return Err(format!(
                "Gmail account '{}' is disabled",
                record.summary.alias
            ));
        }
    }
    if !required.is_allowed_by(record.summary.access_profile) {
        return Err(format!(
            "Gmail account '{}' uses the '{}' profile, which cannot {}; reconnect it with the appropriate access profile",
            record.summary.alias, record.summary.access_profile, required.label()
        ));
    }
    Ok(())
}

fn open_vault(home: &Path) -> Result<CredentialVault, String> {
    let mut vault = CredentialVault::new(home.join("vault.enc"));
    if !vault.exists() {
        return Err(
            "Captain credential vault is missing; reconnect Gmail after initializing the vault"
                .to_string(),
        );
    }
    vault
        .unlock()
        .map_err(|error| format!("Could not unlock Captain credential vault: {error}"))?;
    Ok(vault)
}

fn mark_reauth_if_current(
    store: &GmailAccountStore,
    snapshot: &GmailAccountRecord,
    code: &'static str,
) -> Result<(), String> {
    let Some(latest) = store
        .get(&snapshot.summary.alias)
        .map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    if account_matches_snapshot(&latest, snapshot) {
        store
            .set_status(
                &snapshot.summary.alias,
                GmailAccountStatus::ReauthRequired,
                Some(code),
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn account_matches_snapshot(latest: &GmailAccountRecord, snapshot: &GmailAccountRecord) -> bool {
    latest.token_vault_key == snapshot.token_vault_key
        && latest.client_vault_key == snapshot.client_vault_key
        && latest
            .summary
            .email_address
            .eq_ignore_ascii_case(&snapshot.summary.email_address)
        && latest.summary.access_profile == snapshot.summary.access_profile
        && latest.summary.status == snapshot.summary.status
        && latest.summary.enabled == snapshot.summary.enabled
}

fn now_unix_seconds() -> Result<i64, String> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock is invalid: {error}"))?
        .as_secs();
    i64::try_from(seconds).map_err(|_| "System clock exceeds supported range".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::email::GmailAccountSummary;
    use chrono::Utc;

    fn record(profile: GmailAccessProfile, status: GmailAccountStatus) -> GmailAccountRecord {
        GmailAccountRecord {
            summary: GmailAccountSummary {
                alias: GmailAccountAlias::parse("work").unwrap(),
                email_address: "person@example.com".to_string(),
                access_profile: profile,
                granted_scopes: profile
                    .required_scopes()
                    .iter()
                    .map(|scope| (*scope).to_string())
                    .collect(),
                history_id: profile.can_read().then(|| "123".to_string()),
                status,
                enabled: status != GmailAccountStatus::Disabled,
                is_default: true,
                last_sync_at: None,
                last_error_code: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            token_vault_key: "CAPTAIN_GMAIL_TOKEN_A".to_string(),
            client_vault_key: "CAPTAIN_GMAIL_CLIENT_A".to_string(),
        }
    }

    #[test]
    fn access_profiles_enforce_least_privilege() {
        let send = record(GmailAccessProfile::Send, GmailAccountStatus::Ready);
        assert!(require_ready_access(&send, GmailRequiredAccess::Send).is_ok());
        assert!(require_ready_access(&send, GmailRequiredAccess::Read).is_err());
        assert!(require_ready_access(&send, GmailRequiredAccess::Modify).is_err());

        let read = record(GmailAccessProfile::Read, GmailAccountStatus::Ready);
        assert!(require_ready_access(&read, GmailRequiredAccess::Read).is_ok());
        assert!(require_ready_access(&read, GmailRequiredAccess::Send).is_err());

        let assistant = record(GmailAccessProfile::Assistant, GmailAccountStatus::Ready);
        assert!(require_ready_access(&assistant, GmailRequiredAccess::Read).is_ok());
        assert!(require_ready_access(&assistant, GmailRequiredAccess::Send).is_ok());
        assert!(require_ready_access(&assistant, GmailRequiredAccess::Modify).is_ok());
    }

    #[test]
    fn oauth_refresh_never_skips_the_persisted_mailbox_cursor() {
        let readable = record(GmailAccessProfile::Assistant, GmailAccountStatus::Ready);
        assert_eq!(
            preserved_sync_cursor(readable.summary.history_id.clone(), Some("999".to_string()))
                .as_deref(),
            Some("123")
        );

        let send_only = record(GmailAccessProfile::Send, GmailAccountStatus::Ready);
        assert_eq!(
            preserved_sync_cursor(
                send_only.summary.history_id.clone(),
                Some("999".to_string())
            )
            .as_deref(),
            Some("999")
        );
    }

    #[test]
    fn unavailable_accounts_fail_before_credentials_are_read() {
        let reauth = record(
            GmailAccessProfile::Assistant,
            GmailAccountStatus::ReauthRequired,
        );
        assert!(require_ready_access(&reauth, GmailRequiredAccess::Read)
            .unwrap_err()
            .contains("reauthorization"));
        let disabled = record(GmailAccessProfile::Assistant, GmailAccountStatus::Disabled);
        assert!(require_ready_access(&disabled, GmailRequiredAccess::Read)
            .unwrap_err()
            .contains("disabled"));
    }

    #[test]
    fn optimistic_snapshot_detects_reconnects() {
        let original = record(GmailAccessProfile::Assistant, GmailAccountStatus::Ready);
        assert!(account_matches_snapshot(&original, &original));
        let mut replaced = original.clone();
        replaced.token_vault_key = "CAPTAIN_GMAIL_TOKEN_B".to_string();
        assert!(!account_matches_snapshot(&replaced, &original));

        let mut disabled = original.clone();
        disabled.summary.status = GmailAccountStatus::Disabled;
        disabled.summary.enabled = false;
        assert!(!account_matches_snapshot(&disabled, &original));
    }
}
