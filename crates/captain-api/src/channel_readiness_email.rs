//! Multi-account Email channel readiness without credential disclosure.

use captain_channels::email::email_allowlist_rule_is_valid;
use captain_types::config::{EmailAccountConfig, EmailConfig};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct EmailAccountReadiness {
    pub(crate) alias: String,
    pub(crate) adapter: String,
    pub(crate) address: String,
    pub(crate) enabled: bool,
    pub(crate) is_default: bool,
    pub(crate) ready: bool,
    pub(crate) credential_ready: bool,
    pub(crate) security_state: &'static str,
    pub(crate) missing_required_fields: Vec<String>,
    pub(crate) operator_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct EmailChannelReadiness {
    pub(crate) ready: bool,
    pub(crate) has_required_secrets: bool,
    pub(crate) operational_state: &'static str,
    pub(crate) security_state: &'static str,
    pub(crate) default_account: Option<String>,
    pub(crate) total_accounts: usize,
    pub(crate) enabled_accounts: usize,
    pub(crate) ready_accounts: usize,
    pub(crate) missing_required_fields: Vec<String>,
    pub(crate) operator_actions: Vec<String>,
    pub(crate) accounts: Vec<EmailAccountReadiness>,
}

pub(crate) fn email_channel_readiness(
    config: Option<&EmailConfig>,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> EmailChannelReadiness {
    let Some(config) = config else {
        return EmailChannelReadiness {
            ready: false,
            has_required_secrets: false,
            operational_state: "not_configured",
            security_state: "locked",
            default_account: None,
            total_accounts: 0,
            enabled_accounts: 0,
            ready_accounts: 0,
            missing_required_fields: vec!["accounts".to_string()],
            operator_actions: vec![
                "Run `captain channel setup email` to connect an IMAP/SMTP mailbox.".to_string(),
            ],
            accounts: Vec::new(),
        };
    };

    let config_errors = config.validation_errors();
    let default_account = config.effective_default_account();
    let accounts = config
        .effective_accounts()
        .iter()
        .map(|account| {
            account_readiness(
                account,
                default_account.as_deref() == Some(account.alias.as_str()),
                resolve,
            )
        })
        .collect::<Vec<_>>();
    let enabled_accounts = accounts.iter().filter(|account| account.enabled).count();
    let ready_accounts = accounts.iter().filter(|account| account.ready).count();
    let has_required_secrets = enabled_accounts > 0
        && accounts
            .iter()
            .filter(|account| account.enabled)
            .all(|account| account.credential_ready);
    let structurally_valid = config_errors.is_empty();
    let ready = structurally_valid && ready_accounts > 0;
    let operational_state = if !structurally_valid {
        "invalid"
    } else if enabled_accounts == 0 {
        "disabled"
    } else if ready_accounts == enabled_accounts {
        "ready"
    } else if ready_accounts > 0 {
        "partial"
    } else {
        "locked"
    };
    let security_state = aggregate_security_state(&accounts);
    let mut missing_required_fields = config_errors
        .iter()
        .map(|error| format!("config: {error}"))
        .collect::<Vec<_>>();
    let mut operator_actions = config_errors
        .iter()
        .map(|error| format!("Fix Email configuration: {error}"))
        .collect::<Vec<_>>();
    for account in accounts.iter().filter(|account| account.enabled) {
        missing_required_fields.extend(
            account
                .missing_required_fields
                .iter()
                .map(|field| format!("{}:{field}", account.alias)),
        );
        operator_actions.extend(account.operator_actions.iter().cloned());
    }

    EmailChannelReadiness {
        ready,
        has_required_secrets,
        operational_state,
        security_state,
        default_account,
        total_accounts: accounts.len(),
        enabled_accounts,
        ready_accounts,
        missing_required_fields,
        operator_actions,
        accounts,
    }
}

fn account_readiness(
    account: &EmailAccountConfig,
    is_default: bool,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> EmailAccountReadiness {
    let validation_errors = account.validation_errors();
    let mut missing_required_fields = validation_errors
        .iter()
        .map(|error| format!("config:{error}"))
        .collect::<Vec<_>>();
    let mut operator_actions = validation_errors
        .iter()
        .map(|error| format!("Fix Email account '{}': {error}", account.alias))
        .collect::<Vec<_>>();
    let invalid_allowlist = account
        .allowed_senders
        .iter()
        .any(|rule| !email_allowlist_rule_is_valid(rule));
    if account.allowed_senders.is_empty() {
        missing_required_fields.push("allowed_senders".to_string());
        operator_actions.push(format!(
            "Add explicit allowed senders to Email account '{}'; use '*' only intentionally.",
            account.alias
        ));
    } else if invalid_allowlist {
        missing_required_fields.push("allowed_senders".to_string());
        operator_actions.push(format!(
            "Replace invalid allowed_senders entries for Email account '{}'.",
            account.alias
        ));
    }
    let credential_ready = resolve(&account.password_env).is_some_and(|value| !value.is_empty());
    if !credential_ready {
        missing_required_fields.push("credential".to_string());
        operator_actions.push(format!(
            "Store the app password for Email account '{}' or reconnect it.",
            account.alias
        ));
    }
    let security_state = if !account.enabled {
        "disabled"
    } else if account.allowed_senders.is_empty() || invalid_allowlist {
        "locked"
    } else if account
        .allowed_senders
        .iter()
        .any(|rule| rule.trim() == "*")
    {
        "allow_all_explicit"
    } else {
        "allowlist"
    };
    let ready = account.enabled && missing_required_fields.is_empty();
    EmailAccountReadiness {
        alias: account.alias.clone(),
        adapter: if is_default {
            "email".to_string()
        } else {
            format!("email:{}", account.alias)
        },
        address: account.username.clone(),
        enabled: account.enabled,
        is_default,
        ready,
        credential_ready,
        security_state,
        missing_required_fields,
        operator_actions,
    }
}

fn aggregate_security_state(accounts: &[EmailAccountReadiness]) -> &'static str {
    let enabled = accounts
        .iter()
        .filter(|account| account.enabled)
        .collect::<Vec<_>>();
    if enabled.is_empty()
        || enabled
            .iter()
            .all(|account| account.security_state == "locked")
    {
        return "locked";
    }
    if enabled
        .iter()
        .all(|account| account.security_state == "allow_all_explicit")
    {
        return "allow_all_explicit";
    }
    if enabled
        .iter()
        .all(|account| account.security_state == "allowlist")
    {
        return "allowlist";
    }
    "mixed"
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn multi_account_readiness_is_partial_when_one_secret_is_missing() {
        let config = EmailConfig {
            accounts: vec![
                account("work", "WORK_SECRET"),
                account("home", "HOME_SECRET"),
            ],
            default_account: Some("work".to_string()),
            ..EmailConfig::default()
        };
        let readiness = email_channel_readiness(Some(&config), &|key| {
            (key == "WORK_SECRET").then(|| "present".to_string())
        });

        assert!(readiness.ready);
        assert!(!readiness.has_required_secrets);
        assert_eq!(readiness.operational_state, "partial");
        assert_eq!(readiness.ready_accounts, 1);
        assert_eq!(readiness.accounts[0].adapter, "email");
        assert_eq!(readiness.accounts[1].adapter, "email:home");
    }

    #[test]
    fn invalid_allowlist_rule_stays_locked_even_with_a_secret() {
        let mut invalid = account("work", "WORK_SECRET");
        invalid.allowed_senders = vec!["@example.com/attacker".to_string()];
        let config = EmailConfig {
            accounts: vec![invalid],
            default_account: Some("work".to_string()),
            ..EmailConfig::default()
        };
        let readiness = email_channel_readiness(Some(&config), &|_| Some("present".to_string()));

        assert!(!readiness.ready);
        assert_eq!(readiness.operational_state, "locked");
        assert_eq!(readiness.accounts[0].security_state, "locked");
        assert_eq!(
            readiness.accounts[0].missing_required_fields,
            ["allowed_senders"]
        );
    }

    #[test]
    fn serialized_readiness_never_exposes_secret_reference_or_value() {
        let config = EmailConfig {
            accounts: vec![account("work", "PRIVATE_SECRET_NAME")],
            default_account: Some("work".to_string()),
            ..EmailConfig::default()
        };
        let readiness =
            email_channel_readiness(Some(&config), &|_| Some("super-secret".to_string()));
        let json = serde_json::to_string(&readiness).unwrap();

        assert!(!json.contains("PRIVATE_SECRET_NAME"));
        assert!(!json.contains("super-secret"));
        assert!(json.contains("work@example.com"));
    }
}
