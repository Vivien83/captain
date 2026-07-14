//! Agent prompt text for inbound channel messages after media preprocessing.

use super::inbound_audio::build_inbound_voice_prompt;
use super::inbound_image::{build_inbound_image_prompt, InboundImageFile};
use super::inbound_video::build_inbound_video_prompt;
use crate::types::ChannelContent;
use std::path::Path;

pub(super) struct InboundPromptContext<'a> {
    pub(super) channel_type: &'a str,
    pub(super) content: &'a ChannelContent,
    pub(super) image_file: Option<&'a InboundImageFile>,
    pub(super) image_description: Option<&'a str>,
    pub(super) image_processing_error: Option<&'a str>,
    pub(super) voice_transcript: Option<&'a str>,
    pub(super) voice_local_path: Option<&'a Path>,
    pub(super) voice_transcription_error: Option<&'a str>,
    pub(super) video_local_path: Option<&'a Path>,
}

pub(super) fn build_inbound_message_text(context: InboundPromptContext<'_>) -> String {
    match context.content {
        ChannelContent::Text(text) => text.clone(),
        ChannelContent::Command { .. } => unreachable!("commands are handled before prompt build"),
        ChannelContent::Image { url, caption } => build_inbound_image_prompt(
            context.channel_type,
            url,
            caption.as_deref(),
            context.image_file,
            context.image_description,
            context.image_processing_error,
        ),
        ChannelContent::File { url, filename } => {
            format!("[User sent a file ({filename}): {url}]")
        }
        ChannelContent::Voice {
            url,
            duration_seconds,
        } => build_inbound_voice_prompt(
            url,
            *duration_seconds,
            context.voice_transcript,
            context.voice_local_path,
            context.voice_transcription_error,
        ),
        ChannelContent::Video {
            url,
            duration_seconds,
            caption,
        } => build_inbound_video_prompt(
            url,
            *duration_seconds,
            caption.as_deref(),
            context.video_local_path,
        ),
        ChannelContent::Location { lat, lon } => {
            format!("[User shared location: {lat}, {lon}]")
        }
        ChannelContent::FileData { filename, .. } => {
            format!("[User sent a local file: {filename}]")
        }
        ChannelContent::ImageData {
            mime_type, caption, ..
        } => match caption {
            Some(caption) => {
                format!("[User sent a local image ({mime_type}) — caption: {caption}]")
            }
            None => format!("[User sent a local image ({mime_type})]"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt_for(content: ChannelContent) -> String {
        build_inbound_message_text(InboundPromptContext {
            channel_type: "telegram",
            content: &content,
            image_file: None,
            image_description: None,
            image_processing_error: None,
            voice_transcript: None,
            voice_local_path: None,
            voice_transcription_error: None,
            video_local_path: None,
        })
    }

    #[test]
    fn prompt_text_passes_through_plain_text() {
        assert_eq!(
            prompt_for(ChannelContent::Text("bonjour".to_string())),
            "bonjour"
        );
    }

    #[test]
    fn prompt_formats_file_and_location_fallbacks() {
        assert_eq!(
            prompt_for(ChannelContent::File {
                url: "https://example.test/file.pdf".to_string(),
                filename: "file.pdf".to_string(),
            }),
            "[User sent a file (file.pdf): https://example.test/file.pdf]"
        );
        assert_eq!(
            prompt_for(ChannelContent::Location {
                lat: 48.8566,
                lon: 2.3522,
            }),
            "[User shared location: 48.8566, 2.3522]"
        );
    }

    #[test]
    fn prompt_formats_local_image_data_caption() {
        assert_eq!(
            prompt_for(ChannelContent::ImageData {
                mime_type: "image/png".to_string(),
                data: vec![1, 2, 3],
                caption: Some("schema".to_string()),
            }),
            "[User sent a local image (image/png) — caption: schema]"
        );
    }

    #[test]
    fn prompt_routes_voice_and_video_to_media_prompt_builders() {
        let voice = prompt_for(ChannelContent::Voice {
            url: "https://example.test/voice.ogg".to_string(),
            duration_seconds: 9,
        });
        assert!(voice.contains("[Message vocal reçu depuis Telegram"));
        assert!(voice.contains("Durée: 9s"));

        let video = prompt_for(ChannelContent::Video {
            url: "https://example.test/video.mp4".to_string(),
            duration_seconds: 7,
            caption: Some("analyse".to_string()),
        });
        assert!(video.contains("[Vidéo reçue depuis Telegram"));
        assert!(video.contains("Légende: analyse"));
    }
}
