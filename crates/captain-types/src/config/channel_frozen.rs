//! Compatibility config structs for frozen, non-core channels.

use super::{default_thread_ttl, default_true, deserialize_string_or_int_vec, ChannelOverrides};
use serde::{Deserialize, Serialize};

/// Slack channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SlackConfig {
    /// Env var name holding the app-level token (xapp-) for Socket Mode.
    pub app_token_env: String,
    /// Env var name holding the bot token (xoxb-) for REST API.
    pub bot_token_env: String,
    /// Slack user IDs (e.g. `U01ABCD23`) allowed to address the bot.
    /// Empty = deny all (B.8 contract). Use `["*"]` to allow everyone explicitly.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Channel IDs allowed to interact (empty = allow all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_channels: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
    /// Automatically reply to follow-up messages in threads where bot was mentioned.
    #[serde(default = "default_true")]
    pub auto_thread_reply: bool,
    /// Hours to track a thread after last interaction (default: 24).
    #[serde(default = "default_thread_ttl")]
    pub thread_ttl_hours: u64,
    /// Whether to unfurl (expand previews for) links in messages (default: true).
    #[serde(default = "default_true")]
    pub unfurl_links: bool,
}

impl Default for SlackConfig {
    fn default() -> Self {
        Self {
            app_token_env: "SLACK_APP_TOKEN".to_string(),
            bot_token_env: "SLACK_BOT_TOKEN".to_string(),
            allowed_users: vec![],
            allowed_channels: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
            auto_thread_reply: true,
            thread_ttl_hours: 24,
            unfurl_links: true,
        }
    }
}

/// WhatsApp Cloud API channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WhatsAppConfig {
    /// Env var name holding the access token (Cloud API mode).
    pub access_token_env: String,
    /// Env var name holding the webhook verify token (Cloud API mode).
    pub verify_token_env: String,
    /// WhatsApp Business phone number ID (Cloud API mode).
    pub phone_number_id: String,
    /// Port to listen for webhook callbacks (Cloud API mode).
    pub webhook_port: u16,
    /// Env var name holding the WhatsApp Web gateway URL (QR/Web mode).
    /// When set, outgoing messages are routed through the gateway instead of Cloud API.
    pub gateway_url_env: String,
    /// Allowed phone numbers (empty = deny all, ["*"] = allow all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for WhatsAppConfig {
    fn default() -> Self {
        Self {
            access_token_env: "WHATSAPP_ACCESS_TOKEN".to_string(),
            verify_token_env: "WHATSAPP_VERIFY_TOKEN".to_string(),
            phone_number_id: String::new(),
            webhook_port: 8443,
            gateway_url_env: "WHATSAPP_WEB_GATEWAY_URL".to_string(),
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Matrix protocol channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MatrixConfig {
    /// Matrix homeserver URL (e.g., `"https://matrix.org"`).
    pub homeserver_url: String,
    /// Bot user ID (e.g., "@captain:matrix.org").
    pub user_id: String,
    /// Env var name holding the access token.
    pub access_token_env: String,
    /// Matrix user IDs (e.g. `@alice:example.org`) allowed to address the bot.
    /// Empty = deny all (B.8 contract). Use `["*"]` to allow everyone.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Room IDs to listen in (empty = all joined rooms).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_rooms: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for MatrixConfig {
    fn default() -> Self {
        Self {
            homeserver_url: "https://matrix.org".to_string(),
            user_id: String::new(),
            access_token_env: "MATRIX_ACCESS_TOKEN".to_string(),
            allowed_users: vec![],
            allowed_rooms: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Microsoft Teams (Bot Framework v3) channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TeamsConfig {
    /// Azure Bot App ID.
    pub app_id: String,
    /// Env var name holding the app password.
    pub app_password_env: String,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// Teams sender IDs (`from.id`) allowed to address the bot.
    /// Empty = deny all (B.8 contract). Use `["*"]` to allow everyone explicitly.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Allowed tenant IDs (empty = allow all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_tenants: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for TeamsConfig {
    fn default() -> Self {
        Self {
            app_id: String::new(),
            app_password_env: "TEAMS_APP_PASSWORD".to_string(),
            webhook_port: 3978,
            allowed_users: vec![],
            allowed_tenants: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Mattermost channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MattermostConfig {
    /// Mattermost server URL (e.g., `"https://mattermost.example.com"`).
    pub server_url: String,
    /// Env var name holding the bot token.
    pub token_env: String,
    /// Mattermost user IDs allowed to address the bot.
    /// Empty = deny all (B.8 contract). Use `["*"]` to allow everyone explicitly.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Allowed channel IDs (empty = all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_channels: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for MattermostConfig {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            token_env: "MATTERMOST_TOKEN".to_string(),
            allowed_users: vec![],
            allowed_channels: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// IRC channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IrcConfig {
    /// IRC server hostname.
    pub server: String,
    /// IRC server port.
    pub port: u16,
    /// Bot nickname.
    pub nick: String,
    /// Env var name holding the server password (optional).
    pub password_env: Option<String>,
    /// IRC nicks allowed to address the bot.
    /// Empty = deny all (B.8 contract). Use `["*"]` to allow everyone explicitly.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Channels to join (e.g., `["#captain", "#general"]`).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub channels: Vec<String>,
    /// Use TLS (requires tokio-native-tls).
    pub use_tls: bool,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for IrcConfig {
    fn default() -> Self {
        Self {
            server: "irc.libera.chat".to_string(),
            port: 6667,
            nick: "captain".to_string(),
            password_env: None,
            allowed_users: vec![],
            channels: vec![],
            use_tls: false,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Google Chat channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GoogleChatConfig {
    /// Env var name holding the service account JSON key.
    pub service_account_env: String,
    /// Space IDs to listen in.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub space_ids: Vec<String>,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// Sender resource names allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against `message.sender.name`.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for GoogleChatConfig {
    fn default() -> Self {
        Self {
            service_account_env: "GOOGLE_CHAT_SERVICE_ACCOUNT".to_string(),
            space_ids: vec![],
            webhook_port: 8444,
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Twitch chat channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TwitchConfig {
    /// Env var name holding the OAuth token.
    pub oauth_token_env: String,
    /// Twitch channels to join (without #).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub channels: Vec<String>,
    /// Bot nickname.
    pub nick: String,
    /// Twitch nicks allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all, otherwise exact nick match).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for TwitchConfig {
    fn default() -> Self {
        Self {
            oauth_token_env: "TWITCH_OAUTH_TOKEN".to_string(),
            channels: vec![],
            nick: "captain".to_string(),
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Rocket.Chat channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RocketChatConfig {
    /// Rocket.Chat server URL.
    pub server_url: String,
    /// Env var name holding the auth token.
    pub token_env: String,
    /// User ID for the bot.
    pub user_id: String,
    /// Allowed channel IDs (empty = all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_channels: Vec<String>,
    /// User `_id` values allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against `msg.u._id`.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for RocketChatConfig {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            token_env: "ROCKETCHAT_TOKEN".to_string(),
            user_id: String::new(),
            allowed_channels: vec![],
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Zulip channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ZulipConfig {
    /// Zulip server URL.
    pub server_url: String,
    /// Bot email address.
    pub bot_email: String,
    /// Env var name holding the API key.
    pub api_key_env: String,
    /// Streams to listen in.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub streams: Vec<String>,
    /// Sender emails allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against `sender_email`.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for ZulipConfig {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            bot_email: String::new(),
            api_key_env: "ZULIP_API_KEY".to_string(),
            streams: vec![],
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// XMPP/Jabber channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct XmppConfig {
    /// JID (e.g., "bot@jabber.org").
    pub jid: String,
    /// Env var name holding the password.
    pub password_env: String,
    /// XMPP server hostname (defaults to JID domain).
    pub server: String,
    /// XMPP server port.
    pub port: u16,
    /// MUC rooms to join.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub rooms: Vec<String>,
    /// JIDs allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against the inbound stanza
    /// `from` JID. The XMPP adapter is currently a stub; the field is
    /// stored on the adapter so the future tokio-xmpp implementation can
    /// enforce the gate without further config plumbing.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for XmppConfig {
    fn default() -> Self {
        Self {
            jid: String::new(),
            password_env: "XMPP_PASSWORD".to_string(),
            server: String::new(),
            port: 5222,
            rooms: vec![],
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

// ── Wave 3 channel configs ─────────────────────────────────────────

/// LINE Messaging API channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LineConfig {
    /// Env var name holding the channel secret.
    pub channel_secret_env: String,
    /// Env var name holding the channel access token.
    pub access_token_env: String,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// LINE user IDs allowed to address the bot.
    /// Empty = deny all (B.8 contract). Use `["*"]` to allow everyone explicitly.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for LineConfig {
    fn default() -> Self {
        Self {
            channel_secret_env: "LINE_CHANNEL_SECRET".to_string(),
            access_token_env: "LINE_CHANNEL_ACCESS_TOKEN".to_string(),
            webhook_port: 8450,
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Viber Bot API channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ViberConfig {
    /// Env var name holding the auth token.
    pub auth_token_env: String,
    /// Webhook URL for receiving messages.
    pub webhook_url: String,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// Viber sender IDs allowed to address the bot.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for ViberConfig {
    fn default() -> Self {
        Self {
            auth_token_env: "VIBER_AUTH_TOKEN".to_string(),
            webhook_url: String::new(),
            webhook_port: 8451,
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Facebook Messenger Platform channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MessengerConfig {
    /// Env var name holding the page access token.
    pub page_token_env: String,
    /// Env var name holding the webhook verify token.
    pub verify_token_env: String,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// Messenger sender IDs allowed to address the bot.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for MessengerConfig {
    fn default() -> Self {
        Self {
            page_token_env: "MESSENGER_PAGE_TOKEN".to_string(),
            verify_token_env: "MESSENGER_VERIFY_TOKEN".to_string(),
            webhook_port: 8452,
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Reddit API channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RedditConfig {
    /// Reddit app client ID.
    pub client_id: String,
    /// Env var name holding the client secret.
    pub client_secret_env: String,
    /// Reddit bot username.
    pub username: String,
    /// Env var name holding the bot password.
    pub password_env: String,
    /// Subreddits to monitor.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub subreddits: Vec<String>,
    /// Reddit usernames allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all, otherwise exact username match).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for RedditConfig {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            client_secret_env: "REDDIT_CLIENT_SECRET".to_string(),
            username: String::new(),
            password_env: "REDDIT_PASSWORD".to_string(),
            subreddits: vec![],
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Mastodon Streaming API channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MastodonConfig {
    /// Mastodon instance URL (e.g., `"https://mastodon.social"`).
    pub instance_url: String,
    /// Env var name holding the access token.
    pub access_token_env: String,
    /// Mastodon `acct` handles allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against `account.acct` (e.g.
    /// `"alice"` for local, `"alice@mastodon.social"` for remote).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for MastodonConfig {
    fn default() -> Self {
        Self {
            instance_url: String::new(),
            access_token_env: "MASTODON_ACCESS_TOKEN".to_string(),
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Bluesky/AT Protocol channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BlueskyConfig {
    /// Bluesky identifier (handle or DID).
    pub identifier: String,
    /// Env var name holding the app password.
    pub app_password_env: String,
    /// PDS service URL.
    pub service_url: String,
    /// Bluesky handles allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against `author.handle`.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for BlueskyConfig {
    fn default() -> Self {
        Self {
            identifier: String::new(),
            app_password_env: "BLUESKY_APP_PASSWORD".to_string(),
            service_url: "https://bsky.social".to_string(),
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Feishu/Lark Open Platform channel adapter configuration.
///
/// Supports both Feishu (China domestic, `open.feishu.cn`) and Lark
/// (International, `open.larksuite.com`) via the `region` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FeishuConfig {
    /// Feishu app ID.
    pub app_id: String,
    /// Env var name holding the app secret.
    pub app_secret_env: String,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// Region: "cn" for Feishu (open.feishu.cn), "intl" for Lark (open.larksuite.com).
    pub region: String,
    /// Webhook URL path (default: "/feishu/webhook").
    pub webhook_path: String,
    /// Optional verification token for webhook event validation.
    pub verification_token: Option<String>,
    /// Env var name holding the encrypt key for event decryption (AES-256-CBC).
    pub encrypt_key_env: Option<String>,
    /// Bot name aliases for group-chat @mention detection.
    #[serde(default)]
    pub bot_names: Vec<String>,
    /// Feishu/Lark open IDs allowed to address the bot.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for FeishuConfig {
    fn default() -> Self {
        Self {
            app_id: String::new(),
            app_secret_env: "FEISHU_APP_SECRET".to_string(),
            webhook_port: 8453,
            region: "cn".to_string(),
            webhook_path: "/feishu/webhook".to_string(),
            verification_token: None,
            encrypt_key_env: None,
            bot_names: Vec::new(),
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// WeCom/WeChat Work channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WeComConfig {
    /// WeCom corp ID.
    pub corp_id: String,
    /// WeCom application agent ID.
    pub agent_id: String,
    /// Env var name holding the application secret.
    pub secret_env: String,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// Callback verification token (optional, for URL verification).
    pub token: Option<String>,
    /// Encoding AES key for callback (optional, for encrypted mode).
    pub encoding_aes_key: Option<String>,
    /// WeCom user IDs (`FromUserName`) allowed to address the bot.
    /// Empty = deny all (B.8 contract). Use `["*"]` to allow everyone explicitly.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for WeComConfig {
    fn default() -> Self {
        Self {
            corp_id: String::new(),
            agent_id: String::new(),
            secret_env: "WECOM_SECRET".to_string(),
            webhook_port: 8454,
            token: None,
            encoding_aes_key: None,
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Revolt (Discord-like) channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RevoltConfig {
    /// Env var name holding the bot token.
    pub bot_token_env: String,
    /// Revolt API URL.
    pub api_url: String,
    /// Revolt user IDs allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against `author` id.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for RevoltConfig {
    fn default() -> Self {
        Self {
            bot_token_env: "REVOLT_BOT_TOKEN".to_string(),
            api_url: "https://api.revolt.chat".to_string(),
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

// ── Wave 4 channel configs ─────────────────────────────────────────

/// Nextcloud Talk channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NextcloudConfig {
    /// Nextcloud server URL.
    pub server_url: String,
    /// Env var name holding the auth token.
    pub token_env: String,
    /// Room tokens to listen in (empty = all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_rooms: Vec<String>,
    /// Actor IDs allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against `actorId`.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for NextcloudConfig {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            token_env: "NEXTCLOUD_TOKEN".to_string(),
            allowed_rooms: vec![],
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Guilded bot channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GuildedConfig {
    /// Env var name holding the bot token.
    pub bot_token_env: String,
    /// Server IDs to listen in (empty = all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub server_ids: Vec<String>,
    /// User IDs allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against `message.createdBy`.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for GuildedConfig {
    fn default() -> Self {
        Self {
            bot_token_env: "GUILDED_BOT_TOKEN".to_string(),
            server_ids: vec![],
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Keybase chat channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KeybaseConfig {
    /// Keybase username.
    pub username: String,
    /// Env var name holding the paper key.
    pub paperkey_env: String,
    /// Team names to listen in (empty = all DMs).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_teams: Vec<String>,
    /// Usernames allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against `sender.username`.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for KeybaseConfig {
    fn default() -> Self {
        Self {
            username: String::new(),
            paperkey_env: "KEYBASE_PAPERKEY".to_string(),
            allowed_teams: vec![],
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Threema Gateway channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ThreemaConfig {
    /// Threema Gateway ID.
    pub threema_id: String,
    /// Env var name holding the API secret.
    pub secret_env: String,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// Threema IDs allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against the inbound `from` field.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for ThreemaConfig {
    fn default() -> Self {
        Self {
            threema_id: String::new(),
            secret_env: "THREEMA_SECRET".to_string(),
            webhook_port: 8454,
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Nostr relay channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NostrConfig {
    /// Env var name holding the private key (nsec or hex).
    pub private_key_env: String,
    /// Relay URLs to connect to.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub relays: Vec<String>,
    /// Pubkeys allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against `event.pubkey`.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for NostrConfig {
    fn default() -> Self {
        Self {
            private_key_env: "NOSTR_PRIVATE_KEY".to_string(),
            relays: vec!["wss://relay.damus.io".to_string()],
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Webex bot channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebexConfig {
    /// Env var name holding the bot token.
    pub bot_token_env: String,
    /// Room IDs to listen in (empty = all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_rooms: Vec<String>,
    /// personIds allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against `message.personId`.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for WebexConfig {
    fn default() -> Self {
        Self {
            bot_token_env: "WEBEX_BOT_TOKEN".to_string(),
            allowed_rooms: vec![],
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Pumble bot channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PumbleConfig {
    /// Env var name holding the bot token.
    pub bot_token_env: String,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// Pumble user IDs allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against `event.user` / `event.user_id`.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for PumbleConfig {
    fn default() -> Self {
        Self {
            bot_token_env: "PUMBLE_BOT_TOKEN".to_string(),
            webhook_port: 8455,
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Flock bot channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FlockConfig {
    /// Env var name holding the bot token.
    pub bot_token_env: String,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// Flock user IDs allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against `message.from`.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for FlockConfig {
    fn default() -> Self {
        Self {
            bot_token_env: "FLOCK_BOT_TOKEN".to_string(),
            webhook_port: 8456,
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Twist API v3 channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TwistConfig {
    /// Env var name holding the API token.
    pub token_env: String,
    /// Workspace ID.
    pub workspace_id: String,
    /// Channel IDs to listen in (empty = all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_channels: Vec<String>,
    /// User IDs allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against `comment.creator` (stringified).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for TwistConfig {
    fn default() -> Self {
        Self {
            token_env: "TWIST_TOKEN".to_string(),
            workspace_id: String::new(),
            allowed_channels: vec![],
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

// ── Wave 5 channel configs ─────────────────────────────────────────

/// Mumble text chat channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MumbleConfig {
    /// Mumble server hostname.
    pub host: String,
    /// Mumble server port.
    pub port: u16,
    /// Bot username.
    pub username: String,
    /// Env var name holding the server password.
    pub password_env: String,
    /// Channel to join.
    pub channel: String,
    /// Mumble session IDs allowed to address the bot.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for MumbleConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 64738,
            username: "captain".to_string(),
            password_env: "MUMBLE_PASSWORD".to_string(),
            channel: String::new(),
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// DingTalk Robot API channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DingTalkConfig {
    /// Env var name holding the webhook access token.
    pub access_token_env: String,
    /// Env var name holding the signing secret.
    pub secret_env: String,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// Sender IDs allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against `senderId`.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for DingTalkConfig {
    fn default() -> Self {
        Self {
            access_token_env: "DINGTALK_ACCESS_TOKEN".to_string(),
            secret_env: "DINGTALK_SECRET".to_string(),
            webhook_port: 8457,
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// DingTalk Stream channel adapter configuration.
///
/// Uses the DingTalk Stream Mode (WebSocket long-connection) instead of
/// the legacy webhook approach. Requires an Enterprise Internal App with
/// Stream Mode enabled in the DingTalk Open Platform console.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DingTalkStreamConfig {
    /// Env var holding the App Key (client_id).
    pub app_key_env: String,
    /// Env var holding the App Secret (client_secret).
    pub app_secret_env: String,
    /// Robot code for outbound batchSend (often same as app_key).
    pub robot_code_env: String,
    /// Staff/sender IDs allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against `senderStaffId` (or
    /// `senderId` when staff id is empty).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for DingTalkStreamConfig {
    fn default() -> Self {
        Self {
            app_key_env: "DINGTALK_APP_KEY".to_string(),
            app_secret_env: "DINGTALK_APP_SECRET".to_string(),
            robot_code_env: "DINGTALK_ROBOT_CODE".to_string(),
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Discourse forum channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscourseConfig {
    /// Discourse base URL.
    pub base_url: String,
    /// Env var name holding the API key.
    pub api_key_env: String,
    /// API username.
    pub api_username: String,
    /// Category slugs to monitor.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub categories: Vec<String>,
    /// Usernames allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against `post.username`.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for DiscourseConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            api_key_env: "DISCOURSE_API_KEY".to_string(),
            api_username: "system".to_string(),
            categories: vec![],
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Gitter Streaming API channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GitterConfig {
    /// Env var name holding the auth token.
    pub token_env: String,
    /// Room ID to listen in.
    pub room_id: String,
    /// Gitter usernames allowed to address the bot.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for GitterConfig {
    fn default() -> Self {
        Self {
            token_env: "GITTER_TOKEN".to_string(),
            room_id: String::new(),
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// ntfy.sh pub/sub channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NtfyConfig {
    /// ntfy server URL.
    pub server_url: String,
    /// Topic to subscribe/publish to.
    pub topic: String,
    /// Env var name holding the auth token (optional for public topics).
    pub token_env: String,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for NtfyConfig {
    fn default() -> Self {
        Self {
            server_url: "https://ntfy.sh".to_string(),
            topic: String::new(),
            token_env: "NTFY_TOKEN".to_string(),
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Gotify WebSocket channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GotifyConfig {
    /// Gotify server URL.
    pub server_url: String,
    /// Env var name holding the app token (for sending).
    pub app_token_env: String,
    /// Env var name holding the client token (for receiving).
    pub client_token_env: String,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for GotifyConfig {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            app_token_env: "GOTIFY_APP_TOKEN".to_string(),
            client_token_env: "GOTIFY_CLIENT_TOKEN".to_string(),
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Generic webhook channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebhookConfig {
    /// Env var name holding the HMAC signing secret.
    pub secret_env: String,
    /// Port to listen for incoming webhooks.
    pub listen_port: u16,
    /// URL to POST outgoing messages to.
    pub callback_url: Option<String>,
    /// Sender IDs allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against `body.sender_id`.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            secret_env: "WEBHOOK_SECRET".to_string(),
            listen_port: 8460,
            callback_url: None,
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// LinkedIn Messaging API channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LinkedInConfig {
    /// Env var name holding the OAuth2 access token.
    pub access_token_env: String,
    /// Organization ID for messaging.
    pub organization_id: String,
    /// Member URNs allowed to address the bot (B.8: empty = deny all,
    /// `["*"]` = allow all). Match is exact against `element.from`
    /// (e.g. `"urn:li:person:abc123"`).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for LinkedInConfig {
    fn default() -> Self {
        Self {
            access_token_env: "LINKEDIN_ACCESS_TOKEN".to_string(),
            organization_id: String::new(),
            allowed_users: vec![],
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChannelsConfig, KernelConfig};

    #[test]
    fn test_slack_config_defaults() {
        let sl = SlackConfig::default();
        assert_eq!(sl.app_token_env, "SLACK_APP_TOKEN");
        assert_eq!(sl.bot_token_env, "SLACK_BOT_TOKEN");
        assert!(sl.allowed_channels.is_empty());
    }

    #[test]
    fn test_whatsapp_config_defaults() {
        let wa = WhatsAppConfig::default();
        assert_eq!(wa.access_token_env, "WHATSAPP_ACCESS_TOKEN");
        assert_eq!(wa.webhook_port, 8443);
        assert!(wa.allowed_users.is_empty());
    }

    #[test]
    fn test_matrix_config_defaults() {
        let mx = MatrixConfig::default();
        assert_eq!(mx.homeserver_url, "https://matrix.org");
        assert_eq!(mx.access_token_env, "MATRIX_ACCESS_TOKEN");
        assert!(mx.allowed_rooms.is_empty());
    }

    #[test]
    fn test_whatsapp_config_serde() {
        let wa = WhatsAppConfig {
            phone_number_id: "12345".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&wa).unwrap();
        let back: WhatsAppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.phone_number_id, "12345");
    }

    #[test]
    fn test_matrix_config_serde() {
        let mx = MatrixConfig {
            user_id: "@bot:matrix.org".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&mx).unwrap();
        let back: MatrixConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.user_id, "@bot:matrix.org");
    }

    #[test]
    fn test_teams_config_defaults() {
        let t = TeamsConfig::default();
        assert_eq!(t.app_password_env, "TEAMS_APP_PASSWORD");
        assert_eq!(t.webhook_port, 3978);
        assert!(t.allowed_tenants.is_empty());
    }

    #[test]
    fn test_mattermost_config_defaults() {
        let m = MattermostConfig::default();
        assert_eq!(m.token_env, "MATTERMOST_TOKEN");
        assert!(m.server_url.is_empty());
    }

    #[test]
    fn test_irc_config_defaults() {
        let irc = IrcConfig::default();
        assert_eq!(irc.server, "irc.libera.chat");
        assert_eq!(irc.port, 6667);
        assert_eq!(irc.nick, "captain");
        assert!(!irc.use_tls);
    }

    #[test]
    fn test_google_chat_config_defaults() {
        let gc = GoogleChatConfig::default();
        assert_eq!(gc.service_account_env, "GOOGLE_CHAT_SERVICE_ACCOUNT");
        assert_eq!(gc.webhook_port, 8444);
    }

    #[test]
    fn test_twitch_config_defaults() {
        let tw = TwitchConfig::default();
        assert_eq!(tw.oauth_token_env, "TWITCH_OAUTH_TOKEN");
        assert_eq!(tw.nick, "captain");
    }

    #[test]
    fn test_rocketchat_config_defaults() {
        let rc = RocketChatConfig::default();
        assert_eq!(rc.token_env, "ROCKETCHAT_TOKEN");
        assert!(rc.server_url.is_empty());
    }

    #[test]
    fn test_zulip_config_defaults() {
        let z = ZulipConfig::default();
        assert_eq!(z.api_key_env, "ZULIP_API_KEY");
        assert!(z.bot_email.is_empty());
    }

    #[test]
    fn test_xmpp_config_defaults() {
        let x = XmppConfig::default();
        assert_eq!(x.password_env, "XMPP_PASSWORD");
        assert_eq!(x.port, 5222);
        assert!(x.rooms.is_empty());
    }

    #[test]
    fn test_all_new_channel_configs_serde() {
        let config = KernelConfig {
            channels: ChannelsConfig {
                teams: Some(TeamsConfig::default()),
                mattermost: Some(MattermostConfig::default()),
                irc: Some(IrcConfig::default()),
                google_chat: Some(GoogleChatConfig::default()),
                twitch: Some(TwitchConfig::default()),
                rocketchat: Some(RocketChatConfig::default()),
                zulip: Some(ZulipConfig::default()),
                xmpp: Some(XmppConfig::default()),
                ..Default::default()
            },
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let back: KernelConfig = toml::from_str(&toml_str).unwrap();
        assert!(back.channels.teams.is_some());
        assert!(back.channels.mattermost.is_some());
        assert!(back.channels.irc.is_some());
        assert!(back.channels.google_chat.is_some());
        assert!(back.channels.twitch.is_some());
        assert!(back.channels.rocketchat.is_some());
        assert!(back.channels.zulip.is_some());
        assert!(back.channels.xmpp.is_some());
    }

    #[test]
    fn frozen_long_tail_defaults_keep_compat_contracts() {
        assert_eq!(
            LineConfig::default().access_token_env,
            "LINE_CHANNEL_ACCESS_TOKEN"
        );
        assert_eq!(ViberConfig::default().webhook_port, 8451);
        assert_eq!(
            MessengerConfig::default().verify_token_env,
            "MESSENGER_VERIFY_TOKEN"
        );
        assert_eq!(RedditConfig::default().password_env, "REDDIT_PASSWORD");
        assert!(MastodonConfig::default().allowed_users.is_empty());
        assert_eq!(BlueskyConfig::default().service_url, "https://bsky.social");
        assert_eq!(FeishuConfig::default().region, "cn");
        assert_eq!(WeComConfig::default().secret_env, "WECOM_SECRET");
        assert_eq!(RevoltConfig::default().api_url, "https://api.revolt.chat");
        assert_eq!(NextcloudConfig::default().token_env, "NEXTCLOUD_TOKEN");
        assert_eq!(GuildedConfig::default().bot_token_env, "GUILDED_BOT_TOKEN");
        assert_eq!(KeybaseConfig::default().paperkey_env, "KEYBASE_PAPERKEY");
        assert_eq!(ThreemaConfig::default().webhook_port, 8454);
        assert_eq!(NostrConfig::default().relays, vec!["wss://relay.damus.io"]);
        assert_eq!(WebexConfig::default().bot_token_env, "WEBEX_BOT_TOKEN");
        assert_eq!(PumbleConfig::default().webhook_port, 8455);
        assert_eq!(FlockConfig::default().webhook_port, 8456);
        assert_eq!(TwistConfig::default().token_env, "TWIST_TOKEN");
        assert_eq!(MumbleConfig::default().port, 64738);
        assert_eq!(DingTalkConfig::default().webhook_port, 8457);
        assert_eq!(
            DingTalkStreamConfig::default().robot_code_env,
            "DINGTALK_ROBOT_CODE"
        );
        assert_eq!(DiscourseConfig::default().api_username, "system");
        assert_eq!(GitterConfig::default().token_env, "GITTER_TOKEN");
        assert_eq!(NtfyConfig::default().server_url, "https://ntfy.sh");
        assert_eq!(
            GotifyConfig::default().client_token_env,
            "GOTIFY_CLIENT_TOKEN"
        );
        assert_eq!(WebhookConfig::default().listen_port, 8460);
        assert_eq!(
            LinkedInConfig::default().access_token_env,
            "LINKEDIN_ACCESS_TOKEN"
        );
    }

    #[test]
    fn frozen_long_tail_channels_roundtrip_through_kernel_config() {
        let config = KernelConfig {
            channels: ChannelsConfig {
                line: Some(LineConfig::default()),
                viber: Some(ViberConfig::default()),
                messenger: Some(MessengerConfig::default()),
                reddit: Some(RedditConfig::default()),
                mastodon: Some(MastodonConfig::default()),
                bluesky: Some(BlueskyConfig::default()),
                feishu: Some(FeishuConfig::default()),
                revolt: Some(RevoltConfig::default()),
                nextcloud: Some(NextcloudConfig::default()),
                guilded: Some(GuildedConfig::default()),
                keybase: Some(KeybaseConfig::default()),
                threema: Some(ThreemaConfig::default()),
                nostr: Some(NostrConfig::default()),
                webex: Some(WebexConfig::default()),
                pumble: Some(PumbleConfig::default()),
                flock: Some(FlockConfig::default()),
                twist: Some(TwistConfig::default()),
                mumble: Some(MumbleConfig::default()),
                dingtalk: Some(DingTalkConfig::default()),
                dingtalk_stream: Some(DingTalkStreamConfig::default()),
                discourse: Some(DiscourseConfig::default()),
                gitter: Some(GitterConfig::default()),
                ntfy: Some(NtfyConfig::default()),
                gotify: Some(GotifyConfig::default()),
                webhook: Some(WebhookConfig::default()),
                linkedin: Some(LinkedInConfig::default()),
                wecom: Some(WeComConfig::default()),
                ..Default::default()
            },
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let back: KernelConfig = toml::from_str(&toml_str).unwrap();

        assert!(back.channels.line.is_some());
        assert!(back.channels.viber.is_some());
        assert!(back.channels.messenger.is_some());
        assert!(back.channels.reddit.is_some());
        assert!(back.channels.mastodon.is_some());
        assert!(back.channels.bluesky.is_some());
        assert!(back.channels.feishu.is_some());
        assert!(back.channels.revolt.is_some());
        assert!(back.channels.nextcloud.is_some());
        assert!(back.channels.guilded.is_some());
        assert!(back.channels.keybase.is_some());
        assert!(back.channels.threema.is_some());
        assert!(back.channels.nostr.is_some());
        assert!(back.channels.webex.is_some());
        assert!(back.channels.pumble.is_some());
        assert!(back.channels.flock.is_some());
        assert!(back.channels.twist.is_some());
        assert!(back.channels.mumble.is_some());
        assert!(back.channels.dingtalk.is_some());
        assert!(back.channels.dingtalk_stream.is_some());
        assert!(back.channels.discourse.is_some());
        assert!(back.channels.gitter.is_some());
        assert!(back.channels.ntfy.is_some());
        assert!(back.channels.gotify.is_some());
        assert!(back.channels.webhook.is_some());
        assert!(back.channels.linkedin.is_some());
        assert!(back.channels.wecom.is_some());
    }

    #[test]
    fn test_slack_config_unfurl_links_defaults_true() {
        let config: SlackConfig = toml::from_str("").unwrap();
        assert!(config.unfurl_links);
    }

    #[test]
    fn test_slack_config_unfurl_links_explicit_false() {
        let config: SlackConfig = toml::from_str("unfurl_links = false").unwrap();
        assert!(!config.unfurl_links);
    }

    #[test]
    fn test_slack_config_unfurl_links_explicit_true() {
        let config: SlackConfig = toml::from_str("unfurl_links = true").unwrap();
        assert!(config.unfurl_links);
    }
}
