//! Operator-facing readiness for active channel setup.

use crate::channel_registry::{field_env_name, field_is_ready, ChannelField, ChannelMeta};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelReadiness {
    pub(crate) ready: bool,
    pub(crate) has_required_secrets: bool,
    pub(crate) missing_required_fields: Vec<String>,
    pub(crate) operator_actions: Vec<String>,
    pub(crate) security_state: &'static str,
}

pub(crate) fn channel_readiness(
    meta: &ChannelMeta,
    config_values: Option<&serde_json::Value>,
) -> ChannelReadiness {
    let mut missing_required_fields = Vec::new();
    let mut operator_actions = Vec::new();

    for field in meta.fields.iter().filter(|field| field.required) {
        if !field_is_ready(field, config_values) {
            missing_required_fields.push(field_display_name(field, config_values));
            operator_actions.push(field_action(field, config_values));
        }
    }

    let has_required_secrets = meta
        .fields
        .iter()
        .filter(|field| field.required && field.env_var.is_some())
        .all(|field| field_is_ready(field, config_values));

    ChannelReadiness {
        ready: missing_required_fields.is_empty(),
        has_required_secrets,
        missing_required_fields,
        operator_actions,
        security_state: security_state(meta, config_values),
    }
}

fn field_display_name(field: &ChannelField, config_values: Option<&serde_json::Value>) -> String {
    field_env_name(field, config_values).unwrap_or_else(|| field.key.to_string())
}

fn field_action(field: &ChannelField, config_values: Option<&serde_json::Value>) -> String {
    if field.key == "allowed_users" {
        return "Add explicit allowed_users IDs, or [\"*\"] if you intentionally allow everyone."
            .to_string();
    }
    if field.key == "allowed_senders" {
        return "Add explicit allowed_senders addresses/domains, or [\"*\"] if you intentionally allow everyone."
            .to_string();
    }
    if let Some(env_var) = field_env_name(field, config_values) {
        return format!("Set {env_var} in secrets.env or through the channel setup form.");
    }
    format!("Set {} in channels config.", field.key)
}

fn security_state(meta: &ChannelMeta, config_values: Option<&serde_json::Value>) -> &'static str {
    let Some(allowlist_field) = meta
        .fields
        .iter()
        .find(|field| field.key == "allowed_users" || field.key == "allowed_senders")
    else {
        return "not_applicable";
    };
    if !field_is_ready(allowlist_field, config_values) {
        return "locked";
    }
    if allowlist_allows_all(allowlist_field.key, config_values) {
        "allow_all_explicit"
    } else {
        "allowlist"
    }
}

fn allowlist_allows_all(key: &str, config_values: Option<&serde_json::Value>) -> bool {
    let Some(value) = config_values
        .and_then(|config| config.as_object())
        .and_then(|object| object.get(key))
    else {
        return false;
    };
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str())
            .any(|item| item.trim() == "*"),
        serde_json::Value::String(text) => text.split(',').any(|item| item.trim() == "*"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel_registry::{channel_config_values, find_channel_meta};

    #[test]
    fn missing_allowlist_keeps_discord_locked() {
        let meta = find_channel_meta("discord").unwrap();
        let config = captain_types::config::ChannelsConfig {
            discord: Some(captain_types::config::DiscordConfig::default()),
            ..Default::default()
        };
        let values = channel_config_values(&config, "discord");

        let readiness = channel_readiness(meta, values.as_ref());

        assert!(!readiness.ready);
        assert_eq!(readiness.security_state, "locked");
        assert!(readiness
            .missing_required_fields
            .contains(&"allowed_users".to_string()));
    }

    #[test]
    fn explicit_wildcard_is_visible_as_allow_all() {
        let meta = find_channel_meta("telegram").unwrap();
        let config = captain_types::config::ChannelsConfig {
            telegram: Some(captain_types::config::TelegramConfig {
                allowed_users: vec!["*".to_string()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let values = channel_config_values(&config, "telegram");

        let readiness = channel_readiness(meta, values.as_ref());

        assert_eq!(readiness.security_state, "allow_all_explicit");
    }

    #[test]
    fn missing_allowed_senders_keeps_email_locked() {
        let meta = find_channel_meta("email").unwrap();
        let config = captain_types::config::ChannelsConfig {
            email: Some(captain_types::config::EmailConfig {
                username: "captain@example.com".to_string(),
                imap_host: "imap.example.com".to_string(),
                smtp_host: "smtp.example.com".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let values = channel_config_values(&config, "email");

        let readiness = channel_readiness(meta, values.as_ref());

        assert!(!readiness.ready);
        assert_eq!(readiness.security_state, "locked");
        assert!(readiness
            .missing_required_fields
            .contains(&"allowed_senders".to_string()));
        assert!(readiness
            .operator_actions
            .iter()
            .any(|action| action.contains("allowed_senders")));
    }

    #[test]
    fn email_wildcard_is_visible_as_allow_all() {
        let meta = find_channel_meta("email").unwrap();
        let config = captain_types::config::ChannelsConfig {
            email: Some(captain_types::config::EmailConfig {
                username: "captain@example.com".to_string(),
                imap_host: "imap.example.com".to_string(),
                smtp_host: "smtp.example.com".to_string(),
                allowed_senders: vec!["*".to_string()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let values = channel_config_values(&config, "email");

        let readiness = channel_readiness(meta, values.as_ref());

        assert_eq!(readiness.security_state, "allow_all_explicit");
    }
}
