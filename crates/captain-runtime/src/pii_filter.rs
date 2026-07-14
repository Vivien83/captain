//! Commit-A — Personally-Identifiable / Sensitive material detector.
//!
//! Run on every `memory_save` candidate before it lands in MemPalace.
//! The goal is *defense-in-depth*, not perfection: a verbose LLM may
//! leak a fragment of a credential it just saw, and we want to refuse
//! to persist that fragment for the next session to re-read.
//!
//! Returns `Some(pattern_name)` on first match (so the caller can log
//! WHY a candidate was rejected), `None` if the input looks clean.

use regex::Regex;
use std::sync::OnceLock;

/// Compiled regex cache. Built once on first use.
struct PatternBundle {
    name: &'static str,
    re: Regex,
}

fn patterns() -> &'static [PatternBundle] {
    static PATTERNS: OnceLock<Vec<PatternBundle>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        // Order matters: more specific prefixes BEFORE generic ones so
        // `sk-ant-…` doesn't get classified as `openai_key`. Same for
        // structured PII (IBAN starts with letters, must beat phone_fr
        // which starts with digits).
        let raw: &[(&str, &str)] = &[
            // Provider API keys — specific prefixes first
            ("anthropic_key", r"sk-ant-[A-Za-z0-9_-]{20,}"),
            ("openrouter_key", r"sk-or-v1-[A-Za-z0-9]{32,}"),
            ("stripe_key", r"sk_live_[A-Za-z0-9]{20,}"),
            ("openai_key", r"sk-[A-Za-z0-9_-]{16,}"),
            ("groq_key", r"gsk_[A-Za-z0-9]{20,}"),
            ("google_api_key", r"AIza[0-9A-Za-z_-]{35}"),
            ("elevenlabs_key", r"\bxi-[A-Za-z0-9]{20,}\b"),
            // SaaS tokens
            ("slack_token", r"xox[baprs]-[A-Za-z0-9-]{10,}"),
            ("github_pat", r"ghp_[A-Za-z0-9]{30,}"),
            ("github_classic_pat", r"github_pat_[A-Za-z0-9_]{30,}"),
            ("aws_access_key", r"\bAKIA[0-9A-Z]{16}\b"),
            // Telegram bot tokens (numeric:secret format, ≥35 chars secret)
            ("telegram_bot_token", r"\b\d{6,12}:[A-Za-z0-9_-]{30,}\b"),
            // Generic high-entropy bearer / JWT
            (
                "jwt",
                r"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
            ),
            // Structured PII — IBAN before phone (starts with [A-Z]{2}, more specific)
            ("iban", r"\b[A-Z]{2}\d{2}[A-Z0-9]{11,30}\b"),
            // PII — emails
            (
                "email",
                r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b",
            ),
            // PII — French SSN ("numéro de sécu") 13-15 digits with optional spaces
            (
                "ssn_fr",
                r"\b[12]\s?\d{2}\s?\d{2}\s?\d{2}\s?\d{3}\s?\d{3}\b",
            ),
            // PII — French phone numbers (06 12 34 56 78, +33 6 12 34 56 78)
            ("phone_fr", r"(?:\+33[\s.-]?|0)[1-9](?:[\s.-]?\d{2}){4}"),
            // Credit-card-shaped numbers (Luhn not enforced — lossy ok)
            ("credit_card_like", r"\b(?:\d[ -]?){13,19}\b"),
        ];

        raw.iter()
            .map(|(name, pat)| PatternBundle {
                name,
                re: Regex::new(pat).expect("static regex"),
            })
            .collect()
    })
}

/// Scan `text`. Returns the first matched pattern's name (so the caller
/// can include it in error messages / logs), or `None` if no rule fires.
pub fn detect_pii(text: &str) -> Option<&'static str> {
    if text.is_empty() {
        return None;
    }
    for p in patterns() {
        if p.re.is_match(text) {
            return Some(p.name);
        }
    }
    None
}

/// Check the whole `(subject, predicate, object)` triple. Subject and
/// predicate are typically short structured tags (e.g. `pref:lang`,
/// `prefers`) — we skip them by default. Only the `object` payload is
/// scanned because that's where free-text (and thus secrets) lands.
pub fn check_memory_triple(_subject: &str, _predicate: &str, object: &str) -> Option<&'static str> {
    detect_pii(object)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_openai_key() {
        let s = "OPENAI_API_KEY=sk-abcdefghij1234567890ABCDEF";
        assert_eq!(detect_pii(s), Some("openai_key"));
    }

    #[test]
    fn detects_anthropic_key() {
        let s = "ANTHROPIC_API_KEY=sk-ant-api03-abcdefghij1234567890ABC";
        assert_eq!(detect_pii(s), Some("anthropic_key"));
    }

    #[test]
    fn detects_openrouter_key() {
        // Not a real key: an obviously-fake, all-zero/sequential placeholder
        // that still matches the regex shape — a previous fixture here
        // turned out to be a real, still-active key (found live by the
        // Secrets Scan CI job's trufflehog run), which got committed and
        // pushed before anyone noticed. Never reuse a plausible-looking
        // real-shaped secret in a test fixture; use a pattern that's
        // unambiguously synthetic.
        let s =
            "key sk-or-v1-0000000000000000000000000000000000000000000000000000000000000000 here";
        assert_eq!(detect_pii(s), Some("openrouter_key"));
    }

    #[test]
    fn detects_groq_key() {
        let s = "GROQ_API_KEY=gsk_abcdefghij1234567890ABCDEFGHIJ";
        assert_eq!(detect_pii(s), Some("groq_key"));
    }

    #[test]
    fn detects_google_api_key() {
        let s = "key=AIzaSyABCDEFGHIJKLMNOPQRSTUVWXYZ123456789";
        assert_eq!(detect_pii(s), Some("google_api_key"));
    }

    #[test]
    fn detects_slack_bot_token() {
        let s = "Slack: xoxb-1234567890-1234567890-abcdef0123456789";
        assert_eq!(detect_pii(s), Some("slack_token"));
    }

    #[test]
    fn detects_github_pat() {
        let s = "GitHub: ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        assert_eq!(detect_pii(s), Some("github_pat"));
    }

    #[test]
    fn detects_aws_access_key() {
        let s = "AKIAIOSFODNN7EXAMPLE in env";
        assert_eq!(detect_pii(s), Some("aws_access_key"));
    }

    #[test]
    fn detects_jwt() {
        let s = "Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ1c2VyIn0.signature_blob";
        assert_eq!(detect_pii(s), Some("jwt"));
    }

    #[test]
    fn detects_email() {
        let s = "Contacte-moi à alex.private@example.test";
        assert_eq!(detect_pii(s), Some("email"));
    }

    #[test]
    fn detects_french_phone_number_with_zero_prefix() {
        let s = "Mon numéro est 06 12 34 56 78 si besoin";
        assert_eq!(detect_pii(s), Some("phone_fr"));
    }

    #[test]
    fn detects_french_phone_number_with_plus33() {
        let s = "Appelle-moi au +33 6 12 34 56 78";
        assert_eq!(detect_pii(s), Some("phone_fr"));
    }

    #[test]
    fn detects_iban_fr() {
        let s = "IBAN: FR1420041010050500013M02606";
        assert_eq!(detect_pii(s), Some("iban"));
    }

    #[test]
    fn detects_telegram_bot_token() {
        let s = "TELEGRAM_BOT_TOKEN=1234567890:AAFakeSecretSegmentForTesting12345";
        assert_eq!(detect_pii(s), Some("telegram_bot_token"));
    }

    #[test]
    fn detects_credit_card_shape() {
        let s = "Card: 4242 4242 4242 4242";
        assert_eq!(detect_pii(s), Some("credit_card_like"));
    }

    // ------------------------------------------------------------------
    // Negative cases — these MUST pass cleanly so legitimate facts can
    // be memorized without the filter going trigger-happy.
    // ------------------------------------------------------------------

    #[test]
    fn allows_color_preference() {
        assert_eq!(detect_pii("ma couleur préférée est le vert sapin"), None);
    }

    #[test]
    fn allows_short_timestamp_text() {
        assert_eq!(detect_pii("le rendez-vous est à 14h30"), None);
    }

    #[test]
    fn allows_french_normal_sentence() {
        assert_eq!(
            detect_pii("L'utilisateur préfère le français avec le tutoiement."),
            None
        );
    }

    #[test]
    fn allows_command_snippet_without_keys() {
        assert_eq!(detect_pii("captain integration setup telegram"), None);
    }

    #[test]
    fn allows_short_number_in_text() {
        assert_eq!(detect_pii("J'ai 38 ans et 2 enfants."), None);
    }

    #[test]
    fn allows_year_dates() {
        assert_eq!(detect_pii("Né en 1985, marié depuis 2014."), None);
    }

    #[test]
    fn empty_input_returns_none() {
        assert_eq!(detect_pii(""), None);
    }

    #[test]
    fn check_memory_triple_only_scans_object() {
        // A subject like "phone:emergency" naming a category should NOT
        // trip the phone_fr regex — we only check the object.
        let r = check_memory_triple("phone:emergency", "is_documented", "yes");
        assert_eq!(r, None);
    }

    #[test]
    fn check_memory_triple_flags_object_pii() {
        let r = check_memory_triple("user", "phone", "Mon numéro est 06 12 34 56 78");
        assert_eq!(r, Some("phone_fr"));
    }
}
