//! Telegram `reply_to_message` context helpers.

use crate::types::ChannelContent;

pub(crate) fn apply_telegram_reply_context(
    content: ChannelContent,
    message: &serde_json::Value,
) -> ChannelContent {
    let Some(reply_msg) = message.get("reply_to_message") else {
        return content;
    };
    let reply_text = reply_msg["text"]
        .as_str()
        .or_else(|| reply_msg["caption"].as_str());
    let Some(quoted_text) = reply_text else {
        return content;
    };

    let sender_label = reply_msg["from"]["first_name"]
        .as_str()
        .unwrap_or("Unknown");
    let prefix = format!("[Replying to {sender_label}: {quoted_text}]\n\n");

    match content {
        ChannelContent::Text(text) => ChannelContent::Text(format!("{prefix}{text}")),
        ChannelContent::Command { name, args } => {
            let mut new_args = vec![format!("{prefix}{}", args.join(" "))];
            new_args.retain(|arg| !arg.trim().is_empty());
            ChannelContent::Command {
                name,
                args: new_args,
            }
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_reply_context_prepends_text_reply() {
        let message = serde_json::json!({
            "text": "I agree",
            "reply_to_message": {
                "from": { "first_name": "Bob" },
                "text": "Use Rust"
            }
        });

        let content =
            apply_telegram_reply_context(ChannelContent::Text("I agree".to_string()), &message);

        match content {
            ChannelContent::Text(text) => {
                assert_eq!(text, "[Replying to Bob: Use Rust]\n\nI agree")
            }
            other => panic!("expected text content, got {other:?}"),
        }
    }

    #[test]
    fn telegram_reply_context_uses_caption_and_unknown_sender() {
        let message = serde_json::json!({
            "text": "Nice",
            "reply_to_message": {
                "caption": "Sunset"
            }
        });

        let content =
            apply_telegram_reply_context(ChannelContent::Text("Nice".to_string()), &message);

        match content {
            ChannelContent::Text(text) => {
                assert_eq!(text, "[Replying to Unknown: Sunset]\n\nNice")
            }
            other => panic!("expected text content, got {other:?}"),
        }
    }

    #[test]
    fn telegram_reply_context_preserves_command_shape() {
        let message = serde_json::json!({
            "text": "/do thing",
            "reply_to_message": {
                "from": { "first_name": "Bob" },
                "text": "Context"
            }
        });
        let content = ChannelContent::Command {
            name: "do".to_string(),
            args: vec!["thing".to_string()],
        };

        let content = apply_telegram_reply_context(content, &message);

        match content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "do");
                assert_eq!(args, vec!["[Replying to Bob: Context]\n\nthing"]);
            }
            other => panic!("expected command content, got {other:?}"),
        }
    }

    #[test]
    fn telegram_reply_context_leaves_media_and_empty_replies_unchanged() {
        let message = serde_json::json!({
            "reply_to_message": {
                "message_id": 42
            }
        });
        let content = ChannelContent::Location {
            lat: 48.0,
            lon: 2.0,
        };

        assert_location(apply_telegram_reply_context(content.clone(), &message));
        assert_location(apply_telegram_reply_context(
            content,
            &serde_json::json!({}),
        ));
    }

    fn assert_location(content: ChannelContent) {
        match content {
            ChannelContent::Location { lat, lon } => {
                assert_eq!(lat, 48.0);
                assert_eq!(lon, 2.0);
            }
            other => panic!("expected location content, got {other:?}"),
        }
    }
}
