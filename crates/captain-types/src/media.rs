//! Media understanding types — shared data structures for media processing.

use serde::{Deserialize, Serialize};

/// Supported media types for understanding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    Image,
    Audio,
    Video,
}

impl std::fmt::Display for MediaType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaType::Image => write!(f, "image"),
            MediaType::Audio => write!(f, "audio"),
            MediaType::Video => write!(f, "video"),
        }
    }
}

/// Source of media content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum MediaSource {
    /// Path to a local file.
    FilePath { path: String },
    /// URL to fetch the media from (SSRF-checked).
    Url { url: String },
    /// Base64-encoded data.
    Base64 { data: String, mime_type: String },
}

/// A media attachment to be analyzed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaAttachment {
    /// What kind of media this is.
    pub media_type: MediaType,
    /// MIME type (e.g., "image/png", "audio/mp3").
    pub mime_type: String,
    /// Where to get the media data.
    pub source: MediaSource,
    /// File size in bytes (for validation).
    pub size_bytes: u64,
    /// Optional caller-provided guidance for `describe_image` /
    /// `transcribe_audio`. Concatenated with the engine default prompt.
    /// Examples:
    ///   - User intent: `"Décris l'action principale"`.
    ///   - Sequence wrapper: `"Frame 3/10 d'une vidéo, t = 2.0s."`.
    ///
    /// `serde(default)` keeps backward compatibility with previously
    /// serialised payloads (cache, IPC) where the field is absent.
    #[serde(default)]
    pub context_hint: Option<String>,
    /// Optional hint to the engine about the **batch size** the caller is
    /// about to issue. Used by `MediaEngine::describe_image` to pick a
    /// cheaper vision model when the batch is small (≤ 5 frames) — short
    /// clips don't benefit from the larger model's narrative coherence
    /// across many frames, so we pay 5× less per frame. See V.8h (#184).
    ///
    /// `None` (single-image callers) → engine uses the provider's default
    /// model. `serde(default)` keeps prior payloads roundtripping.
    #[serde(default)]
    pub batch_size_hint: Option<usize>,
}

/// Result of media analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaUnderstanding {
    /// What type of media was analyzed.
    pub media_type: MediaType,
    /// Human-readable description or transcription.
    pub description: String,
    /// Which provider produced this result.
    pub provider: String,
    /// Which model was used.
    pub model: String,
}

/// Configuration for media understanding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaConfig {
    /// Enable image description. Default: true.
    pub image_description: bool,
    /// Enable audio transcription. Default: true.
    pub audio_transcription: bool,
    /// Enable video description. Default: false (expensive).
    pub video_description: bool,
    /// Max concurrent media processing tasks. Default: 5. Clamped to [1, 8] by the engine.
    pub max_concurrency: usize,
    /// Preferred image description provider (auto-detect if None).
    pub image_provider: Option<String>,
    /// Preferred audio transcription provider (auto-detect if None).
    /// Supported: `local-whisper`, `parakeet-mlx`, `groq`, `openai`, `elevenlabs`.
    pub audio_provider: Option<String>,
    /// Preferred audio transcription model. When unset, Captain picks the
    /// provider default (`whisper-small`, `whisper-large-v3-turbo`,
    /// `whisper-1`, `scribe_v2`, etc.).
    pub audio_model: Option<String>,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            image_description: true,
            audio_transcription: true,
            video_description: false,
            max_concurrency: 5,
            image_provider: None,
            audio_provider: None,
            audio_model: None,
        }
    }
}

/// Configuration for link understanding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LinkConfig {
    /// Enable automatic link understanding. Default: false.
    pub enabled: bool,
    /// Max links to process per message. Default: 3.
    pub max_links: usize,
    /// Max content size to fetch per link in bytes. Default: 100KB.
    pub max_content_bytes: usize,
    /// Timeout per link fetch in seconds. Default: 10.
    pub timeout_secs: u64,
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_links: 3,
            max_content_bytes: 102_400,
            timeout_secs: 10,
        }
    }
}

// ---------------------------------------------------------------------------
// Validation constants (SECURITY)
// ---------------------------------------------------------------------------

/// Maximum image size in bytes (10 MB).
pub const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
/// Maximum audio size in bytes (20 MB).
pub const MAX_AUDIO_BYTES: u64 = 20 * 1024 * 1024;
/// Maximum video size in bytes (50 MB).
pub const MAX_VIDEO_BYTES: u64 = 50 * 1024 * 1024;
/// Maximum base64 decoded size (70 MB).
pub const MAX_BASE64_DECODED_BYTES: u64 = 70 * 1024 * 1024;

/// Allowed image MIME types.
pub const ALLOWED_IMAGE_TYPES: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/gif"];

/// Allowed audio MIME types.
pub const ALLOWED_AUDIO_TYPES: &[&str] = &[
    "audio/mpeg",
    "audio/wav",
    "audio/ogg",
    "audio/mp4",
    "audio/webm",
    "audio/x-wav",
    "audio/flac",
];

/// Allowed video MIME types.
pub const ALLOWED_VIDEO_TYPES: &[&str] = &["video/mp4", "video/quicktime", "video/webm"];

impl MediaAttachment {
    /// Validate the attachment against security constraints.
    pub fn validate(&self) -> Result<(), String> {
        // Check MIME type allowlist
        let allowed = match self.media_type {
            MediaType::Image => ALLOWED_IMAGE_TYPES.contains(&self.mime_type.as_str()),
            MediaType::Audio => ALLOWED_AUDIO_TYPES.contains(&self.mime_type.as_str()),
            MediaType::Video => ALLOWED_VIDEO_TYPES.contains(&self.mime_type.as_str()),
        };
        if !allowed {
            return Err(format!(
                "Unsupported MIME type '{}' for {:?} media",
                self.mime_type, self.media_type
            ));
        }

        // Check size limits
        let max_bytes = match self.media_type {
            MediaType::Image => MAX_IMAGE_BYTES,
            MediaType::Audio => MAX_AUDIO_BYTES,
            MediaType::Video => MAX_VIDEO_BYTES,
        };
        if self.size_bytes > max_bytes {
            return Err(format!(
                "{} file too large: {} bytes (max {} bytes)",
                self.media_type, self.size_bytes, max_bytes
            ));
        }

        Ok(())
    }
}

/// Supported image generation models.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageGenModel {
    #[default]
    DallE3,
    DallE2,
    #[serde(rename = "gpt-image-2")]
    GptImage2,
    #[serde(rename = "gpt-image-1.5")]
    GptImage15,
    #[serde(rename = "gpt-image-1")]
    GptImage1,
    #[serde(rename = "gpt-image-1-mini")]
    GptImage1Mini,
}

impl std::fmt::Display for ImageGenModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageGenModel::DallE3 => write!(f, "dall-e-3"),
            ImageGenModel::DallE2 => write!(f, "dall-e-2"),
            ImageGenModel::GptImage2 => write!(f, "gpt-image-2"),
            ImageGenModel::GptImage15 => write!(f, "gpt-image-1.5"),
            ImageGenModel::GptImage1 => write!(f, "gpt-image-1"),
            ImageGenModel::GptImage1Mini => write!(f, "gpt-image-1-mini"),
        }
    }
}

/// Image generation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenRequest {
    /// The prompt describing the image to generate.
    pub prompt: String,
    /// Which model to use.
    #[serde(default)]
    pub model: ImageGenModel,
    /// Image size (e.g., "1024x1024").
    #[serde(default = "default_image_size")]
    pub size: String,
    /// Quality level (e.g., "standard", "hd").
    #[serde(default = "default_image_quality")]
    pub quality: String,
    /// Number of images to generate (1-4, DALL-E 3 only supports 1).
    #[serde(default = "default_image_count")]
    pub count: u8,
}

fn default_image_size() -> String {
    "1024x1024".to_string()
}

fn default_image_quality() -> String {
    "standard".to_string()
}

fn default_image_count() -> u8 {
    1
}

/// Allowed sizes per model.
pub const DALLE3_SIZES: &[&str] = &["1024x1024", "1792x1024", "1024x1792"];
pub const DALLE2_SIZES: &[&str] = &["256x256", "512x512", "1024x1024"];
pub const GPT_IMAGE_LEGACY_SIZES: &[&str] = &["auto", "1024x1024", "1536x1024", "1024x1536"];
pub const GPT_IMAGE2_COMMON_SIZES: &[&str] = &[
    "auto",
    "1024x1024",
    "1536x1024",
    "1024x1536",
    "2048x2048",
    "2048x1152",
    "3840x2160",
    "2160x3840",
];

impl ImageGenRequest {
    /// Max prompt length in characters.
    pub const MAX_PROMPT_LEN: usize = 4000;

    /// Validate the request against model-specific constraints.
    pub fn validate(&self) -> Result<(), String> {
        // Prompt length
        if self.prompt.is_empty() {
            return Err("Image generation prompt cannot be empty".into());
        }
        if self.prompt.len() > Self::MAX_PROMPT_LEN {
            return Err(format!(
                "Prompt too long: {} chars (max {})",
                self.prompt.len(),
                Self::MAX_PROMPT_LEN
            ));
        }
        // Strip control chars check
        if self
            .prompt
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
        {
            return Err("Prompt contains invalid control characters".into());
        }

        // Model-specific size validation
        let allowed_sizes = match self.model {
            ImageGenModel::DallE3 => DALLE3_SIZES,
            ImageGenModel::DallE2 => DALLE2_SIZES,
            ImageGenModel::GptImage2 => GPT_IMAGE2_COMMON_SIZES,
            ImageGenModel::GptImage15 | ImageGenModel::GptImage1 | ImageGenModel::GptImage1Mini => {
                GPT_IMAGE_LEGACY_SIZES
            }
        };
        if !allowed_sizes.contains(&self.size.as_str()) {
            return Err(format!(
                "Invalid size '{}' for {}. Allowed: {:?}",
                self.size, self.model, allowed_sizes
            ));
        }

        // Count validation
        match self.model {
            ImageGenModel::DallE3 => {
                if self.count != 1 {
                    return Err("DALL-E 3 only supports count=1".into());
                }
            }
            ImageGenModel::DallE2
            | ImageGenModel::GptImage2
            | ImageGenModel::GptImage15
            | ImageGenModel::GptImage1
            | ImageGenModel::GptImage1Mini => {
                if self.count == 0 || self.count > 4 {
                    return Err(format!(
                        "Invalid count {} for {}. Must be 1-4",
                        self.count, self.model
                    ));
                }
            }
        }

        // Quality validation
        match self.model {
            ImageGenModel::DallE3 => {
                if self.quality != "standard" && self.quality != "hd" {
                    return Err(format!(
                        "Invalid quality '{}' for DALL-E 3. Must be 'standard' or 'hd'",
                        self.quality
                    ));
                }
            }
            ImageGenModel::DallE2
            | ImageGenModel::GptImage2
            | ImageGenModel::GptImage15
            | ImageGenModel::GptImage1
            | ImageGenModel::GptImage1Mini => {
                if self.quality != "standard"
                    && self.quality != "auto"
                    && self.quality != "high"
                    && self.quality != "medium"
                    && self.quality != "low"
                {
                    return Err(format!(
                        "Invalid quality '{}'. Must be 'standard', 'auto', 'high', 'medium', or 'low'",
                        self.quality
                    ));
                }
            }
        }

        Ok(())
    }
}

/// Result of image generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenResult {
    /// Generated images.
    pub images: Vec<GeneratedImage>,
    /// Which model was used.
    pub model: String,
    /// Revised prompt (DALL-E 3 rewrites prompts for quality).
    pub revised_prompt: Option<String>,
}

/// A single generated image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedImage {
    /// Base64-encoded image data.
    pub data_base64: String,
    /// Temporary URL (may expire).
    pub url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_media_type_display() {
        assert_eq!(MediaType::Image.to_string(), "image");
        assert_eq!(MediaType::Audio.to_string(), "audio");
        assert_eq!(MediaType::Video.to_string(), "video");
    }

    #[test]
    fn test_media_config_default() {
        let config = MediaConfig::default();
        assert!(config.image_description);
        assert!(config.audio_transcription);
        assert!(!config.video_description);
        assert_eq!(config.max_concurrency, 5);
        assert!(config.image_provider.is_none());
    }

    #[test]
    fn test_link_config_default() {
        let config = LinkConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.max_links, 3);
        assert_eq!(config.max_content_bytes, 102_400);
        assert_eq!(config.timeout_secs, 10);
    }

    #[test]
    fn test_attachment_validate_valid_image() {
        let a = MediaAttachment {
            media_type: MediaType::Image,
            mime_type: "image/png".to_string(),
            source: MediaSource::FilePath {
                path: "test.png".to_string(),
            },
            size_bytes: 1024,
            context_hint: None,
            batch_size_hint: None,
        };
        assert!(a.validate().is_ok());
    }

    #[test]
    fn test_attachment_validate_bad_mime() {
        let a = MediaAttachment {
            media_type: MediaType::Image,
            mime_type: "application/pdf".to_string(),
            source: MediaSource::FilePath {
                path: "test.pdf".to_string(),
            },
            size_bytes: 1024,
            context_hint: None,
            batch_size_hint: None,
        };
        assert!(a.validate().is_err());
    }

    #[test]
    fn test_attachment_validate_too_large() {
        let a = MediaAttachment {
            media_type: MediaType::Image,
            mime_type: "image/png".to_string(),
            source: MediaSource::FilePath {
                path: "big.png".to_string(),
            },
            size_bytes: MAX_IMAGE_BYTES + 1,
            context_hint: None,
            batch_size_hint: None,
        };
        assert!(a.validate().is_err());
    }

    #[test]
    fn test_attachment_validate_audio() {
        let a = MediaAttachment {
            media_type: MediaType::Audio,
            mime_type: "audio/mpeg".to_string(),
            source: MediaSource::Url {
                url: "https://example.com/a.mp3".to_string(),
            },
            size_bytes: 5_000_000,
            context_hint: None,
            batch_size_hint: None,
        };
        assert!(a.validate().is_ok());
    }

    #[test]
    fn test_attachment_validate_video_too_large() {
        let a = MediaAttachment {
            media_type: MediaType::Video,
            mime_type: "video/mp4".to_string(),
            source: MediaSource::FilePath {
                path: "big.mp4".to_string(),
            },
            size_bytes: MAX_VIDEO_BYTES + 1,
            context_hint: None,
            batch_size_hint: None,
        };
        assert!(a.validate().is_err());
    }

    /// `context_hint` is `#[serde(default)]` — payloads without the field
    /// must deserialise as `None` for backward-compat with v6 caches/IPC.
    #[test]
    fn test_attachment_context_hint_serde_backcompat() {
        // Old-shape JSON without `context_hint`.
        let json = r#"{
            "media_type": "image",
            "mime_type": "image/png",
            "source": {"type": "file_path", "path": "/tmp/x.png"},
            "size_bytes": 1024
        }"#;
        let a: MediaAttachment = serde_json::from_str(json).expect("v6 payload must parse");
        assert!(a.context_hint.is_none());
    }

    /// New-shape JSON with `context_hint` round-trips correctly.
    #[test]
    fn test_attachment_context_hint_serde_roundtrip() {
        let a = MediaAttachment {
            media_type: MediaType::Image,
            mime_type: "image/png".to_string(),
            source: MediaSource::FilePath {
                path: "/tmp/x.png".to_string(),
            },
            size_bytes: 1024,
            context_hint: Some("Frame 3/10 d'une vidéo, t = 2.0s.".to_string()),
            batch_size_hint: None,
        };
        let json = serde_json::to_string(&a).unwrap();
        let parsed: MediaAttachment = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.context_hint.as_deref(),
            Some("Frame 3/10 d'une vidéo, t = 2.0s.")
        );
    }

    #[test]
    fn test_image_gen_model_display() {
        assert_eq!(ImageGenModel::DallE3.to_string(), "dall-e-3");
        assert_eq!(ImageGenModel::DallE2.to_string(), "dall-e-2");
        assert_eq!(ImageGenModel::GptImage2.to_string(), "gpt-image-2");
        assert_eq!(ImageGenModel::GptImage15.to_string(), "gpt-image-1.5");
        assert_eq!(ImageGenModel::GptImage1.to_string(), "gpt-image-1");
        assert_eq!(ImageGenModel::GptImage1Mini.to_string(), "gpt-image-1-mini");
    }

    #[test]
    fn test_image_gen_request_validate_valid() {
        let req = ImageGenRequest {
            prompt: "A sunset over mountains".to_string(),
            model: ImageGenModel::DallE3,
            size: "1024x1024".to_string(),
            quality: "hd".to_string(),
            count: 1,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_image_gen_request_validate_empty_prompt() {
        let req = ImageGenRequest {
            prompt: String::new(),
            model: ImageGenModel::DallE3,
            size: "1024x1024".to_string(),
            quality: "standard".to_string(),
            count: 1,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_image_gen_request_validate_bad_size() {
        let req = ImageGenRequest {
            prompt: "test".to_string(),
            model: ImageGenModel::DallE3,
            size: "512x512".to_string(),
            quality: "standard".to_string(),
            count: 1,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_image_gen_request_validate_dalle3_count() {
        let req = ImageGenRequest {
            prompt: "test".to_string(),
            model: ImageGenModel::DallE3,
            size: "1024x1024".to_string(),
            quality: "standard".to_string(),
            count: 2,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_image_gen_request_validate_dalle2_multi() {
        let req = ImageGenRequest {
            prompt: "test".to_string(),
            model: ImageGenModel::DallE2,
            size: "512x512".to_string(),
            quality: "standard".to_string(),
            count: 4,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_image_gen_request_validate_control_chars() {
        let req = ImageGenRequest {
            prompt: "test\x00prompt".to_string(),
            model: ImageGenModel::DallE3,
            size: "1024x1024".to_string(),
            quality: "standard".to_string(),
            count: 1,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_media_type_serde_roundtrip() {
        let mt = MediaType::Audio;
        let json = serde_json::to_string(&mt).unwrap();
        assert_eq!(json, "\"audio\"");
        let parsed: MediaType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, mt);
    }

    #[test]
    fn test_image_gen_model_serde_roundtrip() {
        let m = ImageGenModel::GptImage1;
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json, "\"gpt-image-1\"");
        let parsed: ImageGenModel = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn test_media_config_serde_roundtrip() {
        let config = MediaConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let parsed: MediaConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.max_concurrency, 5);
        assert!(parsed.image_description);
    }
}
