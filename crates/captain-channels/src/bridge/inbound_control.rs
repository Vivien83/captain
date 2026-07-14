//! Control-message parsing for active inbound channel sessions.

use crate::types::{ChannelContent, ChannelMessage};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParsedTextCommand {
    pub(super) name: String,
    pub(super) args: Vec<String>,
}

pub(super) fn active_session_bypass_message(message: &ChannelMessage) -> Option<ChannelMessage> {
    match &message.content {
        ChannelContent::Command { name, .. } if is_known_channel_command(name) => {
            Some(message.clone())
        }
        ChannelContent::Text(text) => {
            if slash_command_name(text).is_some_and(is_known_channel_command) {
                return Some(message.clone());
            }
            if is_plain_stop_request(text) {
                let mut stop_message = message.clone();
                stop_message.content = ChannelContent::Command {
                    name: "stop".to_string(),
                    args: Vec::new(),
                };
                return Some(stop_message);
            }
            None
        }
        _ => None,
    }
}

pub(super) fn is_active_session_bypass_message(message: &ChannelMessage) -> bool {
    active_session_bypass_message(message).is_some()
}

pub(super) fn slash_command_name(text: &str) -> Option<&str> {
    let command = text
        .trim_start()
        .strip_prefix('/')?
        .split_whitespace()
        .next()
        .unwrap_or_default();
    let command = command.split('@').next().unwrap_or(command);
    (!command.is_empty()).then_some(command)
}

pub(super) fn is_known_channel_command(command: &str) -> bool {
    let command = command.trim_start_matches('/');
    let command = command.split('@').next().unwrap_or(command);
    matches!(
        command,
        "start"
            | "help"
            | "agents"
            | "agent"
            | "status"
            | "health"
            | "version"
            | "reload"
            | "restart"
            | "shutdown"
            | "config"
            | "models"
            | "providers"
            | "new"
            | "clear"
            | "compact"
            | "model"
            | "stop"
            | "usage"
            | "think"
            | "skills"
            | "hands"
            | "workflows"
            | "workflow"
            | "triggers"
            | "trigger"
            | "schedules"
            | "schedule"
            | "approvals"
            | "approve"
            | "reject"
            | "project_answer"
            | "budget"
            | "peers"
            | "a2a"
            | "sethome"
            | "gethome"
    )
}

pub(super) fn parse_known_text_command(text: &str) -> Option<ParsedTextCommand> {
    if !text.starts_with('/') {
        return None;
    }

    let parts: Vec<&str> = text.splitn(2, ' ').collect();
    let raw_command = &parts[0][1..];
    let command = slash_command_name(text).unwrap_or(raw_command);
    if !is_known_channel_command(command) {
        return None;
    }
    let args = if parts.len() > 1 {
        parts[1].split_whitespace().map(String::from).collect()
    } else {
        Vec::new()
    };

    Some(ParsedTextCommand {
        name: command.to_string(),
        args,
    })
}

fn is_plain_stop_request(text: &str) -> bool {
    matches!(
        text.trim().to_ascii_lowercase().as_str(),
        "stop" | "cancel" | "annule" | "annuler" | "arrete" | "arrête" | "stoppe"
    )
}

pub(super) fn inbound_interjection_text(message: &ChannelMessage) -> Option<String> {
    let ChannelContent::Text(text) = &message.content else {
        return None;
    };
    let text = text.trim();
    if text.is_empty() || text.starts_with('/') || text.starts_with('@') {
        return None;
    }
    Some(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelType, ChannelUser};
    use std::collections::HashMap;

    fn test_message(content: ChannelContent) -> ChannelMessage {
        ChannelMessage {
            channel: ChannelType::Telegram,
            platform_message_id: "m1".to_string(),
            sender: ChannelUser {
                platform_id: "chat-1".to_string(),
                display_name: "Ada".to_string(),
                captain_user: Some("captain-user".to_string()),
            },
            content,
            target_agent: None,
            timestamp: chrono::Utc::now(),
            is_group: false,
            thread_id: Some("topic-1".to_string()),
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn known_channel_commands_accept_telegram_bot_suffix() {
        assert_eq!(slash_command_name("/status@CaptainBot now"), Some("status"));
        assert!(is_known_channel_command("status@CaptainBot"));
        assert!(is_active_session_bypass_message(&test_message(
            ChannelContent::Text("/status@CaptainBot".to_string())
        )));
    }

    #[test]
    fn text_command_parser_extracts_known_command_and_args() {
        assert_eq!(
            parse_known_text_command("/agent hello-world"),
            Some(ParsedTextCommand {
                name: "agent".to_string(),
                args: vec!["hello-world".to_string()]
            })
        );
    }

    #[test]
    fn text_command_parser_handles_telegram_bot_suffix() {
        assert_eq!(
            parse_known_text_command("/status@CaptainBot now"),
            Some(ParsedTextCommand {
                name: "status".to_string(),
                args: vec!["now".to_string()]
            })
        );
    }

    #[test]
    fn text_command_parser_keeps_unknown_or_indented_text_for_agent() {
        assert_eq!(
            parse_known_text_command("/unknown should reach agent"),
            None
        );
        assert_eq!(parse_known_text_command(" /status"), None);
    }

    #[test]
    fn plain_stop_request_accepts_mobile_stop_without_slash() {
        let message = test_message(ChannelContent::Text(" Arrête ".to_string()));
        let bypass = active_session_bypass_message(&message).expect("plain stop bypasses");
        match bypass.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "stop");
                assert!(args.is_empty());
            }
            other => panic!("expected stop command, got {other:?}"),
        }
    }

    #[test]
    fn active_session_bypass_keeps_control_commands_immediate() {
        assert!(is_active_session_bypass_message(&test_message(
            ChannelContent::Command {
                name: "stop".to_string(),
                args: Vec::new(),
            }
        )));
        assert!(is_active_session_bypass_message(&test_message(
            ChannelContent::Text("/model codex-mini".to_string())
        )));
        assert!(!is_active_session_bypass_message(&test_message(
            ChannelContent::Text("/unknown should reach agent".to_string())
        )));
        assert!(!is_active_session_bypass_message(&test_message(
            ChannelContent::Text("normal follow up".to_string())
        )));
    }

    #[test]
    fn inbound_interjection_text_accepts_only_plain_context() {
        assert_eq!(
            inbound_interjection_text(&test_message(ChannelContent::Text(
                "  ajoute ce détail  ".to_string()
            ))),
            Some("ajoute ce détail".to_string())
        );
        assert_eq!(
            inbound_interjection_text(&test_message(ChannelContent::Text(
                "/unknown abc".to_string()
            ))),
            None
        );
        assert_eq!(
            inbound_interjection_text(&test_message(ChannelContent::Text(
                "@other fais ça".to_string()
            ))),
            None
        );
        assert_eq!(
            inbound_interjection_text(&test_message(ChannelContent::Image {
                url: "https://example.test/image.png".to_string(),
                caption: None,
            })),
            None
        );
    }
}
