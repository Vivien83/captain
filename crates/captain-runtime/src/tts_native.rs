use super::{TtsResult, MAX_AUDIO_RESPONSE_BYTES};
use captain_types::config::TtsConfig;
use std::{path::PathBuf, process::Stdio};
use tokio::io::AsyncWriteExt;

pub(super) async fn synthesize(
    config: &TtsConfig,
    text: &str,
    voice_override: Option<&str>,
) -> Result<TtsResult, String> {
    let status = crate::native_voice::status();
    let preferred = normalize_engine(&config.local_native.preferred_engine).unwrap_or("kokoro");
    let fallback = normalize_engine(&config.local_native.fallback_engine);
    let mut attempted = Vec::new();
    let mut last_error: Option<String> = None;

    for engine in [Some(preferred), fallback].into_iter().flatten() {
        if engine == "none" || attempted.contains(&engine) {
            continue;
        }
        attempted.push(engine);
        match engine {
            "kokoro" if status.kokoro_ready => {
                match synthesize_kokoro(text, voice_override, config.timeout_secs).await {
                    Ok(result) => return Ok(result),
                    Err(err) => last_error = Some(err),
                }
            }
            "kokoro" => last_error = Some("Kokoro TTS is not ready".to_string()),
            "piper" if status.piper_ready => {
                match synthesize_piper(text, config.timeout_secs).await {
                    Ok(result) => return Ok(result),
                    Err(err) => last_error = Some(err),
                }
            }
            "piper" => last_error = Some("Piper TTS is not ready".to_string()),
            _ => {}
        }
    }

    Err(last_error
        .unwrap_or_else(|| "Native TTS is not installed. Run `captain voice install`.".to_string()))
}

async fn synthesize_piper(text: &str, timeout_secs: u64) -> Result<TtsResult, String> {
    use tokio::time::{timeout, Duration};

    let piper = crate::native_voice::find_piper_binary()
        .ok_or("Piper binary not found. Run `captain voice install`.")?;
    let voice = crate::native_voice::find_piper_voice()
        .ok_or("Piper voice model not found. Run `captain voice install`.")?;
    let output_path = temp_wav_path("captain_piper");

    let mut cmd = tokio::process::Command::new(&piper);
    cmd.arg("--model")
        .arg(&voice)
        .arg("--output_file")
        .arg(&output_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to launch Piper TTS: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .await
            .map_err(|e| format!("Failed to feed Piper text: {e}"))?;
    }
    let output = timeout(
        Duration::from_secs(timeout_secs.max(10)),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| "Piper TTS timed out".to_string())?
    .map_err(|e| format!("Failed to wait for Piper TTS: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = tokio::fs::remove_file(&output_path).await;
        return Err(format!("Piper TTS failed: {}", stderr.trim()));
    }

    let audio_data = tokio::fs::read(&output_path)
        .await
        .map_err(|e| format!("Failed to read Piper output: {e}"))?;
    let _ = tokio::fs::remove_file(&output_path).await;
    check_audio_size(&audio_data)?;

    Ok(local_result(
        audio_data,
        "piper",
        crate::native_voice::PIPER_VOICE_ID,
        text,
    ))
}

async fn synthesize_kokoro(
    text: &str,
    voice_override: Option<&str>,
    timeout_secs: u64,
) -> Result<TtsResult, String> {
    use tokio::time::{timeout, Duration};

    let python = crate::native_voice::find_kokoro_python()
        .ok_or("Kokoro Python runtime not found. Run `captain voice install`.")?;
    let script = crate::native_voice::find_kokoro_script()
        .ok_or("Kokoro script not found. Run `captain voice install`.")?;
    let model = crate::native_voice::find_kokoro_model()
        .ok_or("Kokoro model not found. Run `captain voice install`.")?;
    let voices = crate::native_voice::find_kokoro_voices()
        .ok_or("Kokoro voices file not found. Run `captain voice install`.")?;
    let output_path = temp_wav_path("captain_kokoro");
    let voice = voice_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("ff_siwis");

    let output = timeout(
        Duration::from_secs(timeout_secs.max(20)),
        tokio::process::Command::new(&python)
            .arg(&script)
            .arg("--model")
            .arg(&model)
            .arg("--voices")
            .arg(&voices)
            .arg("--voice")
            .arg(voice)
            .arg("--output")
            .arg(&output_path)
            .arg("--text")
            .arg(text)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "Kokoro TTS timed out".to_string())?
    .map_err(|e| format!("Failed to launch Kokoro TTS: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = tokio::fs::remove_file(&output_path).await;
        return Err(format!("Kokoro TTS failed: {}", stderr.trim()));
    }

    let audio_data = tokio::fs::read(&output_path)
        .await
        .map_err(|e| format!("Failed to read Kokoro output: {e}"))?;
    let _ = tokio::fs::remove_file(&output_path).await;
    check_audio_size(&audio_data)?;

    Ok(local_result(audio_data, "kokoro", voice, text))
}

fn temp_wav_path(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}_{}.wav", uuid::Uuid::new_v4()))
}

fn check_audio_size(audio_data: &[u8]) -> Result<(), String> {
    if audio_data.is_empty() {
        return Err("Native TTS produced an empty audio file".into());
    }
    if audio_data.len() > MAX_AUDIO_RESPONSE_BYTES {
        return Err(format!(
            "Audio data exceeds {}MB limit",
            MAX_AUDIO_RESPONSE_BYTES / 1024 / 1024
        ));
    }
    Ok(())
}

fn local_result(audio_data: Vec<u8>, engine: &str, voice: &str, text: &str) -> TtsResult {
    let word_count = text.split_whitespace().count();
    let duration_ms = (word_count as u64 * 400).max(500);
    TtsResult {
        audio_data,
        format: "wav".to_string(),
        provider: crate::native_voice::NATIVE_TTS_PROVIDER.to_string(),
        voice: voice.to_string(),
        voice_source: engine.to_string(),
        duration_estimate_ms: duration_ms,
    }
}

pub(super) fn normalize_engine(engine: &str) -> Option<&'static str> {
    match engine.trim().to_ascii_lowercase().as_str() {
        "kokoro" => Some("kokoro"),
        "piper" => Some("piper"),
        "none" | "off" | "disabled" => Some("none"),
        "" => None,
        _ => None,
    }
}
