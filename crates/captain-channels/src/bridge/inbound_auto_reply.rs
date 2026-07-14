//! Auto-reply handling for inbound channel turns.

use super::inbound_delivery::record_inbound_delivery_success;
use super::{send_response, ChannelBridgeHandle};
use crate::types::{ChannelAdapter, ChannelMessage};
use captain_types::agent::AgentId;
use captain_types::config::OutputFormat;
use std::sync::Arc;

#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_auto_reply(
    handle: &Arc<dyn ChannelBridgeHandle>,
    adapter: &dyn ChannelAdapter,
    message: &ChannelMessage,
    agent_id: AgentId,
    text: &str,
    channel_type: &str,
    thread_id: Option<&str>,
    output_format: OutputFormat,
) -> bool {
    let Some(reply) = handle.check_auto_reply(agent_id, text).await else {
        return false;
    };

    send_response(adapter, &message.sender, reply, thread_id, output_format).await;
    record_inbound_delivery_success(handle, agent_id, channel_type, message, thread_id).await;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelContent, ChannelType, ChannelUser};
    use async_trait::async_trait;
    use futures::{stream, Stream};
    use std::collections::HashMap;
    use std::pin::Pin;
    use std::sync::Mutex;

    struct MockAutoReplyHandle {
        reply: Mutex<Option<String>>,
        deliveries: Mutex<
            Vec<(
                AgentId,
                String,
                String,
                bool,
                Option<String>,
                Option<String>,
            )>,
        >,
    }

    #[async_trait]
    impl ChannelBridgeHandle for MockAutoReplyHandle {
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

        async fn check_auto_reply(&self, _agent_id: AgentId, _message: &str) -> Option<String> {
            self.reply.lock().unwrap().clone()
        }
    }

    struct RecordingAdapter {
        sent: Mutex<Vec<(String, Option<String>)>>,
    }

    #[async_trait]
    impl ChannelAdapter for RecordingAdapter {
        fn name(&self) -> &str {
            "recording"
        }

        fn channel_type(&self) -> ChannelType {
            ChannelType::Telegram
        }

        async fn start(
            &self,
        ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, Box<dyn std::error::Error>>
        {
            Ok(Box::pin(stream::empty()))
        }

        async fn send(
            &self,
            _user: &ChannelUser,
            content: ChannelContent,
        ) -> Result<(), Box<dyn std::error::Error>> {
            if let ChannelContent::Text(text) = content {
                self.sent.lock().unwrap().push((text, None));
            }
            Ok(())
        }

        async fn send_in_thread(
            &self,
            _user: &ChannelUser,
            content: ChannelContent,
            thread_id: &str,
        ) -> Result<(), Box<dyn std::error::Error>> {
            if let ChannelContent::Text(text) = content {
                self.sent
                    .lock()
                    .unwrap()
                    .push((text, Some(thread_id.to_string())));
            }
            Ok(())
        }

        async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
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

    fn handle(reply: Option<&str>) -> Arc<MockAutoReplyHandle> {
        Arc::new(MockAutoReplyHandle {
            reply: Mutex::new(reply.map(str::to_string)),
            deliveries: Mutex::new(Vec::new()),
        })
    }

    #[tokio::test]
    async fn auto_reply_sends_response_and_records_successful_delivery() {
        let mock_handle = handle(Some("Je m'en occupe."));
        let handle: Arc<dyn ChannelBridgeHandle> = mock_handle.clone();
        let adapter = RecordingAdapter {
            sent: Mutex::new(Vec::new()),
        };
        let agent_id = AgentId::new();

        let handled = handle_auto_reply(
            &handle,
            &adapter,
            &test_message(),
            agent_id,
            "hello",
            "telegram",
            Some("topic-7"),
            OutputFormat::PlainText,
        )
        .await;

        assert!(handled);
        assert_eq!(
            adapter.sent.lock().unwrap().as_slice(),
            &[("Je m'en occupe.".to_string(), Some("topic-7".to_string()))]
        );
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
    async fn auto_reply_returns_false_without_side_effects_when_disabled() {
        let mock_handle = handle(None);
        let handle: Arc<dyn ChannelBridgeHandle> = mock_handle.clone();
        let adapter = RecordingAdapter {
            sent: Mutex::new(Vec::new()),
        };

        let handled = handle_auto_reply(
            &handle,
            &adapter,
            &test_message(),
            AgentId::new(),
            "hello",
            "telegram",
            None,
            OutputFormat::PlainText,
        )
        .await;

        assert!(!handled);
        assert!(adapter.sent.lock().unwrap().is_empty());
        assert!(mock_handle.deliveries.lock().unwrap().is_empty());
    }
}
