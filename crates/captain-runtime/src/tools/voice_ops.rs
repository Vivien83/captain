//! Text-to-speech and speech-to-text runtime handlers.

use std::path::Path;

use crate::media_understanding::MediaEngine;
use crate::tts::TtsEngine;

use super::resolve_file_path;

pub(crate) async fn tool_text_to_speech(
    input: &serde_json::Value,
    tts_engine: Option<&TtsEngine>,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let engine =
        tts_engine.ok_or("TTS engine not available. Ensure tts.enabled=true in config.")?;
    let text = input["text"].as_str().ok_or("Missing 'text' parameter")?;
    let voice = input["voice_id"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| input["voice"].as_str());
    let format = input["format"].as_str();
    let result = engine.synthesize(text, voice, format).await?;

    let saved_path = if let Some(workspace) = workspace_root {
        let output_dir = workspace.join("output");
        tokio::fs::create_dir_all(&output_dir)
            .await
            .map_err(|e| format!("Failed to create output dir: {e}"))?;

        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let path = output_dir.join(format!("tts_{timestamp}.{}", result.format));
        tokio::fs::write(&path, &result.audio_data)
            .await
            .map_err(|e| format!("Failed to write audio file: {e}"))?;
        Some(path.display().to_string())
    } else {
        None
    };

    let mut payload = serde_json::json!({
        "saved_to": saved_path,
        "format": result.format,
        "provider": result.provider,
        "voice": result.voice,
        "voice_source": result.voice_source,
        "duration_estimate_ms": result.duration_estimate_ms,
        "size_bytes": result.audio_data.len(),
    });

    if let Some(note) =
        ignored_voice_note(voice, &result.provider, &result.voice, &result.voice_source)
    {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert(
                "requested_voice_ignored".to_string(),
                serde_json::json!(true),
            );
            obj.insert(
                "requested_voice".to_string(),
                serde_json::json!(note.requested_voice),
            );
            obj.insert("reason".to_string(), serde_json::json!(note.reason));
        }
    }

    serde_json::to_string_pretty(&payload).map_err(|e| format!("Serialize error: {e}"))
}

/// Details reported back to the caller when the voice actually used to
/// synthesize the audio differs from the one it explicitly requested.
struct IgnoredVoiceNote {
    requested_voice: String,
    reason: String,
}

/// Compares the voice the caller asked for against the voice the engine
/// actually used. Returns `None` when no voice was requested, or when
/// the request was honored — the tool result then stays unchanged, as
/// today. Returns `Some` (with a short human-readable reason) when the
/// requested voice was silently overridden, e.g. providers like the
/// local-native Piper engine that only ever speak with their configured
/// voice model regardless of what's asked for.
fn ignored_voice_note(
    requested: Option<&str>,
    result_provider: &str,
    result_voice: &str,
    result_voice_source: &str,
) -> Option<IgnoredVoiceNote> {
    let requested = requested.map(str::trim).filter(|s| !s.is_empty())?;
    if requested.eq_ignore_ascii_case(result_voice) {
        return None;
    }

    let reason = match (result_provider, result_voice_source) {
        (provider, "piper") if provider == crate::native_voice::NATIVE_TTS_PROVIDER => {
            "local-native provider uses the configured Piper voice".to_string()
        }
        (provider, "kokoro") if provider == crate::native_voice::NATIVE_TTS_PROVIDER => {
            "local-native kokoro engine did not honor the requested voice".to_string()
        }
        (_, "config") => {
            "configured TTS provider does not allow a per-request voice override".to_string()
        }
        _ => "the requested voice was not applied by the provider".to_string(),
    };

    Some(IgnoredVoiceNote {
        requested_voice: requested.to_string(),
        reason,
    })
}

pub(crate) async fn tool_speech_to_text(
    input: &serde_json::Value,
    media_engine: Option<&MediaEngine>,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let engine = media_engine.ok_or("Media engine not available for speech-to-text")?;
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let language = input["language"].as_str();
    let resolved = resolve_file_path(raw_path, workspace_root)?;
    let data = tokio::fs::read(&resolved)
        .await
        .map_err(|e| format!("Failed to read audio file: {e}"))?;

    let ext = resolved
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp3");
    let mime_type = match ext {
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "webm" => "audio/webm",
        _ => "audio/mpeg",
    };

    use captain_types::media::{MediaAttachment, MediaSource, MediaType};
    let attachment = MediaAttachment {
        media_type: MediaType::Audio,
        mime_type: mime_type.to_string(),
        source: MediaSource::Base64 {
            data: {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(&data)
            },
            mime_type: mime_type.to_string(),
        },
        size_bytes: data.len() as u64,
        context_hint: language
            .filter(|s| !s.trim().is_empty())
            .map(|s| format!("language:{}", s.trim())),
        batch_size_hint: None,
    };

    let understanding = engine.transcribe_audio(&attachment).await?;
    serde_json::to_string_pretty(&serde_json::json!({
        "transcript": understanding.description,
        "provider": understanding.provider,
        "model": understanding.model,
    }))
    .map_err(|e| format!("Serialize error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignored_voice_note_none_when_no_voice_requested() {
        assert!(ignored_voice_note(None, "local-native", "fr_FR-siwis-medium", "piper").is_none());
    }

    #[test]
    fn ignored_voice_note_none_when_requested_voice_applied() {
        assert!(ignored_voice_note(Some("nova"), "openai", "nova", "request").is_none());
    }

    #[test]
    fn ignored_voice_note_none_when_applied_case_insensitively() {
        // Providers may normalize casing; a case-only difference is not
        // a silent override.
        assert!(ignored_voice_note(Some("Nova"), "openai", "nova", "request").is_none());
    }

    #[test]
    fn ignored_voice_note_flags_piper_ignoring_requested_voice() {
        let note = ignored_voice_note(Some("nova"), "local-native", "fr_FR-siwis-medium", "piper")
            .expect("piper always overrides the requested voice");
        assert_eq!(note.requested_voice, "nova");
        assert_eq!(
            note.reason,
            "local-native provider uses the configured Piper voice"
        );
    }

    #[test]
    fn ignored_voice_note_flags_config_locked_cloud_provider() {
        let note = ignored_voice_note(Some("nova"), "openai", "alloy", "config")
            .expect("configured provider ignored the per-request override");
        assert_eq!(note.requested_voice, "nova");
        assert_eq!(
            note.reason,
            "configured TTS provider does not allow a per-request voice override"
        );
    }

    #[test]
    fn ignored_voice_note_trims_whitespace_before_comparing() {
        assert!(ignored_voice_note(Some("  nova  "), "openai", "nova", "request").is_none());
    }

    #[test]
    fn ignored_voice_note_none_for_blank_request() {
        assert!(
            ignored_voice_note(Some("   "), "local-native", "fr_FR-siwis-medium", "piper")
                .is_none()
        );
    }
}
