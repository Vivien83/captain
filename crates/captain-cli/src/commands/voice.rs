use super::voice_install::install_native_voice_assets;
use crate::{captain_home, restrict_file_permissions, ui};

pub(crate) fn cmd_voice_status(json: bool) {
    let status = captain_runtime::native_voice::status();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&status).unwrap_or_default()
        );
        return;
    }

    ui::section("Native Voice");
    ui::kv("Home", &status.home_dir);
    ui::kv(
        "STT",
        if status.stt_ready {
            "ready (whisper.cpp small)"
        } else {
            "pending"
        },
    );
    ui::kv(
        "Whisper bin",
        status.whisper_binary.as_deref().unwrap_or("-"),
    );
    ui::kv(
        "Whisper model",
        status.whisper_model.as_deref().unwrap_or("-"),
    );
    ui::kv(
        "TTS",
        status
            .tts_engine
            .unwrap_or(if status.tts_ready { "ready" } else { "pending" }),
    );
    ui::kv(
        "Kokoro",
        if status.kokoro_ready {
            "ready"
        } else {
            "pending"
        },
    );
    ui::kv(
        "Piper",
        if status.piper_ready {
            "ready"
        } else {
            "pending"
        },
    );
    ui::kv(
        "ffmpeg",
        if status.ffmpeg_ready {
            "ready"
        } else {
            "lazy/download"
        },
    );
    if !status.stt_ready || !status.tts_ready {
        ui::hint("Run `captain voice install` to repair native STT/TTS.");
    }
}

pub(crate) fn cmd_voice_doctor(json: bool) {
    cmd_voice_status(json);
}

pub(crate) fn cmd_voice_install(best_effort: bool, force: bool) {
    ui::section("Native Voice Install");
    println!("  Installing local STT/TTS defaults: whisper.cpp small + Kokoro/Piper fallback.");

    let mut errors = Vec::new();
    if let Err(e) = install_native_voice_assets(force) {
        errors.push(e);
    }
    if let Err(e) = apply_native_voice_config() {
        errors.push(e);
    }

    let status = captain_runtime::native_voice::status();
    if status.stt_ready && status.tts_ready {
        ui::success("Native voice ready.");
    } else {
        if !status.stt_ready {
            errors.push("STT not ready after install".to_string());
        }
        if !status.tts_ready {
            errors.push("TTS not ready after install".to_string());
        }
    }

    if errors.is_empty() {
        return;
    }

    for err in &errors {
        ui::check_warn(err);
    }
    if !best_effort {
        ui::error_with_fix(
            "Native voice install incomplete",
            "Run `captain voice doctor` then retry `captain voice install`.",
        );
        std::process::exit(1);
    }
}

pub(crate) fn cmd_voice_test(json: bool) {
    let runtime = tokio::runtime::Runtime::new().unwrap_or_else(|e| {
        ui::error(&format!("Failed to create async runtime: {e}"));
        std::process::exit(1);
    });
    let result = runtime.block_on(async {
        let tts_config = captain_types::config::TtsConfig {
            enabled: true,
            provider: Some(captain_runtime::native_voice::NATIVE_TTS_PROVIDER.to_string()),
            ..Default::default()
        };
        let tts = captain_runtime::tts::TtsEngine::new(tts_config);
        let spoken = tts
            .synthesize("bonjour captain", None, None)
            .await
            .map_err(|e| format!("TTS test failed: {e}"))?;
        let wav_path =
            std::env::temp_dir().join(format!("captain_voice_test_{}.wav", uuid::Uuid::new_v4()));
        tokio::fs::write(&wav_path, &spoken.audio_data)
            .await
            .map_err(|e| format!("write test audio failed: {e}"))?;

        let media = captain_types::media::MediaConfig {
            audio_provider: Some(captain_runtime::native_voice::WHISPER_PROVIDER.to_string()),
            ..Default::default()
        };
        let engine = captain_runtime::media_understanding::MediaEngine::new(media);
        let attachment = captain_types::media::MediaAttachment {
            media_type: captain_types::media::MediaType::Audio,
            mime_type: "audio/wav".to_string(),
            source: captain_types::media::MediaSource::FilePath {
                path: wav_path.display().to_string(),
            },
            size_bytes: spoken.audio_data.len() as u64,
            context_hint: Some("language:fr".to_string()),
            batch_size_hint: None,
        };
        let transcript = engine
            .transcribe_audio(&attachment)
            .await
            .map_err(|e| format!("STT test failed: {e}"))?;
        let _ = tokio::fs::remove_file(&wav_path).await;
        Ok::<serde_json::Value, String>(serde_json::json!({
            "status": "ok",
            "tts_provider": spoken.provider,
            "tts_voice_source": spoken.voice_source,
            "transcript": transcript.description,
            "stt_provider": transcript.provider,
            "stt_model": transcript.model,
        }))
    });

    match result {
        Ok(value) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).unwrap_or_default()
                );
            } else {
                ui::section("Native Voice Test");
                ui::kv_ok("Status", "ok");
                ui::kv("Transcript", value["transcript"].as_str().unwrap_or(""));
            }
        }
        Err(e) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "status": "error",
                        "error": e,
                    }))
                    .unwrap_or_default()
                );
            } else {
                ui::error_with_fix(&e, "Run `captain voice install`.");
            }
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_voice_uninstall(confirm: bool) {
    if !confirm {
        ui::error_with_fix(
            "Confirmation required",
            "Run `captain voice uninstall --yes` to remove native voice assets.",
        );
        std::process::exit(1);
    }
    let home = captain_runtime::native_voice::captain_home_dir();
    for path in [
        home.join("native").join("voice-venv"),
        home.join("native").join("kokoro-venv"),
        home.join("native").join("kokoro"),
        home.join("models").join("stt"),
        home.join("models").join("tts"),
    ] {
        if path.exists() {
            if let Err(e) = std::fs::remove_dir_all(&path) {
                ui::check_warn(&format!("Failed to remove {}: {e}", path.display()));
            }
        }
    }
    ui::success("Native voice assets removed.");
}

pub(crate) fn apply_native_voice_config() -> Result<(), String> {
    let config_path = captain_home().join("config.toml");
    let patches = vec![
        captain_runtime::integrations::ConfigPatch {
            path: vec![],
            key: "stt_model".to_string(),
            value: toml_edit::value(captain_runtime::native_voice::WHISPER_MODEL_NAME),
        },
        captain_runtime::integrations::ConfigPatch {
            path: vec!["media".to_string()],
            key: "audio_transcription".to_string(),
            value: toml_edit::value(true),
        },
        captain_runtime::integrations::ConfigPatch {
            path: vec!["media".to_string()],
            key: "audio_provider".to_string(),
            value: toml_edit::value(captain_runtime::native_voice::WHISPER_PROVIDER),
        },
        captain_runtime::integrations::ConfigPatch {
            path: vec!["media".to_string()],
            key: "audio_model".to_string(),
            value: toml_edit::value(captain_runtime::native_voice::WHISPER_MODEL_NAME),
        },
        captain_runtime::integrations::ConfigPatch {
            path: vec!["tts".to_string()],
            key: "enabled".to_string(),
            value: toml_edit::value(true),
        },
        captain_runtime::integrations::ConfigPatch {
            path: vec!["tts".to_string()],
            key: "provider".to_string(),
            value: toml_edit::value(captain_runtime::native_voice::NATIVE_TTS_PROVIDER),
        },
        captain_runtime::integrations::ConfigPatch {
            path: vec!["tts".to_string(), "local_native".to_string()],
            key: "preferred_engine".to_string(),
            value: toml_edit::value("kokoro"),
        },
        captain_runtime::integrations::ConfigPatch {
            path: vec!["tts".to_string(), "local_native".to_string()],
            key: "fallback_engine".to_string(),
            value: toml_edit::value("piper"),
        },
        captain_runtime::integrations::ConfigPatch {
            path: vec!["tts".to_string(), "local_native".to_string()],
            key: "language".to_string(),
            value: toml_edit::value("fr"),
        },
    ];
    captain_runtime::integrations::apply_config_patch(&config_path, &patches)?;
    restrict_file_permissions(&config_path);
    Ok(())
}
