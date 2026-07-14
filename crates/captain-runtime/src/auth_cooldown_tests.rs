use super::*;

fn fast_config() -> CooldownConfig {
    CooldownConfig {
        base_cooldown_secs: 1,
        max_cooldown_secs: 10,
        backoff_multiplier: 2.0,
        max_exponent: 3,
        billing_base_cooldown_secs: 5,
        billing_max_cooldown_secs: 20,
        billing_multiplier: 2.0,
        failure_window_secs: 60,
        probe_enabled: true,
        probe_interval_secs: 0,
    }
}

#[test]
fn test_cooldown_config_defaults() {
    let config = CooldownConfig::default();
    assert_eq!(config.base_cooldown_secs, 60);
    assert_eq!(config.max_cooldown_secs, 3600);
    assert_eq!(config.backoff_multiplier, 5.0);
    assert_eq!(config.max_exponent, 3);
    assert_eq!(config.billing_base_cooldown_secs, 18_000);
    assert_eq!(config.billing_max_cooldown_secs, 86_400);
    assert_eq!(config.billing_multiplier, 2.0);
    assert_eq!(config.failure_window_secs, 86_400);
    assert!(config.probe_enabled);
    assert_eq!(config.probe_interval_secs, 30);
}

#[test]
fn test_new_provider_allows() {
    let cb = ProviderCooldown::new(fast_config());
    assert_eq!(cb.check("openai"), CooldownVerdict::Allow);
    assert_eq!(cb.get_state("openai"), CircuitState::Closed);
}

#[test]
fn test_single_failure_opens_circuit() {
    let cb = ProviderCooldown::new(fast_config());
    cb.record_failure("openai", false);
    assert_eq!(cb.get_state("openai"), CircuitState::Open);
}

#[test]
fn test_cooldown_duration_escalates() {
    let config = fast_config();
    assert_eq!(calculate_cooldown(&config, 1, false).as_secs(), 1);
    assert_eq!(calculate_cooldown(&config, 2, false).as_secs(), 2);
    assert_eq!(calculate_cooldown(&config, 3, false).as_secs(), 4);
    assert_eq!(calculate_cooldown(&config, 4, false).as_secs(), 8);
    assert_eq!(calculate_cooldown(&config, 100, false).as_secs(), 8);
}

#[test]
fn test_billing_longer_cooldown() {
    let config = fast_config();
    let general = calculate_cooldown(&config, 1, false);
    let billing = calculate_cooldown(&config, 1, true);
    assert!(billing > general, "billing cooldown should be longer");
    assert_eq!(billing.as_secs(), 5);
}

#[test]
fn test_billing_max_cap() {
    let config = fast_config();
    let d = calculate_cooldown(&config, 100, true);
    assert_eq!(d.as_secs(), 20);
}

#[test]
fn test_success_resets_circuit() {
    let cb = ProviderCooldown::new(fast_config());
    cb.record_failure("openai", false);
    assert_eq!(cb.get_state("openai"), CircuitState::Open);

    cb.record_success("openai");
    assert_eq!(cb.get_state("openai"), CircuitState::Closed);
    assert_eq!(cb.check("openai"), CooldownVerdict::Allow);
}

#[test]
fn test_probe_allowed_after_cooldown() {
    let mut config = fast_config();
    config.base_cooldown_secs = 0;
    let cb = ProviderCooldown::new(config);

    cb.record_failure("openai", false);
    std::thread::sleep(Duration::from_millis(5));

    let verdict = cb.check("openai");
    assert_eq!(verdict, CooldownVerdict::AllowProbe);
    assert_eq!(cb.get_state("openai"), CircuitState::HalfOpen);
}

#[test]
fn test_probe_interval_throttled() {
    let mut config = fast_config();
    config.probe_interval_secs = 9999;
    config.probe_enabled = true;
    let cb = ProviderCooldown::new(config);

    cb.record_failure("openai", false);

    let v1 = cb.check("openai");
    assert_eq!(v1, CooldownVerdict::AllowProbe);

    cb.record_probe_result("openai", false);

    let v2 = cb.check("openai");
    match v2 {
        CooldownVerdict::Reject { .. } => {}
        other => panic!("expected Reject after probe throttle, got {other:?}"),
    }
}

#[test]
fn test_probe_success_closes_circuit() {
    let cb = ProviderCooldown::new(fast_config());
    cb.record_failure("openai", false);
    assert_eq!(cb.get_state("openai"), CircuitState::Open);

    cb.record_probe_result("openai", true);
    assert_eq!(cb.get_state("openai"), CircuitState::Closed);
}

#[test]
fn test_probe_failure_extends_cooldown() {
    let cb = ProviderCooldown::new(fast_config());
    cb.record_failure("openai", false);

    let state_before = cb.states.get("openai").unwrap().error_count;
    cb.record_probe_result("openai", false);
    let state_after = cb.states.get("openai").unwrap().error_count;

    assert_eq!(
        state_after,
        state_before + 1,
        "error count should increase on probe failure"
    );
    assert_eq!(cb.get_state("openai"), CircuitState::Open);
}

#[test]
fn test_clear_expired() {
    let mut config = fast_config();
    config.base_cooldown_secs = 0;
    let cb = ProviderCooldown::new(config);

    cb.record_failure("openai", false);
    cb.record_success("openai");

    assert!(cb.states.contains_key("openai"));

    cb.force_reset("openai");
    assert!(!cb.states.contains_key("openai"));
}

#[test]
fn test_force_reset() {
    let cb = ProviderCooldown::new(fast_config());
    cb.record_failure("openai", false);
    cb.record_failure("openai", false);
    assert_eq!(cb.get_state("openai"), CircuitState::Open);

    cb.force_reset("openai");
    assert_eq!(cb.get_state("openai"), CircuitState::Closed);
    assert_eq!(cb.check("openai"), CooldownVerdict::Allow);
}

#[test]
fn test_snapshot() {
    let cb = ProviderCooldown::new(fast_config());
    cb.record_failure("openai", false);
    cb.record_failure("anthropic", true);

    let snap = cb.snapshot();
    assert_eq!(snap.len(), 2);

    let openai_snap = snap.iter().find(|s| s.provider == "openai").unwrap();
    assert_eq!(openai_snap.state, CircuitState::Open);
    assert_eq!(openai_snap.error_count, 1);
    assert!(!openai_snap.is_billing);

    let anthropic_snap = snap.iter().find(|s| s.provider == "anthropic").unwrap();
    assert_eq!(anthropic_snap.state, CircuitState::Open);
    assert_eq!(anthropic_snap.error_count, 1);
    assert!(anthropic_snap.is_billing);
}

#[test]
fn test_failure_window_reset() {
    let mut config = fast_config();
    config.failure_window_secs = 0;
    let cb = ProviderCooldown::new(config);

    cb.record_failure("openai", false);
    std::thread::sleep(Duration::from_millis(5));

    cb.record_failure("openai", false);
    let state = cb.states.get("openai").unwrap();
    assert_eq!(state.total_errors_in_window, 1);
}

#[test]
fn test_multiple_providers_independent() {
    let cb = ProviderCooldown::new(fast_config());

    cb.record_failure("openai", false);
    cb.record_failure("openai", false);
    cb.record_failure("anthropic", true);

    assert_eq!(cb.get_state("openai"), CircuitState::Open);
    assert_eq!(cb.get_state("anthropic"), CircuitState::Open);
    assert_eq!(cb.get_state("gemini"), CircuitState::Closed);

    cb.record_success("openai");
    assert_eq!(cb.get_state("openai"), CircuitState::Closed);
    assert_eq!(cb.get_state("anthropic"), CircuitState::Open);
}
