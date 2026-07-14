//! Aggregate inbound media preparation before prompt construction.

use super::inbound_audio::{prepare_inbound_voice_content, InboundVoicePreparation};
use super::inbound_image::{prepare_inbound_image_content, InboundImagePreparation};
use super::inbound_media_path::captain_inbound_dir;
use super::inbound_video::prepare_inbound_video_local_path;
use super::ChannelBridgeHandle;
use crate::types::ChannelContent;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Default)]
pub(super) struct InboundMediaPreparation {
    pub(super) image: InboundImagePreparation,
    pub(super) video_local_path: Option<PathBuf>,
    pub(super) voice: InboundVoicePreparation,
}

pub(super) async fn prepare_inbound_media_content(
    handle: &Arc<dyn ChannelBridgeHandle>,
    content: &ChannelContent,
    channel_type: &str,
) -> InboundMediaPreparation {
    let dest_dir = captain_inbound_dir(channel_type);

    // Preserve the historical preparation order: image description, video
    // download, then voice transcription.
    let image = prepare_inbound_image_content(handle, content, &dest_dir, channel_type).await;
    let video_local_path = prepare_inbound_video_local_path(content, &dest_dir, channel_type).await;
    let voice = prepare_inbound_voice_content(handle, content, &dest_dir, channel_type).await;

    InboundMediaPreparation {
        image,
        video_local_path,
        voice,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use captain_types::agent::AgentId;

    struct MockMediaHandle;

    #[async_trait]
    impl ChannelBridgeHandle for MockMediaHandle {
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

    #[tokio::test]
    async fn media_preparation_ignores_plain_text_content() {
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockMediaHandle);

        let media = prepare_inbound_media_content(
            &handle,
            &ChannelContent::Text("hello".to_string()),
            "telegram",
        )
        .await;

        assert!(media.image.file.is_none());
        assert!(media.image.description.is_none());
        assert!(media.image.processing_error.is_none());
        assert!(media.video_local_path.is_none());
        assert!(media.voice.local_path.is_none());
        assert!(media.voice.transcript.is_none());
        assert!(media.voice.transcription_error.is_none());
    }
}
