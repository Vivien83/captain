//! Channel key and default output-format mappings.

use crate::types::ChannelType;
use captain_types::config::OutputFormat;

/// Resolve channel type to its config string key.
pub(super) fn channel_type_str(channel: &ChannelType) -> &str {
    match channel {
        ChannelType::Telegram => "telegram",
        ChannelType::Discord => "discord",
        ChannelType::Slack => "slack",
        ChannelType::WhatsApp => "whatsapp",
        ChannelType::Signal => "signal",
        ChannelType::Matrix => "matrix",
        ChannelType::Email => "email",
        ChannelType::Teams => "teams",
        ChannelType::Mattermost => "mattermost",
        ChannelType::WebChat => "webchat",
        ChannelType::CLI => "cli",
        ChannelType::Custom(name) => name.as_str(),
    }
}

/// Inverse of `channel_type_str`.
///
/// Unknown strings fall back to `Custom(s)` so command routing never crashes on
/// an unrecognised channel key.
pub(super) fn channel_type_from_str(channel: &str) -> ChannelType {
    match channel {
        "telegram" => ChannelType::Telegram,
        "discord" => ChannelType::Discord,
        "slack" => ChannelType::Slack,
        "whatsapp" => ChannelType::WhatsApp,
        "signal" => ChannelType::Signal,
        "matrix" => ChannelType::Matrix,
        "email" => ChannelType::Email,
        "teams" => ChannelType::Teams,
        "mattermost" => ChannelType::Mattermost,
        "webchat" => ChannelType::WebChat,
        "cli" => ChannelType::CLI,
        other => ChannelType::Custom(other.to_string()),
    }
}

pub(super) fn default_output_format_for_channel(channel_type: &str) -> OutputFormat {
    match channel_type {
        "telegram" => OutputFormat::TelegramHtml,
        "slack" => OutputFormat::SlackMrkdwn,
        "wecom" => OutputFormat::PlainText,
        _ => OutputFormat::Markdown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_type_str_keeps_config_keys() {
        assert_eq!(channel_type_str(&ChannelType::Telegram), "telegram");
        assert_eq!(channel_type_str(&ChannelType::Matrix), "matrix");
        assert_eq!(channel_type_str(&ChannelType::Email), "email");
        assert_eq!(
            channel_type_str(&ChannelType::Custom("irc".to_string())),
            "irc"
        );
    }

    #[test]
    fn channel_type_from_str_preserves_known_and_custom_channels() {
        assert_eq!(channel_type_from_str("discord"), ChannelType::Discord);
        assert_eq!(channel_type_from_str("signal"), ChannelType::Signal);
        assert_eq!(
            channel_type_from_str("zulip"),
            ChannelType::Custom("zulip".to_string())
        );
    }

    #[test]
    fn default_output_format_matches_channel_adapters() {
        assert_eq!(
            default_output_format_for_channel("telegram"),
            OutputFormat::TelegramHtml
        );
        assert_eq!(
            default_output_format_for_channel("slack"),
            OutputFormat::SlackMrkdwn
        );
        assert_eq!(
            default_output_format_for_channel("wecom"),
            OutputFormat::PlainText
        );
        assert_eq!(
            default_output_format_for_channel("discord"),
            OutputFormat::Markdown
        );
    }
}
