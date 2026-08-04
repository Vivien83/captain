use std::io::IsTerminal;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use captain_extensions::gmail_oauth::{GmailTokenSet, GoogleDesktopClient};
use captain_extensions::vault::CredentialVault;
use captain_kernel::gmail_persistence::{
    acquire_gmail_persistence_lock, new_gmail_client_vault_key, sweep_orphaned_gmail_secrets,
    GmailPersistenceLock,
};
use captain_memory::gmail_accounts::GmailAccountRecord;
use captain_memory::MemorySubstrate;
#[cfg(test)]
use captain_types::email::GmailAccountSummary;
use captain_types::email::{GmailAccessProfile, GmailAccountAlias, GmailAccountStatus};
use zeroize::Zeroizing;

use super::email_render::EmailAccountView;
use crate::prompt_input;

const MAX_GOOGLE_CLIENT_JSON_BYTES: u64 = 64 * 1024;

pub(super) struct ResolvedGoogleOAuthClient {
    pub(super) client: GoogleDesktopClient,
    pub(super) client_vault_key: String,
    pub(super) new_client_secret: Option<Zeroizing<String>>,
    pub(super) source: &'static str,
}

pub(super) struct EmailState {
    pub(super) vault: CredentialVault,
    pub(super) memory: MemorySubstrate,
    pub(super) lock: GmailPersistenceLock,
}

impl EmailState {
    pub(super) fn open(config_path: Option<&Path>) -> Result<(Self, Vec<String>), String> {
        let config = captain_kernel::config::load_config(config_path);
        std::fs::create_dir_all(&config.home_dir)
            .map_err(|error| format!("Could not create Captain home: {error}"))?;
        std::fs::create_dir_all(&config.data_dir)
            .map_err(|error| format!("Could not create Captain data directory: {error}"))?;
        let lock =
            acquire_gmail_persistence_lock(&config.home_dir).map_err(|error| error.to_string())?;

        let mut vault = CredentialVault::new(config.home_dir.join("vault.enc"));
        if vault.exists() {
            vault
                .unlock()
                .map_err(|error| format!("Could not unlock credential vault: {error}"))?;
        } else {
            vault.init().map_err(|error| {
                format!(
                    "Could not initialize credential vault: {error}. Headless systems may set CAPTAIN_VAULT_KEY."
                )
            })?;
        }

        let db_path = config
            .memory
            .sqlite_path
            .clone()
            .unwrap_or_else(|| config.data_dir.join("captain.db"));
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create Gmail database directory: {error}"))?;
        }
        let memory = MemorySubstrate::open(&db_path, config.memory.decay_rate)
            .map_err(|error| format!("Could not open Captain memory database: {error}"))?;
        let cleanup_warnings =
            sweep_orphaned_gmail_secrets(&lock, &mut vault, memory.gmail_accounts());
        Ok((
            Self {
                vault,
                memory,
                lock,
            },
            cleanup_warnings,
        ))
    }
}

pub(super) fn select_records(
    state: &EmailState,
    alias: Option<&str>,
    default_when_omitted: bool,
) -> Result<Vec<GmailAccountRecord>, String> {
    let store = state.memory.gmail_accounts();
    if let Some(alias) = alias {
        let alias = GmailAccountAlias::parse(alias).map_err(|error| error.to_string())?;
        return store
            .get(&alias)
            .map_err(|error| error.to_string())?
            .map(|record| vec![record])
            .ok_or_else(|| format!("Gmail account '{alias}' was not found"));
    }
    let records = store.list().map_err(|error| error.to_string())?;
    if default_when_omitted {
        let default = records
            .into_iter()
            .find(|record| record.summary.is_default)
            .ok_or_else(|| "No default Gmail account is connected".to_string())?;
        Ok(vec![default])
    } else {
        Ok(records)
    }
}

pub(super) fn reusable_client_record<'a>(
    records: &'a [GmailAccountRecord],
    requested_alias: Option<&GmailAccountAlias>,
    vault: &CredentialVault,
) -> Option<&'a GmailAccountRecord> {
    let preferred = requested_alias
        .and_then(|alias| records.iter().find(|record| &record.summary.alias == alias))
        .or_else(|| records.iter().find(|record| record.summary.is_default));
    preferred
        .filter(|record| valid_stored_client(vault, record))
        .or_else(|| {
            records
                .iter()
                .find(|record| valid_stored_client(vault, record))
        })
}

pub(super) fn stored_client_record_by_id<'a>(
    records: &'a [GmailAccountRecord],
    client_id: &str,
    vault: &CredentialVault,
) -> Option<&'a GmailAccountRecord> {
    records.iter().find(|record| {
        vault
            .get(&record.client_vault_key)
            .and_then(|secret| GoogleDesktopClient::from_secret_json(&secret).ok())
            .is_some_and(|client| client.client_id() == client_id)
    })
}

pub(super) fn resolve_google_oauth_client(
    records: &[GmailAccountRecord],
    requested_alias: Option<&GmailAccountAlias>,
    vault: &CredentialVault,
    explicit_client: Option<GoogleDesktopClient>,
    bundled_client: Option<GoogleDesktopClient>,
) -> Result<ResolvedGoogleOAuthClient, String> {
    if let Some(client) = explicit_client {
        let secret = client.to_secret_json().map_err(|error| error.to_string())?;
        return Ok(ResolvedGoogleOAuthClient {
            client,
            client_vault_key: new_gmail_client_vault_key(),
            new_client_secret: Some(secret),
            source: "operator-supplied Google Desktop client",
        });
    }

    if let Some(record) = requested_alias.and_then(|alias| {
        records
            .iter()
            .find(|record| &record.summary.alias == alias)
            .filter(|record| valid_stored_client(vault, record))
    }) {
        return Ok(ResolvedGoogleOAuthClient {
            client: load_client(vault, record)?,
            client_vault_key: record.client_vault_key.clone(),
            new_client_secret: None,
            source: "client already bound to this account",
        });
    }

    if let Some(client) = bundled_client {
        if let Some(record) = stored_client_record_by_id(records, client.client_id(), vault) {
            return Ok(ResolvedGoogleOAuthClient {
                client,
                client_vault_key: record.client_vault_key.clone(),
                new_client_secret: None,
                source: "Captain official Google OAuth client",
            });
        }
        let secret = client.to_secret_json().map_err(|error| error.to_string())?;
        return Ok(ResolvedGoogleOAuthClient {
            client,
            client_vault_key: new_gmail_client_vault_key(),
            new_client_secret: Some(secret),
            source: "Captain official Google OAuth client",
        });
    }

    let record = reusable_client_record(records, requested_alias, vault).ok_or_else(|| {
        "This Captain build has no official Google OAuth client and no stored Desktop client. Retry with --client-json PATH, or connect an IMAP/SMTP account instead."
            .to_string()
    })?;
    Ok(ResolvedGoogleOAuthClient {
        client: load_client(vault, record)?,
        client_vault_key: record.client_vault_key.clone(),
        new_client_secret: None,
        source: "stored operator Google Desktop client",
    })
}

fn valid_stored_client(vault: &CredentialVault, record: &GmailAccountRecord) -> bool {
    vault
        .get(&record.client_vault_key)
        .and_then(|secret| GoogleDesktopClient::from_secret_json(&secret).ok())
        .is_some()
}

pub(super) fn select_connected_alias(
    requested: Option<GmailAccountAlias>,
    email: &str,
    records: &[GmailAccountRecord],
) -> Result<GmailAccountAlias, String> {
    if let Some(alias) = requested {
        if let Some(existing) = records.iter().find(|record| record.summary.alias == alias) {
            if !existing.summary.email_address.eq_ignore_ascii_case(email) {
                return Err(format!(
                    "Gmail alias '{}' already belongs to {}; choose another alias or disconnect it first",
                    alias, existing.summary.email_address
                ));
            }
        }
        return Ok(alias);
    }
    if let Some(existing) = records
        .iter()
        .find(|record| record.summary.email_address.eq_ignore_ascii_case(email))
    {
        return Ok(existing.summary.alias.clone());
    }
    derive_available_alias(email, records)
}

fn derive_available_alias(
    email: &str,
    records: &[GmailAccountRecord],
) -> Result<GmailAccountAlias, String> {
    let local = email.split('@').next().unwrap_or("gmail");
    let mut base = String::new();
    let mut last_separator = false;
    for character in local.chars() {
        let mapped = if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
            character.to_ascii_lowercase()
        } else {
            '-'
        };
        if mapped == '-' && last_separator {
            continue;
        }
        base.push(mapped);
        last_separator = mapped == '-';
    }
    base = base
        .trim_matches(|character: char| !character.is_ascii_alphanumeric())
        .to_string();
    if base.is_empty() {
        base = "gmail".to_string();
    }
    base.truncate(GmailAccountAlias::MAX_LEN);
    let occupied = |candidate: &str| {
        records
            .iter()
            .any(|record| record.summary.alias.as_str() == candidate)
    };
    if !occupied(&base) {
        return GmailAccountAlias::parse(&base).map_err(|error| error.to_string());
    }
    for suffix in 2..=9_999_u16 {
        let suffix = format!("-{suffix}");
        let keep = GmailAccountAlias::MAX_LEN.saturating_sub(suffix.len());
        let mut candidate = base.chars().take(keep).collect::<String>();
        candidate.push_str(&suffix);
        if !occupied(&candidate) {
            return GmailAccountAlias::parse(&candidate).map_err(|error| error.to_string());
        }
    }
    Err("Could not derive an unused Gmail account alias; pass --alias explicitly".to_string())
}

pub(super) fn require_unchanged_account(
    state: &EmailState,
    alias: &GmailAccountAlias,
    token_key: &str,
    client_key: &str,
    email: &str,
    profile: GmailAccessProfile,
) -> Result<GmailAccountRecord, String> {
    let latest = state
        .memory
        .gmail_accounts()
        .get(alias)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Gmail account '{alias}' changed during the live check; retry"))?;
    let unchanged = account_matches_snapshot(&latest, token_key, client_key, email, profile);
    if unchanged {
        Ok(latest)
    } else {
        Err(format!(
            "Gmail account '{alias}' changed during the live check; retry without overwriting the newer grant"
        ))
    }
}

fn account_matches_snapshot(
    latest: &GmailAccountRecord,
    token_key: &str,
    client_key: &str,
    email: &str,
    profile: GmailAccessProfile,
) -> bool {
    latest.token_vault_key == token_key
        && latest.client_vault_key == client_key
        && latest.summary.email_address.eq_ignore_ascii_case(email)
        && latest.summary.access_profile == profile
}

pub(super) fn account_view(
    state: &mut EmailState,
    mut record: GmailAccountRecord,
) -> Result<EmailAccountView, String> {
    let client_valid = load_client(&state.vault, &record).is_ok();
    let tokens = load_tokens(&state.vault, &record);
    let token_valid = tokens.as_ref().is_ok_and(|tokens| {
        tokens
            .granted_scopes()
            .iter()
            .any(|scope| scope == record.summary.access_profile.required_gmail_scope())
    });
    let credential_status = if client_valid && token_valid {
        "ready"
    } else if state.vault.get(&record.client_vault_key).is_none()
        || state.vault.get(&record.token_vault_key).is_none()
    {
        "missing"
    } else {
        "invalid"
    };
    if credential_status != "ready" {
        state
            .memory
            .gmail_accounts()
            .set_status(
                &record.summary.alias,
                GmailAccountStatus::ReauthRequired,
                Some("credential_invalid"),
            )
            .map_err(|error| error.to_string())?;
        record.summary.status = GmailAccountStatus::ReauthRequired;
        record.summary.last_error_code = Some("credential_invalid".to_string());
    }
    let token_expires_at = tokens.as_ref().ok().map(|tokens| tokens.expires_at());
    let token_refresh_due = token_expires_at
        .is_some_and(|expiry| expiry <= now_unix_seconds().unwrap_or(i64::MAX).saturating_add(60));
    Ok(EmailAccountView {
        provider: "gmail",
        summary: record.summary,
        credential_status,
        token_expires_at,
        token_refresh_due,
    })
}

pub(super) fn load_client(
    vault: &CredentialVault,
    record: &GmailAccountRecord,
) -> Result<GoogleDesktopClient, String> {
    let secret = vault
        .get(&record.client_vault_key)
        .ok_or_else(|| "Stored Gmail OAuth client is missing; reconnect the account".to_string())?;
    GoogleDesktopClient::from_secret_json(&secret).map_err(|error| error.to_string())
}

pub(super) fn load_tokens(
    vault: &CredentialVault,
    record: &GmailAccountRecord,
) -> Result<GmailTokenSet, String> {
    let secret = vault
        .get(&record.token_vault_key)
        .ok_or_else(|| "Stored Gmail tokens are missing; reconnect the account".to_string())?;
    GmailTokenSet::from_secret_json(&secret).map_err(|error| error.to_string())
}

pub(super) fn mark_reauth(state: &EmailState, alias: &GmailAccountAlias, code: &str) {
    let _ = state.memory.gmail_accounts().set_status(
        alias,
        GmailAccountStatus::ReauthRequired,
        Some(code),
    );
}

pub(super) fn confirm_revocation(record: &GmailAccountRecord, yes: bool) -> Result<(), String> {
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err("Grant revocation requires --yes in non-interactive mode".to_string());
    }
    let answer = prompt_input(&format!(
        "Revoke Google consent for {}? This can affect other scopes granted to the same Google OAuth project. [y/N]: ",
        record.summary.email_address
    ));
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err("Revocation cancelled; local account was not changed".to_string())
    }
}

pub(super) fn read_google_client_json(path: &Path) -> Result<Zeroizing<String>, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err("Google OAuth client JSON must be a regular, non-symlink file".to_string());
    }
    if metadata.len() > MAX_GOOGLE_CLIENT_JSON_BYTES {
        return Err(format!(
            "Google OAuth client JSON exceeds {MAX_GOOGLE_CLIENT_JSON_BYTES} bytes"
        ));
    }
    std::fs::read_to_string(path)
        .map(Zeroizing::new)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))
}

pub(super) fn oauth_runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("Could not start Gmail OAuth runtime: {error}"))
}

pub(super) fn now_unix_seconds() -> Result<i64, String> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock is invalid: {error}"))?
        .as_secs();
    i64::try_from(seconds).map_err(|_| "System clock exceeds supported range".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(alias: &str, email: &str) -> GmailAccountRecord {
        GmailAccountRecord {
            summary: GmailAccountSummary {
                alias: GmailAccountAlias::parse(alias).unwrap(),
                email_address: email.to_string(),
                access_profile: GmailAccessProfile::Assistant,
                granted_scopes: GmailAccessProfile::Assistant
                    .required_scopes()
                    .iter()
                    .map(|scope| (*scope).to_string())
                    .collect(),
                history_id: Some("123".to_string()),
                status: GmailAccountStatus::Ready,
                enabled: true,
                is_default: false,
                last_sync_at: None,
                last_error_code: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            token_vault_key: "CAPTAIN_GMAIL_TOKEN_SECRETREF".to_string(),
            client_vault_key: "CAPTAIN_GMAIL_CLIENT_SECRETREF".to_string(),
        }
    }

    #[test]
    fn aliases_are_derived_deterministically_and_never_replace_another_account() {
        let records = vec![record("person.name", "other@example.com")];
        let alias = derive_available_alias("Person.Name+alerts@gmail.com", &records).unwrap();
        assert_eq!(alias.as_str(), "person.name-alerts");

        let collision = derive_available_alias("person.name@gmail.com", &records).unwrap();
        assert_eq!(collision.as_str(), "person.name-2");
    }

    #[test]
    fn reconnect_without_alias_reuses_the_existing_identity_alias() {
        let records = vec![record("work", "Person@Example.com")];
        let alias = select_connected_alias(None, "person@example.com", &records).unwrap();
        assert_eq!(alias.as_str(), "work");
    }

    #[test]
    fn explicit_alias_never_silently_switches_google_identity() {
        let records = vec![record("work", "first@example.com")];
        let error = select_connected_alias(
            Some(GmailAccountAlias::parse("work").unwrap()),
            "second@example.com",
            &records,
        )
        .unwrap_err();
        assert!(error.contains("already belongs"));
    }

    #[test]
    fn account_snapshot_detects_a_concurrent_token_replacement() {
        let current = record("work", "first@example.com");
        assert!(account_matches_snapshot(
            &current,
            "CAPTAIN_GMAIL_TOKEN_SECRETREF",
            "CAPTAIN_GMAIL_CLIENT_SECRETREF",
            "FIRST@example.com",
            GmailAccessProfile::Assistant,
        ));
        assert!(!account_matches_snapshot(
            &current,
            "CAPTAIN_GMAIL_TOKEN_NEWER",
            "CAPTAIN_GMAIL_CLIENT_SECRETREF",
            "first@example.com",
            GmailAccessProfile::Assistant,
        ));
    }

    #[test]
    fn google_client_file_reader_rejects_oversized_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("client.json");
        std::fs::write(&path, vec![b'x'; MAX_GOOGLE_CLIENT_JSON_BYTES as usize + 1]).unwrap();
        let error = read_google_client_json(&path).unwrap_err();
        assert!(error.contains("exceeds"));
    }

    fn public_client(name: &str) -> GoogleDesktopClient {
        GoogleDesktopClient::from_public_client(&format!("{name}.apps.googleusercontent.com"), None)
            .unwrap()
    }

    fn test_vault() -> (tempfile::TempDir, CredentialVault) {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = CredentialVault::new(dir.path().join("vault.enc"));
        vault.init_with_key(Zeroizing::new([7_u8; 32])).unwrap();
        (dir, vault)
    }

    #[test]
    fn oauth_client_resolution_prefers_explicit_operator_identity() {
        let (_dir, vault) = test_vault();
        let resolved = resolve_google_oauth_client(
            &[],
            None,
            &vault,
            Some(public_client("operator")),
            Some(public_client("captain")),
        )
        .unwrap();

        assert_eq!(
            resolved.client.client_id(),
            "operator.apps.googleusercontent.com"
        );
        assert!(resolved.new_client_secret.is_some());
        assert_eq!(resolved.source, "operator-supplied Google Desktop client");
    }

    #[test]
    fn oauth_client_resolution_preserves_existing_alias_binding() {
        let (_dir, mut vault) = test_vault();
        let mut existing = record("work", "work@example.com");
        existing.client_vault_key = "CAPTAIN_GMAIL_CLIENT_WORK".to_string();
        vault
            .set(
                existing.client_vault_key.clone(),
                public_client("existing").to_secret_json().unwrap(),
            )
            .unwrap();

        let resolved = resolve_google_oauth_client(
            &[existing],
            Some(&GmailAccountAlias::parse("work").unwrap()),
            &vault,
            None,
            Some(public_client("captain")),
        )
        .unwrap();

        assert_eq!(
            resolved.client.client_id(),
            "existing.apps.googleusercontent.com"
        );
        assert_eq!(resolved.client_vault_key, "CAPTAIN_GMAIL_CLIENT_WORK");
        assert!(resolved.new_client_secret.is_none());
    }

    #[test]
    fn oauth_client_resolution_reuses_matching_official_vault_entry() {
        let (_dir, mut vault) = test_vault();
        let mut existing = record("personal", "personal@example.com");
        existing.client_vault_key = "CAPTAIN_GMAIL_CLIENT_OFFICIAL".to_string();
        vault
            .set(
                existing.client_vault_key.clone(),
                public_client("captain").to_secret_json().unwrap(),
            )
            .unwrap();

        let resolved = resolve_google_oauth_client(
            &[existing],
            None,
            &vault,
            None,
            Some(public_client("captain")),
        )
        .unwrap();

        assert_eq!(resolved.client_vault_key, "CAPTAIN_GMAIL_CLIENT_OFFICIAL");
        assert!(resolved.new_client_secret.is_none());
        assert_eq!(resolved.source, "Captain official Google OAuth client");
    }

    #[test]
    fn oauth_client_resolution_fails_closed_without_any_identity() {
        let (_dir, vault) = test_vault();
        let error = match resolve_google_oauth_client(&[], None, &vault, None, None) {
            Ok(_) => panic!("missing OAuth identity must fail closed"),
            Err(error) => error,
        };
        assert!(error.contains("no official Google OAuth client"));
        assert!(error.contains("IMAP/SMTP"));
    }
}
