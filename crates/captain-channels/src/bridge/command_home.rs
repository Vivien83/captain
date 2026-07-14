//! Home-channel command helpers.

pub(crate) fn resolve_home_chat_id(args: &[String], sender_platform_id: &str) -> String {
    args.first()
        .cloned()
        .unwrap_or_else(|| sender_platform_id.to_string())
}

pub(crate) fn format_set_home_success(channel: &str, chat_id: &str) -> String {
    format!(
        "🏠 Home channel set for {channel}: cron results and \
         proactive notifications will land in chat {chat_id}."
    )
}

pub(crate) fn format_set_home_error(error: &str) -> String {
    format!("Failed to set home channel: {error}")
}

pub(crate) fn format_get_home_response(channel: &str, chat_id: Option<&str>) -> String {
    match chat_id {
        Some(chat_id) => {
            format!("🏠 Home channel for {channel}: {chat_id}. Use /sethome to change.")
        }
        None => {
            format!("No home channel set for {channel}. Type /sethome to register this chat.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_chat_id_defaults_to_sender_platform_id() {
        let args = Vec::new();

        assert_eq!(resolve_home_chat_id(&args, "sender-42"), "sender-42");
    }

    #[test]
    fn home_chat_id_uses_explicit_argument() {
        let args = vec!["chat-99".to_string()];

        assert_eq!(resolve_home_chat_id(&args, "sender-42"), "chat-99");
    }

    #[test]
    fn set_home_success_mentions_channel_and_chat() {
        let text = format_set_home_success("telegram", "chat-99");

        assert!(text.contains("Home channel set for telegram"));
        assert!(text.contains("proactive notifications"));
        assert!(text.contains("chat chat-99"));
    }

    #[test]
    fn set_home_error_is_operator_safe() {
        assert_eq!(
            format_set_home_error("permission denied"),
            "Failed to set home channel: permission denied"
        );
    }

    #[test]
    fn get_home_response_reports_configured_chat() {
        assert_eq!(
            format_get_home_response("discord", Some("thread-1")),
            "🏠 Home channel for discord: thread-1. Use /sethome to change."
        );
    }

    #[test]
    fn get_home_response_explains_missing_chat() {
        assert_eq!(
            format_get_home_response("telegram", None),
            "No home channel set for telegram. Type /sethome to register this chat."
        );
    }
}
