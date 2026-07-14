use super::*;

#[test]
fn registry_contains_only_active_channels() {
    assert_eq!(
        active_channel_names(),
        vec!["telegram", "discord", "signal", "email"]
    );
}

#[test]
fn configured_check_is_active_only() {
    let mut config = captain_types::config::ChannelsConfig::default();
    assert!(!is_channel_configured(&config, "telegram"));
    assert!(!is_channel_configured(&config, "email"));
    assert!(!is_channel_configured(&config, "wecom"));
    config.telegram = Some(captain_types::config::TelegramConfig::default());
    assert!(is_channel_configured(&config, "telegram"));
    config.email = Some(captain_types::config::EmailConfig::default());
    assert!(is_channel_configured(&config, "email"));
    assert!(!is_channel_configured(&config, "wecom"));
}

#[test]
fn frozen_channels_are_known_but_not_active() {
    assert!(is_frozen_channel("slack"));
    assert!(is_frozen_channel("WhatsApp"));
    assert!(!is_frozen_channel("telegram"));
    assert!(!is_frozen_channel("email"));
    assert!(!active_channel_names().contains(&"slack"));
}

#[test]
fn empty_required_allowlist_is_not_ready() {
    let meta = find_channel_meta("telegram").unwrap();
    let allowed_users = meta
        .fields
        .iter()
        .find(|field| field.key == "allowed_users")
        .unwrap();
    let config = captain_types::config::ChannelsConfig {
        telegram: Some(captain_types::config::TelegramConfig::default()),
        ..Default::default()
    };
    let values = channel_config_values(&config, "telegram");

    assert!(!field_is_ready(allowed_users, values.as_ref()));
}

#[test]
fn empty_required_allowed_senders_is_not_ready() {
    let meta = find_channel_meta("email").unwrap();
    let allowed_senders = meta
        .fields
        .iter()
        .find(|field| field.key == "allowed_senders")
        .unwrap();
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

    assert!(!field_is_ready(allowed_senders, values.as_ref()));
}

#[test]
fn token_readiness_uses_configured_env_pointer() {
    let env_name = "CAPTAIN_TEST_CHANNEL_TOKEN";
    unsafe {
        std::env::remove_var(env_name);
    }
    let meta = find_channel_meta("telegram").unwrap();
    let token_field = meta
        .fields
        .iter()
        .find(|field| field.key == "bot_token_env")
        .unwrap();
    let config = captain_types::config::ChannelsConfig {
        telegram: Some(captain_types::config::TelegramConfig {
            bot_token_env: env_name.to_string(),
            allowed_users: vec!["123".to_string()],
            ..Default::default()
        }),
        ..Default::default()
    };
    let values = channel_config_values(&config, "telegram");

    assert!(!field_is_ready(token_field, values.as_ref()));
    unsafe {
        std::env::set_var(env_name, "token");
    }
    assert!(field_is_ready(token_field, values.as_ref()));
    unsafe {
        std::env::remove_var(env_name);
    }
}
