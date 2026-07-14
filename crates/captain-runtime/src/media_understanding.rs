//! Media understanding engine - image description, audio transcription, video analysis.
//!
//! Auto-cascades through available providers based on configured API keys.

use crate::media_cache::VisionCache;
use crate::{media_audio, media_vision};
use captain_types::media::{
    MediaAttachment, MediaConfig, MediaSource, MediaType, MediaUnderstanding,
};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{debug, info};

/// Media understanding engine.
///
/// `Clone` is intentional and cheap: every interior field is either
/// `Clone` itself (`MediaConfig`) or wrapped in `Arc`. Cloning is what
/// lets the V.8f per-frame describe loop in `tool_video_analyze` spawn
/// a task per frame while keeping a single shared semaphore + cache.
#[derive(Clone)]
pub struct MediaEngine {
    config: MediaConfig,
    semaphore: Arc<Semaphore>,
    /// V.8g (#184) - content-addressed cache for vision descriptions.
    /// Re-analysing the same video frame is free after the first call.
    cache: Arc<VisionCache>,
}

impl MediaEngine {
    pub fn new(config: MediaConfig) -> Self {
        let max = config.max_concurrency.clamp(1, 8);
        Self {
            config,
            semaphore: Arc::new(Semaphore::new(max)),
            cache: Arc::new(VisionCache::new()),
        }
    }

    /// Build an engine with an externally-supplied cache. Lets concurrent
    /// engines (e.g. the inner per-attachment engine spun up inside
    /// `process_attachments`) share a single cache instance.
    pub fn new_with_cache(config: MediaConfig, cache: Arc<VisionCache>) -> Self {
        let max = config.max_concurrency.clamp(1, 8);
        Self {
            config,
            semaphore: Arc::new(Semaphore::new(max)),
            cache,
        }
    }

    /// Describe an image using a vision-capable LLM.
    /// Auto-cascade: Anthropic -> OpenAI -> Gemini (based on API key availability).
    pub async fn describe_image(
        &self,
        attachment: &MediaAttachment,
    ) -> Result<MediaUnderstanding, String> {
        attachment.validate()?;
        if attachment.media_type != MediaType::Image {
            return Err("Expected image attachment".into());
        }

        let provider = if let Some(provider) = self.config.image_provider.as_deref() {
            provider
        } else {
            media_vision::detect_vision_provider().ok_or("No vision-capable LLM provider configured. Set ANTHROPIC_API_KEY, OPENAI_API_KEY, or GEMINI_API_KEY")?
        };

        let _permit = self.semaphore.acquire().await.map_err(|e| e.to_string())?;
        let image_bytes = read_image_bytes(attachment).await?;
        let model = media_vision::pick_vision_model(provider, attachment.batch_size_hint);
        let mime = attachment.mime_type.clone();
        let prompt = media_vision::vision_prompt(attachment.context_hint.as_deref());

        let cache_key = VisionCache::make_key(provider, model, &mime, &prompt, &image_bytes);
        if let Some(cached) = self.cache.get(&cache_key).await {
            debug!(
                provider,
                model,
                mime = %mime,
                "vision cache hit - skipping API call"
            );
            return Ok(cached);
        }

        info!(
            provider,
            model,
            mime = %mime,
            size = image_bytes.len(),
            has_hint = attachment.context_hint.is_some(),
            "Sending image for description"
        );

        let description =
            media_vision::describe_with_provider(provider, &image_bytes, &mime, model, &prompt)
                .await?;
        let description = description.trim().to_string();
        if description.is_empty() {
            return Err("Vision API returned empty description".into());
        }

        info!(
            provider,
            model,
            chars = description.len(),
            "Image description complete"
        );

        let understanding = MediaUnderstanding {
            media_type: MediaType::Image,
            description,
            provider: provider.to_string(),
            model: model.to_string(),
        };

        self.cache.put(cache_key, &understanding).await;
        Ok(understanding)
    }

    /// Transcribe audio using speech-to-text.
    pub async fn transcribe_audio(
        &self,
        attachment: &MediaAttachment,
    ) -> Result<MediaUnderstanding, String> {
        attachment.validate()?;
        if attachment.media_type != MediaType::Audio {
            return Err("Expected audio attachment".into());
        }

        let _permit = self.semaphore.acquire().await.map_err(|e| e.to_string())?;
        media_audio::transcribe_audio(&self.config, attachment).await
    }

    /// Describe video using Gemini.
    pub async fn describe_video(
        &self,
        attachment: &MediaAttachment,
    ) -> Result<MediaUnderstanding, String> {
        attachment.validate()?;
        if attachment.media_type != MediaType::Video {
            return Err("Expected video attachment".into());
        }

        if !self.config.video_description {
            return Err("Video description is disabled in configuration".into());
        }

        if std::env::var("GEMINI_API_KEY").is_err() && std::env::var("GOOGLE_API_KEY").is_err() {
            return Err("Video description requires GEMINI_API_KEY or GOOGLE_API_KEY".into());
        }

        Ok(MediaUnderstanding {
            media_type: MediaType::Video,
            description: "[Video description would be generated by Gemini]".to_string(),
            provider: "gemini".to_string(),
            model: "gemini-2.5-flash".to_string(),
        })
    }

    /// Process multiple attachments concurrently (bounded by max_concurrency).
    pub async fn process_attachments(
        &self,
        attachments: Vec<MediaAttachment>,
    ) -> Vec<Result<MediaUnderstanding, String>> {
        let mut handles = Vec::new();

        for attachment in attachments {
            let sem = self.semaphore.clone();
            let config = self.config.clone();
            let cache = self.cache.clone();
            let handle = tokio::spawn(async move {
                let _permit = sem.acquire().await.map_err(|e| e.to_string())?;
                let engine = MediaEngine {
                    config,
                    semaphore: Arc::new(Semaphore::new(1)),
                    cache,
                };
                match attachment.media_type {
                    MediaType::Image => engine.describe_image(&attachment).await,
                    MediaType::Audio => engine.transcribe_audio(&attachment).await,
                    MediaType::Video => engine.describe_video(&attachment).await,
                }
            });
            handles.push(handle);
        }

        let mut results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(result) => results.push(result),
                Err(e) => results.push(Err(format!("Task failed: {e}"))),
            }
        }
        results
    }
}

async fn read_image_bytes(attachment: &MediaAttachment) -> Result<Vec<u8>, String> {
    match &attachment.source {
        MediaSource::FilePath { path } => tokio::fs::read(path)
            .await
            .map_err(|e| format!("Failed to read image file '{}': {}", path, e)),
        MediaSource::Base64 { data, .. } => {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| format!("Failed to decode base64 image: {}", e))
        }
        MediaSource::Url { url } => Err(format!(
            "URL-based image source not supported for description: {}",
            url
        )),
    }
}

#[cfg(test)]
#[path = "media_understanding_tests.rs"]
mod tests;
