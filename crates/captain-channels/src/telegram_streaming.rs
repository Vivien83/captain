//! Telegram streaming target and edit retry helpers.

use crate::stream_consumer::STREAM_CURSOR;
use crate::telegram::{RichDraftOutcome, TelegramAdapter};
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Disable progressive edits after this many consecutive flood-control
/// rejections on the same target.
pub const TELEGRAM_MAX_FLOOD_STRIKES: u32 = 3;

/// Conservative byte ceiling for a Bot API rich message. Telegram's native
/// limit is 32,768 UTF-8 characters; a byte ceiling at the same value can only
/// split earlier for non-ASCII text and therefore remains safe.
pub const TELEGRAM_MAX_MESSAGE_BYTES: usize = 32_768;

const TELEGRAM_DRAFT_ID_PREFIX: &str = "draft:";
static NEXT_TELEGRAM_DRAFT_ID: AtomicI64 = AtomicI64::new(1);

#[derive(Debug)]
struct TelegramProgressState {
    last_visible_activity: Instant,
    waiting_for_user: bool,
}

/// Shareable control for the private-chat thinking draft shown only when the
/// main stream has been silent. It deliberately has no persistent fallback:
/// progress is advisory and must never create transcript noise.
#[derive(Clone)]
pub struct TelegramProgressDraft {
    adapter: Arc<TelegramAdapter>,
    chat_id: i64,
    thread_id: Option<i64>,
    draft_id: i64,
    state: Arc<Mutex<TelegramProgressState>>,
}

impl TelegramProgressDraft {
    pub fn idle_for(&self) -> Duration {
        self.state
            .lock()
            .map(|state| state.last_visible_activity.elapsed())
            .unwrap_or_default()
    }

    pub fn is_waiting_for_user(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.waiting_for_user)
            .unwrap_or(true)
    }

    pub fn set_waiting_for_user(&self, waiting: bool) {
        if let Ok(mut state) = self.state.lock() {
            state.waiting_for_user = waiting;
            state.last_visible_activity = Instant::now();
        }
    }

    pub fn mark_visible_activity(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.last_visible_activity = Instant::now();
        }
    }

    /// Refresh the same ephemeral draft. `Ok(false)` means native Rich drafts
    /// are unavailable and the caller should stop the advisory loop.
    pub async fn refresh(&self, text: &str) -> Result<bool, String> {
        match self
            .adapter
            .api_send_rich_message_draft(self.chat_id, self.draft_id, text, self.thread_id)
            .await
            .map_err(|error| format!("progress draft HTTP error: {error}"))?
        {
            RichDraftOutcome::Sent => Ok(true),
            RichDraftOutcome::FallbackRequired => Ok(false),
        }
    }
}

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
    draft_fallback_message: Mutex<Option<(i64, i64)>>,
    progress_state: Arc<Mutex<TelegramProgressState>>,
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
            draft_fallback_message: Mutex::new(None),
            progress_state: Arc::new(Mutex::new(TelegramProgressState {
                last_visible_activity: Instant::now(),
                waiting_for_user: false,
            })),
        }
    }

    /// Build a distinct advisory draft for private chats. Groups cannot use
    /// `sendRichMessageDraft`, so they intentionally return `None`.
    pub fn progress_draft(&self) -> Option<TelegramProgressDraft> {
        (self.chat_id > 0).then(|| TelegramProgressDraft {
            adapter: Arc::clone(&self.adapter),
            chat_id: self.chat_id,
            thread_id: self.thread_id,
            draft_id: Self::next_draft_id(),
            state: Arc::clone(&self.progress_state),
        })
    }

    fn mark_visible_activity(&self) {
        if let Ok(mut state) = self.progress_state.lock() {
            state.last_visible_activity = Instant::now();
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

    fn take_armed_reply_to(&self) -> Option<i64> {
        if !self.armed_quote.swap(false, Ordering::Relaxed) {
            return None;
        }
        self.pending_reply_to
            .lock()
            .ok()
            .and_then(|mut guard| guard.take())
    }

    async fn send_persistent(&self, text: &str) -> Result<String, String> {
        let reply_to = self.take_armed_reply_to();
        match self
            .adapter
            .api_send_message_rich_full(self.chat_id, text, self.thread_id, None, reply_to)
            .await
        {
            Ok(Some(message_id)) => {
                self.mark_visible_activity();
                Ok(message_id.to_string())
            }
            Ok(None) => Err("send_new_message: Telegram returned no message_id".into()),
            Err(err) => Err(format!("send_new_message HTTP error: {err}")),
        }
    }

    fn parse_draft_message_id(message_id: &str) -> Option<i64> {
        message_id
            .strip_prefix(TELEGRAM_DRAFT_ID_PREFIX)
            .and_then(|id| id.parse().ok())
    }

    fn next_draft_id() -> i64 {
        NEXT_TELEGRAM_DRAFT_ID
            .fetch_add(1, Ordering::Relaxed)
            .max(1)
    }

    async fn edit_draft(&self, draft_id: i64, text: &str) -> Result<(), String> {
        let fallback_message_id = self
            .draft_fallback_message
            .lock()
            .ok()
            .and_then(|guard| guard.filter(|(id, _)| *id == draft_id).map(|(_, mid)| mid));
        if let Some(message_id) = fallback_message_id {
            return self.try_edit_with_retry(message_id, text).await;
        }

        if text.ends_with(STREAM_CURSOR) {
            match self
                .adapter
                .api_send_rich_message_draft(self.chat_id, draft_id, text, self.thread_id)
                .await
                .map_err(|error| format!("draft update HTTP error: {error}"))?
            {
                RichDraftOutcome::Sent => {
                    self.mark_visible_activity();
                    return Ok(());
                }
                RichDraftOutcome::FallbackRequired => {
                    let persistent_id =
                        self.send_persistent(text)
                            .await?
                            .parse::<i64>()
                            .map_err(|error| {
                                format!("draft fallback returned invalid message id: {error}")
                            })?;
                    if let Ok(mut guard) = self.draft_fallback_message.lock() {
                        *guard = Some((draft_id, persistent_id));
                    }
                    return Ok(());
                }
            }
        }

        self.send_persistent(text).await.map(|_| ())
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
                self.mark_visible_activity();
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
                        self.mark_visible_activity();
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
        if self.chat_id > 0 && text.ends_with(STREAM_CURSOR) {
            let draft_id = Self::next_draft_id();
            match self
                .adapter
                .api_send_rich_message_draft(self.chat_id, draft_id, text, self.thread_id)
                .await
                .map_err(|error| format!("send_new_message draft HTTP error: {error}"))?
            {
                RichDraftOutcome::Sent => {
                    self.mark_visible_activity();
                    return Ok(format!("{TELEGRAM_DRAFT_ID_PREFIX}{draft_id}"));
                }
                RichDraftOutcome::FallbackRequired => {}
            }
        }
        self.send_persistent(text).await
    }

    fn arm_final_reply_quote(&self) {
        self.armed_quote.store(true, Ordering::Relaxed);
    }

    async fn edit_message(&self, message_id: &str, text: &str) -> Result<(), String> {
        if let Some(draft_id) = Self::parse_draft_message_id(message_id) {
            return self.edit_draft(draft_id, text).await;
        }
        let mid: i64 = message_id
            .parse()
            .map_err(|err| format!("edit_message: invalid id '{message_id}': {err}"))?;
        self.try_edit_with_retry(mid, text).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_consumer::StreamingChannelAdapter;

    fn test_target() -> TelegramStreamTarget {
        let adapter = Arc::new(TelegramAdapter::new(
            "fake:token".to_string(),
            vec!["*".to_string()],
            Duration::from_secs(1),
            None,
        ));
        TelegramStreamTarget::new(adapter, 42, Some(7))
    }

    #[tokio::test]
    async fn telegram_private_stream_uses_ephemeral_draft_then_persists_final_rich_message() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendRichMessageDraft"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": true
            })))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendRichMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {"message_id": 77, "rich_message": {"blocks": []}}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let adapter = Arc::new(TelegramAdapter::new(
            "123:ABC".to_string(),
            vec!["*".to_string()],
            Duration::from_secs(1),
            Some(server.uri()),
        ));
        let target = TelegramStreamTarget::new(adapter, 42, Some(7)).with_reply_to(Some(99));
        target.arm_final_reply_quote();

        let draft_message_id = target
            .send_new_message(&format!("## Analyse{STREAM_CURSOR}"))
            .await
            .expect("draft open");
        assert!(draft_message_id.starts_with(TELEGRAM_DRAFT_ID_PREFIX));
        target
            .edit_message(
                &draft_message_id,
                &format!("## Analyse\n\nEn cours{STREAM_CURSOR}"),
            )
            .await
            .expect("draft update");
        target
            .edit_message(&draft_message_id, "## Analyse\n\nTerminé")
            .await
            .expect("persistent final");

        let requests = server.received_requests().await.expect("requests");
        let draft_ids: Vec<i64> = requests
            .iter()
            .filter(|request| request.url.path().ends_with("sendRichMessageDraft"))
            .map(|request| {
                serde_json::from_slice::<serde_json::Value>(&request.body).expect("draft json")
                    ["draft_id"]
                    .as_i64()
                    .expect("draft id")
            })
            .collect();
        assert_eq!(draft_ids.len(), 2);
        assert_eq!(draft_ids[0], draft_ids[1], "updates animate one draft");
        let final_request = requests
            .iter()
            .find(|request| request.url.path().ends_with("sendRichMessage"))
            .expect("final rich request");
        let final_body: serde_json::Value =
            serde_json::from_slice(&final_request.body).expect("final json");
        assert_eq!(
            final_body["rich_message"]["markdown"],
            "## Analyse\n\nTerminé"
        );
        assert_eq!(final_body["reply_parameters"]["message_id"], 99);
    }

    #[tokio::test]
    async fn telegram_group_stream_uses_persistent_rich_message_not_private_draft() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendRichMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {"message_id": 88, "rich_message": {"blocks": []}}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let adapter = Arc::new(TelegramAdapter::new(
            "123:ABC".to_string(),
            vec!["*".to_string()],
            Duration::from_secs(1),
            Some(server.uri()),
        ));
        let target = TelegramStreamTarget::new(adapter, -42, None);

        let message_id = target
            .send_new_message(&format!("Working{STREAM_CURSOR}"))
            .await
            .expect("persistent group send");
        assert_eq!(message_id, "88");
        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 1);
        assert!(requests[0].url.path().ends_with("sendRichMessage"));
    }

    #[tokio::test]
    async fn telegram_progress_refreshes_one_ephemeral_draft_without_persistent_noise() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendRichMessageDraft"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": true
            })))
            .expect(2)
            .mount(&server)
            .await;
        let adapter = Arc::new(TelegramAdapter::new(
            "123:ABC".to_string(),
            vec!["*".to_string()],
            Duration::from_secs(1),
            Some(server.uri()),
        ));
        let target = TelegramStreamTarget::new(adapter, 42, None);
        let progress = target.progress_draft().expect("private progress draft");

        assert!(progress
            .refresh("<tg-thinking>Captain travaille…</tg-thinking>")
            .await
            .unwrap());
        assert!(progress
            .refresh("<tg-thinking>Captain travaille encore…</tg-thinking>")
            .await
            .unwrap());

        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 2);
        let bodies = requests
            .iter()
            .map(|request| {
                serde_json::from_slice::<serde_json::Value>(&request.body).expect("draft body")
            })
            .collect::<Vec<_>>();
        assert_eq!(bodies[0]["draft_id"], bodies[1]["draft_id"]);
        assert!(bodies[1]["rich_message"]["markdown"]
            .as_str()
            .unwrap()
            .contains("encore"));
        assert!(requests
            .iter()
            .all(|request| request.url.path().ends_with("sendRichMessageDraft")));
    }

    #[tokio::test]
    async fn telegram_progress_stops_when_rich_drafts_are_unsupported() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendRichMessageDraft"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "ok": false,
                "description": "Not Found"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let adapter = Arc::new(TelegramAdapter::new(
            "123:ABC".to_string(),
            vec!["*".to_string()],
            Duration::from_secs(1),
            Some(server.uri()),
        ));
        let target = TelegramStreamTarget::new(adapter, 42, None);
        let progress = target.progress_draft().expect("private progress draft");

        assert!(!progress.refresh("working").await.unwrap());
        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 1, "must not fall back to a persistent send");
    }

    #[test]
    fn telegram_progress_is_private_and_tracks_waiting_state() {
        assert!(TelegramStreamTarget::new(
            Arc::new(TelegramAdapter::new(
                "fake:token".to_string(),
                vec!["*".to_string()],
                Duration::from_secs(1),
                None,
            )),
            -42,
            None,
        )
        .progress_draft()
        .is_none());

        let target = test_target();
        let progress = target.progress_draft().expect("private progress draft");
        assert!(!progress.is_waiting_for_user());
        progress.set_waiting_for_user(true);
        assert!(progress.is_waiting_for_user());
        progress.set_waiting_for_user(false);
        assert!(!progress.is_waiting_for_user());
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
