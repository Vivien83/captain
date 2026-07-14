//! Shared helpers for channel-neutral safe model-switch confirmations.
//!
//! The TUI has an in-memory pending prompt, but Telegram and other channels
//! arrive as ordinary user messages. These helpers give the runtime/kernel a
//! common key and parser so a reply like "Nouvelle" can complete the same
//! safe switch without relying on the LLM to infer the next tool call.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingModelSwitchChoice {
    NewSession,
    CompactSession,
    Cancel,
}

impl PendingModelSwitchChoice {
    pub fn as_session_strategy(self) -> Option<&'static str> {
        match self {
            Self::NewSession => Some("new_session"),
            Self::CompactSession => Some("compact_session"),
            Self::Cancel => None,
        }
    }
}

pub fn pending_model_switch_key(agent_id: &str) -> String {
    format!("__captain_model_switch_pending_v1:{agent_id}")
}

pub fn parse_pending_model_switch_choice(input: &str) -> Option<PendingModelSwitchChoice> {
    let normalized = input
        .trim()
        .trim_matches(|c: char| c.is_ascii_punctuation() || matches!(c, '…' | '«' | '»'))
        .to_ascii_lowercase();
    let compact = normalized.split_whitespace().collect::<Vec<_>>().join(" ");

    if compact.is_empty() {
        return None;
    }

    match compact.as_str() {
        "1" | "new" | "new session" | "new_session" | "nouvelle" | "nouvelle session" | "reset"
        | "fresh" | "fresh session" | "from scratch" | "zero" | "zéro" => {
            return Some(PendingModelSwitchChoice::NewSession)
        }
        "2"
        | "compact"
        | "compact session"
        | "compact_session"
        | "resume"
        | "résumé"
        | "resume compact"
        | "résumé compact"
        | "summary"
        | "garder le contexte"
        | "conserver le contexte" => return Some(PendingModelSwitchChoice::CompactSession),
        "annule" | "annuler" | "cancel" | "stop" | "non" | "no" => {
            return Some(PendingModelSwitchChoice::Cancel)
        }
        _ => {}
    }

    if compact.starts_with("nouvelle ") && compact.len() <= 80 {
        return Some(PendingModelSwitchChoice::NewSession);
    }
    if compact.contains("nouvelle session") || compact.contains("repartir de zero") {
        return Some(PendingModelSwitchChoice::NewSession);
    }
    if compact.contains("compact") || compact.contains("résumé") || compact.contains("resume") {
        return Some(PendingModelSwitchChoice::CompactSession);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{parse_pending_model_switch_choice, PendingModelSwitchChoice};

    #[test]
    fn parses_french_new_session_reply() {
        assert_eq!(
            parse_pending_model_switch_choice("Nouvelle"),
            Some(PendingModelSwitchChoice::NewSession)
        );
        assert_eq!(
            parse_pending_model_switch_choice("Nouvelle session"),
            Some(PendingModelSwitchChoice::NewSession)
        );
    }

    #[test]
    fn parses_compact_reply() {
        assert_eq!(
            parse_pending_model_switch_choice("Résumé compact"),
            Some(PendingModelSwitchChoice::CompactSession)
        );
    }

    #[test]
    fn ignores_unrelated_messages() {
        assert_eq!(parse_pending_model_switch_choice("Alors ?"), None);
    }
}
