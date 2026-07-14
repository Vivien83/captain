use super::{default_true, deserialize_string_or_int_vec, ChannelOverrides};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Telegram channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TelegramConfig {
    /// Env var name holding the bot token (NOT the token itself).
    pub bot_token_env: String,
    /// Telegram user IDs allowed to interact (empty = deny all, ["*"] = allow all).
    /// Accepts strings for consistency; numeric TOML integers are coerced to strings.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Polling interval in seconds.
    pub poll_interval_secs: u64,
    /// Custom Telegram Bot API base URL for proxies or mirrors.
    /// Defaults to `https://api.telegram.org` when not set.
    #[serde(default)]
    pub api_url: Option<String>,
    /// Default chat ID for outgoing messages when no recipient is specified.
    /// Allows channel_send(channel="telegram", message="...") without a recipient.
    #[serde(default)]
    pub default_chat_id: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
    /// Mapping of agent/hand names to Telegram forum topic IDs.
    /// When an agent sends a message, it auto-routes to its topic.
    /// Example: { "OpsHand" = "123", "Research" = "456" }
    #[serde(default)]
    pub topics: HashMap<String, String>,
    /// HS.3b: live message rendering. When true, the bridge streams agent
    /// output as text edits and intercalated tool-call bubbles.
    #[serde(default = "default_true")]
    pub streaming: bool,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            bot_token_env: "TELEGRAM_BOT_TOKEN".to_string(),
            allowed_users: vec![],
            default_agent: None,
            poll_interval_secs: 1,
            api_url: None,
            default_chat_id: None,
            overrides: ChannelOverrides::default(),
            topics: HashMap::new(),
            streaming: true,
        }
    }
}

/// Discord channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscordConfig {
    /// Env var name holding the bot token (NOT the token itself).
    pub bot_token_env: String,
    /// Guild (server) IDs allowed to interact (empty = allow all).
    /// Accepts strings for consistency with other channel configs.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_guilds: Vec<String>,
    /// User IDs allowed to interact (empty = deny all, ["*"] = allow all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Gateway intents bitmask (default: 37376 = GUILD_MESSAGES | DIRECT_MESSAGES | MESSAGE_CONTENT).
    pub intents: u64,
    /// Ignore messages from other bots (default: true).
    /// Set to false to allow bot-to-bot interactions in multi-agent setups.
    #[serde(default = "default_true")]
    pub ignore_bots: bool,
    /// Default channel ID for outgoing messages when no recipient is specified.
    #[serde(default)]
    pub default_channel_id: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            bot_token_env: "DISCORD_BOT_TOKEN".to_string(),
            allowed_guilds: vec![],
            allowed_users: vec![],
            default_agent: None,
            intents: 37376,
            ignore_bots: true,
            default_channel_id: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Signal channel adapter configuration (via signal-cli REST API).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SignalConfig {
    /// URL of the signal-cli REST API (e.g., "http://localhost:8080").
    pub api_url: String,
    /// Registered phone number.
    pub phone_number: String,
    /// Allowed phone numbers (empty = deny all, ["*"] = allow all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for SignalConfig {
    fn default() -> Self {
        Self {
            api_url: "http://localhost:8080".to_string(),
            phone_number: String::new(),
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Email (IMAP/SMTP) channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EmailConfig {
    /// IMAP server host.
    pub imap_host: String,
    /// IMAP port (993 for TLS).
    pub imap_port: u16,
    /// SMTP server host.
    pub smtp_host: String,
    /// SMTP port (587 for STARTTLS).
    pub smtp_port: u16,
    /// Email address (used for both IMAP and SMTP).
    pub username: String,
    /// Env var name holding the password.
    pub password_env: String,
    /// Poll interval in seconds.
    pub poll_interval_secs: u64,
    /// IMAP folders to monitor.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub folders: Vec<String>,
    /// Only process emails from these senders (empty = deny all,
    /// `["*"]` = allow all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_senders: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            imap_host: String::new(),
            imap_port: 993,
            smtp_host: String::new(),
            smtp_port: 587,
            username: String::new(),
            password_env: "EMAIL_PASSWORD".to_string(),
            poll_interval_secs: 30,
            folders: vec!["INBOX".to_string()],
            allowed_senders: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_defaults_keep_streaming_and_deny_by_default() {
        let config = TelegramConfig::default();

        assert_eq!(config.bot_token_env, "TELEGRAM_BOT_TOKEN");
        assert!(config.allowed_users.is_empty());
        assert_eq!(config.poll_interval_secs, 1);
        assert!(config.default_chat_id.is_none());
        assert!(config.streaming);
    }

    #[test]
    fn discord_defaults_keep_gateway_and_bot_guard() {
        let config = DiscordConfig::default();

        assert_eq!(config.bot_token_env, "DISCORD_BOT_TOKEN");
        assert!(config.allowed_guilds.is_empty());
        assert!(config.allowed_users.is_empty());
        assert_eq!(config.intents, 37376);
        assert!(config.ignore_bots);
    }

    #[test]
    fn discord_ignore_bots_defaults_true_when_missing() {
        let explicit: DiscordConfig =
            toml::from_str("bot_token_env = \"DISCORD_BOT_TOKEN\"\nignore_bots = false").unwrap();
        let missing: DiscordConfig =
            toml::from_str("bot_token_env = \"DISCORD_BOT_TOKEN\"").unwrap();

        assert!(!explicit.ignore_bots);
        assert!(missing.ignore_bots);
    }

    #[test]
    fn signal_defaults_keep_local_signal_cli_endpoint() {
        let config = SignalConfig::default();

        assert_eq!(config.api_url, "http://localhost:8080");
        assert!(config.phone_number.is_empty());
        assert!(config.allowed_users.is_empty());
    }

    #[test]
    fn email_defaults_keep_sender_allowlist_empty() {
        let config = EmailConfig::default();

        assert_eq!(config.imap_port, 993);
        assert_eq!(config.smtp_port, 587);
        assert_eq!(config.password_env, "EMAIL_PASSWORD");
        assert_eq!(config.folders, vec!["INBOX".to_string()]);
        assert!(config.allowed_senders.is_empty());
    }

    #[test]
    fn active_channel_id_lists_accept_numeric_toml_values() {
        let telegram: TelegramConfig = toml::from_str("allowed_users = [123, \"456\"]").unwrap();
        let discord: DiscordConfig =
            toml::from_str("allowed_guilds = [42]\nallowed_users = [7]").unwrap();
        let email: EmailConfig =
            toml::from_str("folders = [2026]\nallowed_senders = [99]").unwrap();

        assert_eq!(telegram.allowed_users, vec!["123", "456"]);
        assert_eq!(discord.allowed_guilds, vec!["42"]);
        assert_eq!(discord.allowed_users, vec!["7"]);
        assert_eq!(email.folders, vec!["2026"]);
        assert_eq!(email.allowed_senders, vec!["99"]);
    }
}
