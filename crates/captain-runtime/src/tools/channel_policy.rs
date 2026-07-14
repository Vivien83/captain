pub(crate) const ACTIVE_CHANNELS: &[&str] = &["telegram", "discord", "signal", "email"];
pub(crate) const ACTIVE_CHANNELS_TEXT: &str = "telegram, discord, signal, email";

pub(crate) fn is_active_channel(name: &str) -> bool {
    ACTIVE_CHANNELS.contains(&name)
}

pub(crate) fn ensure_active_channel(name: &str) -> Result<(), String> {
    if is_active_channel(name) {
        return Ok(());
    }
    Err(format!(
        "channel '{name}' is not active. Active channels: {ACTIVE_CHANNELS_TEXT}. Non-core channels are frozen until the core is Hermes-level."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_channel_policy_is_small_and_explicit() {
        assert!(is_active_channel("telegram"));
        assert!(is_active_channel("discord"));
        assert!(is_active_channel("signal"));
        assert!(is_active_channel("email"));
        assert!(!is_active_channel("slack"));
    }

    #[test]
    fn inactive_channel_error_names_active_set() {
        let err = ensure_active_channel("matrix").unwrap_err();
        assert!(err.contains("matrix"));
        assert!(err.contains(ACTIVE_CHANNELS_TEXT));
        assert!(err.contains("frozen"));
    }
}
