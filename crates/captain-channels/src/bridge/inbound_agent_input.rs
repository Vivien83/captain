//! Prepared agent-facing input for inbound channel messages.

use super::inbound_image_blocks::prepare_image_blocks_for_agent;
use super::inbound_media::prepare_inbound_media_content;
use super::inbound_prompt::{build_inbound_message_text, InboundPromptContext};
use super::ChannelBridgeHandle;
use crate::types::ChannelMessage;
use captain_types::message::ContentBlock;
use std::sync::Arc;

pub(super) struct InboundAgentInput {
    pub(super) text: String,
    pub(super) image_blocks_for_agent: Option<Vec<ContentBlock>>,
}

pub(super) async fn prepare_inbound_agent_input(
    handle: &Arc<dyn ChannelBridgeHandle>,
    message: &ChannelMessage,
    channel_type: &str,
) -> InboundAgentInput {
    let media = prepare_inbound_media_content(handle, &message.content, channel_type).await;

    let text = build_inbound_message_text(InboundPromptContext {
        channel_type,
        content: &message.content,
        image_file: media.image.file.as_ref(),
        image_description: media.image.description.as_deref(),
        image_processing_error: media.image.processing_error.as_deref(),
        voice_transcript: media.voice.transcript.as_deref(),
        voice_local_path: media.voice.local_path.as_deref(),
        voice_transcription_error: media.voice.transcription_error.as_deref(),
        video_local_path: media.video_local_path.as_deref(),
    });

    let image_blocks_for_agent = prepare_image_blocks_for_agent(
        &message.channel,
        &message.content,
        media.image.file.as_ref(),
        &text,
        channel_type,
    )
    .await;

    InboundAgentInput {
        text,
        image_blocks_for_agent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelContent, ChannelType, ChannelUser};
    use async_trait::async_trait;
    use captain_types::agent::AgentId;
    use std::collections::HashMap;

    struct MockInputHandle;

    #[async_trait]
    impl ChannelBridgeHandle for MockInputHandle {
        async fn send_message(
            &self,
            _agent_id: AgentId,
            message: &str,
            _channel_type: Option<&str>,
        ) -> Result<String, String> {
            Ok(message.to_string())
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

    fn message(content: ChannelContent) -> ChannelMessage {
        ChannelMessage {
            channel: ChannelType::Telegram,
            platform_message_id: "m1".to_string(),
            sender: ChannelUser {
                platform_id: "u1".to_string(),
                display_name: "Ada".to_string(),
                captain_user: None,
            },
            content,
            target_agent: None,
            timestamp: chrono::Utc::now(),
            is_group: false,
            thread_id: Some("topic-1".to_string()),
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn agent_input_passes_through_text_without_image_blocks() {
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockInputHandle);
        let input = prepare_inbound_agent_input(
            &handle,
            &message(ChannelContent::Text("bonjour".to_string())),
            "telegram",
        )
        .await;

        assert_eq!(input.text, "bonjour");
        assert!(input.image_blocks_for_agent.is_none());
    }

    #[tokio::test]
    async fn agent_input_formats_file_fallback_without_image_blocks() {
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockInputHandle);
        let input = prepare_inbound_agent_input(
            &handle,
            &message(ChannelContent::File {
                url: "https://example.test/file.pdf".to_string(),
                filename: "file.pdf".to_string(),
            }),
            "telegram",
        )
        .await;

        assert_eq!(
            input.text,
            "[User sent a file (file.pdf): https://example.test/file.pdf]"
        );
        assert!(input.image_blocks_for_agent.is_none());
    }
}
