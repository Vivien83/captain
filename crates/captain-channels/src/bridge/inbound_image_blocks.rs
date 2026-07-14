//! Inline content-block support for inbound images.

use super::inbound_image::{image_file_to_blocks, InboundImageFile};
use crate::types::{ChannelContent, ChannelType};
use captain_types::message::ContentBlock;
use tracing::warn;

pub(super) async fn build_image_blocks_for_agent(
    channel: &ChannelType,
    content: &ChannelContent,
    image_file: Option<&InboundImageFile>,
    text: &str,
) -> Result<Option<Vec<ContentBlock>>, String> {
    if !matches!(channel, ChannelType::Telegram) || !matches!(content, ChannelContent::Image { .. })
    {
        return Ok(None);
    }
    let Some(file) = image_file else {
        return Ok(None);
    };
    image_file_to_blocks(file, text).await.map(Some)
}

pub(super) async fn prepare_image_blocks_for_agent(
    channel: &ChannelType,
    content: &ChannelContent,
    image_file: Option<&InboundImageFile>,
    text: &str,
    channel_type: &str,
) -> Option<Vec<ContentBlock>> {
    match build_image_blocks_for_agent(channel, content, image_file, text).await {
        Ok(blocks) => blocks,
        Err(e) => {
            warn!("Inbound image block build failed for {channel_type}: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_content() -> ChannelContent {
        ChannelContent::Image {
            url: "https://example.test/photo.png".to_string(),
            caption: None,
        }
    }

    #[tokio::test]
    async fn image_blocks_only_apply_to_telegram_images() {
        assert!(build_image_blocks_for_agent(
            &ChannelType::Discord,
            &image_content(),
            None,
            "texte"
        )
        .await
        .unwrap()
        .is_none());
        assert!(build_image_blocks_for_agent(
            &ChannelType::Telegram,
            &ChannelContent::Text("hello".to_string()),
            None,
            "texte",
        )
        .await
        .unwrap()
        .is_none());
    }

    #[tokio::test]
    async fn image_blocks_skip_when_download_failed() {
        assert!(build_image_blocks_for_agent(
            &ChannelType::Telegram,
            &image_content(),
            None,
            "texte"
        )
        .await
        .unwrap()
        .is_none());
    }

    #[tokio::test]
    async fn image_blocks_include_text_and_local_image_data() {
        let path =
            std::env::temp_dir().join(format!("captain-inline-{}.png", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, &[1_u8, 2, 3]).await.unwrap();
        let file = InboundImageFile {
            path: path.clone(),
            mime_type: "image/png".to_string(),
            size_bytes: 3,
        };

        let blocks = build_image_blocks_for_agent(
            &ChannelType::Telegram,
            &image_content(),
            Some(&file),
            "texte",
        )
        .await
        .unwrap()
        .expect("telegram image builds blocks");

        assert!(matches!(blocks.first(), Some(ContentBlock::Text { text, .. }) if text == "texte"));
        assert!(
            matches!(blocks.get(1), Some(ContentBlock::Image { media_type, data }) if media_type == "image/png" && data == "AQID")
        );

        let _ = tokio::fs::remove_file(path).await;
    }

    #[tokio::test]
    async fn prepare_image_blocks_suppresses_read_errors() {
        let file = InboundImageFile {
            path: std::env::temp_dir().join(format!(
                "captain-inline-missing-{}.png",
                uuid::Uuid::new_v4()
            )),
            mime_type: "image/png".to_string(),
            size_bytes: 3,
        };

        let blocks = prepare_image_blocks_for_agent(
            &ChannelType::Telegram,
            &image_content(),
            Some(&file),
            "texte",
            "telegram",
        )
        .await;

        assert!(blocks.is_none());
    }
}
