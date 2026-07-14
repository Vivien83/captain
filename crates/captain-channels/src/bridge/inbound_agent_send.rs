//! Send an inbound channel turn to the selected agent.

use super::ChannelBridgeHandle;
use crate::types::{ChannelAdapter, ChannelMessage, ChannelType};
use captain_types::agent::AgentId;
use captain_types::message::ContentBlock;
use std::sync::Arc;

pub(super) struct InboundAgentSendOutcome {
    pub(super) result: Result<String, String>,
    pub(super) posted_inline: bool,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn send_inbound_agent_message(
    handle: &Arc<dyn ChannelBridgeHandle>,
    adapter_arc: &Arc<dyn ChannelAdapter>,
    message: &ChannelMessage,
    agent_id: AgentId,
    image_blocks_for_agent: Option<&[ContentBlock]>,
    final_text: &str,
    channel_type: &str,
    thread_id: Option<&str>,
    active_session_key: Option<&str>,
) -> InboundAgentSendOutcome {
    if let Some(blocks) = image_blocks_for_agent {
        return InboundAgentSendOutcome {
            result: handle
                .send_message_with_blocks(agent_id, blocks.to_vec())
                .await,
            posted_inline: false,
        };
    }

    if matches!(message.channel, ChannelType::Telegram) {
        let telegram = adapter_arc.clone().as_telegram_arc();
        let chat_id = message.sender.platform_id.parse::<i64>().ok();
        if let (Some(telegram), Some(chat_id)) = (telegram, chat_id) {
            let thread_id = thread_id.and_then(|value| value.parse::<i64>().ok());
            let user_message_id = message.platform_message_id.parse::<i64>().ok();
            if let Some(result) = handle
                .try_stream_telegram_response(
                    telegram,
                    chat_id,
                    thread_id,
                    user_message_id,
                    agent_id,
                    active_session_key,
                    final_text,
                )
                .await
            {
                return InboundAgentSendOutcome {
                    result,
                    posted_inline: true,
                };
            }
        }
    }

    InboundAgentSendOutcome {
        result: handle
            .send_message(agent_id, final_text, Some(channel_type))
            .await,
        posted_inline: false,
    }
}

pub(super) async fn retry_inbound_agent_message(
    handle: &Arc<dyn ChannelBridgeHandle>,
    agent_id: AgentId,
    image_blocks_for_agent: Option<&[ContentBlock]>,
    text: &str,
    channel_type: &str,
) -> Result<String, String> {
    if let Some(blocks) = image_blocks_for_agent {
        handle
            .send_message_with_blocks(agent_id, blocks.to_vec())
            .await
    } else {
        handle
            .send_message(agent_id, text, Some(channel_type))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelContent, ChannelUser};
    use async_trait::async_trait;
    use futures::stream;
    use futures::Stream;
    use std::collections::HashMap;
    use std::pin::Pin;
    use std::sync::Mutex;

    struct MockSendHandle {
        text_calls: Mutex<Vec<(AgentId, String, Option<String>)>>,
        block_calls: Mutex<usize>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for MockSendHandle {
        async fn send_message(
            &self,
            agent_id: AgentId,
            message: &str,
            channel_type: Option<&str>,
        ) -> Result<String, String> {
            self.text_calls.lock().unwrap().push((
                agent_id,
                message.to_string(),
                channel_type.map(str::to_string),
            ));
            Ok(format!("text:{message}"))
        }

        async fn send_message_with_blocks(
            &self,
            _agent_id: AgentId,
            blocks: Vec<ContentBlock>,
        ) -> Result<String, String> {
            *self.block_calls.lock().unwrap() += 1;
            Ok(format!("blocks:{}", blocks.len()))
        }

        async fn find_agent_by_name(&self, _name: &str) -> Result<Option<AgentId>, String> {
            Ok(None)
        }

        async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
            Ok(Vec::new())
        }

        async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
            Err("not implemented".to_string())
        }
    }

    struct DummyAdapter;

    #[async_trait]
    impl ChannelAdapter for DummyAdapter {
        fn name(&self) -> &str {
            "dummy"
        }

        fn channel_type(&self) -> ChannelType {
            ChannelType::Discord
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
            _content: ChannelContent,
        ) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }

        async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }
    }

    fn test_message(channel: ChannelType) -> ChannelMessage {
        ChannelMessage {
            channel,
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
            thread_id: Some("7".to_string()),
            metadata: HashMap::new(),
        }
    }

    fn handle() -> Arc<dyn ChannelBridgeHandle> {
        Arc::new(MockSendHandle {
            text_calls: Mutex::new(Vec::new()),
            block_calls: Mutex::new(0),
        })
    }

    #[tokio::test]
    async fn inbound_send_prefers_image_blocks_when_available() {
        let handle = handle();
        let adapter: Arc<dyn ChannelAdapter> = Arc::new(DummyAdapter);
        let agent_id = AgentId::new();
        let blocks = vec![ContentBlock::Text {
            text: "with image".to_string(),
            provider_metadata: None,
        }];

        let outcome = send_inbound_agent_message(
            &handle,
            &adapter,
            &test_message(ChannelType::Telegram),
            agent_id,
            Some(&blocks),
            "plain",
            "telegram",
            Some("7"),
            Some("telegram:user"),
        )
        .await;

        assert_eq!(outcome.result.unwrap(), "blocks:1");
        assert!(!outcome.posted_inline);
    }

    #[tokio::test]
    async fn inbound_send_falls_back_to_plain_message_for_non_telegram() {
        let handle = handle();
        let adapter: Arc<dyn ChannelAdapter> = Arc::new(DummyAdapter);
        let agent_id = AgentId::new();

        let outcome = send_inbound_agent_message(
            &handle,
            &adapter,
            &test_message(ChannelType::Discord),
            agent_id,
            None,
            "plain",
            "discord",
            None,
            None,
        )
        .await;

        assert_eq!(outcome.result.unwrap(), "text:plain");
        assert!(!outcome.posted_inline);
    }

    #[tokio::test]
    async fn inbound_send_falls_back_when_telegram_adapter_has_no_streaming_identity() {
        let handle = handle();
        let adapter: Arc<dyn ChannelAdapter> = Arc::new(DummyAdapter);
        let agent_id = AgentId::new();

        let outcome = send_inbound_agent_message(
            &handle,
            &adapter,
            &test_message(ChannelType::Telegram),
            agent_id,
            None,
            "plain",
            "telegram",
            Some("7"),
            Some("telegram:user"),
        )
        .await;

        assert_eq!(outcome.result.unwrap(), "text:plain");
        assert!(!outcome.posted_inline);
    }

    #[tokio::test]
    async fn inbound_retry_prefers_image_blocks_when_available() {
        let handle = handle();
        let agent_id = AgentId::new();
        let blocks = vec![ContentBlock::Text {
            text: "with image".to_string(),
            provider_metadata: None,
        }];

        let response =
            retry_inbound_agent_message(&handle, agent_id, Some(&blocks), "plain", "telegram")
                .await
                .unwrap();

        assert_eq!(response, "blocks:1");
    }

    #[tokio::test]
    async fn inbound_retry_falls_back_to_plain_message_with_channel_type() {
        let mock_handle = Arc::new(MockSendHandle {
            text_calls: Mutex::new(Vec::new()),
            block_calls: Mutex::new(0),
        });
        let handle: Arc<dyn ChannelBridgeHandle> = mock_handle.clone();
        let agent_id = AgentId::new();

        let response = retry_inbound_agent_message(&handle, agent_id, None, "plain", "telegram")
            .await
            .unwrap();

        assert_eq!(response, "text:plain");
        assert_eq!(
            mock_handle.text_calls.lock().unwrap().as_slice(),
            &[(agent_id, "plain".to_string(), Some("telegram".to_string()))]
        );
    }
}
