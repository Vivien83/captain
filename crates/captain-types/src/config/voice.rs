use serde::{Deserialize, Serialize};

/// Text-to-speech configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TtsConfig {
    /// Enable TTS. Default: false.
    pub enabled: bool,
    /// Default provider: "local-native", "openai" or "elevenlabs".
    pub provider: Option<String>,
    /// OpenAI TTS settings.
    pub openai: TtsOpenAiConfig,
    /// ElevenLabs TTS settings.
    pub elevenlabs: TtsElevenLabsConfig,
    /// Local native TTS settings (Kokoro/Piper).
    pub local_native: TtsLocalNativeConfig,
    /// Max text length for TTS (chars). Default: 4096.
    pub max_text_length: usize,
    /// Timeout per TTS request in seconds. Default: 30.
    pub timeout_secs: u64,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: None,
            openai: TtsOpenAiConfig::default(),
            elevenlabs: TtsElevenLabsConfig::default(),
            local_native: TtsLocalNativeConfig::default(),
            max_text_length: 4096,
            timeout_secs: 30,
        }
    }
}

/// OpenAI TTS settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TtsOpenAiConfig {
    /// Voice: alloy, echo, fable, onyx, nova, shimmer. Default: "alloy".
    pub voice: String,
    /// Model: "tts-1" or "tts-1-hd". Default: "tts-1".
    pub model: String,
    /// Output format: "mp3", "opus", "aac", "flac". Default: "mp3".
    pub format: String,
    /// Speed: 0.25 to 4.0. Default: 1.0.
    pub speed: f32,
}

impl Default for TtsOpenAiConfig {
    fn default() -> Self {
        Self {
            voice: "alloy".to_string(),
            model: "tts-1".to_string(),
            format: "mp3".to_string(),
            speed: 1.0,
        }
    }
}

/// ElevenLabs TTS settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TtsElevenLabsConfig {
    /// Environment variable that stores the ElevenLabs API key.
    pub api_key_env: String,
    /// Voice ID. Default: "21m00Tcm4TlvDq8ikWAM" (Rachel).
    pub voice_id: String,
    /// Model ID. Default: "eleven_monolingual_v1".
    pub model_id: String,
    /// Stability (0.0-1.0). Default: 0.5.
    pub stability: f32,
    /// Similarity boost (0.0-1.0). Default: 0.75.
    pub similarity_boost: f32,
}

impl Default for TtsElevenLabsConfig {
    fn default() -> Self {
        Self {
            api_key_env: "ELEVENLABS_API_KEY".to_string(),
            voice_id: "21m00Tcm4TlvDq8ikWAM".to_string(),
            model_id: "eleven_monolingual_v1".to_string(),
            stability: 0.5,
            similarity_boost: 0.75,
        }
    }
}

/// Local native TTS settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TtsLocalNativeConfig {
    /// Preferred local engine: "kokoro" or "piper". Default: "kokoro".
    pub preferred_engine: String,
    /// Fallback engine: "piper", "kokoro" or "none". Default: "piper".
    pub fallback_engine: String,
    /// Language hint used by setup and UI metadata. Default: "fr".
    pub language: String,
}

impl Default for TtsLocalNativeConfig {
    fn default() -> Self {
        Self {
            preferred_engine: "kokoro".to_string(),
            fallback_engine: "piper".to_string(),
            language: "fr".to_string(),
        }
    }
}

/// Live voice call settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceCallConfig {
    /// Enable browser WebRTC live calls. Default: true.
    pub enabled: bool,
    /// Realtime provider. Currently only "openai" is supported.
    pub provider: String,
    /// Realtime model used for live speech-to-speech calls.
    pub model: String,
    /// Realtime output voice.
    pub voice: String,
    /// Optional API key environment variable override.
    pub api_key_env: Option<String>,
    /// Let the Realtime session route user turns to the Captain agent.
    pub enable_agent_tool: bool,
    /// Auto-end a live call after no microphone sound and no Realtime activity.
    /// Set to 0 to disable.
    pub auto_end_silence_secs: u64,
    /// Auto-end a live call after the discussion has no sound or Realtime events.
    /// Set to 0 to disable.
    pub auto_end_inactive_secs: u64,
    /// System instructions for the live voice model.
    pub instructions: String,
}

impl Default for VoiceCallConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: "openai".to_string(),
            model: "gpt-realtime-2".to_string(),
            voice: "marin".to_string(),
            api_key_env: None,
            enable_agent_tool: true,
            auto_end_silence_secs: 90,
            auto_end_inactive_secs: 180,
            instructions: "You are Captain's live voice interface, not a separate assistant. The Captain agent is the single source of intelligence, memory, tools, and execution. For every substantive user request, question, instruction, preference, decision, or follow-up, call captain_message with the user's intent and wait for Captain's answer. Do not answer from your own knowledge except for tiny conversational acknowledgements such as 'I heard you' while routing. Speak in the user's language, naturally and briefly, and read Captain's result as Captain speaking. If the user asks what Captain is doing, asks for details, or asks for recent actions, call captain_activity_summary instead of inventing status.".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TtsConfig, VoiceCallConfig};

    #[test]
    fn tts_config_defaults_keep_local_native_disabled() {
        let config = TtsConfig::default();

        assert!(!config.enabled);
        assert!(config.provider.is_none());
        assert_eq!(config.max_text_length, 4096);
        assert_eq!(config.timeout_secs, 30);
        assert_eq!(config.openai.voice, "alloy");
        assert_eq!(config.openai.model, "tts-1");
        assert_eq!(config.elevenlabs.api_key_env, "ELEVENLABS_API_KEY");
        assert_eq!(config.local_native.preferred_engine, "kokoro");
        assert_eq!(config.local_native.fallback_engine, "piper");
        assert_eq!(config.local_native.language, "fr");
    }

    #[test]
    fn tts_config_deserializes_partial_toml_with_defaults() {
        let config: TtsConfig = toml::from_str(
            r#"
            enabled = true
            provider = "openai"
            max_text_length = 1024

            [openai]
            voice = "nova"
            "#,
        )
        .unwrap();

        assert!(config.enabled);
        assert_eq!(config.provider.as_deref(), Some("openai"));
        assert_eq!(config.max_text_length, 1024);
        assert_eq!(config.timeout_secs, 30);
        assert_eq!(config.openai.voice, "nova");
        assert_eq!(config.openai.model, "tts-1");
        assert_eq!(config.openai.format, "mp3");
        assert_eq!(config.elevenlabs.api_key_env, "ELEVENLABS_API_KEY");
        assert_eq!(config.local_native.preferred_engine, "kokoro");
    }

    #[test]
    fn tts_config_roundtrips_all_provider_overrides() {
        let config = TtsConfig {
            enabled: true,
            provider: Some("elevenlabs".to_string()),
            max_text_length: 2048,
            timeout_secs: 45,
            openai: super::TtsOpenAiConfig {
                voice: "shimmer".to_string(),
                model: "tts-1-hd".to_string(),
                format: "opus".to_string(),
                speed: 1.25,
            },
            elevenlabs: super::TtsElevenLabsConfig {
                api_key_env: "ELEVENLABS_SECONDARY_KEY".to_string(),
                voice_id: "voice-123".to_string(),
                model_id: "eleven_multilingual_v2".to_string(),
                stability: 0.4,
                similarity_boost: 0.8,
            },
            local_native: super::TtsLocalNativeConfig {
                preferred_engine: "piper".to_string(),
                fallback_engine: "none".to_string(),
                language: "en".to_string(),
            },
        };

        let encoded = toml::to_string(&config).unwrap();
        let decoded: TtsConfig = toml::from_str(&encoded).unwrap();

        assert!(decoded.enabled);
        assert_eq!(decoded.provider.as_deref(), Some("elevenlabs"));
        assert_eq!(decoded.openai.model, "tts-1-hd");
        assert_eq!(decoded.elevenlabs.api_key_env, "ELEVENLABS_SECONDARY_KEY");
        assert_eq!(decoded.local_native.fallback_engine, "none");
        assert_eq!(decoded.timeout_secs, 45);
    }

    #[test]
    fn voice_call_defaults_route_through_captain_agent() {
        let config = VoiceCallConfig::default();

        assert!(config.enabled);
        assert_eq!(config.provider, "openai");
        assert_eq!(config.model, "gpt-realtime-2");
        assert_eq!(config.voice, "marin");
        assert!(config.api_key_env.is_none());
        assert!(config.enable_agent_tool);
        assert_eq!(config.auto_end_silence_secs, 90);
        assert_eq!(config.auto_end_inactive_secs, 180);
        assert!(config.instructions.contains("captain_message"));
        assert!(config.instructions.contains("not a separate assistant"));
    }

    #[test]
    fn voice_call_deserializes_partial_toml_with_defaults() {
        let config: VoiceCallConfig = toml::from_str(
            r#"
            enabled = false
            api_key_env = "OPENAI_REALTIME_KEY"
            auto_end_silence_secs = 0
            "#,
        )
        .unwrap();

        assert!(!config.enabled);
        assert_eq!(config.provider, "openai");
        assert_eq!(config.model, "gpt-realtime-2");
        assert_eq!(config.voice, "marin");
        assert_eq!(config.api_key_env.as_deref(), Some("OPENAI_REALTIME_KEY"));
        assert!(config.enable_agent_tool);
        assert_eq!(config.auto_end_silence_secs, 0);
        assert_eq!(config.auto_end_inactive_secs, 180);
    }
}
