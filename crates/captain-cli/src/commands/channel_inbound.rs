use crate::{daemon_client, daemon_json, find_daemon, ui};

pub(crate) fn cmd_clear_inbound_dead_letters(channel: Option<&str>) {
    let Some(base) = find_daemon() else {
        ui::error_with_fix(
            "Clearing inbound dead letters requires a running daemon",
            "Start the daemon: captain start",
        );
        std::process::exit(1);
    };

    let client = daemon_client();
    let mut request = client.delete(format!("{base}/api/channels/inbound-queue/dead-letters"));
    if let Some(channel) = channel.map(str::trim).filter(|value| !value.is_empty()) {
        request = request.query(&[("channel", channel)]);
    }
    let body = daemon_json(request.send());

    println!("{}", inbound_dead_letter_clear_summary(&body));
    if let Some(hint) = inbound_dead_letter_clear_hint(&body) {
        ui::hint(&hint);
    }
}

fn inbound_dead_letter_clear_summary(body: &serde_json::Value) -> String {
    if body["status"] == "unavailable" {
        return "Channel bridge is not running; no inbound dead letters are loaded.".to_string();
    }
    let messages = body["cleared_dead_letter_messages"].as_u64().unwrap_or(0);
    let sessions = body["cleared_dead_letter_sessions"].as_u64().unwrap_or(0);
    if messages == 0 {
        return "No inbound dead letters to clear.".to_string();
    }
    format!("Cleared {messages} inbound dead-letter message(s) across {sessions} session(s).")
}

fn inbound_dead_letter_clear_hint(body: &serde_json::Value) -> Option<String> {
    let remaining = body["remaining_dead_letter_messages"].as_u64().unwrap_or(0);
    (remaining > 0).then(|| format!("{remaining} inbound dead-letter message(s) still remain."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_summary_never_prints_payload_fields() {
        let body = serde_json::json!({
            "status": "ok",
            "cleared_dead_letter_sessions": 1,
            "cleared_dead_letter_messages": 2,
            "remaining_dead_letter_messages": 0,
            "message": "private content should not be displayed"
        });

        let summary = inbound_dead_letter_clear_summary(&body);

        assert_eq!(
            summary,
            "Cleared 2 inbound dead-letter message(s) across 1 session(s)."
        );
        assert!(!summary.contains("private content"));
    }

    #[test]
    fn clear_hint_reports_remaining_count_only() {
        let body = serde_json::json!({
            "remaining_dead_letter_messages": 3,
            "thread_id": "private-thread"
        });

        let hint = inbound_dead_letter_clear_hint(&body).unwrap();

        assert_eq!(hint, "3 inbound dead-letter message(s) still remain.");
        assert!(!hint.contains("private-thread"));
    }
}
