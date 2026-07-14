//! Sender identity helpers for inbound channel messages.

use crate::types::ChannelMessage;

/// Extract the sender's user identity from a message.
///
/// Some adapters set `platform_id` to the channel/conversation ID needed for
/// the send path and store the actual user ID in metadata. This helper returns
/// the stable user ID for RBAC, rate limiting and inbound session keys.
pub(super) fn sender_user_id(message: &ChannelMessage) -> &str {
    message
        .metadata
        .get("sender_user_id")
        .and_then(|value| value.as_str())
        .unwrap_or(&message.sender.platform_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelContent, ChannelMessage, ChannelType, ChannelUser};
    use std::collections::HashMap;

    fn message(metadata: HashMap<String, serde_json::Value>) -> ChannelMessage {
        ChannelMessage {
            channel: ChannelType::Slack,
            sender: ChannelUser {
                platform_id: "channel-42".to_string(),
                display_name: "Alex".to_string(),
                captain_user: None,
            },
            content: ChannelContent::Text("hello".to_string()),
            target_agent: None,
            timestamp: chrono::Utc::now(),
            is_group: true,
            thread_id: None,
            metadata,
            platform_message_id: "msg-1".to_string(),
        }
    }

    #[test]
    fn sender_user_id_prefers_metadata_user_id() {
        let mut metadata = HashMap::new();
        metadata.insert("sender_user_id".to_string(), serde_json::json!("user-99"));

        assert_eq!(sender_user_id(&message(metadata)), "user-99");
    }

    #[test]
    fn sender_user_id_falls_back_to_platform_id() {
        assert_eq!(sender_user_id(&message(HashMap::new())), "channel-42");
    }

    #[test]
    fn sender_user_id_ignores_non_string_metadata() {
        let mut metadata = HashMap::new();
        metadata.insert("sender_user_id".to_string(), serde_json::json!(42));

        assert_eq!(sender_user_id(&message(metadata)), "channel-42");
    }
}
