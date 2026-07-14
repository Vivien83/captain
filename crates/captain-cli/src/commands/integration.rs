use super::setup_support::{
    setup_config_string, setup_config_string_array, setup_read_config_value, setup_secret_env_value,
};
use crate::{captain_home, daemon_client, dotenv, find_daemon, prompt_input, prompt_secret, ui};

// R.3.2 — `captain integration setup <name>` interactive flow.
pub(crate) fn cmd_integration_setup_list() {
    println!("Native integrations supported by `captain integration setup`:");
    for n in captain_runtime::integrations::list_integrations() {
        if let Some(integ) = captain_runtime::integrations::get_integration(n) {
            println!("  {n}  — {}", integ.description());
        }
    }
}

pub(crate) fn apply_native_integration(
    name: &str,
    creds: &serde_json::Value,
    run_test: bool,
) -> Result<captain_runtime::integrations::ApplyOutcome, String> {
    let mut secret_set =
        |key: &str, value: &str| -> Result<(), String> { dotenv::save_secret_key(key, value) };

    let config_path = captain_home().join("config.toml");

    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| format!("Failed to create tokio runtime: {e}"))?;

    runtime.block_on(captain_runtime::integrations::setup_integration(
        name,
        creds,
        &config_path,
        &mut secret_set,
        run_test,
        None,
    ))
}

pub(crate) fn run_integration_setup_native(
    name: &str,
    run_test: bool,
) -> Result<captain_runtime::integrations::ApplyOutcome, String> {
    let integration = match captain_runtime::integrations::get_integration(name) {
        Some(i) => i,
        None => return Err(format!("unknown native integration: '{name}'")),
    };

    let creds = match name {
        "telegram" => prompt_telegram_credentials(),
        "tts_elevenlabs" => prompt_tts_elevenlabs_credentials(),
        "tts_openai" => prompt_tts_openai_credentials(),
        "stt_whisper" => prompt_stt_whisper_credentials(),
        _ => {
            return Err(format!(
                "No interactive prompt yet for '{name}' — use the agent tool config_setup."
            ));
        }
    };

    if let Err(e) = integration.validate(&creds) {
        return Err(format!("Invalid credentials: {e}"));
    }

    apply_native_integration(name, &creds, run_test)
}

pub(crate) fn print_integration_setup_outcome(out: &captain_runtime::integrations::ApplyOutcome) {
    ui::success(&format!("Integration '{}' configured.", out.integration));
    if let Some(ref bp) = out.backup_path {
        println!("  backup    : {}", bp.display());
    }
    println!("  secrets   : {}", out.vault_keys.join(", "));
    if !out.env_exports.is_empty() {
        println!("  env       : {}", out.env_exports.join(", "));
    }
    println!("  patched   : {}", out.patched_paths.join(", "));
    if let Some(msg) = &out.test_message {
        println!("  live test : {msg}");
    }
    if let Some(base_url) = find_daemon() {
        let client = daemon_client();
        let _ = client.post(format!("{base_url}/api/config/reload")).send();
        println!("  daemon    : reload requested");
    } else {
        println!("  daemon    : not running (config will apply on next start)");
    }
}

pub(crate) fn cmd_integration_setup_native(name: &str, run_test: bool) {
    match run_integration_setup_native(name, run_test) {
        Ok(out) => {
            print_integration_setup_outcome(&out);
        }
        Err(e) => {
            ui::error(&format!("Setup failed: {e}"));
            std::process::exit(1);
        }
    }
}

pub(crate) fn telegram_get_me(bot_token: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .no_proxy()
        .build()
        .map_err(|e| format!("HTTP client: {e}"))?;
    let url = format!("https://api.telegram.org/bot{bot_token}/getMe");
    let body: serde_json::Value = client
        .get(url)
        .send()
        .map_err(|e| format!("getMe HTTP error: {e}"))?
        .json()
        .map_err(|e| format!("getMe JSON parse: {e}"))?;
    if !body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        return Err("Telegram rejected the bot token".to_string());
    }
    Ok(body
        .pointer("/result/username")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>")
        .to_string())
}

fn telegram_discover_user(bot_token: &str) -> Option<(String, String)> {
    println!();
    ui::section("Telegram user discovery");
    println!("  1. Open your bot in Telegram.");
    println!("  2. Send this exact message: /start");
    println!("  3. Captain will poll Telegram for up to 45 seconds.");
    let answer = prompt_input("  Discover chat_id/user_id now? [Y/n] ");
    if !(answer.is_empty() || answer.starts_with(['y', 'Y'])) {
        return None;
    }

    match telegram_poll_chat_id(bot_token, 9) {
        TelegramDiscovery::Found { chat_id, user_id } => Some((chat_id, user_id)),
        TelegramDiscovery::PollingConflict => {
            ui::check_warn(
                "Another process is already polling this bot (getUpdates conflict) — \
                 likely a running Captain daemon. Stop it or provide chat_id manually.",
            );
            None
        }
        TelegramDiscovery::NotFound => {
            ui::check_warn("No Telegram /start update discovered.");
            None
        }
    }
}

pub(crate) enum TelegramDiscovery {
    Found { chat_id: String, user_id: String },
    NotFound,
    PollingConflict,
}

/// Polls getUpdates for a message to derive (chat_id, user_id) from. Also
/// reads the pending backlog (Telegram keeps unacknowledged updates ~24h),
/// so a user who already messaged the bot is discovered without any waiting
/// — which is what makes non-interactive setup able to use this too.
pub(crate) fn telegram_poll_chat_id(bot_token: &str, max_attempts: u32) -> TelegramDiscovery {
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .no_proxy()
        .build()
    else {
        return TelegramDiscovery::NotFound;
    };
    let mut offset: Option<i64> = None;
    for _ in 0..max_attempts {
        let mut url = format!("https://api.telegram.org/bot{bot_token}/getUpdates?timeout=5");
        if let Some(o) = offset {
            url.push_str(&format!("&offset={o}"));
        }
        let Ok(resp) = client.get(&url).send() else {
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        };
        if resp.status().as_u16() == 409 {
            return TelegramDiscovery::PollingConflict;
        }
        let Ok(body) = resp.json::<serde_json::Value>() else {
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        };
        let Some(updates) = body.get("result").and_then(|v| v.as_array()) else {
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        };
        for update in updates {
            if let Some(update_id) = update.get("update_id").and_then(|v| v.as_i64()) {
                offset = Some(update_id + 1);
            }
            let msg = update
                .get("message")
                .or_else(|| update.get("edited_message"));
            let Some(msg) = msg else {
                continue;
            };
            let chat_id = msg
                .pointer("/chat/id")
                .and_then(|v| v.as_i64())
                .map(|v| v.to_string());
            let user_id = msg
                .pointer("/from/id")
                .and_then(|v| v.as_i64())
                .map(|v| v.to_string());
            if let (Some(chat_id), Some(user_id)) = (chat_id, user_id) {
                ui::success(&format!(
                    "Discovered Telegram chat_id={chat_id}, user_id={user_id}"
                ));
                return TelegramDiscovery::Found { chat_id, user_id };
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    TelegramDiscovery::NotFound
}

pub(crate) fn telegram_send_validation(bot_token: &str, chat_id: &str) {
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .no_proxy()
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            ui::check_warn(&format!("Telegram validation client failed: {e}"));
            return;
        }
    };
    let url = format!("https://api.telegram.org/bot{bot_token}/sendMessage");
    let resp = client
        .post(url)
        .json(&serde_json::json!({
            "chat_id": chat_id,
            "text": "Captain Telegram est configuré."
        }))
        .send();
    match resp {
        Ok(r) if r.status().is_success() => {
            ui::success("Telegram validation message sent.");
        }
        Ok(r) => ui::check_warn(&format!("Telegram sendMessage returned {}", r.status())),
        Err(e) => ui::check_warn(&format!("Telegram sendMessage failed: {e}")),
    }
}

fn prompt_telegram_credentials() -> serde_json::Value {
    println!("\nTelegram bot setup\n");
    println!("  1. Open Telegram and search for @BotFather.");
    println!("  2. Send /newbot.");
    println!("  3. Choose a display name, for example: Captain.");
    println!("  4. Choose a username ending with bot, for example: my_captain_bot.");
    println!("  5. Copy the token shown by BotFather and paste it below.\n");
    let home = captain_home();
    let config = setup_read_config_value(&home);
    let bot_token_env = setup_config_string(config.as_ref(), "channels.telegram.bot_token_env")
        .unwrap_or_else(|| "TELEGRAM_BOT_TOKEN".to_string());
    let existing_bot_token = setup_secret_env_value(&bot_token_env);
    let token_prompt = if existing_bot_token.is_some() {
        "bot_token [déjà configuré, Entrée pour conserver]: "
    } else {
        "bot_token (<id>:<secret>): "
    };
    let bot_token_in = prompt_secret(token_prompt);
    let bot_token = if bot_token_in.is_empty() {
        existing_bot_token.unwrap_or_default()
    } else {
        bot_token_in
    };
    if bot_token.is_empty() {
        ui::check_warn("Aucun bot_token fourni.");
    } else {
        match telegram_get_me(&bot_token) {
            Ok(username) => {
                ui::success(&format!("Telegram bot reachable: @{username}"));
                println!("  Bot link: https://t.me/{username}?start=captain_setup");
            }
            Err(e) => {
                ui::check_warn(&format!("{e}. You can still enter chat IDs manually."));
            }
        }
    };

    let discovered = if bot_token.is_empty() {
        None
    } else {
        telegram_discover_user(&bot_token)
    };
    let existing_default_chat =
        setup_config_string(config.as_ref(), "channels.telegram.default_chat_id");
    let default_default_chat = discovered
        .as_ref()
        .map(|(chat_id, _)| chat_id.as_str())
        .or(existing_default_chat.as_deref())
        .unwrap_or("");
    let prompt = if default_default_chat.is_empty() {
        "default_chat_id (numeric, your user ID or a group ID): ".to_string()
    } else {
        format!("default_chat_id [{default_default_chat}]: ")
    };
    let chat_in = prompt_input(&prompt);
    let default_chat_id = if chat_in.is_empty() {
        default_default_chat.to_string()
    } else {
        chat_in
    };

    let discovered_user = discovered
        .as_ref()
        .map(|(_, user_id)| user_id.as_str())
        .unwrap_or("");
    let existing_allowed_users =
        setup_config_string_array(config.as_ref(), "channels.telegram.allowed_users");
    let existing_allowed_joined = existing_allowed_users.join(",");
    let default_allowed_users = if discovered_user.is_empty() {
        existing_allowed_joined.as_str()
    } else {
        discovered_user
    };
    let allowed_prompt = if discovered_user.is_empty() {
        if default_allowed_users.is_empty() {
            "allowed_users (comma-separated numeric IDs, empty for none): ".to_string()
        } else {
            format!("allowed_users [{default_allowed_users}]: ")
        }
    } else {
        format!("allowed_users [{discovered_user}]: ")
    };
    let allowed_in = prompt_input(&allowed_prompt);
    let allowed: Vec<&str> = allowed_in
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    let allowed = if allowed.is_empty() && !default_allowed_users.is_empty() {
        default_allowed_users
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        allowed
    };

    if !bot_token.is_empty() && !default_chat_id.is_empty() {
        telegram_send_validation(&bot_token, &default_chat_id);
    }

    serde_json::json!({
        "bot_token": bot_token,
        "default_chat_id": default_chat_id,
        "allowed_users": allowed,
    })
}

fn prompt_tts_elevenlabs_credentials() -> serde_json::Value {
    println!("\nElevenLabs TTS setup");
    println!("(Get an API key from https://elevenlabs.io/app/settings/api-keys)\n");
    let home = captain_home();
    let config = setup_read_config_value(&home);
    let api_env = setup_config_string(config.as_ref(), "tts.elevenlabs.api_key_env")
        .unwrap_or_else(|| "ELEVENLABS_API_KEY".to_string());
    let existing_key = setup_secret_env_value(&api_env);
    let api_key_in = prompt_secret(if existing_key.is_some() {
        "api_key [déjà configurée, Entrée pour conserver]: "
    } else {
        "api_key: "
    });
    let api_key = if api_key_in.is_empty() {
        existing_key.unwrap_or_default()
    } else {
        api_key_in
    };
    let existing_voice = setup_config_string(config.as_ref(), "tts.elevenlabs.voice_id");
    let voice_prompt = existing_voice
        .as_deref()
        .map(|voice| format!("voice_id [{voice}]: "))
        .unwrap_or_else(|| "voice_id (empty for default): ".to_string());
    let voice_id_in = prompt_input(&voice_prompt);
    let existing_model = setup_config_string(config.as_ref(), "tts.elevenlabs.model_id")
        .unwrap_or_else(|| "eleven_turbo_v2_5".to_string());
    let model_id_in = prompt_input(&format!("model_id [{existing_model}]: "));
    let mut creds = serde_json::json!({"api_key": api_key});
    if !voice_id_in.is_empty() {
        creds["voice_id"] = serde_json::Value::String(voice_id_in);
    } else if let Some(voice) = existing_voice {
        creds["voice_id"] = serde_json::Value::String(voice);
    }
    creds["model_id"] = serde_json::Value::String(if model_id_in.is_empty() {
        existing_model
    } else {
        model_id_in
    });
    creds
}

fn prompt_tts_openai_credentials() -> serde_json::Value {
    println!("\nOpenAI TTS setup");
    println!("(Get an API key from https://platform.openai.com/api-keys)");
    println!("(This key also unlocks chat / embeddings / Whisper STT for the OpenAI provider)\n");
    let home = captain_home();
    let config = setup_read_config_value(&home);
    let api_env = setup_config_string(config.as_ref(), "tts.openai.api_key_env")
        .unwrap_or_else(|| "OPENAI_API_KEY".to_string());
    let existing_key = setup_secret_env_value(&api_env);
    let api_key_in = prompt_secret(if existing_key.is_some() {
        "api_key [déjà configurée, Entrée pour conserver]: "
    } else {
        "api_key (sk-…): "
    });
    let api_key = if api_key_in.is_empty() {
        existing_key.unwrap_or_default()
    } else {
        api_key_in
    };
    let existing_voice =
        setup_config_string(config.as_ref(), "tts.openai.voice").unwrap_or_else(|| "nova".into());
    let existing_model =
        setup_config_string(config.as_ref(), "tts.openai.model").unwrap_or_else(|| "tts-1".into());
    let existing_format =
        setup_config_string(config.as_ref(), "tts.openai.format").unwrap_or_else(|| "mp3".into());
    let voice_in = prompt_input(&format!(
        "voice (alloy|echo|fable|onyx|nova|shimmer) [{existing_voice}]: "
    ));
    let model_in = prompt_input(&format!("model (tts-1 | tts-1-hd) [{existing_model}]: "));
    let format_in = prompt_input(&format!("format (mp3|wav|opus|flac) [{existing_format}]: "));
    let mut creds = serde_json::json!({"api_key": api_key});
    creds["voice"] = serde_json::Value::String(if voice_in.is_empty() {
        existing_voice
    } else {
        voice_in
    });
    creds["model"] = serde_json::Value::String(if model_in.is_empty() {
        existing_model
    } else {
        model_in
    });
    creds["format"] = serde_json::Value::String(if format_in.is_empty() {
        existing_format
    } else {
        format_in
    });
    creds
}

fn prompt_stt_whisper_credentials() -> serde_json::Value {
    println!("\nWhisper STT setup");
    println!("(Default provider: Groq — fast & generous free tier. https://console.groq.com/keys)");
    println!("(Use 'openai' provider with sk-… key to use OpenAI Whisper instead)\n");
    let home = captain_home();
    let config = setup_read_config_value(&home);
    let existing_provider = setup_config_string(config.as_ref(), "media.audio_provider")
        .filter(|provider| provider == "groq" || provider == "openai")
        .unwrap_or_else(|| "groq".to_string());
    let provider_in = prompt_input(&format!("provider (groq | openai) [{existing_provider}]: "));
    let provider = if provider_in.is_empty() {
        existing_provider.as_str()
    } else {
        provider_in.as_str()
    };
    let key_hint = if provider == "openai" {
        "sk-…"
    } else {
        "gsk_…"
    };
    let existing_key = match provider {
        "openai" => setup_secret_env_value("OPENAI_API_KEY"),
        "groq" => setup_secret_env_value("GROQ_API_KEY"),
        _ => None,
    };
    let api_key_prompt = if existing_key.is_some() {
        "api_key [déjà configurée, Entrée pour conserver]: ".to_string()
    } else {
        format!("api_key ({key_hint}): ")
    };
    let api_key_in = prompt_secret(&api_key_prompt);
    let api_key = if api_key_in.is_empty() {
        existing_key.unwrap_or_default()
    } else {
        api_key_in
    };
    let mut creds = serde_json::json!({"api_key": api_key});
    creds["provider"] = serde_json::Value::String(provider.to_string());
    creds
}
