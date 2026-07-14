//! RAII cleanup for active inbound sessions.

use crate::inbound_queue::InboundSessionQueue;

pub(super) struct InboundSessionCleanup {
    queue: Option<InboundSessionQueue>,
    key: String,
}

impl InboundSessionCleanup {
    pub(super) fn new(queue: InboundSessionQueue, key: String) -> Self {
        Self {
            queue: Some(queue),
            key,
        }
    }

    pub(super) fn disarm(&mut self) {
        self.queue = None;
    }
}

impl Drop for InboundSessionCleanup {
    fn drop(&mut self) {
        if let Some(queue) = self.queue.take() {
            queue.clear(&self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbound_queue_types::InboundStart;
    use crate::types::{ChannelContent, ChannelMessage, ChannelType, ChannelUser};
    use std::collections::HashMap;

    fn message() -> ChannelMessage {
        ChannelMessage {
            channel: ChannelType::Telegram,
            platform_message_id: "m1".to_string(),
            sender: ChannelUser {
                platform_id: "chat-1".to_string(),
                display_name: "Ada".to_string(),
                captain_user: None,
            },
            content: ChannelContent::Text("hello".to_string()),
            target_agent: None,
            timestamp: chrono::Utc::now(),
            is_group: false,
            thread_id: None,
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn cleanup_clears_session_on_drop() {
        let queue = InboundSessionQueue::default();
        let key = "telegram|chat:chat-1|user:user-1|captain:-|thread:-".to_string();
        assert!(matches!(
            queue.start_or_queue(key.clone(), message()),
            InboundStart::Started { .. }
        ));
        assert_eq!(queue.active_len(), 1);

        drop(InboundSessionCleanup::new(queue.clone(), key));

        assert_eq!(queue.active_len(), 0);
    }

    #[test]
    fn disarmed_cleanup_leaves_session_to_queue_flow() {
        let queue = InboundSessionQueue::default();
        let key = "telegram|chat:chat-1|user:user-1|captain:-|thread:-".to_string();
        assert!(matches!(
            queue.start_or_queue(key.clone(), message()),
            InboundStart::Started { .. }
        ));
        let mut cleanup = InboundSessionCleanup::new(queue.clone(), key);

        cleanup.disarm();
        drop(cleanup);

        assert_eq!(queue.active_len(), 1);
    }
}
