//! Pure Telegram Bot API payload builders.

pub(crate) fn telegram_send_message_body(
    chat_id: i64,
    text: &str,
    thread_id: Option<i64>,
    reply_to_message_id: Option<i64>,
    reply_markup: Option<&serde_json::Value>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "chat_id": chat_id,
        "text": text,
        "parse_mode": "HTML",
    });
    if let Some(tid) = thread_id {
        body["message_thread_id"] = serde_json::json!(tid);
    }
    if let Some(rid) = reply_to_message_id {
        body["reply_parameters"] = serde_json::json!({
            "message_id": rid,
            "allow_sending_without_reply": true,
        });
    }
    if let Some(markup) = reply_markup {
        body["reply_markup"] = markup.clone();
    }
    body
}

pub(crate) fn telegram_plain_text_fallback_body(
    html_body: &serde_json::Value,
    plain_text: String,
) -> serde_json::Value {
    let mut plain_body = html_body.clone();
    if let serde_json::Value::Object(map) = &mut plain_body {
        map.remove("parse_mode");
    }
    plain_body["text"] = serde_json::Value::String(plain_text);
    plain_body
}

pub(crate) fn telegram_photo_upload_filename(mime_type: &str) -> &'static str {
    match mime_type {
        "image/jpeg" => "photo.jpg",
        "image/gif" => "photo.gif",
        "image/webp" => "photo.webp",
        _ => "photo.png",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_api_payload_send_message_keeps_thread_reply_and_markup() {
        let markup = serde_json::json!({
            "inline_keyboard": [[{"text": "OK", "callback_data": "ok"}]]
        });
        let body = telegram_send_message_body(42, "<b>Hello</b>", Some(7), Some(99), Some(&markup));

        assert_eq!(body["chat_id"], serde_json::json!(42));
        assert_eq!(body["text"], serde_json::json!("<b>Hello</b>"));
        assert_eq!(body["parse_mode"], serde_json::json!("HTML"));
        assert_eq!(body["message_thread_id"], serde_json::json!(7));
        assert_eq!(
            body["reply_parameters"]["message_id"],
            serde_json::json!(99)
        );
        assert_eq!(
            body["reply_parameters"]["allow_sending_without_reply"],
            serde_json::json!(true)
        );
        assert_eq!(body["reply_markup"], markup);
    }

    #[test]
    fn telegram_api_payload_send_message_omits_optional_fields() {
        let body = telegram_send_message_body(42, "Hello", None, None, None);

        assert!(body.get("message_thread_id").is_none());
        assert!(body.get("reply_parameters").is_none());
        assert!(body.get("reply_markup").is_none());
    }

    #[test]
    fn telegram_api_payload_plain_fallback_removes_parse_mode_only() {
        let markup = serde_json::json!({
            "inline_keyboard": [[{"text": "OK", "callback_data": "ok"}]]
        });
        let html_body =
            telegram_send_message_body(42, "<bad>Hello</bad>", Some(7), Some(99), Some(&markup));

        let plain_body = telegram_plain_text_fallback_body(&html_body, "Hello".to_string());

        assert!(plain_body.get("parse_mode").is_none());
        assert_eq!(plain_body["text"], serde_json::json!("Hello"));
        assert_eq!(plain_body["message_thread_id"], serde_json::json!(7));
        assert_eq!(
            plain_body["reply_parameters"]["message_id"],
            serde_json::json!(99)
        );
        assert_eq!(plain_body["reply_markup"], markup);
    }

    #[test]
    fn telegram_api_payload_photo_upload_filename_matches_mime_type() {
        assert_eq!(telegram_photo_upload_filename("image/jpeg"), "photo.jpg");
        assert_eq!(telegram_photo_upload_filename("image/gif"), "photo.gif");
        assert_eq!(telegram_photo_upload_filename("image/webp"), "photo.webp");
        assert_eq!(telegram_photo_upload_filename("image/png"), "photo.png");
        assert_eq!(
            telegram_photo_upload_filename("application/octet-stream"),
            "photo.png"
        );
    }
}
