//! Audit helpers for channel operator actions.

use captain_runtime::audit::{AuditAction, AuditLog};

pub(crate) fn record_inbound_dead_letters_cleared(
    audit_log: &AuditLog,
    channel: Option<&str>,
    cleared_sessions: u64,
    cleared_messages: u64,
    remaining_messages: u64,
) {
    audit_log.record(
        "system",
        AuditAction::ConfigChange,
        inbound_dead_letter_clear_detail(channel),
        format!(
            "cleared_sessions={cleared_sessions} cleared_messages={cleared_messages} remaining_messages={remaining_messages}"
        ),
    );
}

fn inbound_dead_letter_clear_detail(channel: Option<&str>) -> String {
    match channel {
        Some(channel) => format!("channel inbound dead letters cleared channel={channel}"),
        None => "channel inbound dead letters cleared channel=all".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_detail_contains_only_operator_scope() {
        let detail = inbound_dead_letter_clear_detail(Some("telegram"));

        assert_eq!(
            detail,
            "channel inbound dead letters cleared channel=telegram"
        );
        assert!(!detail.contains("chat"));
        assert!(!detail.contains("thread"));
    }
}
