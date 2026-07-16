//! Pure Telegram Rich Message payload and compatibility helpers.

/// Telegram Bot API 10.2 rich-message text limit.
pub(crate) const TELEGRAM_RICH_MAX_CHARS: usize = 32_768;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RichFallbackReason {
    Unsupported,
    InvalidContent,
}

pub(crate) fn telegram_send_rich_message_body(
    chat_id: i64,
    markdown: &str,
    thread_id: Option<i64>,
    reply_to_message_id: Option<i64>,
    reply_markup: Option<&serde_json::Value>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "chat_id": chat_id,
        "rich_message": { "markdown": markdown },
    });
    apply_thread(&mut body, thread_id);
    if let Some(reply_to_message_id) = reply_to_message_id {
        body["reply_parameters"] = serde_json::json!({
            "message_id": reply_to_message_id,
            "allow_sending_without_reply": true,
        });
    }
    if let Some(reply_markup) = reply_markup {
        body["reply_markup"] = reply_markup.clone();
    }
    body
}

pub(crate) fn telegram_send_rich_message_draft_body(
    chat_id: i64,
    draft_id: i64,
    markdown: &str,
    thread_id: Option<i64>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "chat_id": chat_id,
        "draft_id": draft_id,
        "rich_message": { "markdown": markdown },
    });
    apply_thread(&mut body, thread_id);
    body
}

pub(crate) fn telegram_edit_rich_message_body(
    chat_id: i64,
    message_id: i64,
    markdown: &str,
    reply_markup: Option<&serde_json::Value>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "rich_message": { "markdown": markdown },
    });
    if let Some(reply_markup) = reply_markup {
        body["reply_markup"] = reply_markup.clone();
    }
    body
}

fn apply_thread(body: &mut serde_json::Value, thread_id: Option<i64>) {
    if let Some(thread_id) = thread_id {
        body["message_thread_id"] = serde_json::json!(thread_id);
    }
}

/// Decide whether an explicit Bot API response can safely fall back to the
/// legacy `sendMessage`/HTML path. Transport errors never reach this helper:
/// retrying those could duplicate a message whose response was lost.
pub(crate) fn telegram_rich_fallback_reason(status: u16, body: &str) -> Option<RichFallbackReason> {
    let description = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|json| json["description"].as_str().map(str::to_owned))
        .unwrap_or_default()
        .to_ascii_lowercase();

    if matches!(status, 404 | 405 | 501)
        || description.contains("method not found")
        || description.contains("unknown method")
    {
        return Some(RichFallbackReason::Unsupported);
    }
    (status == 400).then_some(RichFallbackReason::InvalidContent)
}

/// Split rich Markdown without exceeding Telegram's character limit. Prefer
/// semantic whitespace boundaries; the adapter still has an HTML/plain
/// fallback if a very large fenced block must be cut in the middle.
pub(crate) fn split_telegram_rich_markdown(text: &str) -> Vec<String> {
    split_rich_markdown_at(text, TELEGRAM_RICH_MAX_CHARS)
}

fn split_rich_markdown_at(text: &str, max_chars: usize) -> Vec<String> {
    if text.is_empty() || max_chars == 0 {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;
    while remaining.chars().count() > max_chars {
        let hard_cut = remaining
            .char_indices()
            .nth(max_chars)
            .map(|(index, _)| index)
            .unwrap_or(remaining.len());
        let window = &remaining[..hard_cut];
        let (cut, separator_len) = preferred_split(window).unwrap_or((hard_cut, 0));
        if cut == 0 {
            chunks.push(remaining[..hard_cut].to_string());
            remaining = &remaining[hard_cut..];
        } else {
            chunks.push(remaining[..cut].to_string());
            remaining = &remaining[cut + separator_len..];
        }
    }
    if !remaining.is_empty() || chunks.is_empty() {
        chunks.push(remaining.to_string());
    }
    chunks
}

fn preferred_split(window: &str) -> Option<(usize, usize)> {
    if let Some(index) = window.rfind("\n\n") {
        return (index > 0).then_some((index, 2));
    }
    if let Some(index) = window.rfind('\n') {
        return (index > 0).then_some((index, 1));
    }
    if let Some(index) = window.rfind(' ') {
        return (index > 0).then_some((index, 1));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_send_payload_preserves_markdown_thread_reply_and_keyboard() {
        let keyboard = serde_json::json!({"inline_keyboard": [[{
            "text": "OK",
            "callback_data": "ok"
        }]]});
        let body = telegram_send_rich_message_body(
            42,
            "| A | B |\n|---|---|\n| 1 | 2 |",
            Some(7),
            Some(99),
            Some(&keyboard),
        );

        assert_eq!(body["chat_id"], serde_json::json!(42));
        assert!(body.get("text").is_none());
        assert_eq!(
            body["rich_message"]["markdown"],
            serde_json::json!("| A | B |\n|---|---|\n| 1 | 2 |")
        );
        assert_eq!(body["message_thread_id"], serde_json::json!(7));
        assert_eq!(
            body["reply_parameters"]["message_id"],
            serde_json::json!(99)
        );
        assert_eq!(body["reply_markup"], keyboard);
    }

    #[test]
    fn rich_draft_and_edit_payloads_use_native_rich_message_field() {
        let draft = telegram_send_rich_message_draft_body(42, 123, "partial", Some(7));
        assert_eq!(draft["draft_id"], serde_json::json!(123));
        assert_eq!(draft["rich_message"]["markdown"], "partial");
        assert_eq!(draft["message_thread_id"], serde_json::json!(7));

        let edit = telegram_edit_rich_message_body(42, 9, "final", None);
        assert_eq!(edit["message_id"], serde_json::json!(9));
        assert_eq!(edit["rich_message"]["markdown"], "final");
        assert!(edit.get("text").is_none());
    }

    #[test]
    fn fallback_classifier_separates_unsupported_content_and_server_failures() {
        assert_eq!(
            telegram_rich_fallback_reason(404, r#"{"description":"Not Found"}"#),
            Some(RichFallbackReason::Unsupported)
        );
        assert_eq!(
            telegram_rich_fallback_reason(
                400,
                r#"{"description":"Bad Request: can't parse rich message"}"#
            ),
            Some(RichFallbackReason::InvalidContent)
        );
        assert_eq!(telegram_rich_fallback_reason(401, "{}"), None);
        assert_eq!(telegram_rich_fallback_reason(500, "{}"), None);
    }

    #[test]
    fn rich_split_prefers_paragraphs_and_keeps_utf8_valid() {
        let chunks = split_rich_markdown_at("alpha beta\n\ngamma delta", 12);
        assert_eq!(chunks, vec!["alpha beta", "gamma delta"]);

        let utf8 = split_rich_markdown_at("ééééé", 3);
        assert_eq!(utf8, vec!["ééé", "éé"]);
        assert_eq!(utf8.concat(), "ééééé");
    }

    #[test]
    fn rich_split_never_exceeds_limit_and_preserves_unsplit_text() {
        let input = "one two three four five";
        let chunks = split_rich_markdown_at(input, 8);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 8));
        assert_eq!(chunks.join(" "), input);
        assert_eq!(split_rich_markdown_at("short", 8), vec!["short"]);
    }
}
