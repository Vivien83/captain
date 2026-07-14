use super::integration::{
    apply_native_integration, telegram_get_me, telegram_poll_chat_id, telegram_send_validation,
    TelegramDiscovery,
};
use super::setup_support::{setup_csv_or_answer_array, setup_env_or_answer_any};
use crate::ui;

pub(crate) fn setup_configure_telegram_non_interactive(answers: Option<&toml::Value>) -> bool {
    // Fall back to the runtime secret name: exporting TELEGRAM_BOT_TOKEN
    // (what the channel itself reads) is enough for install-time
    // auto-detection, mirroring how STT/TTS fall back to
    // OPENAI/GROQ/ELEVENLABS keys below.
    let bot_token = setup_env_or_answer_any(
        "CAPTAIN_TELEGRAM_BOT_TOKEN",
        answers,
        &["telegram.bot_token", "channels.telegram.bot_token"],
    )
    .or_else(|| {
        std::env::var("TELEGRAM_BOT_TOKEN")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    });
    let Some(bot_token) = bot_token else {
        return false;
    };

    let bot_username = match telegram_get_me(&bot_token) {
        Ok(username) => username,
        Err(e) => {
            ui::warn_with_fix(
                &format!("Telegram bot token rejected, Telegram not configured: {e}"),
                "Check the token from @BotFather, then re-run setup or `captain integration setup telegram`.",
            );
            return false;
        }
    };
    ui::success(&format!("Telegram bot token validated (@{bot_username})"));

    let mut discovered_user: Option<String> = None;
    let default_chat_id = setup_env_or_answer_any(
        "CAPTAIN_TELEGRAM_CHAT_ID",
        answers,
        &[
            "telegram.default_chat_id",
            "channels.telegram.default_chat_id",
        ],
    )
    .or_else(
        // No chat id provided: the getUpdates backlog usually has one if the
        // user already messaged the bot (e.g. right after creating it).
        || match telegram_poll_chat_id(&bot_token, 3) {
            TelegramDiscovery::Found { chat_id, user_id } => {
                discovered_user = Some(user_id);
                Some(chat_id)
            }
            TelegramDiscovery::PollingConflict => {
                ui::check_warn(
                    "Telegram chat discovery skipped: another process (a running Captain \
                     daemon?) is already polling this bot.",
                );
                None
            }
            TelegramDiscovery::NotFound => None,
        },
    );
    let Some(default_chat_id) = default_chat_id else {
        ui::warn_with_fix(
            &format!(
                "Telegram token is valid but no chat id was provided or discoverable. \
                 Send any message to @{bot_username} in Telegram, then re-run setup."
            ),
            "Or set CAPTAIN_TELEGRAM_CHAT_ID / telegram.default_chat_id explicitly.",
        );
        return false;
    };

    let mut allowed_users = setup_csv_or_answer_array(
        "CAPTAIN_TELEGRAM_ALLOWED_USERS",
        answers,
        &["telegram.allowed_users", "channels.telegram.allowed_users"],
    );
    if allowed_users.is_empty() {
        if let Some(user_id) = discovered_user {
            allowed_users = vec![user_id];
        }
    }

    let creds = serde_json::json!({
        "bot_token": bot_token,
        "default_chat_id": default_chat_id,
        "allowed_users": allowed_users,
    });
    match apply_native_integration("telegram", &creds, false) {
        Ok(_) => {
            telegram_send_validation(&bot_token, &default_chat_id);
            true
        }
        Err(e) => {
            ui::warn_with_fix(
                &format!("Telegram non-interactive setup skipped: {e}"),
                "Run `captain integration setup telegram --no-test` interactively.",
            );
            false
        }
    }
}

pub(crate) fn setup_configure_stt_non_interactive(answers: Option<&toml::Value>) -> bool {
    let explicit_provider = setup_env_or_answer_any(
        "CAPTAIN_STT_PROVIDER",
        answers,
        &["stt.provider", "media.audio_provider"],
    );
    let explicit_api_key =
        setup_env_or_answer_any("CAPTAIN_STT_API_KEY", answers, &["stt.api_key"]);
    if explicit_provider.is_none() && explicit_api_key.is_none() {
        return false;
    }
    let provider = explicit_provider.unwrap_or_else(|| "groq".to_string());
    let api_key = explicit_api_key.or_else(|| {
        let env_key = if provider == "openai" {
            "OPENAI_API_KEY"
        } else {
            "GROQ_API_KEY"
        };
        std::env::var(env_key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    });
    let Some(api_key) = api_key else {
        return false;
    };
    let creds = serde_json::json!({
        "provider": provider,
        "api_key": api_key,
    });
    match apply_native_integration("stt_whisper", &creds, false) {
        Ok(_) => true,
        Err(e) => {
            ui::warn_with_fix(
                &format!("STT non-interactive setup skipped: {e}"),
                "Run `captain integration setup stt_whisper --no-test` interactively.",
            );
            false
        }
    }
}

pub(crate) fn setup_configure_tts_non_interactive(
    answers: Option<&toml::Value>,
    preferred_voice: &str,
) -> (bool, Option<&'static str>) {
    let provider = setup_env_or_answer_any(
        "CAPTAIN_TTS_PROVIDER",
        answers,
        &["tts.provider", "assistant.voice_provider"],
    )
    .or_else(|| preferred_tts_provider(preferred_voice).map(ToOwned::to_owned));
    let Some(provider) = provider.map(|value| value.to_ascii_lowercase()) else {
        return (false, None);
    };
    if provider.contains("eleven") {
        return setup_configure_elevenlabs_tts(answers);
    }
    if provider.contains("openai") || provider.contains("nova") {
        return setup_configure_openai_tts(answers);
    }

    (false, None)
}

fn setup_configure_elevenlabs_tts(answers: Option<&toml::Value>) -> (bool, Option<&'static str>) {
    let api_key = setup_env_or_answer_any(
        "CAPTAIN_TTS_API_KEY",
        answers,
        &["tts.api_key", "tts.elevenlabs.api_key"],
    )
    .or_else(|| env_secret("ELEVENLABS_API_KEY"));
    let Some(api_key) = api_key else {
        return (false, None);
    };
    let mut creds = serde_json::json!({"api_key": api_key});
    if let Some(voice_id) = setup_env_or_answer_any(
        "CAPTAIN_ELEVENLABS_VOICE_ID",
        answers,
        &["tts.voice_id", "tts.elevenlabs.voice_id"],
    ) {
        creds["voice_id"] = serde_json::Value::String(voice_id);
    }
    if let Some(model_id) = setup_env_or_answer_any(
        "CAPTAIN_ELEVENLABS_MODEL_ID",
        answers,
        &["tts.model_id", "tts.elevenlabs.model_id", "tts.model"],
    ) {
        creds["model_id"] = serde_json::Value::String(model_id);
    }
    match apply_native_integration("tts_elevenlabs", &creds, false) {
        Ok(_) => (true, Some("elevenlabs")),
        Err(e) => {
            ui::warn_with_fix(
                &format!("ElevenLabs non-interactive setup skipped: {e}"),
                "Run `captain integration setup tts_elevenlabs --no-test` interactively.",
            );
            (false, None)
        }
    }
}

fn setup_configure_openai_tts(answers: Option<&toml::Value>) -> (bool, Option<&'static str>) {
    let api_key = setup_env_or_answer_any(
        "CAPTAIN_TTS_API_KEY",
        answers,
        &["tts.api_key", "tts.openai.api_key"],
    )
    .or_else(|| env_secret("OPENAI_API_KEY"));
    let Some(api_key) = api_key else {
        return (false, None);
    };
    let mut creds = serde_json::json!({"api_key": api_key});
    if let Some(voice) = setup_env_or_answer_any(
        "CAPTAIN_OPENAI_TTS_VOICE",
        answers,
        &["tts.voice", "tts.openai.voice"],
    ) {
        creds["voice"] = serde_json::Value::String(voice);
    }
    if let Some(model) = setup_env_or_answer_any(
        "CAPTAIN_OPENAI_TTS_MODEL",
        answers,
        &["tts.model", "tts.openai.model"],
    ) {
        creds["model"] = serde_json::Value::String(model);
    }
    if let Some(format) = setup_env_or_answer_any(
        "CAPTAIN_OPENAI_TTS_FORMAT",
        answers,
        &["tts.format", "tts.openai.format"],
    ) {
        creds["format"] = serde_json::Value::String(format);
    }
    match apply_native_integration("tts_openai", &creds, false) {
        Ok(_) => (true, Some("openai")),
        Err(e) => {
            ui::warn_with_fix(
                &format!("OpenAI TTS non-interactive setup skipped: {e}"),
                "Run `captain integration setup tts_openai --no-test` interactively.",
            );
            (false, None)
        }
    }
}

fn preferred_tts_provider(preference: &str) -> Option<&'static str> {
    let lower = preference.to_ascii_lowercase();
    if lower.contains("eleven") {
        Some("elevenlabs")
    } else if lower.contains("openai")
        || lower.contains("nova")
        || lower.contains("alloy")
        || lower.contains("echo")
        || lower.contains("fable")
        || lower.contains("onyx")
        || lower.contains("shimmer")
    {
        Some("openai")
    } else {
        None
    }
}

fn env_secret(env_key: &str) -> Option<String> {
    std::env::var(env_key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Serialized via the shared env lock in crate::tests would be ideal, but
    // these two only REMOVE vars and assert the no-token early return, which
    // never races with the other env tests' variables.
    #[test]
    fn telegram_non_interactive_without_any_token_is_a_silent_no_op() {
        std::env::remove_var("CAPTAIN_TELEGRAM_BOT_TOKEN");
        std::env::remove_var("TELEGRAM_BOT_TOKEN");
        assert!(!setup_configure_telegram_non_interactive(None));
    }

    #[test]
    fn telegram_non_interactive_ignores_empty_runtime_token() {
        std::env::remove_var("CAPTAIN_TELEGRAM_BOT_TOKEN");
        std::env::set_var("TELEGRAM_BOT_TOKEN", "   ");
        assert!(!setup_configure_telegram_non_interactive(None));
        std::env::remove_var("TELEGRAM_BOT_TOKEN");
    }
}
