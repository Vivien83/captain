use std::path::Path;

use captain_api::{
    email_account_password_key, parse_email_channel_config,
    upsert_email_channel_account_transactional,
};
use captain_types::config::{is_valid_email_account_alias, EmailAccountConfig, EmailConfig};
use zeroize::Zeroizing;

use crate::{
    captain_home, daemon_client, daemon_json, find_daemon, production_credential_resolver_at,
    prompt_input, prompt_secret, restrict_file_permissions, ui,
};

pub(super) fn setup_email() {
    ui::section("Setting up Email (IMAP/SMTP)");
    ui::blank();
    println!("  Gmail OAuth: captain email connect");
    println!("  IMAP/SMTP fallback uses an app password stored outside config.toml.");
    ui::blank();

    let home = captain_home();
    match configure_email_account(&home) {
        Ok(account) => {
            ui::blank();
            ui::success(&format!(
                "Email account '{}' configured as {}.",
                account.alias,
                adapter_name(&account.alias)
            ));
            reload_email_channel();
        }
        Err(error) => ui::error_with_fix(&error, "No Email configuration was changed"),
    }
}

fn configure_email_account(home: &Path) -> Result<EmailAccountConfig, String> {
    std::fs::create_dir_all(home)
        .map_err(|error| format!("Could not create Captain home: {error}"))?;
    let config_path = home.join("config.toml");
    let current = read_email_config(&config_path)?;
    let candidate = normalized_email_config(current);

    let suggested_alias = candidate
        .effective_default_account()
        .unwrap_or_else(|| "default".to_string());
    let alias = prompt_with_default("  Account alias", &suggested_alias);
    if !is_valid_email_account_alias(&alias) {
        return Err(
            "Alias must be 1-32 lowercase letters, digits, '.', '_' or '-', starting with a letter or digit."
                .to_string(),
        );
    }
    let existing = candidate
        .accounts
        .iter()
        .find(|account| account.alias == alias)
        .cloned();

    let username = prompt_required_with_default(
        "  Mailbox address/login",
        existing.as_ref().map(|account| account.username.as_str()),
    )?;
    let (suggested_imap, suggested_smtp) = suggested_hosts(&username);
    let imap_host = prompt_required_with_default(
        "  IMAP host",
        existing
            .as_ref()
            .map(|account| account.imap_host.as_str())
            .filter(|value| !value.is_empty())
            .or(suggested_imap),
    )?;
    let smtp_host = prompt_required_with_default(
        "  SMTP host",
        existing
            .as_ref()
            .map(|account| account.smtp_host.as_str())
            .filter(|value| !value.is_empty())
            .or(suggested_smtp),
    )?;
    let imap_port = prompt_port(
        "  IMAP TLS port",
        existing.as_ref().map_or(993, |account| account.imap_port),
    )?;
    let smtp_port = prompt_port(
        "  SMTP TLS/STARTTLS port",
        existing.as_ref().map_or(587, |account| account.smtp_port),
    )?;
    let allowed_default = existing
        .as_ref()
        .map(|account| account.allowed_senders.join(", "))
        .unwrap_or_default();
    let allowed_raw = prompt_with_default(
        "  Allowed senders (address, @domain, or *)",
        &allowed_default,
    );
    let allowed_senders = parse_list(&allowed_raw);
    if allowed_senders.is_empty() {
        return Err(
            "At least one allowed sender is required; use '*' only when open inbound access is intentional."
                .to_string(),
        );
    }
    let folders_default = existing
        .as_ref()
        .map(|account| account.folders.join(", "))
        .unwrap_or_else(|| "INBOX".to_string());
    let folders = parse_list(&prompt_with_default("  IMAP folders", &folders_default));
    let default_agent = prompt_with_default(
        "  Default agent",
        existing
            .as_ref()
            .and_then(|account| account.default_agent.as_deref())
            .unwrap_or("captain"),
    );
    let password_env = existing
        .as_ref()
        .map(|account| account.password_env.clone())
        .unwrap_or_else(|| email_account_password_key(&alias));

    let mut resolver =
        production_credential_resolver_at(home).map_err(|error| error.to_string())?;
    let credential_exists = resolver.has_credential(&password_env);
    let password_prompt = if credential_exists {
        format!("  App password for {alias} (Enter to keep current): ")
    } else {
        format!("  App password for {alias}: ")
    };
    let password = Zeroizing::new(prompt_secret(&password_prompt));
    if password.is_empty() && !credential_exists {
        return Err("An app password is required before this mailbox can start.".to_string());
    }

    let account = EmailAccountConfig {
        alias: alias.clone(),
        enabled: true,
        imap_host,
        imap_port,
        smtp_host,
        smtp_port,
        username,
        password_env: password_env.clone(),
        poll_interval_secs: existing
            .as_ref()
            .map_or(30, |account| account.poll_interval_secs),
        folders,
        allowed_senders,
        default_agent: (!default_agent.is_empty()).then_some(default_agent),
    };
    let first_account = candidate.accounts.is_empty();
    let make_default = first_account
        || candidate.default_account.as_deref() == Some(alias.as_str())
        || prompt_yes_no("  Use as default Email mailbox?", false);
    upsert_email_channel_account_transactional(
        &config_path,
        account.clone(),
        make_default,
        || {
            if password.is_empty() {
                return Ok(());
            }
            resolver
                .store_credential(&password_env, password.as_str())
                .map_err(|error| format!("Could not store {password_env}: {error}"))
        },
    )?;
    restrict_file_permissions(&config_path);
    Ok(account)
}

fn read_email_config(config_path: &Path) -> Result<EmailConfig, String> {
    let raw = std::fs::read_to_string(config_path).unwrap_or_default();
    if raw.trim().is_empty() {
        return Ok(EmailConfig::default());
    }
    let email = email_config_from_raw(&raw)?.unwrap_or_default();
    Ok(email)
}

pub(super) fn email_config_from_raw(raw: &str) -> Result<Option<EmailConfig>, String> {
    parse_email_channel_config(raw)
}

fn normalized_email_config(current: EmailConfig) -> EmailConfig {
    EmailConfig {
        accounts: current.effective_accounts(),
        default_account: current.effective_default_account(),
        overrides: current.overrides,
        ..EmailConfig::default()
    }
}

fn reload_email_channel() {
    let Some(base) = find_daemon() else {
        ui::hint("Start the daemon to activate this mailbox: captain start");
        return;
    };
    let body = daemon_json(
        daemon_client()
            .post(format!("{base}/api/channels/reload"))
            .send(),
    );
    let active = body
        .get("started")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(|name| name == "email")
        });
    if active {
        ui::success("Email bridge reloaded.");
    } else {
        ui::warn_with_fix(
            "Email configuration was saved but the bridge did not report an active mailbox",
            "Run `captain status --verbose` and inspect the named Email account readiness",
        );
    }
}

fn prompt_with_default(label: &str, default: &str) -> String {
    let prompt = if default.is_empty() {
        format!("{label}: ")
    } else {
        format!("{label} [{default}]: ")
    };
    let value = prompt_input(&prompt);
    if value.is_empty() {
        default.to_string()
    } else {
        value
    }
}

fn prompt_required_with_default(label: &str, default: Option<&str>) -> Result<String, String> {
    let value = prompt_with_default(label, default.unwrap_or_default());
    if value.is_empty() {
        Err(format!("{label} is required."))
    } else {
        Ok(value)
    }
}

fn prompt_port(label: &str, default: u16) -> Result<u16, String> {
    prompt_with_default(label, &default.to_string())
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| format!("{label} must be a non-zero port."))
}

fn prompt_yes_no(label: &str, default: bool) -> bool {
    let suffix = if default { "Y/n" } else { "y/N" };
    let answer = prompt_input(&format!("{label} [{suffix}] "));
    if answer.is_empty() {
        default
    } else {
        answer.starts_with('y') || answer.starts_with('Y')
    }
}

fn parse_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn suggested_hosts(username: &str) -> (Option<&'static str>, Option<&'static str>) {
    let domain = username.rsplit_once('@').map(|(_, domain)| domain);
    match domain.map(str::to_ascii_lowercase).as_deref() {
        Some("gmail.com" | "googlemail.com") => (Some("imap.gmail.com"), Some("smtp.gmail.com")),
        _ => (None, None),
    }
}

fn adapter_name(alias: &str) -> String {
    format!("email:{alias}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_api::upsert_email_channel_account;

    fn account(alias: &str, secret: &str) -> EmailAccountConfig {
        EmailAccountConfig {
            alias: alias.to_string(),
            imap_host: "imap.example.com".to_string(),
            smtp_host: "smtp.example.com".to_string(),
            username: format!("{alias}@example.com"),
            password_env: secret.to_string(),
            allowed_senders: vec!["operator@example.com".to_string()],
            ..EmailAccountConfig::default()
        }
    }

    #[test]
    fn upsert_migrates_legacy_mailbox_and_preserves_other_accounts() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[channels.email]
imap_host = "imap.legacy.test"
smtp_host = "smtp.legacy.test"
username = "legacy@example.com"
allowed_senders = ["operator@example.com"]
default_agent = "captain"
"#,
        )
        .unwrap();
        let config = upsert_email_channel_account(
            &path,
            account("work", "CAPTAIN_EMAIL_WORK_PASSWORD"),
            false,
        )
        .unwrap();

        assert_eq!(config.accounts.len(), 2);
        assert_eq!(config.accounts[0].alias, "default");
        assert_eq!(config.accounts[1].alias, "work");
        assert_eq!(config.default_account.as_deref(), Some("default"));
        assert!(config.validation_errors().is_empty());
    }

    #[test]
    fn email_config_write_preserves_neighbor_comments_and_uses_account_tables() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let raw = "# operator note\n[channels.telegram]\nbot_token_env = \"TOKEN\"\n";
        std::fs::write(&path, raw).unwrap();
        let config = upsert_email_channel_account(
            &path,
            account("work", "CAPTAIN_EMAIL_WORK_PASSWORD"),
            true,
        )
        .unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("# operator note"));
        assert!(written.contains("[[channels.email.accounts]]"));
        assert!(written.contains("alias = \"work\""));
        assert!(!written.contains("password ="));
        let parsed: captain_types::config::KernelConfig = toml::from_str(&written).unwrap();
        assert_eq!(config.effective_accounts()[0].alias, "work");
        assert_eq!(
            parsed.channels.email.unwrap().effective_accounts()[0].alias,
            "work"
        );
    }

    #[test]
    fn only_gmail_gets_a_server_preset() {
        assert_eq!(
            suggested_hosts("person@gmail.com"),
            (Some("imap.gmail.com"), Some("smtp.gmail.com"))
        );
        assert_eq!(suggested_hosts("person@example.com"), (None, None));
    }
}
