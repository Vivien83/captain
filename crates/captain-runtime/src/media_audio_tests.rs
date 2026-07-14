use super::*;
use crate::media_understanding::MediaEngine;

#[test]
fn test_default_audio_models() {
    assert_eq!(
        default_audio_model("parakeet-mlx"),
        "mlx-community/parakeet-tdt-0.6b-v3"
    );
    assert_eq!(default_audio_model("groq"), "whisper-large-v3-turbo");
    assert_eq!(default_audio_model("openai"), "whisper-1");
    assert_eq!(default_audio_model("elevenlabs"), "scribe_v2");
    assert_eq!(default_audio_model("unknown"), "unknown");
}

#[test]
fn test_audio_language_hint() {
    let attachment = MediaAttachment {
        media_type: MediaType::Audio,
        mime_type: "audio/ogg".into(),
        source: MediaSource::FilePath {
            path: "voice.ogg".into(),
        },
        size_bytes: 1024,
        context_hint: Some("language:fr".to_string()),
        batch_size_hint: None,
    };
    assert_eq!(audio_language_hint(&attachment).as_deref(), Some("fr"));
}

#[tokio::test]
async fn test_transcribe_audio_rejects_image_type() {
    let engine = MediaEngine::new(MediaConfig::default());
    let attachment = MediaAttachment {
        media_type: MediaType::Image,
        mime_type: "image/png".into(),
        source: MediaSource::FilePath {
            path: "test.png".into(),
        },
        size_bytes: 1024,
        context_hint: None,
        batch_size_hint: None,
    };
    let result = engine.transcribe_audio(&attachment).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Expected audio"));
}

#[tokio::test]
async fn test_transcribe_audio_no_provider() {
    let engine = MediaEngine::new(MediaConfig::default());
    let attachment = MediaAttachment {
        media_type: MediaType::Audio,
        mime_type: "audio/webm".into(),
        source: MediaSource::FilePath {
            path: "test.webm".into(),
        },
        size_bytes: 1024,
        context_hint: None,
        batch_size_hint: None,
    };
    let result = engine.transcribe_audio(&attachment).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_transcribe_audio_url_source_rejected() {
    let config = MediaConfig {
        audio_provider: Some("groq".to_string()),
        ..Default::default()
    };
    let engine = MediaEngine::new(config);
    let attachment = MediaAttachment {
        media_type: MediaType::Audio,
        mime_type: "audio/mpeg".into(),
        source: MediaSource::Url {
            url: "https://example.com/audio.mp3".into(),
        },
        size_bytes: 1024,
        context_hint: None,
        batch_size_hint: None,
    };
    let result = engine.transcribe_audio(&attachment).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .contains("URL-based audio source not supported"));
}

#[tokio::test]
async fn test_transcribe_audio_file_not_found() {
    let config = MediaConfig {
        audio_provider: Some("groq".to_string()),
        ..Default::default()
    };
    let engine = MediaEngine::new(config);
    let attachment = MediaAttachment {
        media_type: MediaType::Audio,
        mime_type: "audio/webm".into(),
        source: MediaSource::FilePath {
            path: "/nonexistent/path/audio.webm".into(),
        },
        size_bytes: 1024,
        context_hint: None,
        batch_size_hint: None,
    };
    let result = engine.transcribe_audio(&attachment).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Failed to read audio file"));
}
