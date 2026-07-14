//! Delivery audit helpers for inbound channel turns.

use super::ChannelBridgeHandle;
use crate::types::ChannelMessage;
use captain_types::agent::AgentId;
use std::sync::Arc;

pub(super) async fn record_inbound_delivery_success(
    handle: &Arc<dyn ChannelBridgeHandle>,
    agent_id: AgentId,
    channel_type: &str,
    message: &ChannelMessage,
    thread_id: Option<&str>,
) {
    handle
        .record_delivery(
            agent_id,
            channel_type,
            &message.sender.platform_id,
            true,
            None,
            thread_id,
        )
        .await;
}

pub(super) async fn record_inbound_delivery_failure(
    handle: &Arc<dyn ChannelBridgeHandle>,
    agent_id: AgentId,
    channel_type: &str,
    message: &ChannelMessage,
    error: &str,
    thread_id: Option<&str>,
) {
    handle
        .record_delivery(
            agent_id,
            channel_type,
            &message.sender.platform_id,
            false,
            Some(error),
            thread_id,
        )
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelContent, ChannelType, ChannelUser};
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Mutex;

    type DeliveryRecord = (
        AgentId,
        String,
        String,
        bool,
        Option<String>,
        Option<String>,
    );

    struct MockDeliveryHandle {
        deliveries: Mutex<Vec<DeliveryRecord>>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for MockDeliveryHandle {
        async fn send_message(
            &self,
            _agent_id: AgentId,
            _message: &str,
            _channel_type: Option<&str>,
        ) -> Result<String, String> {
            Ok(String::new())
        }

        async fn find_agent_by_name(&self, _name: &str) -> Result<Option<AgentId>, String> {
            Ok(None)
        }

        async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
            Ok(Vec::new())
        }

        async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
            Err("not available".to_string())
        }

        async fn record_delivery(
            &self,
            agent_id: AgentId,
            channel: &str,
            recipient: &str,
            success: bool,
            error: Option<&str>,
            thread_id: Option<&str>,
        ) {
            self.deliveries.lock().unwrap().push((
                agent_id,
                channel.to_string(),
                recipient.to_string(),
                success,
                error.map(str::to_string),
                thread_id.map(str::to_string),
            ));
        }
    }

    fn test_message() -> ChannelMessage {
        ChannelMessage {
            channel: ChannelType::Telegram,
            platform_message_id: "42".to_string(),
            sender: ChannelUser {
                platform_id: "1001".to_string(),
                display_name: "Ada".to_string(),
                captain_user: None,
            },
            content: ChannelContent::Text("hello".to_string()),
            target_agent: None,
            timestamp: chrono::Utc::now(),
            is_group: false,
            thread_id: Some("topic-7".to_string()),
            metadata: HashMap::new(),
        }
    }

    fn handle() -> Arc<MockDeliveryHandle> {
        Arc::new(MockDeliveryHandle {
            deliveries: Mutex::new(Vec::new()),
        })
    }

    #[tokio::test]
    async fn success_records_message_recipient_and_thread() {
        let mock_handle = handle();
        let handle: Arc<dyn ChannelBridgeHandle> = mock_handle.clone();
        let agent_id = AgentId::new();

        record_inbound_delivery_success(
            &handle,
            agent_id,
            "telegram",
            &test_message(),
            Some("topic-7"),
        )
        .await;

        assert_eq!(
            mock_handle.deliveries.lock().unwrap().as_slice(),
            &[(
                agent_id,
                "telegram".to_string(),
                "1001".to_string(),
                true,
                None,
                Some("topic-7".to_string())
            )]
        );
    }

    #[tokio::test]
    async fn failure_records_error_payload_from_caller() {
        let mock_handle = handle();
        let handle: Arc<dyn ChannelBridgeHandle> = mock_handle.clone();
        let agent_id = AgentId::new();

        record_inbound_delivery_failure(
            &handle,
            agent_id,
            "discord",
            &test_message(),
            "Backend unavailable",
            None,
        )
        .await;

        assert_eq!(
            mock_handle.deliveries.lock().unwrap().as_slice(),
            &[(
                agent_id,
                "discord".to_string(),
                "1001".to_string(),
                false,
                Some("Backend unavailable".to_string()),
                None
            )]
        );
    }
}
