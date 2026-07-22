//! Telegram update identity, thread and metadata helpers.

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use tracing::debug;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TelegramUpdateContext {
    pub(crate) user_id: i64,
    pub(crate) display_name: String,
    pub(crate) chat_id: i64,
    pub(crate) is_group: bool,
    pub(crate) message_id: i64,
    pub(crate) timestamp: DateTime<Utc>,
    pub(crate) thread_id: Option<String>,
}

pub(crate) fn telegram_update_message(
    update: &serde_json::Value,
    update_id: i64,
) -> Option<&serde_json::Value> {
    match update
        .get("message")
        .or_else(|| update.get("edited_message"))
    {
        Some(message) => Some(message),
        None => {
            debug!("Telegram: dropping update {update_id} — no message or edited_message field");
            None
        }
    }
}

pub(crate) fn parse_telegram_update_context(
    message: &serde_json::Value,
    update_id: i64,
) -> Option<TelegramUpdateContext> {
    let (user_id, display_name) = telegram_message_sender(message, update_id)?;
    let chat_id = match message["chat"]["id"].as_i64() {
        Some(id) => id,
        None => {
            debug!("Telegram: dropping update {update_id} — chat.id is not an integer");
            return None;
        }
    };

    let chat_type = message["chat"]["type"].as_str().unwrap_or("private");
    let is_group = chat_type == "group" || chat_type == "supergroup";
    let message_id = message["message_id"].as_i64().unwrap_or(0);
    let timestamp = message["date"]
        .as_i64()
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
        .unwrap_or_else(chrono::Utc::now);
    let thread_id = message["message_thread_id"]
        .as_i64()
        .map(|tid| tid.to_string());

    Some(TelegramUpdateContext {
        user_id,
        display_name,
        chat_id,
        is_group,
        message_id,
        timestamp,
        thread_id,
    })
}

fn telegram_message_sender(message: &serde_json::Value, update_id: i64) -> Option<(i64, String)> {
    if let Some(from) = message.get("from") {
        let uid = match from["id"].as_i64() {
            Some(id) => id,
            None => {
                debug!("Telegram: dropping update {update_id} — from.id is not an integer");
                return None;
            }
        };
        let first_name = from["first_name"].as_str().unwrap_or("Unknown");
        let last_name = from["last_name"].as_str().unwrap_or("");
        let name = if last_name.is_empty() {
            first_name.to_string()
        } else {
            format!("{first_name} {last_name}")
        };
        Some((uid, name))
    } else if let Some(sender_chat) = message.get("sender_chat") {
        let uid = match sender_chat["id"].as_i64() {
            Some(id) => id,
            None => {
                debug!("Telegram: dropping update {update_id} — sender_chat.id is not an integer");
                return None;
            }
        };
        let title = sender_chat["title"].as_str().unwrap_or("Unknown Channel");
        Some((uid, title.to_string()))
    } else {
        debug!("Telegram: dropping update {update_id} — no from or sender_chat field");
        None
    }
}

pub(crate) fn telegram_update_metadata(
    message: &serde_json::Value,
    user_id: i64,
    thread_id: Option<&str>,
    is_group: bool,
    bot_username: Option<&str>,
) -> HashMap<String, serde_json::Value> {
    let mut metadata = HashMap::new();
    metadata.insert("sender_user_id".to_string(), serde_json::json!(user_id));
    if let Some(language) = message["from"]["language_code"].as_str() {
        metadata.insert("language".to_string(), serde_json::json!(language));
    }

    if let Some(tid) = thread_id {
        metadata.insert("telegram_thread_id".to_string(), serde_json::json!(tid));
        if let Some(topic_name) = message
            .get("forum_topic_created")
            .and_then(|topic| topic["name"].as_str())
        {
            metadata.insert(
                "telegram_topic_name".to_string(),
                serde_json::json!(topic_name),
            );
        }
    }

    if let Some(reply_msg) = message.get("reply_to_message") {
        if let Some(reply_id) = reply_msg["message_id"].as_i64() {
            metadata.insert(
                "reply_to_message_id".to_string(),
                serde_json::json!(reply_id),
            );
        }
    }

    if is_group {
        if let Some(bot_uname) = bot_username {
            if check_mention_entities(message, bot_uname) {
                metadata.insert("was_mentioned".to_string(), serde_json::json!(true));
            }
        }
    }

    metadata
}

/// Check whether the bot was @mentioned in a Telegram message.
///
/// Inspects both `entities` (for text messages) and `caption_entities` (for media
/// with captions) for entity type `"mention"` whose text matches `@bot_username`.
pub(crate) fn check_mention_entities(message: &serde_json::Value, bot_username: &str) -> bool {
    let bot_mention = format!("@{}", bot_username.to_lowercase());

    for entities_key in &["entities", "caption_entities"] {
        if let Some(entities) = message[entities_key].as_array() {
            let text = if *entities_key == "entities" {
                message["text"].as_str().unwrap_or("")
            } else {
                message["caption"].as_str().unwrap_or("")
            };

            for entity in entities {
                if entity["type"].as_str() != Some("mention") {
                    continue;
                }
                let offset = entity["offset"].as_i64().unwrap_or(0) as usize;
                let length = entity["length"].as_i64().unwrap_or(0) as usize;
                if offset + length <= text.len() {
                    let mention_text = &text[offset..offset + length];
                    if mention_text.to_lowercase() == bot_mention {
                        return true;
                    }
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_update_context_prefers_user_sender_and_thread() {
        let message = serde_json::json!({
            "message_id": 42,
            "message_thread_id": 7,
            "from": { "id": 123, "first_name": "Alice", "last_name": "Smith" },
            "chat": { "id": -100123, "type": "supergroup" },
            "date": 1700000000,
            "text": "hello"
        });

        let ctx = parse_telegram_update_context(&message, 99).expect("context");

        assert_eq!(ctx.user_id, 123);
        assert_eq!(ctx.display_name, "Alice Smith");
        assert_eq!(ctx.chat_id, -100123);
        assert!(ctx.is_group);
        assert_eq!(ctx.message_id, 42);
        assert_eq!(ctx.thread_id.as_deref(), Some("7"));
    }

    #[test]
    fn telegram_update_context_falls_back_to_sender_chat() {
        let message = serde_json::json!({
            "message_id": 43,
            "sender_chat": { "id": -200, "title": "News" },
            "chat": { "id": -100123, "type": "channel" },
            "date": 1700000000,
            "text": "broadcast"
        });

        let ctx = parse_telegram_update_context(&message, 99).expect("context");

        assert_eq!(ctx.user_id, -200);
        assert_eq!(ctx.display_name, "News");
        assert!(!ctx.is_group);
    }

    #[test]
    fn telegram_update_context_drops_missing_sender_or_chat() {
        let missing_sender = serde_json::json!({
            "message_id": 44,
            "chat": { "id": 1, "type": "private" },
            "date": 1700000000,
            "text": "no sender"
        });
        let missing_chat = serde_json::json!({
            "message_id": 45,
            "from": { "id": 123, "first_name": "Alice" },
            "date": 1700000000,
            "text": "no chat"
        });

        assert!(parse_telegram_update_context(&missing_sender, 99).is_none());
        assert!(parse_telegram_update_context(&missing_chat, 99).is_none());
    }

    #[test]
    fn telegram_update_metadata_preserves_thread_reply_topic_and_mentions() {
        let message = serde_json::json!({
            "text": "Salut @mybot",
            "from": { "language_code": "fr" },
            "message_thread_id": 7,
            "forum_topic_created": { "name": "Ops" },
            "reply_to_message": { "message_id": 55 },
            "entities": [{ "type": "mention", "offset": 6, "length": 6 }]
        });

        let metadata = telegram_update_metadata(&message, 123, Some("7"), true, Some("mybot"));

        assert_eq!(metadata["sender_user_id"], serde_json::json!(123));
        assert_eq!(metadata["language"], serde_json::json!("fr"));
        assert_eq!(metadata["telegram_thread_id"], serde_json::json!("7"));
        assert_eq!(metadata["telegram_topic_name"], serde_json::json!("Ops"));
        assert_eq!(metadata["reply_to_message_id"], serde_json::json!(55));
        assert_eq!(metadata["was_mentioned"], serde_json::json!(true));
    }

    #[test]
    fn telegram_update_message_accepts_message_and_edited_message() {
        let message_update = serde_json::json!({ "message": { "message_id": 1 } });
        let edited_update = serde_json::json!({ "edited_message": { "message_id": 2 } });

        assert_eq!(
            telegram_update_message(&message_update, 1)
                .and_then(|message| message["message_id"].as_i64()),
            Some(1)
        );
        assert_eq!(
            telegram_update_message(&edited_update, 2)
                .and_then(|message| message["message_id"].as_i64()),
            Some(2)
        );
        assert!(telegram_update_message(&serde_json::json!({}), 3).is_none());
    }
}
