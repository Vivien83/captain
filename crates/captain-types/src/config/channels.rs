use super::{
    BlueskyConfig, DingTalkConfig, DingTalkStreamConfig, DiscordConfig, DiscourseConfig,
    EmailConfig, FeishuConfig, FlockConfig, GitterConfig, GoogleChatConfig, GotifyConfig,
    GuildedConfig, IrcConfig, KeybaseConfig, LineConfig, LinkedInConfig, MastodonConfig,
    MatrixConfig, MattermostConfig, MessengerConfig, MumbleConfig, NextcloudConfig, NostrConfig,
    NtfyConfig, PumbleConfig, RedditConfig, RevoltConfig, RocketChatConfig, SignalConfig,
    SlackConfig, TeamsConfig, TelegramConfig, ThreemaConfig, TwistConfig, TwitchConfig,
    ViberConfig, WeComConfig, WebexConfig, WebhookConfig, WhatsAppConfig, XmppConfig, ZulipConfig,
};
use serde::{Deserialize, Serialize};

/// Channel bridge configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelsConfig {
    /// Silent mode (airplane mode) - suppresses all proactive notifications
    /// (emergent thoughts, reminders) without stopping the consciousness loop.
    /// The system keeps thinking but doesn't notify. Toggleable via API/UI.
    #[serde(default)]
    pub silent_mode: bool,
    /// Telegram bot configuration (None = disabled).
    pub telegram: Option<TelegramConfig>,
    /// Discord bot configuration (None = disabled).
    pub discord: Option<DiscordConfig>,
    /// Slack bot configuration (None = disabled).
    pub slack: Option<SlackConfig>,
    /// WhatsApp Cloud API configuration (None = disabled).
    pub whatsapp: Option<WhatsAppConfig>,
    /// Signal (via signal-cli) configuration (None = disabled).
    pub signal: Option<SignalConfig>,
    /// Matrix protocol configuration (None = disabled).
    pub matrix: Option<MatrixConfig>,
    /// Email (IMAP/SMTP) configuration (None = disabled).
    pub email: Option<EmailConfig>,
    /// Microsoft Teams configuration (None = disabled).
    pub teams: Option<TeamsConfig>,
    /// Mattermost configuration (None = disabled).
    pub mattermost: Option<MattermostConfig>,
    /// IRC configuration (None = disabled).
    pub irc: Option<IrcConfig>,
    /// Google Chat configuration (None = disabled).
    pub google_chat: Option<GoogleChatConfig>,
    /// Twitch chat configuration (None = disabled).
    pub twitch: Option<TwitchConfig>,
    /// Rocket.Chat configuration (None = disabled).
    pub rocketchat: Option<RocketChatConfig>,
    /// Zulip configuration (None = disabled).
    pub zulip: Option<ZulipConfig>,
    /// XMPP/Jabber configuration (None = disabled).
    pub xmpp: Option<XmppConfig>,
    // Wave 3 - High-value channels
    /// LINE Messaging API configuration (None = disabled).
    pub line: Option<LineConfig>,
    /// Viber Bot API configuration (None = disabled).
    pub viber: Option<ViberConfig>,
    /// Facebook Messenger configuration (None = disabled).
    pub messenger: Option<MessengerConfig>,
    /// Reddit API configuration (None = disabled).
    pub reddit: Option<RedditConfig>,
    /// Mastodon Streaming API configuration (None = disabled).
    pub mastodon: Option<MastodonConfig>,
    /// Bluesky/AT Protocol configuration (None = disabled).
    pub bluesky: Option<BlueskyConfig>,
    /// Feishu/Lark Open Platform configuration (None = disabled).
    pub feishu: Option<FeishuConfig>,
    /// Revolt (Discord-like) configuration (None = disabled).
    pub revolt: Option<RevoltConfig>,
    // Wave 4 - Enterprise & community channels
    /// Nextcloud Talk configuration (None = disabled).
    pub nextcloud: Option<NextcloudConfig>,
    /// Guilded bot configuration (None = disabled).
    pub guilded: Option<GuildedConfig>,
    /// Keybase chat configuration (None = disabled).
    pub keybase: Option<KeybaseConfig>,
    /// Threema Gateway configuration (None = disabled).
    pub threema: Option<ThreemaConfig>,
    /// Nostr relay configuration (None = disabled).
    pub nostr: Option<NostrConfig>,
    /// Webex bot configuration (None = disabled).
    pub webex: Option<WebexConfig>,
    /// Pumble bot configuration (None = disabled).
    pub pumble: Option<PumbleConfig>,
    /// Flock bot configuration (None = disabled).
    pub flock: Option<FlockConfig>,
    /// Twist API configuration (None = disabled).
    pub twist: Option<TwistConfig>,
    // Wave 5 - Niche & differentiating channels
    /// Mumble text chat configuration (None = disabled).
    pub mumble: Option<MumbleConfig>,
    /// DingTalk robot configuration - webhook mode (None = disabled).
    pub dingtalk: Option<DingTalkConfig>,
    /// DingTalk Stream mode - long-lived WebSocket (None = disabled).
    pub dingtalk_stream: Option<DingTalkStreamConfig>,
    /// Discourse forum configuration (None = disabled).
    pub discourse: Option<DiscourseConfig>,
    /// Gitter streaming configuration (None = disabled).
    pub gitter: Option<GitterConfig>,
    /// ntfy.sh pub/sub configuration (None = disabled).
    pub ntfy: Option<NtfyConfig>,
    /// Gotify notification configuration (None = disabled).
    pub gotify: Option<GotifyConfig>,
    /// Generic webhook configuration (None = disabled).
    pub webhook: Option<WebhookConfig>,
    /// LinkedIn messaging configuration (None = disabled).
    pub linkedin: Option<LinkedInConfig>,
    /// WeCom/WeChat Work configuration (None = disabled).
    pub wecom: Option<WeComConfig>,
}

#[cfg(test)]
mod tests {
    use super::ChannelsConfig;
    use crate::config::{
        DiscordConfig, EmailConfig, KernelConfig, MatrixConfig, SignalConfig, SlackConfig,
        TelegramConfig, WeComConfig, WhatsAppConfig,
    };

    #[test]
    fn channels_config_defaults_keep_all_adapters_disabled() {
        let config = ChannelsConfig::default();

        assert!(!config.silent_mode);
        assert!(config.telegram.is_none());
        assert!(config.discord.is_none());
        assert!(config.signal.is_none());
        assert!(config.email.is_none());
        assert!(config.slack.is_none());
        assert!(config.wecom.is_none());
    }

    #[test]
    fn channels_config_supports_active_and_frozen_adapters() {
        let config = ChannelsConfig {
            telegram: Some(TelegramConfig::default()),
            discord: Some(DiscordConfig::default()),
            signal: Some(SignalConfig::default()),
            email: Some(EmailConfig::default()),
            whatsapp: Some(WhatsAppConfig::default()),
            matrix: Some(MatrixConfig::default()),
            slack: Some(SlackConfig::default()),
            wecom: Some(WeComConfig::default()),
            ..Default::default()
        };

        assert!(config.telegram.is_some());
        assert!(config.discord.is_some());
        assert!(config.signal.is_some());
        assert!(config.email.is_some());
        assert!(config.whatsapp.is_some());
        assert!(config.matrix.is_some());
        assert!(config.slack.is_some());
        assert!(config.wecom.is_some());
    }

    #[test]
    fn channels_section_deserializes_from_kernel_toml() {
        let config: KernelConfig = toml::from_str(
            r#"
            [channels]
            silent_mode = true

            [channels.telegram]
            allowed_users = [123, "456"]

            [channels.discord]
            allowed_guilds = ["guild"]

            [channels.matrix]
            homeserver_url = "https://matrix.example"
            "#,
        )
        .unwrap();

        assert!(config.channels.silent_mode);
        assert_eq!(
            config
                .channels
                .telegram
                .as_ref()
                .map(|telegram| telegram.allowed_users.as_slice()),
            Some(["123".to_string(), "456".to_string()].as_slice())
        );
        assert!(config.channels.discord.is_some());
        assert_eq!(
            config
                .channels
                .matrix
                .as_ref()
                .map(|matrix| matrix.homeserver_url.as_str()),
            Some("https://matrix.example")
        );
    }
}
