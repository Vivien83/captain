//! Local speech-to-text backends for media understanding.

use captain_types::media::{MediaAttachment, MediaSource, MediaType, MediaUnderstanding};
use std::path::{Path, PathBuf};

/// Transcribe audio locally with whisper.cpp.
pub(crate) async fn transcribe_with_local_whisper(
    attachment: &MediaAttachment,
) -> Result<MediaUnderstanding, String> {
    use tokio::time::{timeout, Duration};

    let whisper_bin = crate::native_voice::find_whisper_binary()
        .ok_or("local whisper.cpp binary not found. Run `captain voice install`.")?;
    let model_path = crate::native_voice::find_whisper_model()
        .ok_or("local whisper-small model not found. Run `captain voice install`.")?;
    let temp_dir = std::env::temp_dir().join(format!("captain_whisper_{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(|e| format!("Failed to create temp dir: {e}"))?;

    let input_path = materialize_audio_for_local_stt(attachment, &temp_dir).await?;
    let wav_path = temp_dir.join("input.wav");

    crate::video::ensure_ffmpeg().await?;
    crate::video::transcode_audio_to_wav(&input_path, &wav_path).await?;

    let output_base = temp_dir.join("transcript");
    let mut cmd = build_whisper_command(&whisper_bin, &model_path, &wav_path, &output_base);
    let output = timeout(Duration::from_secs(300), cmd.output())
        .await
        .map_err(|_| "local whisper.cpp timed out after 5 minutes".to_string())?
        .map_err(|e| format!("Failed to launch local whisper.cpp: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if !stderr.trim().is_empty() {
            stderr.trim()
        } else {
            stdout.trim()
        };
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        return Err(format!("local whisper.cpp failed: {detail}"));
    }

    let transcription = read_whisper_text_output(&temp_dir, &wav_path, &output_base, &output)
        .await?
        .trim()
        .to_string();
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;

    if transcription.is_empty() {
        return Err("local whisper.cpp returned empty transcription".into());
    }

    tracing::info!(
        provider = crate::native_voice::WHISPER_PROVIDER,
        model = crate::native_voice::WHISPER_MODEL_NAME,
        chars = transcription.len(),
        "Local audio transcription complete"
    );

    Ok(MediaUnderstanding {
        media_type: MediaType::Audio,
        description: transcription,
        provider: crate::native_voice::WHISPER_PROVIDER.to_string(),
        model: crate::native_voice::WHISPER_MODEL_NAME.to_string(),
    })
}

async fn materialize_audio_for_local_stt(
    attachment: &MediaAttachment,
    temp_dir: &Path,
) -> Result<PathBuf, String> {
    match &attachment.source {
        MediaSource::FilePath { path } => Ok(PathBuf::from(path)),
        MediaSource::Base64 { data, mime_type } => {
            use base64::Engine;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| format!("Failed to decode base64 audio: {e}"))?;
            let ext = match mime_type.as_str() {
                "audio/wav" | "audio/x-wav" => "wav",
                "audio/mpeg" | "audio/mp3" => "mp3",
                "audio/ogg" => "ogg",
                "audio/webm" => "webm",
                "audio/mp4" | "audio/m4a" => "m4a",
                "audio/flac" => "flac",
                _ => "audio",
            };
            // Named "source.*" (not "input.*") so it never collides with the
            // "input.wav" transcode target computed by the caller: ffmpeg
            // cannot read and overwrite the same file in one pass, and would
            // fail with "Error opening output files: Invalid argument" if the
            // audio was already a WAV (materialized here as "input.wav" too).
            let path = temp_dir.join(format!("source.{ext}"));
            tokio::fs::write(&path, decoded)
                .await
                .map_err(|e| format!("Failed to write temp audio: {e}"))?;
            Ok(path)
        }
        MediaSource::Url { url } => Err(format!(
            "URL audio not supported for local whisper.cpp transcription: {url}"
        )),
    }
}

fn build_whisper_command(
    whisper_bin: &Path,
    model_path: &Path,
    wav_path: &Path,
    output_base: &Path,
) -> tokio::process::Command {
    let binary_name = whisper_bin
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut cmd = tokio::process::Command::new(whisper_bin);
    if binary_name.contains("whisper-cpp") {
        cmd.arg("-m")
            .arg(model_path)
            .arg(wav_path)
            .arg("--output-txt");
    } else {
        cmd.arg("-m")
            .arg(model_path)
            .arg("-f")
            .arg(wav_path)
            .arg("-otxt")
            .arg("-of")
            .arg(output_base)
            .arg("-l")
            .arg("auto")
            .arg("-np");
    }
    cmd.kill_on_drop(true);
    cmd
}

async fn read_whisper_text_output(
    temp_dir: &Path,
    wav_path: &Path,
    output_base: &Path,
    output: &std::process::Output,
) -> Result<String, String> {
    let candidates = [
        output_base.with_extension("txt"),
        PathBuf::from(format!("{}.txt", wav_path.display())),
        temp_dir.join("input.wav.txt"),
    ];
    for candidate in candidates {
        if candidate.exists() {
            let text = tokio::fs::read_to_string(&candidate)
                .await
                .map_err(|e| format!("Failed to read whisper output: {e}"))?;
            return Ok(clean_whisper_transcript(&text));
        }
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.trim().is_empty() {
        return Ok(clean_whisper_transcript(&stdout));
    }
    Err("local whisper.cpp did not produce a transcript file".into())
}

fn clean_whisper_transcript(raw: &str) -> String {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("whisper_"))
        .filter(|line| !line.starts_with("system_info:"))
        .filter(|line| !line.starts_with("main:"))
        .map(|line| {
            if line.starts_with('[') {
                line.split_once(']')
                    .map(|(_, rest)| rest.trim())
                    .unwrap_or(line)
            } else {
                line
            }
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Transcribe audio using Parakeet MLX (local, via uv + Python).
pub(crate) async fn transcribe_with_parakeet_mlx(
    attachment: &MediaAttachment,
) -> Result<MediaUnderstanding, String> {
    use tokio::time::{timeout, Duration};

    let (audio_path, is_temp) = match &attachment.source {
        MediaSource::FilePath { path } => (PathBuf::from(path), false),
        MediaSource::Base64 { data, mime_type } => {
            use base64::Engine;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| format!("Failed to decode base64 audio: {e}"))?;
            let ext = match mime_type.as_str() {
                "audio/wav" | "audio/x-wav" => "wav",
                "audio/mpeg" | "audio/mp3" => "mp3",
                "audio/ogg" => "ogg",
                "audio/webm" => "webm",
                "audio/mp4" | "audio/m4a" => "m4a",
                "audio/flac" => "flac",
                _ => "wav",
            };
            let path = std::env::temp_dir().join(format!(
                "captain_parakeet_{}.{}",
                uuid::Uuid::new_v4(),
                ext
            ));
            tokio::fs::write(&path, decoded)
                .await
                .map_err(|e| format!("Failed to write temp audio: {e}"))?;
            (path, true)
        }
        MediaSource::Url { url } => {
            return Err(format!("URL audio not supported for parakeet-mlx: {url}"));
        }
    };

    let script = r#"
import json, sys
from parakeet_mlx import from_pretrained
model = from_pretrained("mlx-community/parakeet-tdt-0.6b-v3")
result = model.transcribe(sys.argv[1])
print(json.dumps({"text": result.text, "model": "mlx-community/parakeet-tdt-0.6b-v3"}))
"#;

    let mut cmd = tokio::process::Command::new("uv");
    cmd.args([
        "run",
        "--with",
        "parakeet-mlx",
        "python3",
        "-c",
        script,
        &audio_path.to_string_lossy(),
    ]);
    cmd.env("PYTHONUNBUFFERED", "1");
    cmd.kill_on_drop(true);

    let output = timeout(Duration::from_secs(900), cmd.output())
        .await
        .map_err(|_| "parakeet-mlx timed out after 15 minutes".to_string())?
        .map_err(|e| format!("Failed to launch parakeet-mlx: {e}"))?;

    if is_temp {
        let _ = tokio::fs::remove_file(&audio_path).await;
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("parakeet-mlx failed: {}", stderr.trim()));
    }

    let stdout =
        String::from_utf8(output.stdout).map_err(|e| format!("parakeet-mlx non-UTF8: {e}"))?;
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("parakeet-mlx parse failed: {e}"))?;

    let text = parsed["text"]
        .as_str()
        .ok_or("missing text field")?
        .trim()
        .to_string();
    if text.is_empty() {
        return Err("parakeet-mlx returned empty transcription".into());
    }

    Ok(MediaUnderstanding {
        media_type: MediaType::Audio,
        description: text,
        provider: "parakeet-mlx".to_string(),
        model: parsed["model"]
            .as_str()
            .unwrap_or("parakeet-tdt-0.6b-v3")
            .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav_attachment(data: &[u8]) -> MediaAttachment {
        use base64::Engine;
        MediaAttachment {
            media_type: MediaType::Audio,
            mime_type: "audio/wav".to_string(),
            source: MediaSource::Base64 {
                data: base64::engine::general_purpose::STANDARD.encode(data),
                mime_type: "audio/wav".to_string(),
            },
            size_bytes: data.len() as u64,
            context_hint: None,
            batch_size_hint: None,
        }
    }

    #[tokio::test]
    async fn materialized_wav_input_never_collides_with_transcode_target() {
        let temp_dir = std::env::temp_dir().join(format!(
            "captain_media_local_stt_test_{}",
            uuid::Uuid::new_v4()
        ));
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();

        let attachment = wav_attachment(b"not-really-a-wav-but-irrelevant-here");
        let input_path = materialize_audio_for_local_stt(&attachment, &temp_dir)
            .await
            .unwrap();

        // The caller always transcodes into `temp_dir/input.wav`; the
        // materialized source must never be named identically, or
        // `transcode_audio_to_wav` receives the same path as input and
        // output and fails (see crate::video::transcode_audio_to_wav).
        let wav_transcode_target = temp_dir.join("input.wav");
        assert_ne!(input_path, wav_transcode_target);
        assert!(input_path.exists());

        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }
}
