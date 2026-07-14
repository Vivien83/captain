//! Telegram streaming target and edit retry helpers.

use crate::telegram::TelegramAdapter;
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Disable progressive edits after this many consecutive flood-control
/// rejections on the same target.
pub const TELEGRAM_MAX_FLOOD_STRIKES: u32 = 3;

/// Telegram's Bot API hard limit per single message body.
pub const TELEGRAM_MAX_MESSAGE_BYTES: usize = 4096;

/// Outcome of a single `editMessageText` call.
///
/// `NotModified` is treated as a quiet success on purpose: when two
/// deltas arrive that render identically (e.g. trailing whitespace
/// flap, cursor toggle on/off), Telegram returns 400 with that exact
/// description. Forwarding it as an error would surface false negatives
/// to the consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOutcome {
    /// 2xx: Telegram accepted the edit.
    Ok,
    /// 400 with `description` containing "message is not modified".
    NotModified,
    /// 429: Telegram is asking us to back off for `secs`.
    RetryAfter(u64),
    /// Any other 4xx/5xx, kept verbose so the caller can log.
    PermanentFailure(String),
}

/// Pure classifier mapping (HTTP status, response body) to an [`EditOutcome`].
///
/// Body is parsed best-effort; malformed JSON falls through to
/// `PermanentFailure` rather than panicking.
pub fn classify_telegram_edit_outcome(status: u16, body: &str) -> EditOutcome {
    if (200..300).contains(&status) {
        return EditOutcome::Ok;
    }
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    if status == 429 {
        let secs = parsed["parameters"]["retry_after"].as_u64().unwrap_or(5);
        return EditOutcome::RetryAfter(secs);
    }
    let description = parsed["description"].as_str().unwrap_or("");
    if description.to_ascii_lowercase().contains("not modified") {
        return EditOutcome::NotModified;
    }
    EditOutcome::PermanentFailure(if description.is_empty() {
        format!("status={status}")
    } else {
        format!("status={status} description={description}")
    })
}

pub(crate) fn is_telegram_html_parse_failure(outcome: &EditOutcome) -> bool {
    let EditOutcome::PermanentFailure(description) = outcome else {
        return false;
    };
    let lower = description.to_ascii_lowercase();
    lower.contains("can't parse entities")
        || lower.contains("unsupported start tag")
        || lower.contains("unsupported end tag")
        || lower.contains("can't find end tag")
        || lower.contains("expected end tag")
}

/// One-chat (and optional forum thread) binding of a [`TelegramAdapter`]
/// that satisfies [`crate::stream_consumer::StreamingChannelAdapter`].
pub struct TelegramStreamTarget {
    adapter: Arc<TelegramAdapter>,
    chat_id: i64,
    thread_id: Option<i64>,
    flood_strikes: AtomicU32,
    pending_reply_to: Arc<Mutex<Option<i64>>>,
    armed_quote: AtomicBool,
}

impl TelegramStreamTarget {
    /// Build a stream target bound to `chat_id` and optional forum `thread_id`.
    pub fn new(adapter: Arc<TelegramAdapter>, chat_id: i64, thread_id: Option<i64>) -> Self {
        Self {
            adapter,
            chat_id,
            thread_id,
            flood_strikes: AtomicU32::new(0),
            pending_reply_to: Arc::new(Mutex::new(None)),
            armed_quote: AtomicBool::new(false),
        }
    }

    /// Shareable handle to the reply-to slot for active-stream interjections.
    pub fn reply_to_handle(&self) -> Arc<Mutex<Option<i64>>> {
        Arc::clone(&self.pending_reply_to)
    }

    /// Chainable builder that arms the next `send_new_message` to quote the user.
    pub fn with_reply_to(self, user_message_id: Option<i64>) -> Self {
        self.set_reply_to(user_message_id);
        self
    }

    /// Re-arm the reply target during an ongoing stream.
    pub fn set_reply_to(&self, user_message_id: Option<i64>) {
        if let Ok(mut guard) = self.pending_reply_to.lock() {
            *guard = user_message_id;
        }
    }

    /// Apply one edit attempt with at most one inline retry on short flood control.
    async fn try_edit_with_retry(&self, message_id: i64, text: &str) -> Result<(), String> {
        if self.flood_strikes.load(Ordering::Relaxed) >= TELEGRAM_MAX_FLOOD_STRIKES {
            return Err(format!(
                "edit_message: progressive edits disabled after {TELEGRAM_MAX_FLOOD_STRIKES} \
                 consecutive flood strikes — fall back to final-send"
            ));
        }
        let first = self
            .adapter
            .api_edit_message_strict(self.chat_id, message_id, text)
            .await
            .map_err(|err| format!("edit_message HTTP error: {err}"))?;
        match first {
            EditOutcome::Ok | EditOutcome::NotModified => {
                self.flood_strikes.store(0, Ordering::Relaxed);
                Ok(())
            }
            EditOutcome::RetryAfter(secs) if secs <= 5 => {
                tokio::time::sleep(Duration::from_secs(secs)).await;
                let second = self
                    .adapter
                    .api_edit_message_strict(self.chat_id, message_id, text)
                    .await
                    .map_err(|err| format!("edit_message HTTP error (retry): {err}"))?;
                match second {
                    EditOutcome::Ok | EditOutcome::NotModified => {
                        self.flood_strikes.store(0, Ordering::Relaxed);
                        Ok(())
                    }
                    EditOutcome::RetryAfter(s2) => {
                        self.flood_strikes.fetch_add(1, Ordering::Relaxed);
                        Err(format!(
                            "edit_message: persistent flood control (retry_after={s2}s)"
                        ))
                    }
                    EditOutcome::PermanentFailure(description) => Err(description),
                }
            }
            EditOutcome::RetryAfter(secs) => {
                self.flood_strikes.fetch_add(1, Ordering::Relaxed);
                Err(format!(
                    "edit_message: flood control retry_after={secs}s exceeds inline budget"
                ))
            }
            EditOutcome::PermanentFailure(description) => Err(description),
        }
    }
}

#[async_trait]
impl crate::stream_consumer::StreamingChannelAdapter for TelegramStreamTarget {
    async fn send_new_message(&self, text: &str) -> Result<String, String> {
        let reply_to = if self.armed_quote.swap(false, Ordering::Relaxed) {
            self.pending_reply_to
                .lock()
                .ok()
                .and_then(|mut guard| guard.take())
        } else {
            None
        };
        match self
            .adapter
            .api_send_message_rich_full(self.chat_id, text, self.thread_id, None, reply_to)
            .await
        {
            Ok(Some(message_id)) => Ok(message_id.to_string()),
            Ok(None) => Err("send_new_message: Telegram returned no message_id".into()),
            Err(err) => Err(format!("send_new_message HTTP error: {err}")),
        }
    }

    fn arm_final_reply_quote(&self) {
        self.armed_quote.store(true, Ordering::Relaxed);
    }

    async fn edit_message(&self, message_id: &str, text: &str) -> Result<(), String> {
        let mid: i64 = message_id
            .parse()
            .map_err(|err| format!("edit_message: invalid id '{message_id}': {err}"))?;
        self.try_edit_with_retry(mid, text).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_target() -> TelegramStreamTarget {
        let adapter = Arc::new(TelegramAdapter::new(
            "fake:token".to_string(),
            vec!["*".to_string()],
            Duration::from_secs(1),
            None,
        ));
        TelegramStreamTarget::new(adapter, 42, Some(7))
    }

    #[test]
    fn telegram_streaming_reply_to_handle_updates_slot() {
        let target = test_target().with_reply_to(Some(100));
        let handle = target.reply_to_handle();

        target.set_reply_to(Some(101));

        assert_eq!(*handle.lock().expect("reply handle"), Some(101));
    }

    #[test]
    fn telegram_streaming_html_parse_failures_are_retryable_as_plain_text() {
        let outcome = EditOutcome::PermanentFailure(
            "status=400 description=Bad Request: can't parse entities: Unsupported start tag"
                .to_string(),
        );

        assert!(is_telegram_html_parse_failure(&outcome));
        assert!(!is_telegram_html_parse_failure(&EditOutcome::NotModified));
    }

    #[test]
    fn telegram_streaming_classify_2xx_is_ok() {
        assert_eq!(classify_telegram_edit_outcome(200, "{}"), EditOutcome::Ok);
        assert_eq!(classify_telegram_edit_outcome(204, ""), EditOutcome::Ok);
    }

    #[test]
    fn telegram_streaming_classify_429_extracts_retry_after_seconds() {
        let body = r#"{"ok":false,"error_code":429,"description":"Too Many Requests: retry after 7","parameters":{"retry_after":7}}"#;
        assert_eq!(
            classify_telegram_edit_outcome(429, body),
            EditOutcome::RetryAfter(7)
        );
    }

    #[test]
    fn telegram_streaming_classify_429_without_parameters_falls_back_to_default() {
        let body = r#"{"ok":false,"error_code":429,"description":"Too Many Requests"}"#;
        assert_eq!(
            classify_telegram_edit_outcome(429, body),
            EditOutcome::RetryAfter(5)
        );
    }

    #[test]
    fn telegram_streaming_classify_400_message_not_modified_is_silent_success() {
        let body = r#"{"ok":false,"error_code":400,"description":"Bad Request: message is not modified: specified new message content and reply markup are exactly the same as a current content and reply markup of the message"}"#;
        assert_eq!(
            classify_telegram_edit_outcome(400, body),
            EditOutcome::NotModified
        );
    }

    #[test]
    fn telegram_streaming_classify_400_message_too_long_is_permanent_failure() {
        let body =
            r#"{"ok":false,"error_code":400,"description":"Bad Request: message is too long"}"#;
        match classify_telegram_edit_outcome(400, body) {
            EditOutcome::PermanentFailure(description) => {
                assert!(description.contains("too long"), "got {description}");
                assert!(description.contains("400"));
            }
            other => panic!("expected PermanentFailure, got {other:?}"),
        }
    }

    #[test]
    fn telegram_streaming_classify_500_with_no_body_is_permanent_failure() {
        match classify_telegram_edit_outcome(500, "") {
            EditOutcome::PermanentFailure(description) => {
                assert!(description.contains("500"), "got {description}")
            }
            other => panic!("expected PermanentFailure, got {other:?}"),
        }
    }

    #[test]
    fn telegram_streaming_classify_malformed_json_does_not_panic() {
        let body = "<html>502 Bad Gateway</html>";
        match classify_telegram_edit_outcome(502, body) {
            EditOutcome::PermanentFailure(_) => {}
            other => panic!("expected PermanentFailure on malformed body, got {other:?}"),
        }
    }
}
