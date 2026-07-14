use super::UsageFooterMode;
use serde::{Deserialize, Serialize};

/// DM (direct message) policy for a channel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DmPolicy {
    /// Respond to all DMs.
    #[default]
    Respond,
    /// Only respond to DMs from allowed users.
    AllowedOnly,
    /// Ignore all DMs.
    Ignore,
}

/// Group message policy for a channel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupPolicy {
    /// Respond to all group messages.
    All,
    /// Only respond when mentioned (@bot).
    #[default]
    MentionOnly,
    /// Only respond to slash commands.
    CommandsOnly,
    /// Ignore all group messages.
    Ignore,
}

/// Output format hint for channel-specific message formatting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    /// Standard Markdown (default).
    #[default]
    Markdown,
    /// Telegram HTML subset.
    TelegramHtml,
    /// Slack mrkdwn format.
    SlackMrkdwn,
    /// Plain text (no formatting).
    PlainText,
}

/// Typing indicator behavior mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypingMode {
    /// Send typing indicator immediately on message receipt (default).
    #[default]
    Instant,
    /// Send typing indicator only when first text delta arrives.
    Message,
    /// Send typing indicator only during LLM reasoning.
    Thinking,
    /// Never send typing indicators.
    Never,
}

/// Per-channel behavior overrides.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelOverrides {
    /// Model override (uses agent's default if None).
    pub model: Option<String>,
    /// System prompt override.
    pub system_prompt: Option<String>,
    /// DM policy.
    pub dm_policy: DmPolicy,
    /// Group message policy.
    pub group_policy: GroupPolicy,
    /// Per-user rate limit (messages per minute, 0 = unlimited).
    pub rate_limit_per_user: u32,
    /// Enable thread replies.
    pub threading: bool,
    /// Output format override.
    pub output_format: Option<OutputFormat>,
    /// Usage footer mode override.
    pub usage_footer: Option<UsageFooterMode>,
    /// Typing indicator mode override.
    pub typing_mode: Option<TypingMode>,
    /// Whether to send lifecycle emoji reactions on messages.
    /// HS.8 defaults to false; typing indicators are the quieter feedback.
    #[serde(default)]
    pub lifecycle_reactions: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_overrides_defaults_keep_quiet_lifecycle_feedback() {
        let overrides = ChannelOverrides::default();

        assert_eq!(overrides.dm_policy, DmPolicy::Respond);
        assert_eq!(overrides.group_policy, GroupPolicy::MentionOnly);
        assert_eq!(overrides.rate_limit_per_user, 0);
        assert!(!overrides.threading);
        assert!(overrides.output_format.is_none());
        assert!(overrides.model.is_none());
        assert!(overrides.usage_footer.is_none());
        assert!(overrides.typing_mode.is_none());
        assert!(!overrides.lifecycle_reactions);
    }

    #[test]
    fn channel_overrides_roundtrip_behavior_fields() {
        let overrides = ChannelOverrides {
            dm_policy: DmPolicy::Ignore,
            group_policy: GroupPolicy::CommandsOnly,
            rate_limit_per_user: 10,
            threading: true,
            output_format: Some(OutputFormat::TelegramHtml),
            usage_footer: Some(UsageFooterMode::Tokens),
            typing_mode: Some(TypingMode::Thinking),
            ..Default::default()
        };

        let json = serde_json::to_string(&overrides).unwrap();
        let back: ChannelOverrides = serde_json::from_str(&json).unwrap();

        assert_eq!(back.dm_policy, DmPolicy::Ignore);
        assert_eq!(back.group_policy, GroupPolicy::CommandsOnly);
        assert_eq!(back.rate_limit_per_user, 10);
        assert!(back.threading);
        assert_eq!(back.output_format, Some(OutputFormat::TelegramHtml));
        assert_eq!(back.usage_footer, Some(UsageFooterMode::Tokens));
        assert_eq!(back.typing_mode, Some(TypingMode::Thinking));
        assert!(!back.lifecycle_reactions);
    }

    #[test]
    fn lifecycle_reactions_can_be_disabled_explicitly() {
        let json = r#"{"lifecycle_reactions": false}"#;
        let overrides: ChannelOverrides = serde_json::from_str(json).unwrap();

        assert!(!overrides.lifecycle_reactions);
        assert_eq!(overrides.dm_policy, DmPolicy::Respond);
        assert!(overrides.model.is_none());
    }

    #[test]
    fn lifecycle_reactions_missing_defaults_false() {
        let json = r#"{}"#;
        let overrides: ChannelOverrides = serde_json::from_str(json).unwrap();

        assert!(!overrides.lifecycle_reactions);
    }

    #[test]
    fn output_and_typing_modes_use_snake_case_serde() {
        let overrides: ChannelOverrides =
            serde_json::from_str(r#"{"output_format":"plain_text","typing_mode":"message"}"#)
                .unwrap();

        assert_eq!(overrides.output_format, Some(OutputFormat::PlainText));
        assert_eq!(overrides.typing_mode, Some(TypingMode::Message));
    }
}
