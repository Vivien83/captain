use crate::config_reload::{
    build_reload_plan, should_apply_hot, validate_config_for_reload, HotAction, ReloadPlan,
};
use captain_types::config::KernelConfig;

fn default_cfg() -> KernelConfig {
    KernelConfig::default()
}

#[test]
fn test_no_changes_detected() {
    let a = default_cfg();
    let b = default_cfg();
    let plan = build_reload_plan(&a, &b);
    assert!(!plan.has_changes());
    assert!(!plan.restart_required);
    assert!(plan.hot_actions.is_empty());
    assert!(plan.noop_changes.is_empty());
}

#[test]
fn test_api_listen_requires_restart() {
    let a = default_cfg();
    let mut b = default_cfg();
    b.api_listen = "0.0.0.0:8080".to_string();
    let plan = build_reload_plan(&a, &b);
    assert!(plan.restart_required);
    assert!(plan
        .restart_reasons
        .iter()
        .any(|r| r.contains("api_listen")));
}

#[test]
fn test_api_key_requires_restart() {
    let a = default_cfg();
    let mut b = default_cfg();
    b.api_key = "super-secret-key".to_string();
    let plan = build_reload_plan(&a, &b);
    assert!(plan.restart_required);
    assert!(plan.restart_reasons.iter().any(|r| r.contains("api_key")));
}

#[test]
fn test_network_requires_restart() {
    let a = default_cfg();
    let mut b = default_cfg();
    b.network_enabled = true;
    let plan = build_reload_plan(&a, &b);
    assert!(plan.restart_required);
    assert!(plan
        .restart_reasons
        .iter()
        .any(|r| r.contains("network_enabled")));
}

#[test]
fn test_network_config_requires_restart() {
    let a = default_cfg();
    let mut b = default_cfg();
    b.network.shared_secret = "new-secret".to_string();
    let plan = build_reload_plan(&a, &b);
    assert!(plan.restart_required);
    assert!(plan
        .restart_reasons
        .iter()
        .any(|r| r.contains("network config")));
}

#[test]
fn test_memory_config_requires_restart() {
    let a = default_cfg();
    let mut b = default_cfg();
    b.memory.consolidation_threshold = 99_999;
    let plan = build_reload_plan(&a, &b);
    assert!(plan.restart_required);
    assert!(plan
        .restart_reasons
        .iter()
        .any(|r| r.contains("memory config")));
}

#[test]
fn test_default_model_hot_reloadable() {
    let a = default_cfg();
    let mut b = default_cfg();
    b.default_model.model = "gpt-4".to_string();
    let plan = build_reload_plan(&a, &b);
    assert!(
        !plan.restart_required,
        "default_model should be hot-reloadable"
    );
    assert!(plan.hot_actions.contains(&HotAction::UpdateDefaultModel));
}

#[test]
fn test_channels_hot_reload() {
    let a = default_cfg();
    let mut b = default_cfg();
    b.channels.telegram = Some(captain_types::config::TelegramConfig {
        bot_token_env: "TG_TOKEN".to_string(),
        ..Default::default()
    });
    let plan = build_reload_plan(&a, &b);
    assert!(!plan.restart_required);
    assert!(plan.hot_actions.contains(&HotAction::ReloadChannels));
}

#[test]
fn test_usage_footer_hot_reload() {
    use captain_types::config::UsageFooterMode;

    let a = default_cfg();
    let mut b = default_cfg();
    b.usage_footer = UsageFooterMode::Off;
    let plan = build_reload_plan(&a, &b);
    assert!(!plan.restart_required);
    assert!(plan.hot_actions.contains(&HotAction::UpdateUsageFooter));
}

#[test]
fn test_max_cron_jobs_hot_reload() {
    let a = default_cfg();
    let mut b = default_cfg();
    b.max_cron_jobs = 1000;
    let plan = build_reload_plan(&a, &b);
    assert!(!plan.restart_required);
    assert!(plan.hot_actions.contains(&HotAction::UpdateCronConfig));
}

#[test]
fn test_extensions_hot_reload() {
    let a = default_cfg();
    let mut b = default_cfg();
    b.extensions.reconnect_max_attempts = 20;
    let plan = build_reload_plan(&a, &b);
    assert!(!plan.restart_required);
    assert!(plan.hot_actions.contains(&HotAction::ReloadExtensions));
}

#[test]
fn test_provider_urls_hot_reload() {
    let a = default_cfg();
    let mut b = default_cfg();
    b.provider_urls
        .insert("ollama".to_string(), "http://10.0.0.5:11434/v1".to_string());
    let plan = build_reload_plan(&a, &b);
    assert!(!plan.restart_required);
    assert!(plan.hot_actions.contains(&HotAction::ReloadProviderUrls));
}

#[test]
fn test_tts_config_hot_reload() {
    let a = default_cfg();
    let mut b = default_cfg();
    b.tts.enabled = true;
    b.tts.openai.voice = "nova".to_string();
    let plan = build_reload_plan(&a, &b);
    assert!(!plan.restart_required);
    assert!(plan.hot_actions.contains(&HotAction::UpdateTtsConfig));
}

#[test]
fn test_provider_api_keys_noop() {
    let a = default_cfg();
    let mut b = default_cfg();
    b.provider_api_keys
        .insert("openai".to_string(), "OPENAI_API_KEY".to_string());
    let plan = build_reload_plan(&a, &b);
    assert!(!plan.restart_required);
    assert!(plan.hot_actions.is_empty());
    assert!(plan
        .noop_changes
        .iter()
        .any(|change| change.contains("provider_api_keys")));
}

#[test]
fn test_mixed_changes() {
    use captain_types::config::UsageFooterMode;

    let a = default_cfg();
    let mut b = default_cfg();
    b.api_listen = "0.0.0.0:9999".to_string();
    b.usage_footer = UsageFooterMode::Tokens;
    b.max_cron_jobs = 100;
    b.log_level = "debug".to_string();

    let plan = build_reload_plan(&a, &b);
    assert!(plan.restart_required);
    assert!(plan.has_changes());
    assert!(plan.hot_actions.contains(&HotAction::UpdateUsageFooter));
    assert!(plan.hot_actions.contains(&HotAction::UpdateCronConfig));
    assert!(plan.noop_changes.iter().any(|c| c.contains("log_level")));
}

#[test]
fn test_noop_changes() {
    use captain_types::config::KernelMode;

    let a = default_cfg();
    let mut b = default_cfg();
    b.log_level = "debug".to_string();
    b.language = "de".to_string();
    b.mode = KernelMode::Dev;

    let plan = build_reload_plan(&a, &b);
    assert!(!plan.restart_required);
    assert!(plan.hot_actions.is_empty());
    assert_eq!(plan.noop_changes.len(), 3);
    assert!(plan.noop_changes.iter().any(|c| c.contains("log_level")));
    assert!(plan.noop_changes.iter().any(|c| c.contains("language")));
    assert!(plan.noop_changes.iter().any(|c| c.contains("mode")));
}

#[test]
fn test_has_changes() {
    let plan = ReloadPlan {
        restart_required: false,
        restart_reasons: vec![],
        hot_actions: vec![],
        noop_changes: vec![],
    };
    assert!(!plan.has_changes());

    let plan = ReloadPlan {
        restart_required: false,
        restart_reasons: vec![],
        hot_actions: vec![],
        noop_changes: vec!["log_level: info -> debug".to_string()],
    };
    assert!(plan.has_changes());

    let plan = ReloadPlan {
        restart_required: false,
        restart_reasons: vec![],
        hot_actions: vec![HotAction::UpdateCronConfig],
        noop_changes: vec![],
    };
    assert!(plan.has_changes());

    let plan = ReloadPlan {
        restart_required: true,
        restart_reasons: vec!["api_listen changed".to_string()],
        hot_actions: vec![],
        noop_changes: vec![],
    };
    assert!(plan.has_changes());
}

#[test]
fn test_is_hot_reloadable() {
    let plan = ReloadPlan {
        restart_required: false,
        restart_reasons: vec![],
        hot_actions: vec![HotAction::ReloadChannels],
        noop_changes: vec![],
    };
    assert!(plan.is_hot_reloadable());

    let plan = ReloadPlan {
        restart_required: true,
        restart_reasons: vec!["api_listen changed".to_string()],
        hot_actions: vec![HotAction::ReloadChannels],
        noop_changes: vec![],
    };
    assert!(!plan.is_hot_reloadable());
}

#[test]
fn test_validate_config_for_reload_valid() {
    let config = default_cfg();
    assert!(validate_config_for_reload(&config).is_ok());
}

#[test]
fn test_validate_config_for_reload_invalid() {
    let mut config = default_cfg();
    config.api_listen = String::new();
    let err = validate_config_for_reload(&config).unwrap_err();
    assert!(err.iter().any(|e| e.contains("api_listen")));

    let mut config = default_cfg();
    config.max_cron_jobs = 100_000;
    let err = validate_config_for_reload(&config).unwrap_err();
    assert!(err.iter().any(|e| e.contains("max_cron_jobs")));
}

#[test]
fn test_validate_network_enabled_no_secret() {
    let mut config = default_cfg();
    config.network_enabled = true;
    config.network.shared_secret = String::new();
    let err = validate_config_for_reload(&config).unwrap_err();
    assert!(err.iter().any(|e| e.contains("shared_secret")));
}

#[test]
fn test_should_apply_hot_off() {
    let plan = ReloadPlan {
        restart_required: false,
        restart_reasons: vec![],
        hot_actions: vec![HotAction::ReloadChannels],
        noop_changes: vec![],
    };
    assert!(!should_apply_hot(
        captain_types::config::ReloadMode::Off,
        &plan
    ));
}

#[test]
fn test_should_apply_hot_restart_mode() {
    let plan = ReloadPlan {
        restart_required: false,
        restart_reasons: vec![],
        hot_actions: vec![HotAction::ReloadChannels],
        noop_changes: vec![],
    };
    assert!(!should_apply_hot(
        captain_types::config::ReloadMode::Restart,
        &plan
    ));
}

#[test]
fn test_should_apply_hot_hybrid() {
    let plan = ReloadPlan {
        restart_required: false,
        restart_reasons: vec![],
        hot_actions: vec![HotAction::ReloadChannels],
        noop_changes: vec![],
    };
    assert!(should_apply_hot(
        captain_types::config::ReloadMode::Hybrid,
        &plan
    ));
    assert!(should_apply_hot(
        captain_types::config::ReloadMode::Hot,
        &plan
    ));
}

#[test]
fn test_should_apply_hot_empty() {
    let plan = ReloadPlan {
        restart_required: false,
        restart_reasons: vec![],
        hot_actions: vec![],
        noop_changes: vec![],
    };
    assert!(!should_apply_hot(
        captain_types::config::ReloadMode::Hybrid,
        &plan
    ));
}
