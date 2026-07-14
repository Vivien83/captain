use super::*;

fn default_config() -> TtsConfig {
    TtsConfig::default()
}

fn resolve_elevenlabs_voice_id<'a>(
    voice_override: Option<&'a str>,
    default_id: &'a str,
) -> &'a str {
    match voice_override.map(str::trim).filter(|s| !s.is_empty()) {
        Some(voice) if !is_openai_voice_alias(voice) => voice,
        _ => default_id,
    }
}

#[test]
fn test_engine_creation() {
    let engine = TtsEngine::new(default_config());
    assert!(!engine.config_snapshot().enabled);
}

#[test]
fn test_config_defaults() {
    let config = TtsConfig::default();
    assert!(!config.enabled);
    assert_eq!(config.max_text_length, 4096);
    assert_eq!(config.timeout_secs, 30);
    assert_eq!(config.openai.voice, "alloy");
    assert_eq!(config.openai.model, "tts-1");
    assert_eq!(config.openai.format, "mp3");
    assert_eq!(config.openai.speed, 1.0);
    assert_eq!(config.elevenlabs.voice_id, "21m00Tcm4TlvDq8ikWAM");
    assert_eq!(config.elevenlabs.model_id, "eleven_monolingual_v1");
    assert_eq!(config.local_native.preferred_engine, "kokoro");
    assert_eq!(config.local_native.fallback_engine, "piper");
    assert_eq!(config.local_native.language, "fr");
}

#[test]
fn local_native_engine_names_are_normalized() {
    assert_eq!(tts_native::normalize_engine(" Kokoro "), Some("kokoro"));
    assert_eq!(tts_native::normalize_engine("PIPER"), Some("piper"));
    assert_eq!(tts_native::normalize_engine("off"), Some("none"));
    assert_eq!(tts_native::normalize_engine(""), None);
    assert_eq!(tts_native::normalize_engine("unknown"), None);
}

#[test]
fn elevenlabs_ignores_openai_voice_aliases() {
    assert_eq!(
        resolve_elevenlabs_voice_id(Some("nova"), "eleven-id"),
        "eleven-id"
    );
    assert_eq!(
        resolve_elevenlabs_voice_id(Some(" alloy "), "eleven-id"),
        "eleven-id"
    );
    assert_eq!(
        resolve_elevenlabs_voice_id(Some("21m00Tcm4TlvDq8ikWAM"), "eleven-id"),
        "21m00Tcm4TlvDq8ikWAM"
    );
    assert_eq!(resolve_elevenlabs_voice_id(None, "eleven-id"), "eleven-id");
}

#[test]
fn update_config_replaces_runtime_snapshot() {
    let engine = TtsEngine::new(default_config());
    let mut config = default_config();
    config.enabled = true;
    config.provider = Some("elevenlabs".to_string());
    config.elevenlabs.voice_id = "lucie-id".to_string();

    engine.update_config(config);

    let snapshot = engine.config_snapshot();
    assert!(snapshot.enabled);
    assert_eq!(snapshot.provider.as_deref(), Some("elevenlabs"));
    assert_eq!(snapshot.elevenlabs.voice_id, "lucie-id");
}

#[tokio::test]
async fn test_synthesize_disabled() {
    let engine = TtsEngine::new(default_config());
    let result = engine.synthesize("Hello", None, None).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("disabled"));
}

#[tokio::test]
async fn test_synthesize_empty_text() {
    let mut config = default_config();
    config.enabled = true;
    let engine = TtsEngine::new(config);
    let result = engine.synthesize("", None, None).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("empty"));
}

#[tokio::test]
async fn test_synthesize_text_too_long() {
    let mut config = default_config();
    config.enabled = true;
    config.max_text_length = 10;
    let engine = TtsEngine::new(config);
    let result = engine
        .synthesize("This text is definitely longer than ten chars", None, None)
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("too long"));
}

#[test]
fn test_detect_provider_none() {
    let _ = TtsEngine::detect_provider();
}

#[tokio::test]
async fn test_synthesize_no_provider() {
    let mut config = default_config();
    config.enabled = true;
    let engine = TtsEngine::new(config);
    let result = engine.synthesize("Hello world", None, None).await;
    if let Err(err) = result {
        assert!(err.contains("No TTS provider") || err.contains("not set"));
    }
}

#[test]
fn test_max_audio_constant() {
    assert_eq!(MAX_AUDIO_RESPONSE_BYTES, 10 * 1024 * 1024);
}
