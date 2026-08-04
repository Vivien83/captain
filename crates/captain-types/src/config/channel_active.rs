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

/// One named IMAP/SMTP mailbox used by the Email channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EmailAccountConfig {
    /// Stable lowercase alias used in adapter names (`email:<alias>`).
    pub alias: String,
    /// Whether this account should start with the channel bridge.
    pub enabled: bool,
    /// IMAP server host. Connections always use implicit TLS.
    pub imap_host: String,
    /// IMAP TLS port.
    pub imap_port: u16,
    /// SMTP server host. Port 465 uses implicit TLS; others require STARTTLS.
    pub smtp_host: String,
    /// SMTP TLS/STARTTLS port.
    pub smtp_port: u16,
    /// Email address or provider login used for IMAP and SMTP authentication.
    pub username: String,
    /// Canonical Captain secret key holding the password or app password.
    pub password_env: String,
    /// Poll interval in seconds.
    pub poll_interval_secs: u64,
    /// IMAP folders to monitor.
    #[serde(
        default = "default_email_folders",
        deserialize_with = "deserialize_string_or_int_vec"
    )]
    pub folders: Vec<String>,
    /// Only process emails from these senders (empty = deny all,
    /// `["*"]` = allow all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_senders: Vec<String>,
    /// Default agent name to route inbound messages to.
    pub default_agent: Option<String>,
}

impl Default for EmailAccountConfig {
    fn default() -> Self {
        Self {
            alias: String::new(),
            enabled: true,
            imap_host: String::new(),
            imap_port: 993,
            smtp_host: String::new(),
            smtp_port: 587,
            username: String::new(),
            password_env: String::new(),
            poll_interval_secs: 30,
            folders: default_email_folders(),
            allowed_senders: vec![],
            default_agent: None,
        }
    }
}

impl EmailAccountConfig {
    pub fn validation_errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if !is_valid_email_account_alias(&self.alias) {
            errors.push(
                "alias must be 1-32 lowercase ASCII letters, digits, '.', '_' or '-', starting with a letter or digit"
                    .to_string(),
            );
        }
        validate_email_host(&mut errors, "imap_host", &self.imap_host);
        validate_email_host(&mut errors, "smtp_host", &self.smtp_host);
        if self.imap_port == 0 {
            errors.push("imap_port must be non-zero".to_string());
        }
        if self.smtp_port == 0 {
            errors.push("smtp_port must be non-zero".to_string());
        }
        if self.username.is_empty()
            || self.username.len() > 320
            || self.username.contains(char::is_whitespace)
        {
            errors
                .push("username must be a non-empty address/login without whitespace".to_string());
        }
        if !valid_secret_key(&self.password_env) {
            errors
                .push("password_env must be a canonical environment-style secret key".to_string());
        }
        if !(5..=3600).contains(&self.poll_interval_secs) {
            errors.push("poll_interval_secs must be between 5 and 3600".to_string());
        }
        if self.folders.is_empty()
            || self.folders.len() > 32
            || self
                .folders
                .iter()
                .any(|folder| folder.is_empty() || folder.len() > 255 || folder.contains('\0'))
        {
            errors.push("folders must contain 1-32 bounded non-empty IMAP names".to_string());
        }
        errors
    }
}

/// Return whether an Email account alias is safe for config and
/// `email:<alias>` adapter addressing.
pub fn is_valid_email_account_alias(alias: &str) -> bool {
    let bytes = alias.as_bytes();
    (1..=32).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

fn validate_email_host(errors: &mut Vec<String>, field: &str, value: &str) {
    if value.len() > 253 || url::Host::parse(value).is_err() {
        errors.push(format!("{field} must be a valid hostname or IP address"));
    }
}

fn valid_secret_key(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && value.len() <= 128
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn default_email_folders() -> Vec<String> {
    vec!["INBOX".to_string()]
}

/// Email (IMAP/SMTP) channel configuration.
///
/// `accounts` is the current multi-account contract. The scalar fields remain
/// readable for backward compatibility with pre-multi-account installations
/// and are projected as one account named `default` when `accounts` is empty.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EmailConfig {
    /// Named IMAP/SMTP accounts.
    pub accounts: Vec<EmailAccountConfig>,
    /// Account alias used by bare `email` sends. The first enabled account is
    /// used when omitted.
    pub default_account: Option<String>,
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
    #[serde(
        default = "default_email_folders",
        deserialize_with = "deserialize_string_or_int_vec"
    )]
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
            accounts: vec![],
            default_account: None,
            imap_host: String::new(),
            imap_port: 993,
            smtp_host: String::new(),
            smtp_port: 587,
            username: String::new(),
            password_env: "EMAIL_PASSWORD".to_string(),
            poll_interval_secs: 30,
            folders: default_email_folders(),
            allowed_senders: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

impl EmailConfig {
    pub fn effective_accounts(&self) -> Vec<EmailAccountConfig> {
        if !self.accounts.is_empty() {
            return self.accounts.clone();
        }
        if !self.legacy_account_is_configured() {
            return Vec::new();
        }
        vec![EmailAccountConfig {
            alias: "default".to_string(),
            enabled: true,
            imap_host: self.imap_host.clone(),
            imap_port: self.imap_port,
            smtp_host: self.smtp_host.clone(),
            smtp_port: self.smtp_port,
            username: self.username.clone(),
            password_env: self.password_env.clone(),
            poll_interval_secs: self.poll_interval_secs,
            folders: self.folders.clone(),
            allowed_senders: self.allowed_senders.clone(),
            default_agent: self.default_agent.clone(),
        }]
    }

    pub fn effective_default_account(&self) -> Option<String> {
        let accounts = self.effective_accounts();
        self.default_account.clone().or_else(|| {
            accounts
                .iter()
                .find(|account| account.enabled)
                .map(|account| account.alias.clone())
        })
    }

    pub fn validation_errors(&self) -> Vec<String> {
        let accounts = self.effective_accounts();
        let mut errors = Vec::new();
        if accounts.is_empty() {
            errors.push("at least one Email account must be configured".to_string());
        }
        if !self.accounts.is_empty() && self.legacy_account_is_configured() {
            errors.push(
                "multi-account entries cannot be mixed with legacy scalar Email fields".to_string(),
            );
        }
        let mut aliases = std::collections::HashSet::new();
        for account in &accounts {
            for error in account.validation_errors() {
                errors.push(format!("account '{}': {error}", account.alias));
            }
            if !aliases.insert(account.alias.as_str()) {
                errors.push(format!("duplicate account alias '{}'", account.alias));
            }
        }
        if let Some(default_account) = &self.default_account {
            if !accounts
                .iter()
                .any(|account| account.enabled && &account.alias == default_account)
            {
                errors.push(format!(
                    "default_account '{default_account}' does not name an enabled account"
                ));
            }
        }
        errors
    }

    fn legacy_account_is_configured(&self) -> bool {
        !self.imap_host.is_empty()
            || !self.smtp_host.is_empty()
            || !self.username.is_empty()
            || self.password_env != "EMAIL_PASSWORD"
            || self.imap_port != 993
            || self.smtp_port != 587
            || self.poll_interval_secs != 30
            || self.folders != ["INBOX"]
            || !self.allowed_senders.is_empty()
            || self.default_agent.is_some()
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

    #[test]
    fn legacy_email_config_projects_to_one_compatible_account() {
        let config: EmailConfig = toml::from_str(
            r#"
            imap_host = "imap.example.com"
            smtp_host = "smtp.example.com"
            username = "captain@example.com"
            password_env = "EMAIL_PASSWORD"
            allowed_senders = ["@example.com"]
            "#,
        )
        .unwrap();

        let accounts = config.effective_accounts();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].alias, "default");
        assert_eq!(accounts[0].username, "captain@example.com");
        assert_eq!(
            config.effective_default_account().as_deref(),
            Some("default")
        );
        let errors = config.validation_errors();
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn multi_account_email_config_is_named_and_defaulted_explicitly() {
        let config: EmailConfig = toml::from_str(
            r#"
            default_account = "work"

            [[accounts]]
            alias = "personal"
            imap_host = "imap.example.com"
            smtp_host = "smtp.example.com"
            username = "personal@example.com"
            password_env = "EMAIL_PERSONAL_PASSWORD"
            allowed_senders = ["friend@example.com"]

            [[accounts]]
            alias = "work"
            imap_host = "imap.work.example"
            smtp_host = "smtp.work.example"
            username = "captain@work.example"
            password_env = "EMAIL_WORK_PASSWORD"
            allowed_senders = ["@work.example"]
            "#,
        )
        .unwrap();

        assert_eq!(config.effective_accounts().len(), 2);
        assert_eq!(config.effective_default_account().as_deref(), Some("work"));
        let errors = config.validation_errors();
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn multi_account_email_config_rejects_ambiguous_or_unsafe_entries() {
        let config = EmailConfig {
            accounts: vec![
                EmailAccountConfig {
                    alias: "Work".to_string(),
                    imap_host: "https://imap.example.com".to_string(),
                    smtp_host: "smtp.example.com".to_string(),
                    username: "captain@example.com".to_string(),
                    password_env: "bad-key".to_string(),
                    poll_interval_secs: 0,
                    ..Default::default()
                },
                EmailAccountConfig {
                    alias: "Work".to_string(),
                    imap_host: "imap.example.com".to_string(),
                    smtp_host: "smtp.example.com".to_string(),
                    username: "captain@example.com".to_string(),
                    password_env: "EMAIL_WORK_PASSWORD".to_string(),
                    ..Default::default()
                },
            ],
            default_account: Some("missing".to_string()),
            ..Default::default()
        };
        let errors = config.validation_errors().join("\n");

        assert!(errors.contains("alias must"));
        assert!(errors.contains("imap_host"));
        assert!(errors.contains("password_env"));
        assert!(errors.contains("poll_interval_secs"));
        assert!(errors.contains("duplicate account alias"));
        assert!(errors.contains("default_account 'missing'"));
    }

    #[test]
    fn multi_account_email_config_cannot_mix_legacy_fields() {
        let config = EmailConfig {
            accounts: vec![EmailAccountConfig {
                alias: "work".to_string(),
                imap_host: "imap.example.com".to_string(),
                smtp_host: "smtp.example.com".to_string(),
                username: "captain@example.com".to_string(),
                password_env: "EMAIL_WORK_PASSWORD".to_string(),
                ..Default::default()
            }],
            username: "legacy@example.com".to_string(),
            ..Default::default()
        };

        assert!(config
            .validation_errors()
            .iter()
            .any(|error| error.contains("cannot be mixed")));
    }

    #[test]
    fn empty_email_channel_config_is_not_ready() {
        let errors = EmailConfig::default().validation_errors();

        assert_eq!(
            errors,
            vec!["at least one Email account must be configured"]
        );
    }
}
