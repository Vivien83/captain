//! Audio transcription routing for media understanding.

use crate::media_local_stt::{transcribe_with_local_whisper, transcribe_with_parakeet_mlx};
use captain_types::media::{
    MediaAttachment, MediaConfig, MediaSource, MediaType, MediaUnderstanding,
};

/// Transcribe audio using speech-to-text.
/// Auto-cascade: local whisper.cpp -> Parakeet MLX -> Groq Whisper ->
/// OpenAI Whisper -> ElevenLabs Scribe. `audio_provider` can pin any
/// supported provider.
pub(crate) async fn transcribe_audio(
    config: &MediaConfig,
    attachment: &MediaAttachment,
) -> Result<MediaUnderstanding, String> {
    let provider = if let Some(provider) = config.audio_provider.as_deref() {
        provider
    } else {
        detect_audio_provider().ok_or("No audio transcription provider configured. Set GROQ_API_KEY, OPENAI_API_KEY, or ELEVENLABS_API_KEY")?
    };

    if provider == crate::native_voice::WHISPER_PROVIDER {
        return transcribe_with_local_whisper(attachment).await;
    }
    if provider == "parakeet-mlx" {
        return transcribe_with_parakeet_mlx(attachment).await;
    }

    let ext = audio_extension(&attachment.mime_type);
    let audio_bytes = read_audio_bytes(attachment).await?;
    let filename = format!("audio.{}", ext);
    let model = config
        .audio_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default_audio_model(provider));

    if provider == "elevenlabs" {
        return transcribe_with_elevenlabs(
            audio_bytes,
            &attachment.mime_type,
            filename,
            model,
            audio_language_hint(attachment),
        )
        .await;
    }

    transcribe_with_openai_compatible(
        provider,
        audio_bytes,
        &attachment.mime_type,
        filename,
        model,
        audio_language_hint(attachment),
    )
    .await
}

fn audio_extension(mime_type: &str) -> &'static str {
    match mime_type {
        "audio/wav" => "wav",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/ogg" => "ogg",
        "audio/webm" => "webm",
        "audio/mp4" | "audio/m4a" => "m4a",
        "audio/flac" => "flac",
        _ => "wav",
    }
}

async fn read_audio_bytes(attachment: &MediaAttachment) -> Result<Vec<u8>, String> {
    match &attachment.source {
        MediaSource::FilePath { path } => tokio::fs::read(path)
            .await
            .map_err(|e| format!("Failed to read audio file '{}': {}", path, e)),
        MediaSource::Base64 { data, .. } => {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| format!("Failed to decode base64 audio: {}", e))
        }
        MediaSource::Url { url } => Err(format!(
            "URL-based audio source not supported for transcription: {}",
            url
        )),
    }
}

async fn transcribe_with_openai_compatible(
    provider: &str,
    audio_bytes: Vec<u8>,
    mime_type: &str,
    filename: String,
    model: &str,
    language: Option<String>,
) -> Result<MediaUnderstanding, String> {
    let (api_url, api_key) = match provider {
        "groq" => (
            "https://api.groq.com/openai/v1/audio/transcriptions",
            std::env::var("GROQ_API_KEY").map_err(|_| "GROQ_API_KEY not set")?,
        ),
        "openai" => (
            "https://api.openai.com/v1/audio/transcriptions",
            std::env::var("OPENAI_API_KEY").map_err(|_| "OPENAI_API_KEY not set")?,
        ),
        other => return Err(format!("Unsupported audio provider: {}", other)),
    };

    tracing::info!(provider, model, filename = %filename, size = audio_bytes.len(), "Sending audio for transcription");

    let file_part = reqwest::multipart::Part::bytes(audio_bytes)
        .file_name(filename)
        .mime_str(mime_type)
        .map_err(|e| format!("Failed to set MIME type: {}", e))?;

    let mut form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("model", model.to_string())
        .text("response_format", "text");
    if let Some(language) = language {
        form = form.text("language", language);
    }

    let resp = reqwest::Client::new()
        .post(api_url)
        .bearer_auth(&api_key)
        .multipart(form)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("Transcription request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Transcription API error ({}): {}", status, body));
    }

    let transcription = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read transcription response: {}", e))?
        .trim()
        .to_string();
    if transcription.is_empty() {
        return Err("Transcription returned empty text".into());
    }

    tracing::info!(
        provider,
        model,
        chars = transcription.len(),
        "Audio transcription complete"
    );

    Ok(MediaUnderstanding {
        media_type: MediaType::Audio,
        description: transcription,
        provider: provider.to_string(),
        model: model.to_string(),
    })
}

/// Detect which audio transcription provider is available.
pub(crate) fn detect_audio_provider() -> Option<&'static str> {
    if crate::native_voice::status().stt_ready {
        return Some(crate::native_voice::WHISPER_PROVIDER);
    }
    if std::env::var("CAPTAIN_ENABLE_PARAKEET_MLX").is_ok() {
        return Some("parakeet-mlx");
    }
    if std::env::var("GROQ_API_KEY").is_ok() {
        return Some("groq");
    }
    if std::env::var("OPENAI_API_KEY").is_ok() {
        return Some("openai");
    }
    if std::env::var("ELEVENLABS_API_KEY").is_ok() {
        return Some("elevenlabs");
    }
    None
}

pub(crate) fn audio_language_hint(attachment: &MediaAttachment) -> Option<String> {
    let hint = attachment.context_hint.as_deref()?.trim();
    for line in hint.lines() {
        if let Some(value) = line.trim().strip_prefix("language:") {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

async fn transcribe_with_elevenlabs(
    audio_bytes: Vec<u8>,
    mime_type: &str,
    filename: String,
    model: &str,
    language: Option<String>,
) -> Result<MediaUnderstanding, String> {
    let api_key = std::env::var("ELEVENLABS_API_KEY").map_err(|_| "ELEVENLABS_API_KEY not set")?;
    let model = std::env::var("ELEVENLABS_STT_MODEL").unwrap_or_else(|_| model.to_string());

    tracing::info!(
        provider = "elevenlabs",
        model = %model,
        filename = %filename,
        size = audio_bytes.len(),
        "Sending audio for transcription"
    );

    let file_part = reqwest::multipart::Part::bytes(audio_bytes)
        .file_name(filename)
        .mime_str(mime_type)
        .map_err(|e| format!("Failed to set MIME type: {}", e))?;

    let mut form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("model_id", model.clone());
    if let Some(language) = language {
        form = form.text("language_code", language);
    }

    let resp = reqwest::Client::new()
        .post("https://api.elevenlabs.io/v1/speech-to-text")
        .header("xi-api-key", api_key)
        .multipart(form)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("ElevenLabs transcription request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "ElevenLabs transcription API error ({}): {}",
            status, body
        ));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read ElevenLabs transcription response: {}", e))?;
    let parsed: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse ElevenLabs transcription response: {e}: {body}"))?;
    let transcription = parsed["text"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_string();
    if transcription.is_empty() {
        return Err("ElevenLabs transcription returned empty text".into());
    }

    tracing::info!(
        provider = "elevenlabs",
        model = %model,
        chars = transcription.len(),
        "Audio transcription complete"
    );

    Ok(MediaUnderstanding {
        media_type: MediaType::Audio,
        description: transcription,
        provider: "elevenlabs".to_string(),
        model,
    })
}

/// Get the default audio model for a provider.
pub(crate) fn default_audio_model(provider: &str) -> &str {
    match provider {
        crate::native_voice::WHISPER_PROVIDER => crate::native_voice::WHISPER_MODEL_NAME,
        "parakeet-mlx" => "mlx-community/parakeet-tdt-0.6b-v3",
        "groq" => "whisper-large-v3-turbo",
        "openai" => "whisper-1",
        "elevenlabs" => "scribe_v2",
        _ => "unknown",
    }
}

#[cfg(test)]
#[path = "media_audio_tests.rs"]
mod tests;
