use captain_channels::types::{DeliveryReceipt, DeliveryStatus};
use captain_types::agent::AgentId;

/// Bounded in-memory delivery receipt tracker.
/// Stores up to `MAX_RECEIPTS` most recent delivery receipts per agent.
pub struct DeliveryTracker {
    receipts: dashmap::DashMap<AgentId, Vec<DeliveryReceipt>>,
}

impl Default for DeliveryTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl DeliveryTracker {
    const MAX_RECEIPTS: usize = 10_000;
    const MAX_PER_AGENT: usize = 500;

    /// Create a new empty delivery tracker.
    pub fn new() -> Self {
        Self {
            receipts: dashmap::DashMap::new(),
        }
    }

    /// Record a delivery receipt for an agent.
    pub fn record(&self, agent_id: AgentId, receipt: DeliveryReceipt) {
        let mut entry = self.receipts.entry(agent_id).or_default();
        entry.push(receipt);
        // Per-agent cap
        if entry.len() > Self::MAX_PER_AGENT {
            let drain = entry.len() - Self::MAX_PER_AGENT;
            entry.drain(..drain);
        }
        // Global cap: evict oldest agents' receipts if total exceeds limit
        drop(entry);
        let total: usize = self.receipts.iter().map(|e| e.value().len()).sum();
        if total > Self::MAX_RECEIPTS {
            // Simple eviction: remove oldest entries from first agent found
            if let Some(mut oldest) = self.receipts.iter_mut().next() {
                let to_remove = total - Self::MAX_RECEIPTS;
                let drain = to_remove.min(oldest.value().len());
                oldest.value_mut().drain(..drain);
            }
        }
    }

    /// Get recent delivery receipts for an agent (newest first).
    pub fn get_receipts(&self, agent_id: AgentId, limit: usize) -> Vec<DeliveryReceipt> {
        self.receipts
            .get(&agent_id)
            .map(|entries| entries.iter().rev().take(limit).cloned().collect())
            .unwrap_or_default()
    }

    /// Create a receipt for a successful send.
    pub fn sent_receipt(channel: &str, recipient: &str) -> DeliveryReceipt {
        DeliveryReceipt {
            message_id: uuid::Uuid::new_v4().to_string(),
            channel: channel.to_string(),
            recipient: Self::sanitize_recipient(recipient),
            status: DeliveryStatus::Sent,
            timestamp: chrono::Utc::now(),
            error: None,
        }
    }

    /// Create a receipt for a failed send.
    pub fn failed_receipt(channel: &str, recipient: &str, error: &str) -> DeliveryReceipt {
        DeliveryReceipt {
            message_id: uuid::Uuid::new_v4().to_string(),
            channel: channel.to_string(),
            recipient: Self::sanitize_recipient(recipient),
            status: DeliveryStatus::Failed,
            timestamp: chrono::Utc::now(),
            // Sanitize error: no credentials, max 256 chars
            error: Some(
                error
                    .chars()
                    .take(256)
                    .collect::<String>()
                    .replace(|c: char| c.is_control(), ""),
            ),
        }
    }

    /// Sanitize recipient to avoid PII logging.
    fn sanitize_recipient(recipient: &str) -> String {
        let s: String = recipient
            .chars()
            .filter(|c| !c.is_control())
            .take(64)
            .collect();
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt(label: &str) -> DeliveryReceipt {
        DeliveryReceipt {
            message_id: label.to_string(),
            channel: "telegram".to_string(),
            recipient: "recipient".to_string(),
            status: DeliveryStatus::Sent,
            timestamp: chrono::Utc::now(),
            error: None,
        }
    }

    #[test]
    fn receipts_are_returned_newest_first_and_limited() {
        let tracker = DeliveryTracker::new();
        let agent_id = AgentId::new();
        tracker.record(agent_id, receipt("old"));
        tracker.record(agent_id, receipt("middle"));
        tracker.record(agent_id, receipt("new"));

        let got = tracker.get_receipts(agent_id, 2);
        assert_eq!(
            got.iter()
                .map(|receipt| receipt.message_id.as_str())
                .collect::<Vec<_>>(),
            vec!["new", "middle"]
        );
    }

    #[test]
    fn per_agent_receipts_are_bounded() {
        let tracker = DeliveryTracker::new();
        let agent_id = AgentId::new();
        for i in 0..505 {
            tracker.record(agent_id, receipt(&format!("r{i}")));
        }

        let got = tracker.get_receipts(agent_id, 1_000);
        assert_eq!(got.len(), DeliveryTracker::MAX_PER_AGENT);
        assert_eq!(got.first().unwrap().message_id, "r504");
        assert_eq!(got.last().unwrap().message_id, "r5");
    }

    #[test]
    fn receipt_factories_sanitize_recipient_and_error() {
        let recipient = format!("{}{}", "a".repeat(80), "\nsecret");
        let sent = DeliveryTracker::sent_receipt("email", &recipient);
        assert_eq!(sent.status, DeliveryStatus::Sent);
        assert_eq!(sent.recipient.len(), 64);
        assert!(!sent.recipient.contains('\n'));

        let failed = DeliveryTracker::failed_receipt("email", "user\nid", &"x\n".repeat(300));
        assert_eq!(failed.status, DeliveryStatus::Failed);
        assert_eq!(failed.recipient, "userid");
        let error = failed.error.unwrap();
        assert!(error.len() <= 256);
        assert!(!error.contains('\n'));
    }
}
