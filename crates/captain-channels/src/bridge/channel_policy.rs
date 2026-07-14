//! Pure DM/group policy decisions for inbound channel messages.

use crate::types::{ChannelContent, ChannelMessage};
use captain_types::config::{ChannelOverrides, DmPolicy, GroupPolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChannelPolicyIgnoreReason {
    GroupIgnored,
    GroupCommandsOnly,
    GroupMentionOnly,
    DmIgnored,
}

impl ChannelPolicyIgnoreReason {
    pub(crate) fn debug_message(self, channel: &str) -> String {
        match self {
            Self::GroupIgnored => {
                format!("Ignoring group message on {channel} (group_policy=ignore)")
            }
            Self::GroupCommandsOnly => format!(
                "Ignoring non-command group message on {channel} (group_policy=commands_only)"
            ),
            Self::GroupMentionOnly => format!(
                "Ignoring group message on {channel} (group_policy=mention_only, not mentioned)"
            ),
            Self::DmIgnored => format!("Ignoring DM on {channel} (dm_policy=ignore)"),
        }
    }
}

pub(crate) fn channel_policy_ignore_reason(
    message: &ChannelMessage,
    overrides: Option<&ChannelOverrides>,
) -> Option<ChannelPolicyIgnoreReason> {
    let overrides = overrides?;

    if message.is_group {
        match overrides.group_policy {
            GroupPolicy::Ignore => Some(ChannelPolicyIgnoreReason::GroupIgnored),
            GroupPolicy::CommandsOnly => {
                if is_command_message(message) {
                    None
                } else {
                    Some(ChannelPolicyIgnoreReason::GroupCommandsOnly)
                }
            }
            GroupPolicy::MentionOnly => {
                if was_mentioned(message)
                    || matches!(&message.content, ChannelContent::Command { .. })
                {
                    None
                } else {
                    Some(ChannelPolicyIgnoreReason::GroupMentionOnly)
                }
            }
            GroupPolicy::All => None,
        }
    } else {
        match overrides.dm_policy {
            DmPolicy::Ignore => Some(ChannelPolicyIgnoreReason::DmIgnored),
            DmPolicy::AllowedOnly | DmPolicy::Respond => None,
        }
    }
}

fn is_command_message(message: &ChannelMessage) -> bool {
    matches!(&message.content, ChannelContent::Command { .. })
        || matches!(&message.content, ChannelContent::Text(t) if t.starts_with('/'))
}

fn was_mentioned(message: &ChannelMessage) -> bool {
    message
        .metadata
        .get("was_mentioned")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelType, ChannelUser};

    fn message(content: ChannelContent, is_group: bool) -> ChannelMessage {
        ChannelMessage {
            channel: ChannelType::Telegram,
            platform_message_id: "msg-1".to_string(),
            sender: ChannelUser {
                platform_id: "user-1".to_string(),
                display_name: "User".to_string(),
                captain_user: None,
            },
            content,
            target_agent: None,
            timestamp: chrono::Utc::now(),
            is_group,
            thread_id: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    fn overrides(dm_policy: DmPolicy, group_policy: GroupPolicy) -> ChannelOverrides {
        ChannelOverrides {
            dm_policy,
            group_policy,
            ..ChannelOverrides::default()
        }
    }

    #[test]
    fn no_overrides_allows_message() {
        let msg = message(ChannelContent::Text("hello".to_string()), false);

        assert_eq!(channel_policy_ignore_reason(&msg, None), None);
    }

    #[test]
    fn dm_ignore_blocks_direct_messages() {
        let msg = message(ChannelContent::Text("hello".to_string()), false);
        let ov = overrides(DmPolicy::Ignore, GroupPolicy::All);

        assert_eq!(
            channel_policy_ignore_reason(&msg, Some(&ov)),
            Some(ChannelPolicyIgnoreReason::DmIgnored)
        );
    }

    #[test]
    fn commands_only_group_allows_slash_text_and_command_content() {
        let ov = overrides(DmPolicy::Respond, GroupPolicy::CommandsOnly);
        let slash = message(ChannelContent::Text("/status".to_string()), true);
        let command = message(
            ChannelContent::Command {
                name: "status".to_string(),
                args: Vec::new(),
            },
            true,
        );

        assert_eq!(channel_policy_ignore_reason(&slash, Some(&ov)), None);
        assert_eq!(channel_policy_ignore_reason(&command, Some(&ov)), None);
    }

    #[test]
    fn commands_only_group_blocks_plain_text() {
        let msg = message(ChannelContent::Text("hello".to_string()), true);
        let ov = overrides(DmPolicy::Respond, GroupPolicy::CommandsOnly);

        assert_eq!(
            channel_policy_ignore_reason(&msg, Some(&ov)),
            Some(ChannelPolicyIgnoreReason::GroupCommandsOnly)
        );
    }

    #[test]
    fn mention_only_group_allows_mentions_and_command_content() {
        let ov = overrides(DmPolicy::Respond, GroupPolicy::MentionOnly);
        let mut mentioned = message(ChannelContent::Text("hello".to_string()), true);
        mentioned
            .metadata
            .insert("was_mentioned".to_string(), serde_json::json!(true));
        let command = message(
            ChannelContent::Command {
                name: "status".to_string(),
                args: Vec::new(),
            },
            true,
        );

        assert_eq!(channel_policy_ignore_reason(&mentioned, Some(&ov)), None);
        assert_eq!(channel_policy_ignore_reason(&command, Some(&ov)), None);
    }

    #[test]
    fn mention_only_group_blocks_unmentioned_text() {
        let msg = message(ChannelContent::Text("hello".to_string()), true);
        let ov = overrides(DmPolicy::Respond, GroupPolicy::MentionOnly);

        assert_eq!(
            channel_policy_ignore_reason(&msg, Some(&ov)),
            Some(ChannelPolicyIgnoreReason::GroupMentionOnly)
        );
    }

    #[test]
    fn ignore_reason_formats_existing_debug_messages() {
        assert_eq!(
            ChannelPolicyIgnoreReason::GroupIgnored.debug_message("telegram"),
            "Ignoring group message on telegram (group_policy=ignore)"
        );
        assert_eq!(
            ChannelPolicyIgnoreReason::DmIgnored.debug_message("telegram"),
            "Ignoring DM on telegram (dm_policy=ignore)"
        );
    }
}
