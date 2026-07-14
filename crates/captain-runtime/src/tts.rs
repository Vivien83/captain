//! Text-to-speech engine — synthesize text to audio.
//!
//! Auto-cascades through available providers based on configured API keys.

use captain_types::config::TtsConfig;
use std::sync::RwLock;

#[path = "tts_native.rs"]
mod tts_native;

/// Maximum audio response size (10MB).
const MAX_AUDIO_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

/// Result of TTS synthesis.
#[derive(Debug)]
pub struct TtsResult {
    pub audio_data: Vec<u8>,
    pub format: String,
    pub provider: String,
    pub voice: String,
    pub voice_source: String,
    pub duration_estimate_ms: u64,
}

/// Text-to-speech engine.
pub struct TtsEngine {
    config: RwLock<TtsConfig>,
}

impl TtsEngine {
    pub fn new(config: TtsConfig) -> Self {
        Self {
            config: RwLock::new(config),
        }
    }

    pub fn update_config(&self, config: TtsConfig) {
        let mut guard = self.config.write().unwrap_or_else(|e| e.into_inner());
        *guard = config;
    }

    pub fn config_snapshot(&self) -> TtsConfig {
        self.config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Detect which TTS provider is available based on environment variables.
    fn detect_provider() -> Option<&'static str> {
        if crate::native_voice::status().tts_ready {
            return Some(crate::native_voice::NATIVE_TTS_PROVIDER);
        }
        if std::env::var("OPENAI_API_KEY").is_ok() {
            return Some("openai");
        }
        if std::env::var("ELEVENLABS_API_KEY").is_ok() {
            return Some("elevenlabs");
        }
        None
    }

    /// Synthesize text to audio bytes.
    /// Auto-cascade: configured provider -> OpenAI -> ElevenLabs.
    /// Optional overrides for voice and format (per-request, from tool input).
    pub async fn synthesize(
        &self,
        text: &str,
        voice_override: Option<&str>,
        format_override: Option<&str>,
    ) -> Result<TtsResult, String> {
        let config = self.config_snapshot();
        if !config.enabled {
            return Err("TTS is disabled in configuration".into());
        }

        // Validate text length
        if text.is_empty() {
            return Err("Text cannot be empty".into());
        }
        if text.len() > config.max_text_length {
            return Err(format!(
                "Text too long: {} chars (max {})",
                text.len(),
                config.max_text_length
            ));
        }

        let configured_provider = config
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|p| p.to_ascii_lowercase());
        let provider = if let Some(provider) = configured_provider.as_deref() {
            provider
        } else {
            Self::detect_provider()
                .ok_or("No TTS provider configured. Set OPENAI_API_KEY or ELEVENLABS_API_KEY")?
        };
        let allow_request_voice_override = configured_provider.is_none();

        match provider {
            crate::native_voice::NATIVE_TTS_PROVIDER | "local" | "native" => {
                tts_native::synthesize(&config, text, voice_override).await
            }
            "openai" => {
                Self::synthesize_openai(
                    &config,
                    text,
                    voice_override,
                    format_override,
                    allow_request_voice_override,
                )
                .await
            }
            "elevenlabs" => {
                Self::synthesize_elevenlabs(
                    &config,
                    text,
                    voice_override,
                    allow_request_voice_override,
                )
                .await
            }
            other => Err(format!("Unknown TTS provider: {other}")),
        }
    }

    /// Synthesize via OpenAI TTS API.
    async fn synthesize_openai(
        config: &TtsConfig,
        text: &str,
        voice_override: Option<&str>,
        format_override: Option<&str>,
        allow_request_voice_override: bool,
    ) -> Result<TtsResult, String> {
        let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| "OPENAI_API_KEY not set")?;

        let request_voice = voice_override.map(str::trim).filter(|s| !s.is_empty());
        let (voice, voice_source) = if allow_request_voice_override {
            match request_voice {
                Some(voice) => (voice, "request"),
                None => (config.openai.voice.as_str(), "config"),
            }
        } else {
            (config.openai.voice.as_str(), "config")
        };
        let format = format_override.unwrap_or(&config.openai.format);

        let body = serde_json::json!({
            "model": config.openai.model,
            "input": text,
            "voice": voice,
            "response_format": format,
            "speed": config.openai.speed,
        });

        let client = reqwest::Client::new();
        let response = client
            .post("https://api.openai.com/v1/audio/speech")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .send()
            .await
            .map_err(|e| format!("OpenAI TTS request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let err = response.text().await.unwrap_or_default();
            let truncated = crate::str_utils::safe_truncate_str(&err, 500);
            return Err(format!("OpenAI TTS failed (HTTP {status}): {truncated}"));
        }

        // Check content length before downloading
        if let Some(len) = response.content_length() {
            if len as usize > MAX_AUDIO_RESPONSE_BYTES {
                return Err(format!(
                    "Audio response too large: {len} bytes (max {MAX_AUDIO_RESPONSE_BYTES})"
                ));
            }
        }

        let audio_data = response
            .bytes()
            .await
            .map_err(|e| format!("Failed to read audio response: {e}"))?;

        if audio_data.len() > MAX_AUDIO_RESPONSE_BYTES {
            return Err(format!(
                "Audio data exceeds {}MB limit",
                MAX_AUDIO_RESPONSE_BYTES / 1024 / 1024
            ));
        }

        // Rough duration estimate: ~150 words/min at ~12 bytes/ms for MP3
        let word_count = text.split_whitespace().count();
        let duration_ms = (word_count as u64 * 400).max(500); // ~400ms per word, min 500ms

        Ok(TtsResult {
            audio_data: audio_data.to_vec(),
            format: format.to_string(),
            provider: "openai".to_string(),
            voice: voice.to_string(),
            voice_source: voice_source.to_string(),
            duration_estimate_ms: duration_ms,
        })
    }

    /// Synthesize via ElevenLabs TTS API.
    async fn synthesize_elevenlabs(
        config: &TtsConfig,
        text: &str,
        voice_override: Option<&str>,
        allow_request_voice_override: bool,
    ) -> Result<TtsResult, String> {
        let api_key_env = config.elevenlabs.api_key_env.trim();
        let api_key_env = if api_key_env.is_empty() {
            "ELEVENLABS_API_KEY"
        } else {
            api_key_env
        };
        let api_key = std::env::var(api_key_env).map_err(|_| format!("{api_key_env} not set"))?;

        let request_voice_id = voice_override
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .filter(|voice| !is_openai_voice_alias(voice));
        let (voice_id, voice_source) = if allow_request_voice_override {
            match request_voice_id {
                Some(voice) => (voice, "request"),
                None => (config.elevenlabs.voice_id.as_str(), "config"),
            }
        } else {
            (config.elevenlabs.voice_id.as_str(), "config")
        };
        let url = format!("https://api.elevenlabs.io/v1/text-to-speech/{}", voice_id);

        let body = serde_json::json!({
            "text": text,
            "model_id": config.elevenlabs.model_id,
            "voice_settings": {
                "stability": config.elevenlabs.stability,
                "similarity_boost": config.elevenlabs.similarity_boost,
            }
        });

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("xi-api-key", &api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .send()
            .await
            .map_err(|e| format!("ElevenLabs TTS request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let err = response.text().await.unwrap_or_default();
            let truncated = crate::str_utils::safe_truncate_str(&err, 500);
            return Err(format!(
                "ElevenLabs TTS failed (HTTP {status}): {truncated}"
            ));
        }

        if let Some(len) = response.content_length() {
            if len as usize > MAX_AUDIO_RESPONSE_BYTES {
                return Err(format!(
                    "Audio response too large: {len} bytes (max {MAX_AUDIO_RESPONSE_BYTES})"
                ));
            }
        }

        let audio_data = response
            .bytes()
            .await
            .map_err(|e| format!("Failed to read audio response: {e}"))?;

        if audio_data.len() > MAX_AUDIO_RESPONSE_BYTES {
            return Err(format!(
                "Audio data exceeds {}MB limit",
                MAX_AUDIO_RESPONSE_BYTES / 1024 / 1024
            ));
        }

        let word_count = text.split_whitespace().count();
        let duration_ms = (word_count as u64 * 400).max(500);

        Ok(TtsResult {
            audio_data: audio_data.to_vec(),
            format: "mp3".to_string(),
            provider: "elevenlabs".to_string(),
            voice: voice_id.to_string(),
            voice_source: voice_source.to_string(),
            duration_estimate_ms: duration_ms,
        })
    }
}

fn is_openai_voice_alias(voice: &str) -> bool {
    matches!(
        voice.trim().to_ascii_lowercase().as_str(),
        "alloy" | "echo" | "fable" | "onyx" | "nova" | "shimmer"
    )
}

#[cfg(test)]
#[path = "tts_tests.rs"]
mod tests;
