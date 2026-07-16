//! Operator-safe agent error responses for inbound channel turns.

use super::agent_error::sanitize_agent_error;
use super::send_response;
use crate::render_telegram_channel_error;
use crate::types::{ChannelAdapter, ChannelContent, ChannelUser};
use captain_types::config::OutputFormat;
use std::collections::HashMap;
use tracing::error;

pub(super) async fn send_inbound_agent_error_response(
    adapter: &dyn ChannelAdapter,
    sender: &ChannelUser,
    raw_error: &str,
    thread_id: Option<&str>,
    output_format: OutputFormat,
) -> String {
    let err_msg = sanitize_agent_error(raw_error);
    if !adapter.suppress_error_responses() {
        if adapter.name() == "telegram" {
            let mut metadata = HashMap::new();
            if let Some(thread_id) = thread_id {
                metadata.insert("thread_id".to_string(), serde_json::json!(thread_id));
            }
            if let Err(delivery_error) = adapter
                .send_rich(
                    sender,
                    ChannelContent::Text(render_telegram_channel_error(&err_msg)),
                    &metadata,
                )
                .await
            {
                error!(%delivery_error, "failed to send Telegram Rich error response");
            }
        } else {
            send_response(adapter, sender, err_msg.clone(), thread_id, output_format).await;
        }
    }
    err_msg
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelContent, ChannelMessage, ChannelType};
    use async_trait::async_trait;
    use futures::{stream, Stream};
    use std::pin::Pin;
    use std::sync::Mutex;

    struct RecordingAdapter {
        name: &'static str,
        sent: Mutex<Vec<(String, Option<String>)>>,
        rich: Mutex<Vec<(String, Option<String>)>>,
        suppress_errors: bool,
    }

    #[async_trait]
    impl ChannelAdapter for RecordingAdapter {
        fn name(&self) -> &str {
            self.name
        }

        fn channel_type(&self) -> ChannelType {
            ChannelType::Telegram
        }

        async fn start(
            &self,
        ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, Box<dyn std::error::Error>>
        {
            Ok(Box::pin(stream::empty()))
        }

        async fn send(
            &self,
            _user: &ChannelUser,
            content: ChannelContent,
        ) -> Result<(), Box<dyn std::error::Error>> {
            if let ChannelContent::Text(text) = content {
                self.sent.lock().unwrap().push((text, None));
            }
            Ok(())
        }

        async fn send_in_thread(
            &self,
            _user: &ChannelUser,
            content: ChannelContent,
            thread_id: &str,
        ) -> Result<(), Box<dyn std::error::Error>> {
            if let ChannelContent::Text(text) = content {
                self.sent
                    .lock()
                    .unwrap()
                    .push((text, Some(thread_id.to_string())));
            }
            Ok(())
        }

        async fn send_rich(
            &self,
            _user: &ChannelUser,
            content: ChannelContent,
            metadata: &std::collections::HashMap<String, serde_json::Value>,
        ) -> Result<Option<String>, Box<dyn std::error::Error>> {
            if let ChannelContent::Text(text) = content {
                let thread_id = metadata
                    .get("thread_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                self.rich.lock().unwrap().push((text, thread_id));
            }
            Ok(Some("17".to_string()))
        }

        async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }

        fn suppress_error_responses(&self) -> bool {
            self.suppress_errors
        }
    }

    fn sender() -> ChannelUser {
        ChannelUser {
            platform_id: "1001".to_string(),
            display_name: "Ada".to_string(),
            captain_user: None,
        }
    }

    #[tokio::test]
    async fn error_response_sends_sanitized_message_when_allowed() {
        let adapter = RecordingAdapter {
            name: "recording",
            sent: Mutex::new(Vec::new()),
            rich: Mutex::new(Vec::new()),
            suppress_errors: false,
        };

        let err_msg = send_inbound_agent_error_response(
            &adapter,
            &sender(),
            "invalid x-goog-api-key",
            Some("topic-7"),
            OutputFormat::PlainText,
        )
        .await;

        assert_eq!(err_msg, "Service temporarily unavailable.");
        assert_eq!(
            adapter.sent.lock().unwrap().as_slice(),
            &[(
                "Service temporarily unavailable.".to_string(),
                Some("topic-7".to_string())
            )]
        );
        assert!(adapter.rich.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn telegram_error_response_is_sanitized_rich_and_threaded() {
        let adapter = RecordingAdapter {
            name: "telegram",
            sent: Mutex::new(Vec::new()),
            rich: Mutex::new(Vec::new()),
            suppress_errors: false,
        };

        let err_msg = send_inbound_agent_error_response(
            &adapter,
            &sender(),
            "invalid x-goog-api-key </blockquote><script>secret</script>",
            Some("topic-7"),
            OutputFormat::PlainText,
        )
        .await;

        assert_eq!(err_msg, "Service temporarily unavailable.");
        assert!(adapter.sent.lock().unwrap().is_empty());
        let rich = adapter.rich.lock().unwrap();
        assert_eq!(rich.len(), 1);
        assert!(rich[0].0.starts_with("### ⚠️ Captain"));
        assert!(rich[0].0.contains("Service temporarily unavailable."));
        assert!(!rich[0].0.contains("x-goog-api-key"));
        assert_eq!(rich[0].1.as_deref(), Some("topic-7"));
    }

    #[tokio::test]
    async fn error_response_returns_sanitized_message_when_suppressed() {
        let adapter = RecordingAdapter {
            name: "telegram",
            sent: Mutex::new(Vec::new()),
            rich: Mutex::new(Vec::new()),
            suppress_errors: true,
        };

        let err_msg = send_inbound_agent_error_response(
            &adapter,
            &sender(),
            "request timed out while waiting",
            None,
            OutputFormat::PlainText,
        )
        .await;

        assert_eq!(err_msg, "Request timed out, please try again.");
        assert!(adapter.sent.lock().unwrap().is_empty());
        assert!(adapter.rich.lock().unwrap().is_empty());
    }
}
