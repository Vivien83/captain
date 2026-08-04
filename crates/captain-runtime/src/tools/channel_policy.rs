pub(crate) const ACTIVE_CHANNELS: &[&str] = &["telegram", "discord", "signal", "email"];
pub(crate) const ACTIVE_CHANNELS_TEXT: &str = "telegram, discord, signal, email";

pub(crate) fn is_active_channel(name: &str) -> bool {
    ACTIVE_CHANNELS.contains(&name)
        || name
            .strip_prefix("email:")
            .is_some_and(captain_types::config::is_valid_email_account_alias)
}

pub(crate) fn ensure_active_channel(name: &str) -> Result<(), String> {
    if is_active_channel(name) {
        return Ok(());
    }
    Err(format!(
        "channel '{name}' is not active. Active channels: {ACTIVE_CHANNELS_TEXT}. Non-core channels are frozen until the core is production-grade."
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

    #[test]
    fn named_email_accounts_are_active_but_unsafe_aliases_are_not() {
        assert!(is_active_channel("email:work"));
        assert!(is_active_channel("email:personal.mail"));
        assert!(!is_active_channel("email:"));
        assert!(!is_active_channel("email:Work"));
        assert!(!is_active_channel("email:../../work"));
    }
}
