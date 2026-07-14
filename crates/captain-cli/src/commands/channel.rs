use colored::Colorize;

use crate::{
    captain_home, daemon_client, daemon_json, dotenv, find_daemon, prompt_input, prompt_secret,
    restrict_file_permissions, ui,
};

const ACTIVE_CHANNELS: &[(&str, &str, &str)] = &[
    ("telegram", "TELEGRAM_BOT_TOKEN", "Telegram bot (BotFather)"),
    ("discord", "DISCORD_BOT_TOKEN", "Discord bot"),
    ("signal", "", "Signal (signal-cli)"),
    ("email", "EMAIL_PASSWORD", "Email (IMAP/SMTP)"),
];

const FROZEN_CHANNELS: &[&str] = &[
    "bluebubbles",
    "dingtalk",
    "feishu",
    "homeassistant",
    "matrix",
    "mattermost",
    "qq",
    "qqbot",
    "slack",
    "sms",
    "wecom",
    "weixin",
    "whatsapp",
];
const SIGNAL_PHONE_PROMPT: &str = "  Your phone number (+12345678900, or Enter to skip): ";

pub(crate) fn cmd_channel_list() {
    let home = captain_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        println!("No configuration found. Run `captain init` first.");
        return;
    }

    let config_str = std::fs::read_to_string(&config_path).unwrap_or_default();

    println!("Channel Integrations:\n");
    println!("{:<12} {:<10} STATUS", "CHANNEL", "ENV VAR");
    println!("{}", "-".repeat(50));

    for &(name, env_var, _) in ACTIVE_CHANNELS {
        let configured = config_str.contains(&format!("[channels.{name}]"));
        let env_set = env_var.is_empty() || std::env::var(env_var).is_ok();

        let status = match (configured, env_set) {
            (true, true) => "Ready",
            (true, false) => "Missing env",
            (false, _) => "Not configured",
        };

        println!(
            "{:<12} {:<10} {}",
            name,
            if env_var.is_empty() { "—" } else { env_var },
            status,
        );
    }

    println!("\nFrozen for core release: slack, whatsapp, matrix, and long-tail adapters.");
    println!("\nUse `captain channel setup <channel>` to configure a channel.");
}

pub(crate) fn cmd_channel_setup(channel: Option<&str>) {
    let channel = channel
        .map(ToOwned::to_owned)
        .unwrap_or_else(prompt_channel_choice);

    match channel.as_str() {
        "telegram" => setup_telegram(),
        "discord" => setup_discord(),
        "email" => setup_email(),
        "signal" => setup_signal(),
        frozen if FROZEN_CHANNELS.contains(&frozen) => {
            ui::error_with_fix(
                &format!("Channel frozen for core release: {frozen}"),
                "Available active channels: telegram, discord, signal, email",
            );
            std::process::exit(1);
        }
        other => {
            ui::error_with_fix(
                &format!("Unknown channel: {other}"),
                "Available active channels: telegram, discord, signal, email",
            );
            std::process::exit(1);
        }
    }
}

fn prompt_channel_choice() -> String {
    ui::section("Channel Setup");
    ui::blank();
    for (i, (name, _, desc)) in ACTIVE_CHANNELS.iter().enumerate() {
        println!("    {:>2}. {:<12} {}", i + 1, name, desc.dimmed());
    }
    ui::blank();

    let choice = prompt_input("  Choose channel [1]: ");
    let idx = if choice.is_empty() {
        0
    } else {
        choice
            .parse::<usize>()
            .unwrap_or(1)
            .saturating_sub(1)
            .min(ACTIVE_CHANNELS.len() - 1)
    };
    ACTIVE_CHANNELS[idx].0.to_string()
}

fn setup_telegram() {
    ui::section("Setting up Telegram");
    ui::blank();
    println!("  1. Open Telegram and message @BotFather");
    println!("  2. Send /newbot and follow the prompts");
    println!("  3. Copy the bot token");
    ui::blank();

    let token = prompt_secret("  Paste your bot token: ");
    if token.is_empty() {
        ui::error("No token provided. Setup cancelled.");
        return;
    }

    let config_block =
        "\n[channels.telegram]\nbot_token_env = \"TELEGRAM_BOT_TOKEN\"\ndefault_agent = \"captain\"\n";
    maybe_write_channel_config("telegram", config_block);

    match dotenv::save_env_key("TELEGRAM_BOT_TOKEN", &token) {
        Ok(()) => ui::success("Token saved to ~/.captain/.env"),
        Err(_) => println!("    export TELEGRAM_BOT_TOKEN={token}"),
    }

    ui::blank();
    ui::success("Telegram configured");
    notify_daemon_restart();
}

fn setup_discord() {
    ui::section("Setting up Discord");
    ui::blank();
    println!("  1. Go to https://discord.com/developers/applications");
    println!("  2. Create a New Application");
    println!("  3. Go to Bot section and click 'Add Bot'");
    println!("  4. Copy the bot token");
    println!("  5. Under Privileged Gateway Intents, enable:");
    println!("     - Message Content Intent");
    println!("  6. Use OAuth2 URL Generator to invite bot to your server");
    ui::blank();

    let token = prompt_secret("  Paste your bot token: ");
    if token.is_empty() {
        ui::error("No token provided. Setup cancelled.");
        return;
    }

    let config_block =
        "\n[channels.discord]\nbot_token_env = \"DISCORD_BOT_TOKEN\"\ndefault_agent = \"captain\"\n";
    maybe_write_channel_config("discord", config_block);

    match dotenv::save_env_key("DISCORD_BOT_TOKEN", &token) {
        Ok(()) => ui::success("Token saved to ~/.captain/.env"),
        Err(_) => println!("    export DISCORD_BOT_TOKEN={token}"),
    }

    ui::blank();
    ui::success("Discord configured");
    notify_daemon_restart();
}

fn setup_email() {
    ui::section("Setting up Email");
    ui::blank();
    println!("  For Gmail, use an App Password:");
    println!("  https://myaccount.google.com/apppasswords");
    ui::blank();

    let username = prompt_input("  Email address: ");
    if username.is_empty() {
        ui::error("No email provided. Setup cancelled.");
        return;
    }

    let password = prompt_secret("  App password (or Enter to set later): ");
    let config_block = format!(
        "\n[channels.email]\nimap_host = \"imap.gmail.com\"\nimap_port = 993\nsmtp_host = \"smtp.gmail.com\"\nsmtp_port = 587\nusername = \"{username}\"\npassword_env = \"EMAIL_PASSWORD\"\npoll_interval = 30\ndefault_agent = \"captain\"\n"
    );
    maybe_write_channel_config("email", &config_block);

    if !password.is_empty() {
        match dotenv::save_env_key("EMAIL_PASSWORD", &password) {
            Ok(()) => ui::success("Password saved to ~/.captain/.env"),
            Err(_) => println!("    export EMAIL_PASSWORD=your_app_password"),
        }
    } else {
        ui::hint("Set later: captain config set-key email (or export EMAIL_PASSWORD=...)");
    }

    ui::blank();
    ui::success("Email configured");
    notify_daemon_restart();
}

fn setup_signal() {
    ui::section("Setting up Signal");
    ui::blank();
    println!("  Signal requires signal-cli (https://github.com/AsamK/signal-cli).");
    ui::blank();
    println!("  1. Install signal-cli:");
    println!("     - macOS: brew install signal-cli");
    println!("     - Linux: download from GitHub releases");
    println!("     - Or use the Docker image");
    println!("  2. Register or link a phone number:");
    println!("     signal-cli -u +1YOURPHONE register");
    println!("     signal-cli -u +1YOURPHONE verify CODE");
    println!("  3. Start signal-cli in JSON-RPC mode:");
    println!("     signal-cli -u +1YOURPHONE jsonRpc --socket /tmp/signal-cli.sock");
    ui::blank();

    let phone = prompt_input(SIGNAL_PHONE_PROMPT);

    let config_block =
        "\n[channels.signal]\nphone_env = \"SIGNAL_PHONE\"\nsocket_path = \"/tmp/signal-cli.sock\"\ndefault_agent = \"captain\"\n";
    maybe_write_channel_config("signal", config_block);

    if !phone.is_empty() {
        match dotenv::save_env_key("SIGNAL_PHONE", &phone) {
            Ok(()) => ui::success("Phone saved to ~/.captain/.env"),
            Err(_) => println!("    export SIGNAL_PHONE={phone}"),
        }
    }

    ui::blank();
    ui::success("Signal configured");
    notify_daemon_restart();
}

fn maybe_write_channel_config(channel: &str, config_block: &str) {
    let home = captain_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::hint("No config.toml found. Run `captain init` first.");
        return;
    }

    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
    let section_header = format!("[channels.{channel}]");
    if existing.contains(&section_header) {
        ui::check_ok(&format!("{section_header} already in config.toml"));
        return;
    }

    let answer = prompt_input("  Write to config.toml? [Y/n] ");
    if answer.is_empty() || answer.starts_with('y') || answer.starts_with('Y') {
        let mut content = existing;
        content.push_str(config_block);
        if std::fs::write(&config_path, &content).is_ok() {
            restrict_file_permissions(&config_path);
            ui::check_ok(&format!("Added {section_header} to config.toml"));
        } else {
            ui::check_fail("Failed to write config.toml");
        }
    }
}

fn notify_daemon_restart() {
    if find_daemon().is_some() {
        ui::check_warn("Restart the daemon to activate this channel");
    } else {
        ui::hint("Start the daemon: captain start");
    }
}

pub(crate) fn cmd_channel_test(channel: &str) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(
            client
                .post(format!("{base}/api/channels/{channel}/test"))
                .send(),
        );
        if body.get("status").is_some() {
            println!("Test message sent to {channel}!");
        } else {
            eprintln!(
                "Failed: {}",
                body["error"].as_str().unwrap_or("Unknown error")
            );
        }
    } else {
        eprintln!("Channel test requires a running daemon. Start with: captain start");
        std::process::exit(1);
    }
}

pub(crate) fn cmd_channel_toggle(channel: &str, enable: bool) {
    let action = if enable { "enabled" } else { "disabled" };
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let endpoint = if enable { "enable" } else { "disable" };
        let body = daemon_json(
            client
                .post(format!("{base}/api/channels/{channel}/{endpoint}"))
                .send(),
        );
        if body.get("status").is_some() {
            println!("Channel {channel} {action}.");
        } else {
            eprintln!(
                "Failed: {}",
                body["error"].as_str().unwrap_or("Unknown error")
            );
        }
    } else {
        println!("Note: Channel {channel} will be {action} when the daemon starts.");
        println!("Edit ~/.captain/config.toml to persist this change.");
    }
}

#[cfg(test)]
mod tests {
    use super::{ACTIVE_CHANNELS, FROZEN_CHANNELS, SIGNAL_PHONE_PROMPT};

    #[test]
    fn active_channel_choices_stay_core_only() {
        let names = ACTIVE_CHANNELS
            .iter()
            .map(|(name, _, _)| *name)
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["telegram", "discord", "signal", "email"]);
        assert!(FROZEN_CHANNELS.contains(&"slack"));
        assert!(FROZEN_CHANNELS.contains(&"whatsapp"));
        assert!(FROZEN_CHANNELS.contains(&"matrix"));
        assert!(FROZEN_CHANNELS.contains(&"mattermost"));
    }

    #[test]
    fn signal_phone_prompt_uses_release_safe_example() {
        assert!(SIGNAL_PHONE_PROMPT.contains("+12345678900"));
        assert!(!SIGNAL_PHONE_PROMPT.contains(&"X".repeat(3)));
    }
}
