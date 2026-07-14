//! B.8 — Per-channel RBAC primitive.
//!
//! Each channel adapter that supports a user allowlist (`allowed_users` on
//! Telegram, Signal, WhatsApp, Discord, Slack, …) used to interpret an
//! empty list as "allow everyone" — the legacy permissive default. That
//! made it trivial to ship a channel section to production with no
//! `allowed_users` and no awareness that anyone with the bot's contact
//! could now reach the agent.
//!
//! `is_authorized` inverts the default: an **empty list denies all**.
//! To explicitly allow everyone, the operator declares `allowed_users =
//! ["*"]` in `config.toml` — the wildcard makes the decision visible in
//! review.

/// Decide whether a user is authorized for a channel that uses an
/// `allowed_users` allowlist.
///
/// Rules:
/// - Empty list → deny all.
/// - Single entry `"*"` → allow all (explicit opt-in).
/// - Otherwise → user is allowed iff their id is in the list.
pub fn is_authorized(allowed_users: &[String], user_id: &str) -> bool {
    if allowed_users.is_empty() {
        return false;
    }
    if allowed_users.iter().any(|u| u == "*") {
        return true;
    }
    allowed_users.iter().any(|u| u == user_id)
}

#[cfg(test)]
mod tests {
    use super::is_authorized;

    /// B.8 — empty allow_list must reject every user, not silently allow.
    #[test]
    fn empty_list_denies_all() {
        let allowed: Vec<String> = vec![];
        assert!(!is_authorized(&allowed, "12345"));
        assert!(!is_authorized(&allowed, ""));
        assert!(!is_authorized(&allowed, "anyone"));
    }

    /// B.8 — explicit `["*"]` is the operator's opt-in to "anyone can chat".
    #[test]
    fn wildcard_allows_all() {
        let allowed: Vec<String> = vec!["*".to_string()];
        assert!(is_authorized(&allowed, "12345"));
        assert!(is_authorized(&allowed, "alice"));
    }

    /// B.8 — listed users pass, unlisted users fail.
    #[test]
    fn explicit_list_filters_correctly() {
        let allowed: Vec<String> = vec!["alice".into(), "bob".into()];
        assert!(is_authorized(&allowed, "alice"));
        assert!(is_authorized(&allowed, "bob"));
        assert!(!is_authorized(&allowed, "carol"));
    }

    /// B.8 — wildcard among other entries still allows everyone, the
    /// other entries are documentation only.
    #[test]
    fn wildcard_with_other_entries_allows_all() {
        let allowed: Vec<String> = vec!["alice".into(), "*".into()];
        assert!(is_authorized(&allowed, "carol"));
    }

    /// B.8 — exact match is required; partial / case-insensitive is not.
    #[test]
    fn match_is_exact_and_case_sensitive() {
        let allowed: Vec<String> = vec!["alice".into()];
        assert!(!is_authorized(&allowed, "Alice"));
        assert!(!is_authorized(&allowed, "alic"));
        assert!(!is_authorized(&allowed, "alicebob"));
    }
}
