//! TOML persistence helpers for active channel configuration.

use crate::channel_registry::FieldType;
use captain_channels::email::email_allowlist_rule_is_valid;
use captain_types::config::{ChannelOverrides, EmailAccountConfig, EmailConfig};
use fs2::FileExt;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::time::{Duration, Instant};
use toml_edit::{DocumentMut, Item, Table};

const EMAIL_CONFIG_LOCK_NAME: &str = "email-config.lock";
const EMAIL_CONFIG_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const EMAIL_CONFIG_LOCK_RETRY: Duration = Duration::from_millis(25);

struct EmailConfigLock {
    _file: File,
}

#[derive(Serialize)]
struct PersistedEmailConfig<'a> {
    default_account: Option<&'a str>,
    accounts: &'a [EmailAccountConfig],
    overrides: &'a ChannelOverrides,
}

pub fn parse_email_channel_config(raw: &str) -> Result<Option<EmailConfig>, String> {
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let root = raw
        .parse::<toml::Value>()
        .map_err(|error| format!("Could not parse config.toml: {error}"))?;
    root.get("channels")
        .and_then(|channels| channels.get("email"))
        .cloned()
        .map(|value| value.try_into::<EmailConfig>())
        .transpose()
        .map_err(|error| format!("Invalid channels.email configuration: {error}"))
}

/// Return the stable, collision-resistant credential key for a named mailbox.
pub fn email_account_password_key(alias: &str) -> String {
    let readable = alias
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() {
                char::from(byte.to_ascii_uppercase())
            } else {
                '_'
            }
        })
        .collect::<String>();
    let hash = alias.bytes().fold(0x811c9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x01000193)
    });
    format!("CAPTAIN_EMAIL_{readable}_{hash:08X}_PASSWORD")
}

pub fn upsert_email_channel_account(
    config_path: &std::path::Path,
    account: EmailAccountConfig,
    make_default: bool,
) -> Result<EmailConfig, String> {
    upsert_email_channel_account_transactional(config_path, account, make_default, || Ok(()))
}

pub fn upsert_email_channel_account_transactional(
    config_path: &std::path::Path,
    account: EmailAccountConfig,
    make_default: bool,
    after_config_write: impl FnOnce() -> Result<(), String>,
) -> Result<EmailConfig, String> {
    mutate_email_channel_account_transactional(
        config_path,
        make_default,
        |_| Ok(account),
        |_| after_config_write(),
    )
    .map(|(config, _)| config)
}

pub(crate) fn mutate_email_channel_account_transactional(
    config_path: &std::path::Path,
    make_default: bool,
    build_account: impl FnOnce(Option<&EmailConfig>) -> Result<EmailAccountConfig, String>,
    after_config_write: impl FnOnce(&EmailAccountConfig) -> Result<(), String>,
) -> Result<(EmailConfig, EmailAccountConfig), String> {
    with_email_config_lock(config_path, || {
        let (config_existed, original_config) = read_config_or_empty(config_path)?;
        let current = parse_email_channel_config(&original_config)?;
        let account = build_account(current.as_ref())?;
        let configured =
            upsert_email_channel_account_unlocked(config_path, account.clone(), make_default)?;
        if let Err(error) = after_config_write(&account) {
            return match restore_email_config(config_path, config_existed, &original_config) {
                Ok(()) => Err(format!("{error}; config.toml was restored")),
                Err(rollback_error) => Err(format!(
                    "{error}; config rollback also failed: {rollback_error}"
                )),
            };
        }
        Ok((configured, account))
    })
}

pub(crate) fn with_email_config_lock<T>(
    config_path: &std::path::Path,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _lock = acquire_email_config_lock(config_path)?;
    operation()
}

fn upsert_email_channel_account_unlocked(
    config_path: &std::path::Path,
    account: EmailAccountConfig,
    make_default: bool,
) -> Result<EmailConfig, String> {
    let (_, raw) = read_config_or_empty(config_path)?;
    let mut document = if raw.trim().is_empty() {
        DocumentMut::new()
    } else {
        raw.parse::<DocumentMut>()
            .map_err(|error| format!("Could not parse {}: {error}", config_path.display()))?
    };
    let current = parse_email_channel_config(&raw)?.unwrap_or_default();
    let mut candidate = EmailConfig {
        accounts: current.effective_accounts(),
        default_account: current.effective_default_account(),
        overrides: current.overrides,
        ..EmailConfig::default()
    };
    if let Some(existing) = candidate
        .accounts
        .iter_mut()
        .find(|existing| existing.alias == account.alias)
    {
        *existing = account.clone();
    } else {
        candidate.accounts.push(account.clone());
    }
    if make_default {
        if !account.enabled {
            return Err(format!(
                "Invalid Email configuration: disabled account '{}' cannot be the default",
                account.alias
            ));
        }
        candidate.default_account = Some(account.alias.clone());
    }
    let default_is_enabled = candidate.default_account.as_deref().is_some_and(|alias| {
        candidate
            .accounts
            .iter()
            .any(|account| account.enabled && account.alias == alias)
    });
    if !default_is_enabled {
        candidate.default_account = candidate
            .accounts
            .iter()
            .find(|account| account.enabled)
            .map(|account| account.alias.clone());
    }
    for candidate_account in &candidate.accounts {
        let invalid_allowlist = candidate_account
            .allowed_senders
            .iter()
            .filter(|rule| !email_allowlist_rule_is_valid(rule))
            .cloned()
            .collect::<Vec<_>>();
        if !invalid_allowlist.is_empty() {
            return Err(format!(
                "Invalid Email allowlist rule(s) for account '{}': {}. Use an exact address, @domain, or *",
                candidate_account.alias,
                invalid_allowlist.join(", ")
            ));
        }
    }
    let errors = candidate.validation_errors();
    if !errors.is_empty() {
        return Err(format!(
            "Invalid Email configuration: {}",
            errors.join("; ")
        ));
    }

    let persisted = PersistedEmailConfig {
        default_account: candidate.default_account.as_deref(),
        accounts: &candidate.accounts,
        overrides: &candidate.overrides,
    };
    let generated = toml::to_string_pretty(&persisted)
        .map_err(|error| format!("Could not serialize Email configuration: {error}"))?;
    let email_document = generated
        .parse::<DocumentMut>()
        .map_err(|error| format!("Could not prepare Email configuration: {error}"))?;
    if !document.as_table().contains_key("channels") {
        document
            .as_table_mut()
            .insert("channels", Item::Table(Table::new()));
    }
    let channels = document["channels"]
        .as_table_mut()
        .ok_or_else(|| "config.toml field 'channels' is not a table".to_string())?;
    channels.insert("email", Item::Table(email_document.as_table().clone()));
    captain_types::durable_fs::atomic_write(config_path, document.to_string().as_bytes())
        .map_err(|error| format!("Could not write {}: {error}", config_path.display()))?;
    Ok(candidate)
}

fn read_config_or_empty(config_path: &std::path::Path) -> Result<(bool, String), String> {
    if !config_path.exists() {
        return Ok((false, String::new()));
    }
    std::fs::read_to_string(config_path)
        .map(|raw| (true, raw))
        .map_err(|error| format!("Could not read {}: {error}", config_path.display()))
}

fn acquire_email_config_lock(config_path: &std::path::Path) -> Result<EmailConfigLock, String> {
    let parent = config_path
        .parent()
        .ok_or_else(|| "Email config path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    let lock_path = parent.join(EMAIL_CONFIG_LOCK_NAME);
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&lock_path)
        .map_err(|error| format!("Could not open {}: {error}", lock_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("Could not protect {}: {error}", lock_path.display()))?;
    }

    let started = Instant::now();
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(EmailConfigLock { _file: file }),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if started.elapsed() >= EMAIL_CONFIG_LOCK_TIMEOUT {
                    return Err(
                        "Email configuration is busy after 10 seconds; retry the operation"
                            .to_string(),
                    );
                }
                std::thread::sleep(EMAIL_CONFIG_LOCK_RETRY);
            }
            Err(error) => {
                return Err(format!("Could not lock {}: {error}", lock_path.display()));
            }
        }
    }
}

fn restore_email_config(
    config_path: &std::path::Path,
    existed: bool,
    original: &str,
) -> Result<(), String> {
    if existed {
        captain_types::durable_fs::atomic_write(config_path, original.as_bytes())
            .map_err(|error| error.to_string())
    } else if config_path.exists() {
        std::fs::remove_file(config_path).map_err(|error| error.to_string())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod email_tests {
    use super::*;

    fn account(alias: &str) -> EmailAccountConfig {
        EmailAccountConfig {
            alias: alias.to_string(),
            imap_host: "imap.example.com".to_string(),
            smtp_host: "smtp.example.com".to_string(),
            username: format!("{alias}@example.com"),
            password_env: email_account_password_key(alias),
            allowed_senders: vec!["operator@example.com".to_string()],
            ..EmailAccountConfig::default()
        }
    }

    #[test]
    fn email_secret_keys_are_valid_and_alias_collision_resistant() {
        let dashed = email_account_password_key("work-mail");
        let underscored = email_account_password_key("work_mail");

        assert_ne!(dashed, underscored);
        assert!(dashed.starts_with("CAPTAIN_EMAIL_WORK_MAIL_"));
        assert!(dashed.ends_with("_PASSWORD"));
        assert!(dashed
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'));
    }

    #[test]
    fn email_upsert_migrates_legacy_config_and_preserves_neighbor_comments() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(
            &path,
            r#"# keep root
[channels.telegram]
# keep telegram
bot_token_env = "TELEGRAM_BOT_TOKEN"

[channels.email]
imap_host = "imap.legacy.test"
smtp_host = "smtp.legacy.test"
username = "legacy@example.com"
allowed_senders = ["operator@example.com"]
"#,
        )
        .unwrap();

        let result = upsert_email_channel_account(&path, account("work"), true).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();

        assert_eq!(result.accounts.len(), 2);
        assert_eq!(result.default_account.as_deref(), Some("work"));
        assert!(written.contains("# keep root"));
        assert!(written.contains("# keep telegram"));
        assert!(written.contains("[[channels.email.accounts]]"));
        let parsed = parse_email_channel_config(&written).unwrap().unwrap();
        assert_eq!(parsed.accounts.len(), 2);
        assert!(parsed.imap_host.is_empty());
        assert!(parsed
            .accounts
            .iter()
            .any(|account| account.alias == "default"));
    }

    #[test]
    fn email_upsert_rejects_ambiguous_allowlist_before_writing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, "# untouched\n").unwrap();
        let mut invalid = account("work");
        invalid.allowed_senders = vec!["example.com".to_string()];

        let error = upsert_email_channel_account(&path, invalid, true).unwrap_err();

        assert!(error.contains("exact address"), "{error}");
        assert_eq!(std::fs::read_to_string(path).unwrap(), "# untouched\n");
    }

    #[test]
    fn email_transaction_restores_exact_config_when_secret_write_fails() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, "# original\n").unwrap();

        let error =
            upsert_email_channel_account_transactional(&path, account("work"), true, || {
                Err("credential resolver refused the write".to_string())
            })
            .unwrap_err();

        assert!(error.contains("config.toml was restored"), "{error}");
        assert_eq!(std::fs::read_to_string(path).unwrap(), "# original\n");
    }

    #[test]
    fn concurrent_email_upserts_preserve_both_accounts() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let workers = ["work", "home"].map(|alias| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                upsert_email_channel_account(&path, account(alias), false).unwrap();
            })
        });

        barrier.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        let raw = std::fs::read_to_string(path).unwrap();
        let config = parse_email_channel_config(&raw).unwrap().unwrap();
        let mut aliases = config
            .accounts
            .iter()
            .map(|account| account.alias.as_str())
            .collect::<Vec<_>>();
        aliases.sort();

        assert_eq!(aliases, ["home", "work"]);
    }

    #[test]
    fn disabling_default_selects_another_enabled_account_or_none() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        upsert_email_channel_account(&path, account("work"), true).unwrap();
        upsert_email_channel_account(&path, account("home"), false).unwrap();
        let mut disabled_work = account("work");
        disabled_work.enabled = false;

        let config =
            upsert_email_channel_account(&path, disabled_work, false).expect("disable default");
        assert_eq!(config.default_account.as_deref(), Some("home"));

        let mut disabled_home = account("home");
        disabled_home.enabled = false;
        let config = upsert_email_channel_account(&path, disabled_home, false)
            .expect("stage fully disabled config");
        assert_eq!(config.default_account, None);
    }

    #[test]
    fn concurrent_partial_mutations_build_from_the_latest_locked_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        upsert_email_channel_account(&path, account("work"), true).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));

        let username_worker = {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                mutate_email_channel_account_transactional(
                    &path,
                    false,
                    |current| {
                        let mut account = current.unwrap().effective_accounts().remove(0);
                        account.username = "new-work@example.com".to_string();
                        Ok(account)
                    },
                    |_| Ok(()),
                )
                .unwrap();
            })
        };
        let smtp_worker = {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                mutate_email_channel_account_transactional(
                    &path,
                    false,
                    |current| {
                        let mut account = current.unwrap().effective_accounts().remove(0);
                        account.smtp_host = "smtp.changed.example.com".to_string();
                        Ok(account)
                    },
                    |_| Ok(()),
                )
                .unwrap();
            })
        };

        barrier.wait();
        username_worker.join().unwrap();
        smtp_worker.join().unwrap();
        let raw = std::fs::read_to_string(path).unwrap();
        let account = parse_email_channel_config(&raw)
            .unwrap()
            .unwrap()
            .effective_accounts()
            .remove(0);

        assert_eq!(account.username, "new-work@example.com");
        assert_eq!(account.smtp_host, "smtp.changed.example.com");
    }
}

pub(crate) fn upsert_channel_config(
    config_path: &std::path::Path,
    channel_name: &str,
    fields: &HashMap<String, (String, FieldType)>,
) -> Result<(), Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(config_path).unwrap_or_default();
    let mut doc: toml::Value = if content.trim().is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        toml::from_str(&content)?
    };
    let root = doc.as_table_mut().ok_or("Config is not a TOML table")?;
    root.entry("channels".to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let channels = root
        .get_mut("channels")
        .and_then(|value| value.as_table_mut())
        .ok_or("channels is not a table")?;
    let mut channel = toml::map::Map::new();
    for (key, (value, field_type)) in fields {
        channel.insert(key.clone(), toml_value(value, *field_type));
    }
    channels.insert(channel_name.to_string(), toml::Value::Table(channel));
    let serialized = toml::to_string_pretty(&doc)?;
    captain_types::durable_fs::atomic_write(config_path, serialized.as_bytes())?;
    Ok(())
}

fn toml_value(value: &str, field_type: FieldType) -> toml::Value {
    match field_type {
        FieldType::Number => value
            .parse::<i64>()
            .map(toml::Value::Integer)
            .unwrap_or_else(|_| toml::Value::String(value.to_string())),
        FieldType::List => toml::Value::Array(
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| toml::Value::String(item.to_string()))
                .collect(),
        ),
        FieldType::Secret | FieldType::Text => toml::Value::String(value.to_string()),
    }
}

pub(crate) fn remove_channel_config(
    config_path: &std::path::Path,
    channel_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if !config_path.exists() {
        return Ok(());
    }
    let content = std::fs::read_to_string(config_path)?;
    if content.trim().is_empty() {
        return Ok(());
    }
    let mut doc: toml::Value = toml::from_str(&content)?;
    if let Some(channels) = doc
        .as_table_mut()
        .and_then(|root| root.get_mut("channels"))
        .and_then(|value| value.as_table_mut())
    {
        channels.remove(channel_name);
    }
    let serialized = toml::to_string_pretty(&doc)?;
    captain_types::durable_fs::atomic_write(config_path, serialized.as_bytes())?;
    Ok(())
}
