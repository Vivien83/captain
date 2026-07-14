//! Operator-safe agent error responses for inbound channel turns.

use super::agent_error::sanitize_agent_error;
use super::send_response;
use crate::types::{ChannelAdapter, ChannelUser};
use captain_types::config::OutputFormat;

pub(super) async fn send_inbound_agent_error_response(
    adapter: &dyn ChannelAdapter,
    sender: &ChannelUser,
    raw_error: &str,
    thread_id: Option<&str>,
    output_format: OutputFormat,
) -> String {
    let err_msg = sanitize_agent_error(raw_error);
    if !adapter.suppress_error_responses() {
        send_response(adapter, sender, err_msg.clone(), thread_id, output_format).await;
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
        sent: Mutex<Vec<(String, Option<String>)>>,
        suppress_errors: bool,
    }

    #[async_trait]
    impl ChannelAdapter for RecordingAdapter {
        fn name(&self) -> &str {
            "recording"
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
            sent: Mutex::new(Vec::new()),
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
    }

    #[tokio::test]
    async fn error_response_returns_sanitized_message_when_suppressed() {
        let adapter = RecordingAdapter {
            sent: Mutex::new(Vec::new()),
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
    }
}
