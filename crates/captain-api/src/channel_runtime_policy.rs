use captain_types::config::ChannelsConfig;
use serde_json::Value;

pub(crate) const ACTIVE_RUNTIME_CHANNELS: &[&str] = &["telegram", "discord", "signal", "email"];

pub(crate) fn active_channel_config_names(config: &ChannelsConfig) -> Vec<&'static str> {
    let mut names = Vec::new();
    if config.telegram.is_some() {
        names.push("telegram");
    }
    if config.discord.is_some() {
        names.push("discord");
    }
    if config.signal.is_some() {
        names.push("signal");
    }
    if config.email.is_some() {
        names.push("email");
    }
    names
}

pub(crate) fn has_active_channel_config(config: &ChannelsConfig) -> bool {
    !active_channel_config_names(config).is_empty()
}

pub(crate) fn frozen_channel_config_names(config: &ChannelsConfig) -> Vec<String> {
    let Ok(Value::Object(fields)) = serde_json::to_value(config) else {
        return Vec::new();
    };
    let mut names: Vec<String> = fields
        .into_iter()
        .filter_map(|(name, value)| {
            if name == "silent_mode"
                || ACTIVE_RUNTIME_CHANNELS.contains(&name.as_str())
                || value.is_null()
            {
                None
            } else {
                Some(name)
            }
        })
        .collect();
    names.sort();
    names
}

pub(crate) fn frozen_channel_runtime_enabled() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::config::{EmailConfig, TelegramConfig, WeComConfig, WhatsAppConfig};

    #[test]
    fn active_channel_config_names_only_include_core_channels() {
        let config = ChannelsConfig {
            telegram: Some(TelegramConfig::default()),
            wecom: Some(WeComConfig::default()),
            silent_mode: true,
            ..Default::default()
        };

        assert_eq!(active_channel_config_names(&config), vec!["telegram"]);
        assert!(has_active_channel_config(&config));
    }

    #[test]
    fn email_is_an_active_runtime_channel() {
        let config = ChannelsConfig {
            email: Some(EmailConfig::default()),
            ..Default::default()
        };

        assert_eq!(active_channel_config_names(&config), vec!["email"]);
        assert!(has_active_channel_config(&config));
    }

    #[test]
    fn frozen_channel_config_names_exclude_core_and_silent_mode() {
        let config = ChannelsConfig {
            telegram: Some(TelegramConfig::default()),
            email: Some(EmailConfig::default()),
            whatsapp: Some(WhatsAppConfig::default()),
            wecom: Some(WeComConfig::default()),
            silent_mode: true,
            ..Default::default()
        };

        assert_eq!(
            frozen_channel_config_names(&config),
            vec!["wecom".to_string(), "whatsapp".to_string()]
        );
    }
}
