use std::path::Path;

use super::integration::{print_integration_setup_outcome, run_integration_setup_native};
use super::setup_support::{
    setup_config_bool, setup_config_string, setup_config_string_array, setup_config_value,
    setup_read_config_value, setup_secret_env_value,
};
use super::voice::apply_native_voice_config;
use crate::{prompt_input, ui};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SetupOptionState {
    Missing,
    Partial,
    Complete,
}

pub(crate) struct SetupVoiceOutcome {
    pub(crate) stt_done: bool,
    pub(crate) tts_done: bool,
    pub(crate) tts_provider: Option<&'static str>,
}

pub(crate) fn setup_telegram_state(config: Option<&toml::Value>) -> SetupOptionState {
    let token_env = setup_config_string(config, "channels.telegram.bot_token_env");
    let chat_id = setup_config_string(config, "channels.telegram.default_chat_id");
    let allowed_users = setup_config_string_array(config, "channels.telegram.allowed_users");
    let has_poll_interval = setup_config_value(config, "channels.telegram.poll_interval").is_some();
    let has_token = token_env
        .as_deref()
        .and_then(setup_secret_env_value)
        .is_some();

    if token_env.is_some() && has_token && chat_id.is_some() {
        SetupOptionState::Complete
    } else if token_env.is_some()
        || chat_id.is_some()
        || !allowed_users.is_empty()
        || has_poll_interval
    {
        SetupOptionState::Partial
    } else {
        SetupOptionState::Missing
    }
}

pub(crate) fn setup_stt_state(config: Option<&toml::Value>) -> SetupOptionState {
    let provider = setup_config_string(config, "media.audio_provider");
    let model = setup_config_string(config, "media.audio_model");
    if provider.as_deref() == Some(captain_runtime::native_voice::WHISPER_PROVIDER) {
        return if captain_runtime::native_voice::status().stt_ready {
            SetupOptionState::Complete
        } else {
            SetupOptionState::Partial
        };
    }
    let has_secret = match provider.as_deref() {
        Some("groq") => setup_secret_env_value("GROQ_API_KEY").is_some(),
        Some("openai") => setup_secret_env_value("OPENAI_API_KEY").is_some(),
        _ => false,
    };

    if provider.is_some() && model.is_some() && has_secret {
        SetupOptionState::Complete
    } else if provider.is_some() || model.is_some() {
        SetupOptionState::Partial
    } else {
        SetupOptionState::Missing
    }
}

pub(crate) fn setup_tts_state(config: Option<&toml::Value>) -> SetupOptionState {
    let enabled = setup_config_bool(config, "tts.enabled").unwrap_or(false);
    let provider = setup_config_string(config, "tts.provider");
    if enabled && provider.as_deref() == Some(captain_runtime::native_voice::NATIVE_TTS_PROVIDER) {
        return if captain_runtime::native_voice::status().tts_ready {
            SetupOptionState::Complete
        } else {
            SetupOptionState::Partial
        };
    }
    let has_provider_config = provider
        .as_deref()
        .and_then(|p| setup_tts_provider_secret_env(p, config))
        .is_some();
    let has_secret = provider
        .as_deref()
        .and_then(|p| setup_tts_provider_secret_env(p, config))
        .and_then(|env_name| setup_secret_env_value(&env_name))
        .is_some();
    let has_specific_config = setup_config_value(config, "tts.openai.api_key_env").is_some()
        || setup_config_value(config, "tts.openai.voice").is_some()
        || setup_config_value(config, "tts.openai.model").is_some()
        || setup_config_value(config, "tts.elevenlabs.api_key_env").is_some()
        || setup_config_value(config, "tts.elevenlabs.voice_id").is_some()
        || setup_config_value(config, "tts.elevenlabs.model_id").is_some();

    if enabled && has_provider_config && has_secret {
        SetupOptionState::Complete
    } else if provider.is_some() || enabled || has_specific_config {
        SetupOptionState::Partial
    } else {
        SetupOptionState::Missing
    }
}

pub(crate) fn setup_offer_telegram(captain_dir: &Path) -> bool {
    ui::section("Canal de messagerie (optionnel)");
    println!("  Telegram permet de discuter avec Captain depuis ton téléphone.");
    let config = setup_read_config_value(captain_dir);
    let state = setup_telegram_state(config.as_ref());
    let should_configure = setup_should_configure_optional(
        state,
        "Telegram déjà configuré",
        "Telegram est partiellement configuré",
        "  Configurer Telegram maintenant ? [Y/n] ",
    )
    .unwrap_or(false);
    if should_configure {
        setup_optional_native_integration("telegram", false, "Telegram")
    } else {
        if state != SetupOptionState::Complete {
            ui::hint(
                "Tu pourras le faire plus tard : `captain integration setup telegram --no-test`",
            );
        }
        state == SetupOptionState::Complete
    }
}

pub(crate) fn setup_offer_voice_stack(
    captain_dir: &Path,
    preferred_voice: &str,
) -> SetupVoiceOutcome {
    let _ = preferred_voice;
    ui::section("Voix native");
    println!("  Captain active STT/TTS localement par défaut, sans clé API.");
    if let Err(e) = apply_native_voice_config() {
        ui::warn_with_fix(
            &format!("Configuration voix native non appliquée : {e}"),
            "Run `captain voice install` after setup.",
        );
    }

    let config = setup_read_config_value(captain_dir);
    let stt_done = setup_stt_state(config.as_ref()) == SetupOptionState::Complete;
    let tts_done = setup_tts_state(config.as_ref()) == SetupOptionState::Complete;
    if stt_done && tts_done {
        let engine = captain_runtime::native_voice::status()
            .tts_engine
            .unwrap_or("native");
        ui::check_ok(&format!("Voix native prête : whisper-small + {engine}"));
    } else {
        ui::check_warn("Voix native configurée, installation des modèles en attente");
        ui::hint(
            "L'installateur lance `captain voice install` automatiquement. Sinon: captain voice install",
        );
    }

    SetupVoiceOutcome {
        stt_done,
        tts_done,
        tts_provider: Some(captain_runtime::native_voice::NATIVE_TTS_PROVIDER),
    }
}

fn setup_tts_provider_secret_env(provider: &str, config: Option<&toml::Value>) -> Option<String> {
    match provider {
        "elevenlabs" => Some(
            setup_config_string(config, "tts.elevenlabs.api_key_env")
                .unwrap_or_else(|| "ELEVENLABS_API_KEY".to_string()),
        ),
        "openai" => Some(
            setup_config_string(config, "tts.openai.api_key_env")
                .unwrap_or_else(|| "OPENAI_API_KEY".to_string()),
        ),
        _ => None,
    }
}

fn setup_should_configure_optional(
    state: SetupOptionState,
    complete_label: &str,
    partial_label: &str,
    missing_prompt: &str,
) -> Option<bool> {
    match state {
        SetupOptionState::Complete => {
            ui::check_ok(complete_label);
            let answer = prompt_input("  Modifier cette option ? [y/N] ");
            Some(answer.starts_with(['y', 'Y']))
        }
        SetupOptionState::Partial => {
            ui::check_warn(partial_label);
            let answer = prompt_input("  Compléter cette option maintenant ? [Y/n] ");
            Some(answer.is_empty() || answer.starts_with(['y', 'Y']))
        }
        SetupOptionState::Missing => {
            let answer = prompt_input(missing_prompt);
            if missing_prompt.contains("[Y/n]") {
                Some(answer.is_empty() || answer.starts_with(['y', 'Y']))
            } else {
                Some(answer.starts_with(['y', 'Y']))
            }
        }
    }
}

fn setup_optional_native_integration(name: &str, run_test: bool, label: &str) -> bool {
    match run_integration_setup_native(name, run_test) {
        Ok(out) => {
            print_integration_setup_outcome(&out);
            true
        }
        Err(e) => {
            ui::warn_with_fix(
                &format!("{label} ignoré : {e}"),
                &format!(
                    "Tu pourras compléter plus tard : captain integration setup {name} --no-test"
                ),
            );
            false
        }
    }
}
