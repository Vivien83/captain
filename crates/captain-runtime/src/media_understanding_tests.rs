use super::*;
use captain_types::media::{MediaSource, MAX_IMAGE_BYTES};

#[test]
fn test_engine_creation() {
    let config = MediaConfig::default();
    let engine = MediaEngine::new(config);
    assert_eq!(engine.config.max_concurrency, 5);
}

#[test]
fn test_engine_max_concurrency_clamped() {
    let config = MediaConfig {
        max_concurrency: 100,
        ..Default::default()
    };
    let engine = MediaEngine::new(config);
    assert!(engine.semaphore.available_permits() <= 8);
}

#[test]
fn test_audio_model_override_is_optional_config_surface() {
    let config = MediaConfig {
        audio_provider: Some("groq".to_string()),
        audio_model: Some("whisper-large-v3-turbo".to_string()),
        ..Default::default()
    };
    let engine = MediaEngine::new(config);
    assert_eq!(
        engine.config.audio_model.as_deref(),
        Some("whisper-large-v3-turbo")
    );
}

#[tokio::test]
async fn test_describe_image_wrong_type() {
    let engine = MediaEngine::new(MediaConfig::default());
    let attachment = MediaAttachment {
        media_type: MediaType::Audio,
        mime_type: "audio/mpeg".into(),
        source: MediaSource::FilePath {
            path: "test.mp3".into(),
        },
        size_bytes: 1024,
        context_hint: None,
        batch_size_hint: None,
    };
    let result = engine.describe_image(&attachment).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Expected image"));
}

#[tokio::test]
async fn test_describe_image_invalid_mime() {
    let engine = MediaEngine::new(MediaConfig::default());
    let attachment = MediaAttachment {
        media_type: MediaType::Image,
        mime_type: "application/pdf".into(),
        source: MediaSource::FilePath {
            path: "test.pdf".into(),
        },
        size_bytes: 1024,
        context_hint: None,
        batch_size_hint: None,
    };
    let result = engine.describe_image(&attachment).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_describe_image_too_large() {
    let engine = MediaEngine::new(MediaConfig::default());
    let attachment = MediaAttachment {
        media_type: MediaType::Image,
        mime_type: "image/png".into(),
        source: MediaSource::FilePath {
            path: "big.png".into(),
        },
        size_bytes: MAX_IMAGE_BYTES + 1,
        context_hint: None,
        batch_size_hint: None,
    };
    let result = engine.describe_image(&attachment).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_transcribe_audio_wrong_type() {
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
}

#[tokio::test]
async fn test_video_disabled() {
    let config = MediaConfig {
        video_description: false,
        ..Default::default()
    };
    let engine = MediaEngine::new(config);
    let attachment = MediaAttachment {
        media_type: MediaType::Video,
        mime_type: "video/mp4".into(),
        source: MediaSource::FilePath {
            path: "test.mp4".into(),
        },
        size_bytes: 1024,
        context_hint: None,
        batch_size_hint: None,
    };
    let result = engine.describe_video(&attachment).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("disabled"));
}
