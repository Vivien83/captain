//! Operator actions for inbound channel dead letters.

use super::BridgeManager;

impl BridgeManager {
    /// Clear handled inbound dead letters without exposing private message content.
    pub fn clear_inbound_dead_letters(&self, channel: Option<&str>) -> serde_json::Value {
        let (sessions, messages) = self.inbound_sessions.clear_dead_letters(channel);
        let snapshot = self.inbound_sessions.snapshot();
        serde_json::json!({
            "status": "ok",
            "channel": channel,
            "cleared_dead_letter_sessions": sessions,
            "cleared_dead_letter_messages": messages,
            "remaining_dead_letter_sessions": snapshot.dead_letter_sessions,
            "remaining_dead_letter_messages": snapshot.dead_letter_messages,
        })
    }
}
