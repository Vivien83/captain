//! Response formatting and delivery for channel commands.

use crate::formatter;
use crate::types::{ChannelAdapter, ChannelContent, ChannelUser};
use captain_types::config::OutputFormat;
use std::collections::HashMap;
use tracing::error;

/// Send a response, applying output formatting and optional threading.
pub(super) async fn send_response(
    adapter: &dyn ChannelAdapter,
    user: &ChannelUser,
    text: String,
    thread_id: Option<&str>,
    output_format: OutputFormat,
) {
    let formatted = format_channel_text(adapter, &text, output_format);
    let content = ChannelContent::Text(formatted);

    let result = if adapter.name() == "telegram" && output_format == OutputFormat::PlainText {
        let mut metadata = HashMap::new();
        metadata.insert("telegram_plain_text".to_string(), serde_json::json!(true));
        if let Some(tid) = thread_id {
            metadata.insert("thread_id".to_string(), serde_json::json!(tid));
        }
        adapter
            .send_rich(user, content, &metadata)
            .await
            .map(|_| ())
    } else if let Some(tid) = thread_id {
        adapter.send_in_thread(user, content, tid).await
    } else {
        adapter.send(user, content).await
    };

    if let Err(error) = result {
        error!("Failed to send response: {error}");
    }
}

/// Send a response without markdown/channel formatting.
///
/// Used for exact file/content dumps such as `/config`; Telegram still applies
/// its own HTML sanitization and chunking at the adapter boundary.
async fn send_raw_response(
    adapter: &dyn ChannelAdapter,
    user: &ChannelUser,
    text: String,
    thread_id: Option<&str>,
) {
    let content = ChannelContent::Text(text);
    let result = if adapter.name() == "telegram" {
        let mut metadata = HashMap::new();
        metadata.insert("telegram_plain_text".to_string(), serde_json::json!(true));
        if let Some(tid) = thread_id {
            metadata.insert("thread_id".to_string(), serde_json::json!(tid));
        }
        adapter
            .send_rich(user, content, &metadata)
            .await
            .map(|_| ())
    } else if let Some(tid) = thread_id {
        adapter.send_in_thread(user, content, tid).await
    } else {
        adapter.send(user, content).await
    };

    if let Err(error) = result {
        error!("Failed to send raw response: {error}");
    }
}

#[derive(Debug, Clone)]
pub(super) struct CommandResponse {
    text: String,
    reply_markup: Option<serde_json::Value>,
    raw: bool,
}

impl CommandResponse {
    pub(super) fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            reply_markup: None,
            raw: false,
        }
    }

    pub(super) fn raw(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            reply_markup: None,
            raw: true,
        }
    }

    pub(super) fn with_reply_markup(
        text: impl Into<String>,
        reply_markup: serde_json::Value,
    ) -> Self {
        Self {
            text: text.into(),
            reply_markup: Some(reply_markup),
            raw: false,
        }
    }
}

impl std::ops::Deref for CommandResponse {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.text
    }
}

pub(super) async fn send_command_response(
    adapter: &dyn ChannelAdapter,
    user: &ChannelUser,
    response: CommandResponse,
    thread_id: Option<&str>,
    output_format: OutputFormat,
) {
    let CommandResponse {
        text,
        reply_markup,
        raw,
    } = response;
    if raw && reply_markup.is_none() {
        send_raw_response(adapter, user, text, thread_id).await;
        return;
    }
    let Some(reply_markup) = reply_markup else {
        send_response(adapter, user, text, thread_id, output_format).await;
        return;
    };

    let formatted = format_channel_text(adapter, &text, output_format);
    let mut metadata = HashMap::new();
    metadata.insert("reply_markup".to_string(), reply_markup);
    if adapter.name() == "telegram" && output_format == OutputFormat::PlainText {
        metadata.insert("telegram_plain_text".to_string(), serde_json::json!(true));
    }
    if let Some(tid) = thread_id {
        metadata.insert("thread_id".to_string(), serde_json::json!(tid));
    }

    if let Err(error) = adapter
        .send_rich(user, ChannelContent::Text(formatted), &metadata)
        .await
    {
        error!("Failed to send command response: {error}");
    }
}

fn format_channel_text(
    adapter: &dyn ChannelAdapter,
    text: &str,
    output_format: OutputFormat,
) -> String {
    if adapter.name() == "wecom" {
        formatter::format_for_wecom(text, output_format)
    } else if adapter.name() == "telegram" && output_format == OutputFormat::TelegramHtml {
        // `TelegramHtml` remains the compatible config name. Bot API 10.2
        // now receives the original Markdown and renders it natively; the
        // adapter converts to legacy HTML only when the endpoint rejects it.
        text.to_string()
    } else {
        formatter::format_for_channel(text, output_format)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telegram::TelegramAdapter;
    use serde_json::json;
    use std::time::Duration;

    fn telegram_adapter(api_url: String) -> TelegramAdapter {
        TelegramAdapter::new(
            "123:ABC".to_string(),
            vec!["*".to_string()],
            Duration::from_secs(1),
            Some(api_url),
        )
    }

    #[test]
    fn command_response_text_is_formatted_response() {
        let response = CommandResponse::text("hello");

        assert_eq!(&*response, "hello");
        assert!(!response.raw);
        assert!(response.reply_markup.is_none());
    }

    #[test]
    fn command_response_raw_preserves_exact_text() {
        let response = CommandResponse::raw("config = true");

        assert_eq!(&*response, "config = true");
        assert!(response.raw);
        assert!(response.reply_markup.is_none());
    }

    #[test]
    fn command_response_with_markup_is_not_raw() {
        let markup = json!({"inline_keyboard": []});
        let response = CommandResponse::with_reply_markup("choose", markup.clone());

        assert_eq!(&*response, "choose");
        assert!(!response.raw);
        assert_eq!(response.reply_markup, Some(markup));
    }

    #[test]
    fn telegram_default_format_preserves_markdown_for_native_rich_transport() {
        let adapter = telegram_adapter("http://127.0.0.1:1".to_string());
        let input = "## Report\n\n| Metric | Value |\n|---|---:|\n| OK | **1** |";
        assert_eq!(
            format_channel_text(&adapter, input, OutputFormat::TelegramHtml),
            input
        );
    }

    #[tokio::test]
    async fn telegram_raw_and_plain_responses_use_unparsed_send_message() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "result": {"message_id": 42}
            })))
            .expect(2)
            .mount(&server)
            .await;
        let adapter = telegram_adapter(server.uri());
        let user = ChannelUser {
            platform_id: "42".to_string(),
            display_name: "Test".to_string(),
            captain_user: None,
        };

        send_command_response(
            &adapter,
            &user,
            CommandResponse::raw("**literal raw**"),
            Some("7"),
            OutputFormat::TelegramHtml,
        )
        .await;
        send_response(
            &adapter,
            &user,
            "**plain request**".to_string(),
            None,
            OutputFormat::PlainText,
        )
        .await;

        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 2);
        for request in requests {
            let body: serde_json::Value =
                serde_json::from_slice(&request.body).expect("plain JSON body");
            assert!(body.get("parse_mode").is_none());
            assert!(body.get("rich_message").is_none());
        }
        let raw_body: serde_json::Value =
            serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        assert_eq!(raw_body["text"], "**literal raw**");
        assert_eq!(raw_body["message_thread_id"], 7);
    }
}
