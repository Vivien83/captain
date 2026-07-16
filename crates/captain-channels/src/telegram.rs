//! Telegram Bot API adapter for the Captain channel bridge.
//!
//! Uses long-polling via `getUpdates` with exponential backoff on failures.
//! No external Telegram crate — just `reqwest` for full control over error handling.

use crate::telegram_api_payload::{
    telegram_photo_upload_filename, telegram_plain_text_fallback_body, telegram_send_message_body,
};
use crate::telegram_api_response::{
    ensure_success, telegram_message_id_from_response, telegram_retry_after_seconds,
};
use crate::telegram_callbacks::callback_command_message;
pub use crate::telegram_callbacks::{
    build_approval_keyboard, build_ask_user_keyboard, build_learning_approval_keyboard,
    build_model_switch_keyboard, build_model_switch_keyboard_with_recommendation,
    build_project_ask_keyboard, build_skill_proposal_keyboard, build_skill_refinement_keyboard,
    parse_approval_callback, parse_ask_user_callback, parse_learning_callback,
    parse_model_switch_callback, parse_project_ask_callback, parse_skill_proposal_callback,
    parse_skill_refinement_callback,
};
use crate::telegram_html::{sanitize_telegram_html, telegram_html_to_plain_text};
use crate::telegram_reply_context::apply_telegram_reply_context;
use crate::telegram_rich::{
    split_telegram_rich_markdown, telegram_edit_rich_message_body, telegram_rich_fallback_reason,
    telegram_send_rich_message_body, telegram_send_rich_message_draft_body, RichFallbackReason,
};
use crate::telegram_streaming::is_telegram_html_parse_failure;
pub use crate::telegram_streaming::{
    classify_telegram_edit_outcome, EditOutcome, TelegramStreamTarget, TELEGRAM_MAX_FLOOD_STRIKES,
    TELEGRAM_MAX_MESSAGE_BYTES,
};
use crate::telegram_update_content::parse_telegram_update_content;
#[cfg(test)]
use crate::telegram_update_context::check_mention_entities;
use crate::telegram_update_context::{
    parse_telegram_update_context, telegram_update_message, telegram_update_metadata,
};
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
    LifecycleReaction,
};
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};
use zeroize::Zeroizing;

/// Maximum backoff duration on API failures.
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// Initial backoff duration on API failures.
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
/// Telegram long-polling timeout (seconds) — sent as the `timeout` parameter to getUpdates.
const LONG_POLL_TIMEOUT: u64 = 30;

/// Default Telegram Bot API base URL.
const DEFAULT_API_URL: &str = "https://api.telegram.org";
const RICH_CAPABILITY_UNKNOWN: u8 = 0;
const RICH_CAPABILITY_AVAILABLE: u8 = 1;
const RICH_CAPABILITY_UNAVAILABLE: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RichDraftOutcome {
    Sent,
    FallbackRequired,
}

pub struct TelegramAdapter {
    /// SECURITY: Bot token is zeroized on drop to prevent memory disclosure.
    token: Zeroizing<String>,
    client: reqwest::Client,
    allowed_users: Vec<String>,
    poll_interval: Duration,
    /// Base URL for Telegram Bot API (supports proxies/mirrors).
    api_base_url: String,
    /// Bot username (without @), populated from `getMe` during `start()`.
    /// Used for @mention detection in group messages.
    bot_username: Arc<tokio::sync::RwLock<Option<String>>>,
    /// Runtime compatibility cache for custom/older Bot API endpoints.
    rich_capability: AtomicU8,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl TelegramAdapter {
    /// Create a new Telegram adapter.
    ///
    /// `token` is the raw bot token (read from env by the caller).
    /// `allowed_users` is the list of Telegram user IDs allowed to interact
    /// (empty = deny all, ["*"] = allow all).
    /// `api_url` overrides the Telegram Bot API base URL (for proxies/mirrors).
    pub fn new(
        token: String,
        allowed_users: Vec<String>,
        poll_interval: Duration,
        api_url: Option<String>,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let api_base_url = api_url
            .unwrap_or_else(|| DEFAULT_API_URL.to_string())
            .trim_end_matches('/')
            .to_string();
        Self {
            token: Zeroizing::new(token),
            client: reqwest::Client::new(),
            allowed_users,
            poll_interval,
            api_base_url,
            bot_username: Arc::new(tokio::sync::RwLock::new(None)),
            rich_capability: AtomicU8::new(RICH_CAPABILITY_UNKNOWN),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }

    /// Validate the bot token by calling `getMe`.
    pub async fn validate_token(&self) -> Result<String, Box<dyn std::error::Error>> {
        let url = format!("{}/bot{}/getMe", self.api_base_url, self.token.as_str());
        let resp: serde_json::Value = self.client.get(&url).send().await?.json().await?;

        if resp["ok"].as_bool() != Some(true) {
            let desc = resp["description"].as_str().unwrap_or("unknown error");
            let hint = if desc.to_lowercase().contains("unauthorized") {
                " (Check that the bot token is correct. Get it from @BotFather on Telegram.)"
            } else if desc.to_lowercase().contains("not found") {
                " (The bot token format may be invalid. Expected format: 123456789:ABCdefGHI...)"
            } else {
                ""
            };
            return Err(format!("Telegram getMe failed: {desc}{hint}").into());
        }

        let bot_name = resp["result"]["username"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        Ok(bot_name)
    }

    /// Call `sendMessage` on the Telegram API.
    /// Returns the message_id of the last sent message (for editMessage later).
    async fn api_send_message(
        &self,
        chat_id: i64,
        text: &str,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.api_send_message_rich(chat_id, text, thread_id, None)
            .await
            .map(|_| ())
    }

    /// Send a rich message with optional inline keyboard buttons.
    /// Returns the message_id of the last sent message.
    async fn api_send_message_rich(
        &self,
        chat_id: i64,
        text: &str,
        thread_id: Option<i64>,
        reply_markup: Option<&serde_json::Value>,
    ) -> Result<Option<i64>, Box<dyn std::error::Error>> {
        self.api_send_message_rich_full(chat_id, text, thread_id, reply_markup, None)
            .await
    }

    /// HS.7 — Same as `api_send_message_rich` but accepts an optional
    /// `reply_to_message_id` so callers can quote the user's original
    /// message for traceability. Only the FIRST chunk of a
    /// long reply carries the reply marker; threading the rest would
    /// nest visually in Telegram.
    pub(crate) async fn api_send_message_rich_full(
        &self,
        chat_id: i64,
        text: &str,
        thread_id: Option<i64>,
        reply_markup: Option<&serde_json::Value>,
        reply_to_message_id: Option<i64>,
    ) -> Result<Option<i64>, Box<dyn std::error::Error>> {
        if self.rich_messages_unavailable() {
            return self
                .api_send_legacy_markdown_full(
                    chat_id,
                    text,
                    thread_id,
                    reply_markup,
                    reply_to_message_id,
                )
                .await;
        }

        let chunks = split_telegram_rich_markdown(text);
        let mut last_msg_id: Option<i64> = None;
        for (i, chunk) in chunks.iter().enumerate() {
            let reply_to = if i == 0 { reply_to_message_id } else { None };
            let markup = if i == chunks.len() - 1 {
                reply_markup
            } else {
                None
            };
            let body = telegram_send_rich_message_body(chat_id, chunk, thread_id, reply_to, markup);
            let (status, body_text) = self
                .post_json_with_rate_limit("sendRichMessage", &body)
                .await?;
            if (200..300).contains(&status) {
                self.mark_rich_available();
                last_msg_id = telegram_message_id_from_response(&body_text);
                continue;
            }

            let Some(reason) = telegram_rich_fallback_reason(status, &body_text) else {
                return Err(
                    format!("Telegram sendRichMessage failed ({status}): {body_text}").into(),
                );
            };
            if reason == RichFallbackReason::Unsupported {
                self.mark_rich_unavailable();
            }
            warn!(
                ?reason,
                "Telegram Rich Message rejected; using legacy HTML fallback"
            );
            last_msg_id = self
                .api_send_legacy_markdown_full(chat_id, chunk, thread_id, markup, reply_to)
                .await?;
        }
        Ok(last_msg_id)
    }

    pub(crate) async fn api_send_rich_message_draft(
        &self,
        chat_id: i64,
        draft_id: i64,
        text: &str,
        thread_id: Option<i64>,
    ) -> Result<RichDraftOutcome, Box<dyn std::error::Error>> {
        if self.rich_messages_unavailable() {
            return Ok(RichDraftOutcome::FallbackRequired);
        }
        let body = telegram_send_rich_message_draft_body(chat_id, draft_id, text, thread_id);
        let (status, body_text) = self
            .post_json_with_rate_limit("sendRichMessageDraft", &body)
            .await?;
        if (200..300).contains(&status) {
            self.mark_rich_available();
            return Ok(RichDraftOutcome::Sent);
        }
        let Some(reason) = telegram_rich_fallback_reason(status, &body_text) else {
            return Err(
                format!("Telegram sendRichMessageDraft failed ({status}): {body_text}").into(),
            );
        };
        if reason == RichFallbackReason::Unsupported {
            self.mark_rich_unavailable();
        }
        Ok(RichDraftOutcome::FallbackRequired)
    }

    async fn api_send_legacy_markdown_full(
        &self,
        chat_id: i64,
        markdown: &str,
        thread_id: Option<i64>,
        reply_markup: Option<&serde_json::Value>,
        reply_to_message_id: Option<i64>,
    ) -> Result<Option<i64>, Box<dyn std::error::Error>> {
        let html = crate::formatter::format_for_channel(
            markdown,
            captain_types::config::OutputFormat::TelegramHtml,
        );
        self.api_send_legacy_html_full(chat_id, &html, thread_id, reply_markup, reply_to_message_id)
            .await
    }

    async fn api_send_legacy_html_full(
        &self,
        chat_id: i64,
        html: &str,
        thread_id: Option<i64>,
        reply_markup: Option<&serde_json::Value>,
        reply_to_message_id: Option<i64>,
    ) -> Result<Option<i64>, Box<dyn std::error::Error>> {
        let sanitized = sanitize_telegram_html(html);
        let chunks = split_message(&sanitized, 4096);
        let mut last_msg_id = None;
        for (index, chunk) in chunks.iter().enumerate() {
            let reply_to = (index == 0).then_some(reply_to_message_id).flatten();
            let markup = (index == chunks.len() - 1)
                .then_some(reply_markup)
                .flatten();
            let body = telegram_send_message_body(chat_id, chunk, thread_id, reply_to, markup);
            let (status, body_text) = self.post_json_with_rate_limit("sendMessage", &body).await?;
            if (200..300).contains(&status) {
                last_msg_id = telegram_message_id_from_response(&body_text);
                continue;
            }
            if telegram_retry_after_seconds(status, &body_text).is_some() {
                return Err(format!("Telegram sendMessage failed ({status}): {body_text}").into());
            }

            let plain_body =
                telegram_plain_text_fallback_body(&body, telegram_html_to_plain_text(chunk));
            let (plain_status, plain_response) = self
                .post_json_with_rate_limit("sendMessage", &plain_body)
                .await?;
            if (200..300).contains(&plain_status) {
                last_msg_id = telegram_message_id_from_response(&plain_response);
            } else {
                return Err(format!(
                    "Telegram sendMessage failed ({status}): {body_text}; plain fallback failed ({plain_status}): {plain_response}"
                )
                .into());
            }
        }
        Ok(last_msg_id)
    }

    async fn api_send_plain_text_full(
        &self,
        chat_id: i64,
        text: &str,
        thread_id: Option<i64>,
        reply_markup: Option<&serde_json::Value>,
    ) -> Result<Option<i64>, Box<dyn std::error::Error>> {
        let chunks = split_message(text, 4096);
        let mut last_msg_id = None;
        for (index, chunk) in chunks.iter().enumerate() {
            let markup = (index == chunks.len() - 1)
                .then_some(reply_markup)
                .flatten();
            let html_body = telegram_send_message_body(chat_id, chunk, thread_id, None, markup);
            let body = telegram_plain_text_fallback_body(&html_body, (*chunk).to_string());
            let (status, response) = self.post_json_with_rate_limit("sendMessage", &body).await?;
            if !(200..300).contains(&status) {
                return Err(format!("Telegram sendMessage failed ({status}): {response}").into());
            }
            last_msg_id = telegram_message_id_from_response(&response);
        }
        Ok(last_msg_id)
    }

    async fn post_json_with_rate_limit(
        &self,
        endpoint: &str,
        body: &serde_json::Value,
    ) -> Result<(u16, String), Box<dyn std::error::Error>> {
        let url = format!(
            "{}/bot{}/{}",
            self.api_base_url,
            self.token.as_str(),
            endpoint
        );
        let mut attempts = 0;
        loop {
            let response = self.client.post(&url).json(body).send().await?;
            let status = response.status().as_u16();
            let response_body = response.text().await.unwrap_or_default();
            let Some(retry_after) = telegram_retry_after_seconds(status, &response_body) else {
                return Ok((status, response_body));
            };
            if attempts >= 2 {
                return Ok((status, response_body));
            }
            warn!(
                endpoint,
                retry_after, "Telegram rate limited request; retrying"
            );
            tokio::time::sleep(Duration::from_secs(retry_after.saturating_add(1))).await;
            attempts += 1;
        }
    }

    fn rich_messages_unavailable(&self) -> bool {
        self.rich_capability.load(Ordering::Relaxed) == RICH_CAPABILITY_UNAVAILABLE
    }

    fn mark_rich_available(&self) {
        self.rich_capability
            .store(RICH_CAPABILITY_AVAILABLE, Ordering::Relaxed);
    }

    fn mark_rich_unavailable(&self) {
        self.rich_capability
            .store(RICH_CAPABILITY_UNAVAILABLE, Ordering::Relaxed);
    }

    /// Edit a previously sent message's text and/or reply markup.
    async fn api_edit_message(
        &self,
        chat_id: i64,
        message_id: i64,
        text: Option<&str>,
        reply_markup: Option<&serde_json::Value>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(text) = text {
            let outcome = self
                .api_edit_rich_with_markup(chat_id, message_id, text, reply_markup)
                .await?;
            if !matches!(outcome, EditOutcome::Ok | EditOutcome::NotModified) {
                warn!(?outcome, "Telegram editMessageText failed");
            }
            return Ok(());
        }

        let mut body = serde_json::json!({ "chat_id": chat_id, "message_id": message_id });
        if let Some(markup) = reply_markup {
            body["reply_markup"] = markup.clone();
        }
        let (status, response) = self
            .post_json_with_rate_limit("editMessageReplyMarkup", &body)
            .await?;
        if !(200..300).contains(&status) {
            warn!(status, response, "Telegram editMessageReplyMarkup failed");
        }
        Ok(())
    }

    async fn api_edit_rich_with_markup(
        &self,
        chat_id: i64,
        message_id: i64,
        text: &str,
        reply_markup: Option<&serde_json::Value>,
    ) -> Result<EditOutcome, Box<dyn std::error::Error>> {
        if !self.rich_messages_unavailable() {
            let body = telegram_edit_rich_message_body(chat_id, message_id, text, reply_markup);
            let (status, response) = self
                .post_json_with_rate_limit("editMessageText", &body)
                .await?;
            let outcome = classify_telegram_edit_outcome(status, &response);
            if matches!(outcome, EditOutcome::Ok | EditOutcome::NotModified) {
                self.mark_rich_available();
                return Ok(outcome);
            }
            if let Some(reason) = telegram_rich_fallback_reason(status, &response) {
                if reason == RichFallbackReason::Unsupported {
                    self.mark_rich_unavailable();
                }
                return self
                    .api_edit_legacy_markdown_strict(chat_id, message_id, text, reply_markup)
                    .await;
            }
            return Ok(outcome);
        }
        self.api_edit_legacy_markdown_strict(chat_id, message_id, text, reply_markup)
            .await
    }

    async fn api_edit_legacy_markdown_strict(
        &self,
        chat_id: i64,
        message_id: i64,
        markdown: &str,
        reply_markup: Option<&serde_json::Value>,
    ) -> Result<EditOutcome, Box<dyn std::error::Error>> {
        let html = crate::formatter::format_for_channel(
            markdown,
            captain_types::config::OutputFormat::TelegramHtml,
        );
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": sanitize_telegram_html(&html),
            "parse_mode": "HTML",
        });
        if let Some(reply_markup) = reply_markup {
            body["reply_markup"] = reply_markup.clone();
        }
        let (status, response) = self
            .post_json_with_rate_limit("editMessageText", &body)
            .await?;
        let outcome = classify_telegram_edit_outcome(status, &response);
        if !is_telegram_html_parse_failure(&outcome) {
            return Ok(outcome);
        }

        if let serde_json::Value::Object(map) = &mut body {
            map.remove("parse_mode");
        }
        body["text"] = serde_json::Value::String(telegram_html_to_plain_text(&html));
        let (plain_status, plain_response) = self
            .post_json_with_rate_limit("editMessageText", &body)
            .await?;
        let plain_outcome = classify_telegram_edit_outcome(plain_status, &plain_response);
        if let EditOutcome::PermanentFailure(description) = plain_outcome {
            return Ok(EditOutcome::PermanentFailure(format!(
                "{outcome:?}; plain fallback: {description}"
            )));
        }
        Ok(plain_outcome)
    }

    /// Answer a callback query (acknowledge button press).
    #[allow(dead_code)]
    async fn api_answer_callback(
        &self,
        callback_query_id: &str,
        text: Option<&str>,
        show_alert: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "{}/bot{}/answerCallbackQuery",
            self.api_base_url,
            self.token.as_str()
        );
        let mut body = serde_json::json!({ "callback_query_id": callback_query_id });
        if let Some(t) = text {
            body["text"] = serde_json::Value::String(t.to_string());
        }
        body["show_alert"] = serde_json::Value::Bool(show_alert);
        let _ = self.client.post(&url).json(&body).send().await;
        Ok(())
    }

    /// Send a native poll.
    #[allow(dead_code)]
    async fn api_send_poll(
        &self,
        chat_id: i64,
        question: &str,
        options: &[&str],
        is_anonymous: bool,
    ) -> Result<Option<i64>, Box<dyn std::error::Error>> {
        let url = format!("{}/bot{}/sendPoll", self.api_base_url, self.token.as_str());
        let opts: Vec<serde_json::Value> = options
            .iter()
            .map(|o| serde_json::json!({"text": o}))
            .collect();
        let body = serde_json::json!({
            "chat_id": chat_id,
            "question": question,
            "options": opts,
            "is_anonymous": is_anonymous,
        });
        let resp = self.client.post(&url).json(&body).send().await?;
        if resp.status().is_success() {
            let resp_json: serde_json::Value = resp.json().await.unwrap_or_default();
            Ok(resp_json["result"]["message_id"].as_i64())
        } else {
            let body_text = resp.text().await.unwrap_or_default();
            warn!("Telegram sendPoll failed: {body_text}");
            Ok(None)
        }
    }

    /// Set a reaction on a message.
    #[allow(dead_code)]
    async fn api_set_reaction(
        &self,
        chat_id: i64,
        message_id: i64,
        emoji: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "{}/bot{}/setMessageReaction",
            self.api_base_url,
            self.token.as_str()
        );
        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "reaction": [{"type": "emoji", "emoji": emoji}],
        });
        let _ = self.client.post(&url).json(&body).send().await;
        Ok(())
    }

    /// Call `sendPhoto` on the Telegram API.
    async fn api_send_photo(
        &self,
        chat_id: i64,
        photo_url: &str,
        caption: Option<&str>,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("{}/bot{}/sendPhoto", self.api_base_url, self.token.as_str());
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "photo": photo_url,
        });
        if let Some(cap) = caption {
            body["caption"] = serde_json::Value::String(cap.to_string());
            body["parse_mode"] = serde_json::Value::String("HTML".to_string());
        }
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }
        let resp = self.client.post(&url).json(&body).send().await?;
        ensure_success(resp, "sendPhoto").await?;
        Ok(())
    }

    /// Call `sendDocument` on the Telegram API.
    async fn api_send_document(
        &self,
        chat_id: i64,
        document_url: &str,
        filename: &str,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "{}/bot{}/sendDocument",
            self.api_base_url,
            self.token.as_str()
        );
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "document": document_url,
            "caption": filename,
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }
        let resp = self.client.post(&url).json(&body).send().await?;
        ensure_success(resp, "sendDocument").await?;
        Ok(())
    }

    /// Call `sendDocument` with multipart upload for local file data.
    ///
    /// Used by the proactive `channel_send` tool when `file_path` is provided.
    /// Uploads raw bytes as a multipart form instead of passing a URL.
    async fn api_send_document_upload(
        &self,
        chat_id: i64,
        data: Vec<u8>,
        filename: &str,
        mime_type: &str,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "{}/bot{}/sendDocument",
            self.api_base_url,
            self.token.as_str()
        );

        let file_part = reqwest::multipart::Part::bytes(data)
            .file_name(filename.to_string())
            .mime_str(mime_type)?;

        let mut form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("document", file_part);

        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }

        let resp = self.client.post(&url).multipart(form).send().await?;
        ensure_success(resp, "sendDocument upload").await?;
        Ok(())
    }

    /// Call `sendPhoto` with multipart upload for local image data (v3.8b).
    ///
    /// Telegram's `sendPhoto` can accept either a public URL or a multipart-
    /// uploaded file. Local screenshots have no public URL, so we must upload
    /// the raw bytes — which also bypasses Telegram's host resolution quirks.
    /// Arrives as an inline photo (not a downloadable document).
    async fn api_send_photo_upload(
        &self,
        chat_id: i64,
        data: Vec<u8>,
        caption: Option<&str>,
        mime_type: &str,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("{}/bot{}/sendPhoto", self.api_base_url, self.token.as_str());

        let filename = telegram_photo_upload_filename(mime_type);

        let file_part = reqwest::multipart::Part::bytes(data)
            .file_name(filename.to_string())
            .mime_str(mime_type)?;

        let mut form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("photo", file_part);

        if let Some(cap) = caption {
            form = form
                .text("caption", cap.to_string())
                .text("parse_mode", "HTML".to_string());
        }
        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }

        let resp = self.client.post(&url).multipart(form).send().await?;
        ensure_success(resp, "sendPhoto upload").await?;
        Ok(())
    }

    /// Call `sendVoice` on the Telegram API.
    async fn api_send_voice(
        &self,
        chat_id: i64,
        voice_url: &str,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("{}/bot{}/sendVoice", self.api_base_url, self.token.as_str());
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "voice": voice_url,
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }
        let resp = self.client.post(&url).json(&body).send().await?;
        ensure_success(resp, "sendVoice").await?;
        Ok(())
    }

    async fn api_send_voice_upload(
        &self,
        chat_id: i64,
        data: Vec<u8>,
        filename: &str,
        mime_type: &str,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("{}/bot{}/sendVoice", self.api_base_url, self.token.as_str());
        let part = reqwest::multipart::Part::bytes(data)
            .file_name(filename.to_string())
            .mime_str(mime_type)?;
        let mut form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("voice", part);
        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }
        let resp = self.client.post(&url).multipart(form).send().await?;
        ensure_success(resp, "sendVoice upload").await?;
        Ok(())
    }

    async fn api_send_audio_upload(
        &self,
        chat_id: i64,
        data: Vec<u8>,
        filename: &str,
        mime_type: &str,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("{}/bot{}/sendAudio", self.api_base_url, self.token.as_str());
        let part = reqwest::multipart::Part::bytes(data)
            .file_name(filename.to_string())
            .mime_str(mime_type)?;
        let mut form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("audio", part);
        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }
        let resp = self.client.post(&url).multipart(form).send().await?;
        ensure_success(resp, "sendAudio upload").await?;
        Ok(())
    }

    /// Call `sendLocation` on the Telegram API.
    async fn api_send_location(
        &self,
        chat_id: i64,
        lat: f64,
        lon: f64,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "{}/bot{}/sendLocation",
            self.api_base_url,
            self.token.as_str()
        );
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "latitude": lat,
            "longitude": lon,
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }
        let resp = self.client.post(&url).json(&body).send().await?;
        ensure_success(resp, "sendLocation").await?;
        Ok(())
    }

    /// Call `sendChatAction` to show "typing..." indicator.
    ///
    /// When `thread_id` is provided, the typing indicator appears in the forum topic.
    async fn api_send_typing(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "{}/bot{}/sendChatAction",
            self.api_base_url,
            self.token.as_str()
        );
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "action": "typing",
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }
        let _ = self.client.post(&url).json(&body).send().await?;
        Ok(())
    }

    /// Call `setMessageReaction` on the Telegram API (fire-and-forget).
    ///
    /// Sets or replaces the bot's emoji reaction on a message. Each new call
    /// automatically replaces the previous reaction, so there is no need to
    /// explicitly remove old ones.
    fn fire_reaction(&self, chat_id: i64, message_id: i64, emoji: &str) {
        let url = format!(
            "{}/bot{}/setMessageReaction",
            self.api_base_url,
            self.token.as_str()
        );
        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "reaction": [{"type": "emoji", "emoji": emoji}],
        });
        let client = self.client.clone();
        tokio::spawn(async move {
            match client.post(&url).json(&body).send().await {
                Ok(resp) if !resp.status().is_success() => {
                    let body_text = resp.text().await.unwrap_or_default();
                    debug!("Telegram setMessageReaction failed: {body_text}");
                }
                Err(e) => {
                    debug!("Telegram setMessageReaction error: {e}");
                }
                _ => {}
            }
        });
    }
}

impl TelegramAdapter {
    /// Commit D.2 — send a plain-text message with an inline keyboard
    /// directly to a chat by id, without going through the
    /// `ChannelAdapter::send` / `ChannelContent` plumbing (which has no
    /// keyboard variant).
    ///
    /// Used by the kernel-side memory-approval subscriber to surface
    /// the four approval buttons (Approve once / Session / Always /
    /// Reject) right next to the candidate triple, so the user can
    /// decide without opening the dashboard.
    pub async fn send_text_with_keyboard(
        &self,
        chat_id: i64,
        text: &str,
        keyboard: &serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.api_send_message_rich(chat_id, text, None, Some(keyboard))
            .await
            .map(|_| ())
    }

    /// Internal helper: send content with optional forum-topic thread_id.
    ///
    /// Both `send()` and `send_in_thread()` delegate here. When `thread_id` is
    /// `Some(id)`, every outbound Telegram API call includes `message_thread_id`
    /// so the message lands in the correct forum topic.
    async fn send_content(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let chat_id: i64 = user
            .platform_id
            .parse()
            .map_err(|_| format!("Invalid Telegram chat_id: {}", user.platform_id))?;

        match content {
            ChannelContent::Text(text) => {
                self.api_send_message(chat_id, &text, thread_id).await?;
            }
            ChannelContent::Image { url, caption } => {
                self.api_send_photo(chat_id, &url, caption.as_deref(), thread_id)
                    .await?;
            }
            ChannelContent::File { url, filename } => {
                self.api_send_document(chat_id, &url, &filename, thread_id)
                    .await?;
            }
            ChannelContent::FileData {
                data,
                filename,
                mime_type,
            } => {
                if mime_type.starts_with("audio/") {
                    if mime_type == "audio/ogg"
                        || filename.ends_with(".ogg")
                        || filename.ends_with(".oga")
                    {
                        self.api_send_voice_upload(chat_id, data, &filename, &mime_type, thread_id)
                            .await?;
                    } else {
                        self.api_send_audio_upload(chat_id, data, &filename, &mime_type, thread_id)
                            .await?;
                    }
                } else {
                    self.api_send_document_upload(chat_id, data, &filename, &mime_type, thread_id)
                        .await?;
                }
            }
            ChannelContent::ImageData {
                data,
                mime_type,
                caption,
            } => {
                self.api_send_photo_upload(
                    chat_id,
                    data,
                    caption.as_deref(),
                    &mime_type,
                    thread_id,
                )
                .await?;
            }
            ChannelContent::Voice { url, .. } => {
                self.api_send_voice(chat_id, &url, thread_id).await?;
            }
            ChannelContent::Video { url, .. } => {
                // V.5b — outbound video send is not a goal of #184 (we only
                // ingest videos from users); fall back to document semantics
                // so the variant is reachable end-to-end without a new API call.
                self.api_send_document(chat_id, &url, "video.mp4", thread_id)
                    .await?;
            }
            ChannelContent::Location { lat, lon } => {
                self.api_send_location(chat_id, lat, lon, thread_id).await?;
            }
            ChannelContent::Command { name, args } => {
                let text = format!("/{name} {}", args.join(" "));
                self.api_send_message(chat_id, text.trim(), thread_id)
                    .await?;
            }
        }
        Ok(())
    }
}

struct TelegramPollingContext {
    token: Zeroizing<String>,
    client: reqwest::Client,
    allowed_users: Vec<String>,
    poll_interval: Duration,
    api_base_url: String,
    bot_username: Arc<tokio::sync::RwLock<Option<String>>>,
    shutdown: watch::Receiver<bool>,
    tx: mpsc::Sender<ChannelMessage>,
}

enum TelegramPollOutcome {
    Updates(Vec<serde_json::Value>),
    Backoff,
    RetryAfter(Duration),
    Sleep,
    Shutdown,
}

struct TelegramCallbackContext {
    id: String,
    data: String,
    from_id: i64,
    from_name: String,
    chat_id: i64,
    thread_id: Option<String>,
}

struct TelegramKnownCallbackCommand {
    name: String,
    args: Vec<String>,
    route: &'static str,
    stop_on_closed: bool,
}

async fn run_telegram_polling_loop(mut ctx: TelegramPollingContext) {
    let mut offset: Option<i64> = None;
    let mut backoff = INITIAL_BACKOFF;

    loop {
        if *ctx.shutdown.borrow() {
            break;
        }

        match poll_telegram_updates(&mut ctx, offset).await {
            TelegramPollOutcome::Updates(updates) => {
                backoff = INITIAL_BACKOFF;
                for update in updates {
                    if let Some(update_id) = update["update_id"].as_i64() {
                        offset = Some(update_id + 1);
                    }
                    if !dispatch_telegram_update(&ctx, &update).await {
                        return;
                    }
                }
                tokio::time::sleep(ctx.poll_interval).await;
            }
            TelegramPollOutcome::Backoff => {
                tokio::time::sleep(backoff).await;
                backoff = calculate_backoff(backoff);
            }
            TelegramPollOutcome::RetryAfter(duration) => {
                tokio::time::sleep(duration).await;
            }
            TelegramPollOutcome::Sleep => {
                tokio::time::sleep(ctx.poll_interval).await;
            }
            TelegramPollOutcome::Shutdown => break,
        }
    }

    info!("Telegram polling loop stopped");
}

async fn poll_telegram_updates(
    ctx: &mut TelegramPollingContext,
    offset: Option<i64>,
) -> TelegramPollOutcome {
    let url = format!("{}/bot{}/getUpdates", ctx.api_base_url, ctx.token.as_str());
    let mut params = serde_json::json!({
        "timeout": LONG_POLL_TIMEOUT,
        "allowed_updates": ["message", "edited_message", "callback_query"],
    });
    if let Some(off) = offset {
        params["offset"] = serde_json::json!(off);
    }

    let request_timeout = Duration::from_secs(LONG_POLL_TIMEOUT + 10);
    let result = tokio::select! {
        res = async {
            ctx.client
                .get(&url)
                .json(&params)
                .timeout(request_timeout)
                .send()
                .await
        } => res,
        _ = ctx.shutdown.changed() => return TelegramPollOutcome::Shutdown,
    };

    let resp = match result {
        Ok(resp) => resp,
        Err(e) => {
            warn!("Telegram getUpdates network error: {e}, backing off");
            return TelegramPollOutcome::Backoff;
        }
    };

    classify_telegram_poll_response(resp).await
}

async fn classify_telegram_poll_response(resp: reqwest::Response) -> TelegramPollOutcome {
    let status = resp.status();
    if status.as_u16() == 429 {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        let retry_after = body["parameters"]["retry_after"].as_u64().unwrap_or(5);
        warn!("Telegram rate limited, retry after {retry_after}s");
        return TelegramPollOutcome::RetryAfter(Duration::from_secs(retry_after));
    }

    if status.as_u16() == 409 {
        warn!("Telegram 409 Conflict — stale polling session, backing off");
        return TelegramPollOutcome::Backoff;
    }

    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        warn!("Telegram getUpdates failed ({status}): {body_text}, backing off");
        return TelegramPollOutcome::Backoff;
    }

    let body: serde_json::Value = match resp.json().await {
        Ok(value) => value,
        Err(e) => {
            warn!("Telegram getUpdates parse error: {e}");
            return TelegramPollOutcome::Backoff;
        }
    };

    if body["ok"].as_bool() != Some(true) {
        warn!("Telegram getUpdates returned ok=false");
        return TelegramPollOutcome::Sleep;
    }

    body["result"]
        .as_array()
        .cloned()
        .map(TelegramPollOutcome::Updates)
        .unwrap_or(TelegramPollOutcome::Sleep)
}

async fn dispatch_telegram_update(
    ctx: &TelegramPollingContext,
    update: &serde_json::Value,
) -> bool {
    if let Some(callback_query) = update.get("callback_query") {
        return dispatch_telegram_callback(ctx, callback_query).await;
    }

    let bot_username = ctx.bot_username.read().await.clone();
    let msg = match parse_telegram_update(
        update,
        &ctx.allowed_users,
        ctx.token.as_str(),
        &ctx.client,
        &ctx.api_base_url,
        bot_username.as_deref(),
    )
    .await
    {
        Some(message) => message,
        None => return true,
    };

    debug!(
        "Telegram message from {}: {:?}",
        msg.sender.display_name, msg.content
    );

    ctx.tx.send(msg).await.is_ok()
}

async fn dispatch_telegram_callback(
    ctx: &TelegramPollingContext,
    callback_query: &serde_json::Value,
) -> bool {
    let Some(callback) = parse_telegram_callback_context(callback_query, &ctx.allowed_users) else {
        return true;
    };

    acknowledge_telegram_callback(ctx, &callback.id).await;

    if let Some((short_id, idx)) = parse_ask_user_callback(&callback.data) {
        let original_message_id = callback_query["message"]["message_id"].as_i64();
        let msg = crate::telegram_callbacks::ask_user_answer_callback_message(
            &short_id,
            idx,
            callback.chat_id,
            callback.from_id,
            &callback.from_name,
            callback.thread_id.clone(),
            original_message_id,
        );
        info!(callback_data = %callback.data, from = %callback.from_name, "Telegram ask_user callback routed");
        return ctx.tx.send(msg).await.is_ok();
    }

    if let Some(command) = route_known_callback_command(&callback.data) {
        return send_known_callback_command(ctx, &callback, command).await;
    }

    let msg = build_unhandled_callback_message(callback_query, &callback);
    info!(callback_data = %callback.data, from = %callback.from_name, "Telegram callback_query received");
    ctx.tx.send(msg).await.is_ok()
}

fn parse_telegram_callback_context(
    callback_query: &serde_json::Value,
    allowed_users: &[String],
) -> Option<TelegramCallbackContext> {
    let id = callback_query["id"].as_str().unwrap_or("").to_string();
    let data = callback_query["data"].as_str().unwrap_or("").to_string();
    let from_id = match callback_query["from"]["id"].as_i64() {
        Some(id) => id,
        None => {
            debug!("Telegram: dropping callback_query — from.id is not an integer");
            return None;
        }
    };

    if !telegram_user_is_authorized(allowed_users, from_id) {
        debug!(
            "Telegram: ignoring callback_query from unauthorised user {from_id} (allowed_users gate)"
        );
        return None;
    }

    Some(TelegramCallbackContext {
        id,
        data,
        from_id,
        from_name: callback_query["from"]["first_name"]
            .as_str()
            .unwrap_or("User")
            .to_string(),
        chat_id: callback_query["message"]["chat"]["id"]
            .as_i64()
            .unwrap_or(0),
        thread_id: callback_query["message"]["message_thread_id"]
            .as_i64()
            .map(|tid| tid.to_string()),
    })
}

async fn acknowledge_telegram_callback(ctx: &TelegramPollingContext, callback_query_id: &str) {
    let ack_url = format!(
        "{}/bot{}/answerCallbackQuery",
        ctx.api_base_url,
        ctx.token.as_str()
    );
    let _ = ctx
        .client
        .post(&ack_url)
        .json(&serde_json::json!({"callback_query_id": callback_query_id}))
        .send()
        .await;
}

fn route_known_callback_command(data: &str) -> Option<TelegramKnownCallbackCommand> {
    let (name, args, route, stop_on_closed) =
        if let Some((name, args)) = parse_model_switch_callback(data) {
            (name, args, "model_switch", false)
        } else if let Some((name, args)) = parse_project_ask_callback(data) {
            (name, args, "project_ask", false)
        } else if let Some((name, args)) = parse_learning_callback(data) {
            (name, args, "learning", false)
        } else if let Some((name, args)) = parse_skill_proposal_callback(data) {
            (name, args, "skill_proposal", false)
        } else if let Some((name, args)) = parse_skill_refinement_callback(data) {
            (name, args, "skill_refinement", false)
        } else if let Some((name, args)) = parse_approval_callback(data) {
            (name, args, "approval", true)
        } else {
            return None;
        };

    Some(TelegramKnownCallbackCommand {
        name,
        args,
        route,
        stop_on_closed,
    })
}

async fn send_known_callback_command(
    ctx: &TelegramPollingContext,
    callback: &TelegramCallbackContext,
    command: TelegramKnownCallbackCommand,
) -> bool {
    let msg = callback_command_message(
        &callback.id,
        &callback.data,
        command.name,
        command.args,
        callback.chat_id,
        callback.from_id,
        &callback.from_name,
        callback.thread_id.clone(),
    );
    info!(callback_data = %callback.data, from = %callback.from_name, route = command.route, "Telegram callback routed to slash command");
    if ctx.tx.send(msg).await.is_ok() {
        return true;
    }

    warn!(
        route = command.route,
        "Telegram: failed to forward callback command"
    );
    !command.stop_on_closed
}

fn build_unhandled_callback_message(
    callback_query: &serde_json::Value,
    callback: &TelegramCallbackContext,
) -> ChannelMessage {
    let original_preview = callback_original_preview(callback_query);
    let content_text = format!(
        "L'utilisateur a cliqué sur le bouton «{}» en réponse au message: \"{}\"",
        callback.data,
        shortened_callback_preview(&original_preview)
    );

    // The original keyboard message id, NOT the callback_query id — using
    // callback.id here breaks reply threading (source_message_id /
    // reply_to_message_id) since it isn't a real chat message id. Same bug
    // class already found and fixed for ask_user_answer_callback_message.
    let original_message_id = callback_query["message"]["message_id"].as_i64();

    let mut metadata = std::collections::HashMap::new();
    metadata.insert("callback_query".to_string(), serde_json::json!(true));
    metadata.insert(
        "callback_data".to_string(),
        serde_json::json!(callback.data),
    );
    metadata.insert(
        "sender_user_id".to_string(),
        serde_json::json!(callback.from_id),
    );
    metadata.insert(
        "original_message_id".to_string(),
        serde_json::json!(original_message_id),
    );
    metadata.insert("chat_id".to_string(), serde_json::json!(callback.chat_id));
    metadata.insert(
        "original_text".to_string(),
        serde_json::json!(original_preview),
    );

    ChannelMessage {
        channel: ChannelType::Telegram,
        platform_message_id: original_message_id
            .map(|id| id.to_string())
            .unwrap_or_default(),
        sender: ChannelUser {
            platform_id: callback.chat_id.to_string(),
            display_name: callback.from_name.clone(),
            captain_user: None,
        },
        content: ChannelContent::Text(content_text),
        target_agent: None,
        timestamp: chrono::Utc::now(),
        is_group: false,
        thread_id: callback.thread_id.clone(),
        metadata,
    }
}

fn callback_original_preview(callback_query: &serde_json::Value) -> String {
    callback_query["message"]["text"]
        .as_str()
        .unwrap_or("")
        .chars()
        .take(100)
        .collect()
}

fn shortened_callback_preview(preview: &str) -> String {
    if preview.chars().count() > 80 {
        let prefix: String = preview.chars().take(77).collect();
        format!("{prefix}…")
    } else {
        preview.to_string()
    }
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn name(&self) -> &str {
        "telegram"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Telegram
    }

    fn as_telegram_arc(self: Arc<Self>) -> Option<Arc<TelegramAdapter>> {
        Some(self)
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, Box<dyn std::error::Error>>
    {
        // Validate token first (fail fast) and store bot username for mention detection
        let bot_name = self.validate_token().await?;
        {
            let mut username = self.bot_username.write().await;
            *username = Some(bot_name.clone());
        }
        info!("Telegram bot @{bot_name} connected");

        // Clear any existing webhook to avoid 409 Conflict during getUpdates polling.
        // This is necessary when the daemon restarts — the old polling session may
        // still be active on Telegram's side for ~30s, causing 409 errors.
        {
            let delete_url = format!(
                "{}/bot{}/deleteWebhook",
                self.api_base_url,
                self.token.as_str()
            );
            match self
                .client
                .post(&delete_url)
                .json(&serde_json::json!({"drop_pending_updates": true}))
                .send()
                .await
            {
                Ok(_) => info!("Telegram: cleared webhook, polling mode active"),
                Err(e) => tracing::warn!("Telegram: deleteWebhook failed (non-fatal): {e}"),
            }
        }

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);

        let polling_context = TelegramPollingContext {
            token: self.token.clone(),
            client: self.client.clone(),
            allowed_users: self.allowed_users.clone(),
            poll_interval: self.poll_interval,
            api_base_url: self.api_base_url.clone(),
            bot_username: self.bot_username.clone(),
            shutdown: self.shutdown_rx.clone(),
            tx,
        };
        tokio::spawn(async move {
            run_telegram_polling_loop(polling_context).await;
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.send_content(user, content, None).await
    }

    async fn send_typing(&self, user: &ChannelUser) -> Result<(), Box<dyn std::error::Error>> {
        let chat_id: i64 = user
            .platform_id
            .parse()
            .map_err(|_| format!("Invalid Telegram chat_id: {}", user.platform_id))?;
        self.api_send_typing(chat_id, None).await
    }

    async fn send_in_thread(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
        thread_id: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let tid: Option<i64> = thread_id.parse().ok();
        self.send_content(user, content, tid).await
    }

    async fn send_rich(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
        metadata: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let chat_id: i64 = user
            .platform_id
            .parse()
            .map_err(|_| format!("Invalid Telegram chat_id: {}", user.platform_id))?;
        let thread_id: Option<i64> = metadata.get("thread_id").and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        });
        let reply_markup = metadata.get("reply_markup");

        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            _ => {
                self.send_content(user, content, thread_id).await?;
                return Ok(None);
            }
        };

        let plain_text = metadata
            .get("telegram_plain_text")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let msg_id = if plain_text {
            self.api_send_plain_text_full(chat_id, &text, thread_id, reply_markup)
                .await?
        } else {
            self.api_send_message_rich(chat_id, &text, thread_id, reply_markup)
                .await?
        };
        Ok(msg_id.map(|id| id.to_string()))
    }

    async fn edit_rich(
        &self,
        message_id: &str,
        text: &str,
        metadata: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let chat_id: i64 = metadata
            .get("chat_id")
            .and_then(|v| v.as_i64())
            .ok_or("edit_rich: missing chat_id in metadata")?;
        let message_id: i64 = message_id
            .parse()
            .map_err(|e| format!("edit_rich: invalid message_id '{message_id}': {e}"))?;
        // Clearing the keyboard is an explicit empty array, not an absent
        // key — an absent `reply_markup` would leave the existing buttons
        // in place, letting a stale click fire twice.
        let reply_markup = serde_json::json!({ "inline_keyboard": [] });
        self.api_edit_message(chat_id, message_id, Some(text), Some(&reply_markup))
            .await
    }

    async fn send_reaction(
        &self,
        user: &ChannelUser,
        message_id: &str,
        reaction: &LifecycleReaction,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let chat_id: i64 = user
            .platform_id
            .parse()
            .map_err(|_| format!("Invalid Telegram chat_id: {}", user.platform_id))?;
        let msg_id: i64 = message_id
            .parse()
            .map_err(|_| format!("Invalid Telegram message_id: {message_id}"))?;
        self.fire_reaction(chat_id, msg_id, &reaction.emoji);
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }
}

fn telegram_user_is_authorized(allowed_users: &[String], user_id: i64) -> bool {
    crate::rbac::is_authorized(allowed_users, &user_id.to_string())
}

/// Parse a Telegram update JSON into a `ChannelMessage`, or `None` if filtered/unparseable.
/// Handles both `message` and `edited_message` update types.
async fn parse_telegram_update(
    update: &serde_json::Value,
    allowed_users: &[String],
    token: &str,
    client: &reqwest::Client,
    api_base_url: &str,
    bot_username: Option<&str>,
) -> Option<ChannelMessage> {
    let update_id = update["update_id"].as_i64().unwrap_or(0);
    let message = telegram_update_message(update, update_id)?;
    let context = parse_telegram_update_context(message, update_id)?;

    // Security (B.8): empty allowed_users now denies all instead of
    // allowing all. To explicitly open the channel, set
    // `allowed_users = ["*"]` in config.toml.
    if !telegram_user_is_authorized(allowed_users, context.user_id) {
        let user_id = context.user_id;
        debug!("Telegram: ignoring message from unauthorised user {user_id} (allowed_users gate)");
        return None;
    }

    let content =
        parse_telegram_update_content(update_id, message, token, client, api_base_url).await?;

    let content = apply_telegram_reply_context(content, message);

    let thread_id = context.thread_id.clone();

    if let Some(ref tid) = thread_id {
        info!(chat_id = %context.chat_id, thread_id = %tid, from = %context.display_name, "Telegram message from forum topic");
    }

    let metadata = telegram_update_metadata(
        message,
        context.user_id,
        thread_id.as_deref(),
        context.is_group,
        bot_username,
    );

    Some(ChannelMessage {
        channel: ChannelType::Telegram,
        platform_message_id: context.message_id.to_string(),
        sender: ChannelUser {
            platform_id: context.chat_id.to_string(),
            display_name: context.display_name,
            captain_user: None,
        },
        content,
        target_agent: None,
        timestamp: context.timestamp,
        is_group: context.is_group,
        thread_id,
        metadata,
    })
}

/// Calculate exponential backoff capped at MAX_BACKOFF.
pub fn calculate_backoff(current: Duration) -> Duration {
    (current * 2).min(MAX_BACKOFF)
}

impl TelegramAdapter {
    /// Issue a single `editMessageText` call and classify the outcome.
    ///
    /// Unlike the legacy `api_edit_message`, this method never swallows
    /// the failure — the caller decides whether to retry, give up, or
    /// raise a flood strike. It is deliberately scoped to text edits
    /// (no reply_markup) because the streaming consumer never edits
    /// markup mid-flight.
    pub(crate) async fn api_edit_message_strict(
        &self,
        chat_id: i64,
        message_id: i64,
        text: &str,
    ) -> Result<EditOutcome, Box<dyn std::error::Error>> {
        self.api_edit_rich_with_markup(chat_id, message_id, text, None)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_telegram_adapter(server: &wiremock::MockServer) -> TelegramAdapter {
        TelegramAdapter::new(
            "123:ABC".to_string(),
            vec!["*".to_string()],
            Duration::from_secs(1),
            Some(server.uri()),
        )
    }

    #[tokio::test]
    async fn telegram_native_rich_send_preserves_markdown_and_metadata() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendRichMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {"message_id": 77, "rich_message": {"blocks": []}}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let adapter = mock_telegram_adapter(&server);
        let keyboard = serde_json::json!({
            "inline_keyboard": [[{"text": "OK", "callback_data": "ok"}]]
        });
        let markdown = "## Report\n\n| Metric | Value |\n|---|---:|\n| Status | **OK** |";

        let message_id = adapter
            .api_send_message_rich_full(42, markdown, Some(7), Some(&keyboard), Some(99))
            .await
            .expect("native rich send");

        assert_eq!(message_id, Some(77));
        let requests = server.received_requests().await.expect("requests");
        let body: serde_json::Value =
            serde_json::from_slice(&requests[0].body).expect("json request");
        assert_eq!(body["rich_message"]["markdown"], markdown);
        assert_eq!(body["message_thread_id"], 7);
        assert_eq!(body["reply_parameters"]["message_id"], 99);
        assert_eq!(body["reply_markup"], keyboard);
        assert!(body.get("text").is_none());
        assert!(body.get("parse_mode").is_none());
    }

    #[tokio::test]
    async fn telegram_native_rich_draft_and_edit_use_bot_api_10_2_fields() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendRichMessageDraft"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": true
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/editMessageText"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {"message_id": 9}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let adapter = mock_telegram_adapter(&server);

        assert_eq!(
            adapter
                .api_send_rich_message_draft(
                    42,
                    123,
                    "<tg-thinking>Captain travaille</tg-thinking>",
                    Some(7)
                )
                .await
                .expect("draft"),
            RichDraftOutcome::Sent
        );
        assert_eq!(
            adapter
                .api_edit_message_strict(42, 9, "## Final")
                .await
                .expect("rich edit"),
            EditOutcome::Ok
        );

        let requests = server.received_requests().await.expect("requests");
        let draft = requests
            .iter()
            .find(|request| request.url.path().ends_with("sendRichMessageDraft"))
            .expect("draft request");
        let draft_body: serde_json::Value =
            serde_json::from_slice(&draft.body).expect("draft json");
        assert_eq!(draft_body["draft_id"], 123);
        assert_eq!(draft_body["message_thread_id"], 7);
        assert!(draft_body["rich_message"]["markdown"]
            .as_str()
            .expect("markdown")
            .contains("tg-thinking"));

        let edit = requests
            .iter()
            .find(|request| request.url.path().ends_with("editMessageText"))
            .expect("edit request");
        let edit_body: serde_json::Value = serde_json::from_slice(&edit.body).expect("edit json");
        assert_eq!(edit_body["rich_message"]["markdown"], "## Final");
        assert!(edit_body.get("text").is_none());
    }

    #[tokio::test]
    async fn telegram_edit_rich_clears_inline_keyboard_explicitly() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/editMessageText"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {"message_id": 9}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let adapter = mock_telegram_adapter(&server);
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("chat_id".to_string(), serde_json::json!(42));

        adapter
            .edit_rich("9", "### ✓ Décision enregistrée", &metadata)
            .await
            .expect("resolved Rich card");

        let requests = server.received_requests().await.expect("requests");
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).expect("edit json");
        assert_eq!(
            body["rich_message"]["markdown"],
            "### ✓ Décision enregistrée"
        );
        assert_eq!(
            body["reply_markup"],
            serde_json::json!({"inline_keyboard": []})
        );
        assert!(body.get("text").is_none());
    }

    #[tokio::test]
    async fn telegram_unsupported_rich_endpoint_falls_back_and_is_cached() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendRichMessage"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "ok": false,
                "description": "Not Found: method not found"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {"message_id": 78}
            })))
            .expect(2)
            .mount(&server)
            .await;
        let adapter = mock_telegram_adapter(&server);

        for markdown in ["**first**", "**second**"] {
            assert_eq!(
                adapter
                    .api_send_message_rich_full(42, markdown, None, None, None)
                    .await
                    .expect("legacy fallback"),
                Some(78)
            );
        }

        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 3, "one Rich probe then two HTML sends");
        for request in requests
            .iter()
            .filter(|request| request.url.path().ends_with("sendMessage"))
        {
            let body: serde_json::Value =
                serde_json::from_slice(&request.body).expect("legacy json");
            assert_eq!(body["parse_mode"], "HTML");
            assert!(body["text"].as_str().expect("text").contains("<b>"));
        }
    }

    #[tokio::test]
    async fn telegram_rich_server_failure_does_not_risk_duplicate_fallback_send() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendRichMessage"))
            .respond_with(ResponseTemplate::new(500).set_body_string("upstream failure"))
            .expect(1)
            .mount(&server)
            .await;
        let adapter = mock_telegram_adapter(&server);

        let error = adapter
            .api_send_message_rich_full(42, "hello", None, None, None)
            .await
            .expect_err("500 must be surfaced");
        assert!(error.to_string().contains("500"));
        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 1, "no duplicate legacy send after 5xx");
    }

    #[tokio::test]
    async fn telegram_explicit_plain_text_bypasses_rich_and_parse_mode() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {"message_id": 79}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let adapter = mock_telegram_adapter(&server);
        let user = ChannelUser {
            platform_id: "42".to_string(),
            display_name: "Test".to_string(),
            captain_user: None,
        };
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("telegram_plain_text".to_string(), serde_json::json!(true));

        let message_id = adapter
            .send_rich(
                &user,
                ChannelContent::Text("**literal**".to_string()),
                &metadata,
            )
            .await
            .expect("plain send");
        assert_eq!(message_id.as_deref(), Some("79"));

        let requests = server.received_requests().await.expect("requests");
        let body: serde_json::Value =
            serde_json::from_slice(&requests[0].body).expect("plain json");
        assert_eq!(body["text"], "**literal**");
        assert!(body.get("parse_mode").is_none());
        assert!(body.get("rich_message").is_none());
    }

    fn test_client() -> reqwest::Client {
        reqwest::Client::new()
    }

    /// B.8 — Tests that exercise the message *parsing* path (not the
    /// allowlist gate) need a wildcard allow_users so they don't trip on
    /// the "empty list = deny all" policy. New code under test should
    /// declare its allowlist explicitly; this helper is for fixtures.
    fn open_to_all() -> &'static [String] {
        use std::sync::OnceLock;
        static ALLOW: OnceLock<Vec<String>> = OnceLock::new();
        ALLOW.get_or_init(|| vec!["*".to_string()]).as_slice()
    }

    #[test]
    fn test_telegram_user_authorization_policy_covers_callbacks() {
        let empty: Vec<String> = Vec::new();
        let wildcard = vec!["*".to_string()];
        let allow_42 = vec!["42".to_string()];

        assert!(!telegram_user_is_authorized(&empty, 42));
        assert!(telegram_user_is_authorized(&wildcard, 42));
        assert!(telegram_user_is_authorized(&allow_42, 42));
        assert!(!telegram_user_is_authorized(&allow_42, 7));
    }

    /// v3.8i — approval callback payload parses into /approve or /reject.
    #[test]
    fn test_parse_approval_callback_once_maps_to_approve() {
        let parsed = parse_approval_callback("approval:once:abc-123");
        assert_eq!(
            parsed,
            Some(("approve".to_string(), vec!["abc-123".to_string()]))
        );
    }

    // Q.11.b.2 — `session` no longer collapses into `approve`; see
    // `test_q11b_parse_callback_session_routes_to_approve_session` below.

    #[test]
    fn test_parse_approval_callback_deny_maps_to_reject() {
        let parsed = parse_approval_callback("approval:deny:abc-123");
        assert_eq!(
            parsed,
            Some(("reject".to_string(), vec!["abc-123".to_string()]))
        );
    }

    #[test]
    fn test_parse_approval_callback_unknown_is_none() {
        assert!(parse_approval_callback("random:data").is_none());
        assert!(parse_approval_callback("approval:foo:x").is_none());
        assert!(parse_approval_callback("approval:once:").is_none());
    }

    #[test]
    fn test_parse_learning_callback_routes_to_learning_commands() {
        assert_eq!(
            parse_learning_callback("learning:approve:rev-42"),
            Some(("learn_approve".to_string(), vec!["rev-42".to_string()]))
        );
        assert_eq!(
            parse_learning_callback("learning:reject:rev-42"),
            Some(("learn_reject".to_string(), vec!["rev-42".to_string()]))
        );
        assert!(parse_learning_callback("learning:session:rev-42").is_none());
        assert!(parse_learning_callback("learning:approve:").is_none());
    }

    #[test]
    fn callback_command_message_replies_to_chat_and_preserves_clicking_user() {
        let msg = callback_command_message(
            "cbq-1",
            "learning:approve:rev-42",
            "learn_approve".to_string(),
            vec!["rev-42".to_string()],
            -100123,
            4242,
            "Alex",
            Some("7".to_string()),
        );

        assert_eq!(msg.sender.platform_id, "-100123");
        assert_eq!(msg.metadata["sender_user_id"], serde_json::json!(4242));
        assert_eq!(msg.metadata["chat_id"], serde_json::json!(-100123));
        assert_eq!(msg.thread_id.as_deref(), Some("7"));
        match msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "learn_approve");
                assert_eq!(args, vec!["rev-42".to_string()]);
            }
            other => panic!("expected command callback message, got {other:?}"),
        }
    }

    #[test]
    fn known_callback_router_preserves_route_priority_and_close_policy() {
        let model = route_known_callback_command("model:plan-42:new_session").unwrap();
        assert_eq!(model.route, "model_switch");
        assert_eq!(model.name, "model_switch");
        assert_eq!(
            model.args,
            vec!["plan-42".to_string(), "new_session".to_string()]
        );
        assert!(!model.stop_on_closed);

        let approval = route_known_callback_command("approval:once:req-42").unwrap();
        assert_eq!(approval.route, "approval");
        assert_eq!(approval.name, "approve");
        assert_eq!(approval.args, vec!["req-42".to_string()]);
        assert!(approval.stop_on_closed);

        assert!(route_known_callback_command("unknown:payload").is_none());
    }

    #[test]
    fn shortened_callback_preview_truncates_on_char_boundary() {
        let preview = "é".repeat(90);
        let shortened = shortened_callback_preview(&preview);

        assert_eq!(shortened.chars().count(), 78);
        assert!(shortened.ends_with('…'));
    }

    #[test]
    fn test_parse_skill_proposal_callback_routes_to_skill_commands() {
        assert_eq!(
            parse_skill_proposal_callback("skill_proposal:approve:prop-42"),
            Some(("skill_approve".to_string(), vec!["prop-42".to_string()]))
        );
        assert_eq!(
            parse_skill_proposal_callback("skill_proposal:reject:prop-42"),
            Some(("skill_reject".to_string(), vec!["prop-42".to_string()]))
        );
        assert!(parse_skill_proposal_callback("skill_proposal:session:prop-42").is_none());
        assert!(parse_skill_proposal_callback("skill_proposal:approve:").is_none());
    }

    #[test]
    fn test_parse_skill_refinement_callback_routes_to_refinement_commands() {
        assert_eq!(
            parse_skill_refinement_callback("skill_refinement:approve:ref-42"),
            Some((
                "skill_refine_approve".to_string(),
                vec!["ref-42".to_string()]
            ))
        );
        assert_eq!(
            parse_skill_refinement_callback("skill_refinement:reject:ref-42"),
            Some((
                "skill_refine_reject".to_string(),
                vec!["ref-42".to_string()]
            ))
        );
        assert!(parse_skill_refinement_callback("skill_refinement:session:ref-42").is_none());
        assert!(parse_skill_refinement_callback("skill_refinement:approve:").is_none());
    }

    #[test]
    fn test_parse_model_switch_callback_routes_to_bridge_command() {
        assert_eq!(
            parse_model_switch_callback("model:plan-42:new_session"),
            Some((
                "model_switch".to_string(),
                vec!["plan-42".to_string(), "new_session".to_string()]
            ))
        );
        assert_eq!(
            parse_model_switch_callback("model:plan-42:compact_session"),
            Some((
                "model_switch".to_string(),
                vec!["plan-42".to_string(), "compact_session".to_string()]
            ))
        );
    }

    #[test]
    fn test_parse_model_switch_callback_rejects_invalid_payloads() {
        assert!(parse_model_switch_callback("random:data").is_none());
        assert!(parse_model_switch_callback("model:plan-42:always").is_none());
        assert!(parse_model_switch_callback("model::new_session").is_none());
        assert!(parse_model_switch_callback("model:plan-42:").is_none());
    }

    #[test]
    fn test_parse_project_ask_callback_routes_to_project_answer() {
        assert_eq!(
            parse_project_ask_callback("project_ask:ask-42:1"),
            Some((
                "project_answer".to_string(),
                vec!["ask-42".to_string(), "@idx:1".to_string()]
            ))
        );
        assert!(parse_project_ask_callback("project_ask:ask-42:abc").is_none());
        assert!(parse_project_ask_callback("project_ask::1").is_none());
    }

    #[test]
    fn test_build_project_ask_keyboard_uses_indices() {
        let kb =
            build_project_ask_keyboard("ask-42", &["Option A".to_string(), "Option B".to_string()]);
        assert_eq!(
            kb["inline_keyboard"][0][0]["callback_data"].as_str(),
            Some("project_ask:ask-42:0")
        );
        assert_eq!(
            kb["inline_keyboard"][1][0]["callback_data"].as_str(),
            Some("project_ask:ask-42:1")
        );
    }

    #[test]
    fn test_build_model_switch_keyboard_has_three_buttons() {
        let kb = build_model_switch_keyboard("plan-42");
        let rows = kb["inline_keyboard"]
            .as_array()
            .expect("inline_keyboard missing");
        let empty: Vec<serde_json::Value> = Vec::new();
        let mut datas: Vec<&str> = Vec::new();
        for row in rows {
            for button in row.as_array().unwrap_or(&empty) {
                if let Some(data) = button["callback_data"].as_str() {
                    datas.push(data);
                }
            }
        }

        assert_eq!(datas.len(), 3, "expected 3 model switch buttons");
        assert!(datas.contains(&"model:plan-42:new_session"));
        assert!(datas.contains(&"model:plan-42:compact_session"));
        assert!(datas.contains(&"model:plan-42:cancel"));

        let recommended =
            build_model_switch_keyboard_with_recommendation("plan-42", Some("compact_session"));
        assert_eq!(
            recommended["inline_keyboard"][0][1]["text"].as_str(),
            Some("Resume compact (recommandé)")
        );
    }

    /// Q.11.b.2 — keyboard now has the 4 approval choices.
    #[test]
    fn test_q11b_build_approval_keyboard_has_four_buttons() {
        let kb = build_approval_keyboard("req-42");
        // We accept either one row of 4 buttons or two rows summing to 4.
        let rows = kb["inline_keyboard"]
            .as_array()
            .expect("inline_keyboard missing");
        let empty: Vec<serde_json::Value> = Vec::new();
        let mut datas: Vec<&str> = Vec::new();
        for r in rows {
            for b in r.as_array().unwrap_or(&empty) {
                if let Some(d) = b["callback_data"].as_str() {
                    datas.push(d);
                }
            }
        }
        assert_eq!(
            datas.len(),
            4,
            "expected 4 buttons (once/session/always/deny), got {datas:?}"
        );
        assert!(datas.contains(&"approval:once:req-42"));
        assert!(datas.contains(&"approval:session:req-42"));
        assert!(datas.contains(&"approval:always:req-42"));
        assert!(datas.contains(&"approval:deny:req-42"));
    }

    #[test]
    fn test_learning_approval_keyboard_uses_learning_namespace() {
        let kb = build_learning_approval_keyboard("rev-42");
        let rows = kb["inline_keyboard"]
            .as_array()
            .expect("inline_keyboard missing");
        let empty: Vec<serde_json::Value> = Vec::new();
        let mut datas: Vec<&str> = Vec::new();
        for row in rows {
            for button in row.as_array().unwrap_or(&empty) {
                if let Some(data) = button["callback_data"].as_str() {
                    datas.push(data);
                }
            }
        }
        assert_eq!(
            datas,
            vec!["learning:approve:rev-42", "learning:reject:rev-42"]
        );
    }

    /// Q.11.b.2 — `session` must route to `approve_session` (NOT plain
    /// `approve`) so the kernel can distinguish the user's choice.
    #[test]
    fn test_q11b_parse_callback_session_routes_to_approve_session() {
        let parsed = parse_approval_callback("approval:session:abc-123").expect("must parse");
        assert_eq!(parsed.0, "approve_session");
        assert_eq!(parsed.1, vec!["abc-123".to_string()]);
    }

    /// Q.11.b.2 — `always` is a brand-new variant routing to `approve_always`.
    #[test]
    fn test_q11b_parse_callback_always_routes_to_approve_always() {
        let parsed = parse_approval_callback("approval:always:xyz").expect("must parse");
        assert_eq!(parsed.0, "approve_always");
        assert_eq!(parsed.1, vec!["xyz".to_string()]);
    }

    /// Q.11.b.2 — `once` continues to route to plain `approve` (back-compat).
    #[test]
    fn test_q11b_parse_callback_once_still_routes_to_approve() {
        let parsed = parse_approval_callback("approval:once:foo").expect("must parse");
        assert_eq!(parsed.0, "approve");
    }

    /// v3.8c — ImageData routes through the send dispatch to api_send_photo_upload.
    /// Regression guard: local file with image MIME must land on /sendPhoto,
    /// NOT /sendDocument (which would show as a downloadable attachment).
    #[tokio::test]
    async fn test_image_data_routes_to_sendphoto_endpoint() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        // If routing breaks and document endpoint gets hit, this test fails
        // because the photo endpoint never sees the request.
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendPhoto"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = TelegramAdapter::new(
            "123:ABC".into(),
            vec![],
            Duration::from_secs(1),
            Some(server.uri()),
        );

        let user = ChannelUser {
            platform_id: "42".to_string(),
            display_name: "Test".to_string(),
            captain_user: None,
        };
        let content = ChannelContent::ImageData {
            data: vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
            mime_type: "image/png".to_string(),
            caption: Some("screenshot".to_string()),
        };

        let result = adapter.send(&user, content).await;
        assert!(
            result.is_ok(),
            "send must route ImageData to sendPhoto: {result:?}"
        );
    }

    /// Local TTS audio should be delivered as native Telegram audio, not as a
    /// generic document attachment. ElevenLabs returns MP3 by default.
    #[tokio::test]
    async fn test_audio_file_data_routes_to_sendaudio_endpoint() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendAudio"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = TelegramAdapter::new(
            "123:ABC".into(),
            vec![],
            Duration::from_secs(1),
            Some(server.uri()),
        );

        let user = ChannelUser {
            platform_id: "42".to_string(),
            display_name: "Test".to_string(),
            captain_user: None,
        };
        let content = ChannelContent::FileData {
            data: vec![0x49, 0x44, 0x33],
            filename: "tts_reply.mp3".to_string(),
            mime_type: "audio/mpeg".to_string(),
        };

        let result = adapter.send(&user, content).await;
        assert!(
            result.is_ok(),
            "send must route MP3 FileData to sendAudio: {result:?}"
        );
    }

    /// OGG/Opus files are Telegram voice-note compatible and should use
    /// sendVoice, preserving the channel-native voice UX.
    #[tokio::test]
    async fn test_ogg_file_data_routes_to_sendvoice_endpoint() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendVoice"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = TelegramAdapter::new(
            "123:ABC".into(),
            vec![],
            Duration::from_secs(1),
            Some(server.uri()),
        );

        let user = ChannelUser {
            platform_id: "42".to_string(),
            display_name: "Test".to_string(),
            captain_user: None,
        };
        let content = ChannelContent::FileData {
            data: vec![0x4f, 0x67, 0x67, 0x53],
            filename: "reply.ogg".to_string(),
            mime_type: "audio/ogg".to_string(),
        };

        let result = adapter.send(&user, content).await;
        assert!(
            result.is_ok(),
            "send must route OGG FileData to sendVoice: {result:?}"
        );
    }

    /// v3.8b — api_send_photo_upload sends a local file as a multipart photo.
    /// Required for screenshots (which are local files with no public URL).
    /// Regression guard: verifies the endpoint is /sendPhoto (not
    /// /sendDocument) and the upload succeeds.
    #[tokio::test]
    async fn test_api_send_photo_upload_posts_to_sendphoto() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendPhoto"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
            .mount(&server)
            .await;

        let adapter = TelegramAdapter::new(
            "123:ABC".into(),
            vec![],
            Duration::from_secs(1),
            Some(server.uri()),
        );

        let fake_png_bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let result = adapter
            .api_send_photo_upload(42, fake_png_bytes, Some("Screenshot"), "image/png", None)
            .await;
        assert!(result.is_ok(), "upload should succeed, got {result:?}");
    }

    #[tokio::test]
    async fn test_api_send_photo_upload_propagates_error() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendPhoto"))
            .respond_with(ResponseTemplate::new(413))
            .mount(&server)
            .await;

        let adapter = TelegramAdapter::new(
            "123:ABC".into(),
            vec![],
            Duration::from_secs(1),
            Some(server.uri()),
        );

        let result = adapter
            .api_send_photo_upload(42, vec![0; 16], None, "image/png", None)
            .await;
        assert!(result.is_err());
    }

    /// v3.8a — sendPhoto must propagate errors when Telegram returns non-2xx.
    /// Regression guard: before the fix, a 400 response would be `warn!`-logged
    /// and swallowed, letting the agent claim "screenshot sent" when nothing
    /// arrived. Photo path as URL (invalid) → Telegram 400 → must return Err.
    #[tokio::test]
    async fn test_api_send_photo_propagates_400() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendPhoto"))
            .respond_with(ResponseTemplate::new(400).set_body_string(
                r#"{"ok":false,"error_code":400,"description":"Bad Request: wrong file identifier/HTTP URL specified"}"#,
            ))
            .mount(&server)
            .await;

        let adapter = TelegramAdapter::new(
            "123:ABC".into(),
            vec![],
            Duration::from_secs(1),
            Some(server.uri()),
        );

        let result = adapter
            .api_send_photo(42, "/tmp/nonexistent.png", None, None)
            .await;

        assert!(
            result.is_err(),
            "api_send_photo must propagate 400 as Err, got {result:?}"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("400") || err_msg.contains("Bad Request"),
            "error message should expose the HTTP 400 status, got: {err_msg}"
        );
    }

    /// v3.8a — sendDocument must also propagate errors (same pattern).
    #[tokio::test]
    async fn test_api_send_document_propagates_500() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendDocument"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let adapter = TelegramAdapter::new(
            "123:ABC".into(),
            vec![],
            Duration::from_secs(1),
            Some(server.uri()),
        );

        let result = adapter
            .api_send_document(42, "http://example.com/doc.pdf", "doc.pdf", None)
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_parse_telegram_update() {
        let update = serde_json::json!({
            "update_id": 123456,
            "message": {
                "message_id": 42,
                "from": {
                    "id": 111222333,
                    "first_name": "Alice",
                    "last_name": "Smith"
                },
                "chat": {
                    "id": 111222333,
                    "type": "private"
                },
                "date": 1700000000,
                "text": "Hello, agent!"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await
        .unwrap();
        assert_eq!(msg.channel, ChannelType::Telegram);
        assert_eq!(msg.sender.display_name, "Alice Smith");
        assert_eq!(msg.sender.platform_id, "111222333");
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Hello, agent!"));
    }

    #[tokio::test]
    async fn test_parse_telegram_command() {
        let update = serde_json::json!({
            "update_id": 123457,
            "message": {
                "message_id": 43,
                "from": {
                    "id": 111222333,
                    "first_name": "Alice"
                },
                "chat": {
                    "id": 111222333,
                    "type": "private"
                },
                "date": 1700000001,
                "text": "/agent hello-world",
                "entities": [{
                    "type": "bot_command",
                    "offset": 0,
                    "length": 6
                }]
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await
        .unwrap();
        match &msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "agent");
                assert_eq!(args, &["hello-world"]);
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_allowed_users_filter() {
        let update = serde_json::json!({
            "update_id": 123458,
            "message": {
                "message_id": 44,
                "from": {
                    "id": 999,
                    "first_name": "Bob"
                },
                "chat": {
                    "id": 999,
                    "type": "private"
                },
                "date": 1700000002,
                "text": "blocked"
            }
        });

        let client = test_client();

        // B.8 — Empty allowed_users = DENY all (was "allow all" before)
        let msg =
            parse_telegram_update(&update, &[], "fake:token", &client, DEFAULT_API_URL, None).await;
        assert!(
            msg.is_none(),
            "B.8: empty allow_list must reject every user; declare allowed_users = [\"*\"] to opt in"
        );

        // B.8 — Wildcard ["*"] = explicit opt-in to allow all
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await;
        assert!(msg.is_some(), "wildcard must allow every user");

        // Non-matching allowed_users = filter out
        let blocked: Vec<String> = vec!["111".to_string(), "222".to_string()];
        let msg = parse_telegram_update(
            &update,
            &blocked,
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await;
        assert!(msg.is_none());

        // Matching allowed_users = allow
        let allowed: Vec<String> = vec!["999".to_string()];
        let msg = parse_telegram_update(
            &update,
            &allowed,
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await;
        assert!(msg.is_some());
    }

    #[tokio::test]
    async fn test_parse_telegram_edited_message() {
        let update = serde_json::json!({
            "update_id": 123459,
            "edited_message": {
                "message_id": 42,
                "from": {
                    "id": 111222333,
                    "first_name": "Alice",
                    "last_name": "Smith"
                },
                "chat": {
                    "id": 111222333,
                    "type": "private"
                },
                "date": 1700000000,
                "edit_date": 1700000060,
                "text": "Edited message!"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await
        .unwrap();
        assert_eq!(msg.channel, ChannelType::Telegram);
        assert_eq!(msg.sender.display_name, "Alice Smith");
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Edited message!"));
    }

    #[test]
    fn test_backoff_calculation() {
        let b1 = calculate_backoff(Duration::from_secs(1));
        assert_eq!(b1, Duration::from_secs(2));

        let b2 = calculate_backoff(Duration::from_secs(2));
        assert_eq!(b2, Duration::from_secs(4));

        let b3 = calculate_backoff(Duration::from_secs(32));
        assert_eq!(b3, Duration::from_secs(60)); // capped

        let b4 = calculate_backoff(Duration::from_secs(60));
        assert_eq!(b4, Duration::from_secs(60)); // stays at cap
    }

    #[tokio::test]
    async fn test_parse_command_with_botname() {
        let update = serde_json::json!({
            "update_id": 100,
            "message": {
                "message_id": 1,
                "from": { "id": 123, "first_name": "X" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "text": "/agents@mycaptainbot",
                "entities": [{ "type": "bot_command", "offset": 0, "length": 17 }]
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await
        .unwrap();
        match &msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "agents");
                assert!(args.is_empty());
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_location() {
        let update = serde_json::json!({
            "update_id": 200,
            "message": {
                "message_id": 50,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "location": { "latitude": 51.5074, "longitude": -0.1278 }
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await
        .unwrap();
        assert!(matches!(msg.content, ChannelContent::Location { .. }));
    }

    #[tokio::test]
    async fn test_parse_telegram_photo_fallback() {
        // When getFile fails (fake token), photo messages should fall back to
        // a text description rather than being silently dropped.
        let update = serde_json::json!({
            "update_id": 300,
            "message": {
                "message_id": 60,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "photo": [
                    { "file_id": "small_id", "file_unique_id": "a", "width": 90, "height": 90, "file_size": 1234 },
                    { "file_id": "large_id", "file_unique_id": "b", "width": 800, "height": 600, "file_size": 45678 }
                ],
                "caption": "Check this out"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await
        .unwrap();
        // With a fake token, getFile will fail, so we get a text fallback
        match &msg.content {
            ChannelContent::Text(t) => {
                assert!(t.contains("Photo received"));
                assert!(t.contains("Check this out"));
            }
            ChannelContent::Image { caption, .. } => {
                // If somehow the HTTP call succeeded (unlikely with fake token),
                // verify caption was extracted
                assert_eq!(caption.as_deref(), Some("Check this out"));
            }
            other => panic!("Expected Text or Image fallback for photo, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_document_fallback() {
        let update = serde_json::json!({
            "update_id": 301,
            "message": {
                "message_id": 61,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "document": {
                    "file_id": "doc_id",
                    "file_unique_id": "c",
                    "file_name": "report.pdf",
                    "file_size": 102400
                }
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await
        .unwrap();
        match &msg.content {
            ChannelContent::Text(t) => {
                assert!(t.contains("Document received"));
                assert!(t.contains("report.pdf"));
            }
            ChannelContent::File { filename, .. } => {
                assert_eq!(filename, "report.pdf");
            }
            other => panic!("Expected Text or File for document, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_voice_fallback() {
        let update = serde_json::json!({
            "update_id": 302,
            "message": {
                "message_id": 62,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "voice": {
                    "file_id": "voice_id",
                    "file_unique_id": "d",
                    "duration": 15
                }
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await
        .unwrap();
        match &msg.content {
            ChannelContent::Text(t) => {
                assert!(t.contains("Voice message"));
                assert!(t.contains("15s"));
            }
            ChannelContent::Voice {
                duration_seconds, ..
            } => {
                assert_eq!(*duration_seconds, 15);
            }
            other => panic!("Expected Text or Voice for voice message, got {other:?}"),
        }
    }

    /// V.5b (#184) — a Telegram update with a `video` field must produce either a
    /// `ChannelContent::Video` (when getFile resolves) or a Text fallback whose body
    /// announces the failed download. Anything else means the parser dropped the
    /// update — which is exactly the bug V.5b fixes.
    #[tokio::test]
    async fn test_parse_telegram_video_message() {
        let update = serde_json::json!({
            "update_id": 320,
            "message": {
                "message_id": 64,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "video": {
                    "file_id": "video_file_id",
                    "file_unique_id": "vu",
                    "duration": 12,
                    "width": 1280,
                    "height": 720,
                    "mime_type": "video/mp4"
                },
                "caption": "Look at this clip"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await
        .expect("video message must NOT be dropped (V.5b)");
        match &msg.content {
            ChannelContent::Video {
                duration_seconds,
                caption,
                ..
            } => {
                assert_eq!(*duration_seconds, 12);
                assert_eq!(caption.as_deref(), Some("Look at this clip"));
            }
            ChannelContent::Text(t) => {
                // Fallback path when telegram_get_file_url fails (which is what
                // happens with `fake:token` against the real api). The parser
                // emits a French marker so the LLM still knows a video arrived.
                // (The caption is re-attached one layer up in bridge::dispatch_message.)
                assert!(t.contains("Vidéo"), "got: {t}");
                assert!(t.contains("téléchargement échoué"), "got: {t}");
            }
            other => panic!("Expected Video or Text fallback for video message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_video_get_file_error_surfaces_reason() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/getFile"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"ok":false,"error_code":400,"description":"Bad Request: file is too big"}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let update = serde_json::json!({
            "update_id": 322,
            "message": {
                "message_id": 66,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "video": {
                    "file_id": "too_big_video_id",
                    "file_unique_id": "vbig",
                    "duration": 20,
                    "mime_type": "video/mp4"
                }
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "123:ABC",
            &client,
            &server.uri(),
            None,
        )
        .await
        .expect("oversized video must still be surfaced to the agent");

        match &msg.content {
            ChannelContent::Text(t) => {
                assert!(t.contains("Vidéo reçue"), "got: {t}");
                assert!(t.contains("téléchargement échoué"), "got: {t}");
                assert!(t.contains("file is too big"), "got: {t}");
                assert!(
                    !t.contains("123:ABC"),
                    "fallback must not leak the bot token: {t}"
                );
            }
            other => panic!("Expected Text fallback for oversized video, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_video_document_routes_to_video() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/getFile"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"ok":true,"result":{"file_id":"doc_video_id","file_unique_id":"docv","file_path":"videos/from-document.mp4"}}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let update = serde_json::json!({
            "update_id": 323,
            "message": {
                "message_id": 67,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "document": {
                    "file_id": "doc_video_id",
                    "file_unique_id": "docv",
                    "file_name": "clip-original.mp4",
                    "mime_type": "video/mp4",
                    "file_size": 3145728
                },
                "caption": "Analyse cette vidéo"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "123:ABC",
            &client,
            &server.uri(),
            None,
        )
        .await
        .expect("video documents must be handled as videos");

        match &msg.content {
            ChannelContent::Video {
                url,
                duration_seconds,
                caption,
            } => {
                assert_eq!(*duration_seconds, 0);
                assert_eq!(caption.as_deref(), Some("Analyse cette vidéo"));
                assert!(
                    url.ends_with("/file/bot123:ABC/videos/from-document.mp4"),
                    "got: {url}"
                );
            }
            other => panic!("Expected Video for video document, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_video_document_error_surfaces_reason() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/getFile"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"ok":false,"error_code":400,"description":"Bad Request: file is too big"}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let update = serde_json::json!({
            "update_id": 324,
            "message": {
                "message_id": 68,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "document": {
                    "file_id": "doc_video_id",
                    "file_unique_id": "docv",
                    "file_name": "clip-original.mov",
                    "file_size": 31457280
                }
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "123:ABC",
            &client,
            &server.uri(),
            None,
        )
        .await
        .expect("failed video documents must still be surfaced to the agent");

        match &msg.content {
            ChannelContent::Text(t) => {
                assert!(t.contains("Vidéo reçue comme document"), "got: {t}");
                assert!(t.contains("clip-original.mov"), "got: {t}");
                assert!(t.contains("file is too big"), "got: {t}");
            }
            other => panic!("Expected Text fallback for failed video document, got {other:?}"),
        }
    }

    /// V.5b (#184) — Telegram delivers GIFs as `animation` (MP4 under the hood).
    /// They must be parsed the same way as `video` so the LLM gets a path to
    /// the file and can call `video_analyze` on it.
    #[tokio::test]
    async fn test_parse_telegram_animation_message() {
        let update = serde_json::json!({
            "update_id": 321,
            "message": {
                "message_id": 65,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "animation": {
                    "file_id": "anim_file_id",
                    "file_unique_id": "au",
                    "duration": 4,
                    "width": 480,
                    "height": 320,
                    "mime_type": "video/mp4"
                }
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await
        .expect("animation message must NOT be dropped (V.5b)");
        match &msg.content {
            ChannelContent::Video {
                duration_seconds,
                caption,
                ..
            } => {
                assert_eq!(*duration_seconds, 4);
                assert!(caption.is_none());
            }
            ChannelContent::Text(t) => {
                assert!(t.contains("Vidéo") || t.contains("animation"), "got: {t}");
            }
            other => panic!("Expected Video or Text fallback for animation, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_forum_topic_thread_id() {
        // Messages inside a Telegram forum topic include `message_thread_id`.
        let update = serde_json::json!({
            "update_id": 400,
            "message": {
                "message_id": 70,
                "message_thread_id": 42,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": -1001234567890_i64, "type": "supergroup" },
                "date": 1700000000,
                "text": "Hello from a forum topic"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await
        .unwrap();
        assert_eq!(msg.thread_id, Some("42".to_string()));
        assert!(msg.is_group);
    }

    #[tokio::test]
    async fn test_parse_telegram_no_thread_id_in_private_chat() {
        // Private chats should have thread_id = None.
        let update = serde_json::json!({
            "update_id": 401,
            "message": {
                "message_id": 71,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "text": "Hello from DM"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await
        .unwrap();
        assert_eq!(msg.thread_id, None);
        assert!(!msg.is_group);
    }

    #[tokio::test]
    async fn test_parse_telegram_edited_message_in_forum() {
        // Edited messages in forum topics should also preserve thread_id.
        let update = serde_json::json!({
            "update_id": 402,
            "edited_message": {
                "message_id": 72,
                "message_thread_id": 99,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": -1001234567890_i64, "type": "supergroup" },
                "date": 1700000000,
                "edit_date": 1700000060,
                "text": "Edited in forum"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await
        .unwrap();
        assert_eq!(msg.thread_id, Some("99".to_string()));
    }

    #[tokio::test]
    async fn test_parse_sender_chat_fallback() {
        // Messages sent on behalf of a channel have `sender_chat` instead of `from`.
        let update = serde_json::json!({
            "update_id": 500,
            "message": {
                "message_id": 80,
                "sender_chat": {
                    "id": -1001999888777_i64,
                    "title": "My Channel",
                    "type": "channel"
                },
                "chat": { "id": -1001234567890_i64, "type": "supergroup" },
                "date": 1700000000,
                "text": "Forwarded from channel"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await
        .unwrap();
        assert_eq!(msg.sender.display_name, "My Channel");
        assert_eq!(msg.sender.platform_id, "-1001234567890");
        assert!(
            matches!(msg.content, ChannelContent::Text(ref t) if t == "Forwarded from channel")
        );
    }

    #[tokio::test]
    async fn test_parse_no_from_no_sender_chat_drops() {
        // Updates with neither `from` nor `sender_chat` should be dropped with debug logging.
        let update = serde_json::json!({
            "update_id": 501,
            "message": {
                "message_id": 81,
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "text": "orphan"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await;
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_was_mentioned_in_group() {
        // Bot @mentioned in a group message should set metadata["was_mentioned"].
        let update = serde_json::json!({
            "update_id": 600,
            "message": {
                "message_id": 90,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": -1001234567890_i64, "type": "supergroup" },
                "date": 1700000000,
                "text": "Hey @testbot what do you think?",
                "entities": [{
                    "type": "mention",
                    "offset": 4,
                    "length": 8
                }]
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            Some("testbot"),
        )
        .await
        .unwrap();
        assert!(msg.is_group);
        assert_eq!(
            msg.metadata.get("was_mentioned").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn test_not_mentioned_in_group() {
        // Group message without a mention should NOT have was_mentioned.
        let update = serde_json::json!({
            "update_id": 601,
            "message": {
                "message_id": 91,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": -1001234567890_i64, "type": "supergroup" },
                "date": 1700000000,
                "text": "Just chatting"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            Some("testbot"),
        )
        .await
        .unwrap();
        assert!(msg.is_group);
        assert!(!msg.metadata.contains_key("was_mentioned"));
    }

    #[tokio::test]
    async fn test_mentioned_different_bot_not_set() {
        // @mention of a different bot should NOT set was_mentioned.
        let update = serde_json::json!({
            "update_id": 602,
            "message": {
                "message_id": 92,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": -1001234567890_i64, "type": "supergroup" },
                "date": 1700000000,
                "text": "Hey @otherbot what do you think?",
                "entities": [{
                    "type": "mention",
                    "offset": 4,
                    "length": 9
                }]
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            Some("testbot"),
        )
        .await
        .unwrap();
        assert!(msg.is_group);
        assert!(!msg.metadata.contains_key("was_mentioned"));
    }

    #[tokio::test]
    async fn test_mention_in_caption_entities() {
        // Bot mentioned in a photo caption should set was_mentioned.
        let update = serde_json::json!({
            "update_id": 603,
            "message": {
                "message_id": 93,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": -1001234567890_i64, "type": "supergroup" },
                "date": 1700000000,
                "photo": [
                    { "file_id": "photo_id", "file_unique_id": "x", "width": 800, "height": 600 }
                ],
                "caption": "Look @testbot",
                "caption_entities": [{
                    "type": "mention",
                    "offset": 5,
                    "length": 8
                }]
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            Some("testbot"),
        )
        .await
        .unwrap();
        assert!(msg.is_group);
        assert_eq!(
            msg.metadata.get("was_mentioned").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn test_mention_case_insensitive() {
        // Mention detection should be case-insensitive.
        let update = serde_json::json!({
            "update_id": 604,
            "message": {
                "message_id": 94,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": -1001234567890_i64, "type": "supergroup" },
                "date": 1700000000,
                "text": "Hey @TestBot help",
                "entities": [{
                    "type": "mention",
                    "offset": 4,
                    "length": 8
                }]
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            Some("testbot"),
        )
        .await
        .unwrap();
        assert_eq!(
            msg.metadata.get("was_mentioned").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn test_private_chat_no_mention_check() {
        // Private chats should NOT populate was_mentioned even with entities.
        let update = serde_json::json!({
            "update_id": 605,
            "message": {
                "message_id": 95,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "text": "Hey @testbot",
                "entities": [{
                    "type": "mention",
                    "offset": 4,
                    "length": 8
                }]
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            Some("testbot"),
        )
        .await
        .unwrap();
        assert!(!msg.is_group);
        // In private chats, mention detection is skipped — no metadata set
        assert!(!msg.metadata.contains_key("was_mentioned"));
    }

    #[test]
    fn test_check_mention_entities_direct() {
        let message = serde_json::json!({
            "text": "Hello @mybot world",
            "entities": [{
                "type": "mention",
                "offset": 6,
                "length": 6
            }]
        });
        assert!(check_mention_entities(&message, "mybot"));
        assert!(!check_mention_entities(&message, "otherbot"));
    }

    #[tokio::test]
    async fn test_reply_to_message_text_prepended() {
        // When a user replies to a message, the quoted context should be prepended.
        let update = serde_json::json!({
            "update_id": 700,
            "message": {
                "message_id": 100,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "text": "I agree with that",
                "reply_to_message": {
                    "message_id": 99,
                    "from": { "id": 456, "first_name": "Bob" },
                    "chat": { "id": 123, "type": "private" },
                    "date": 1699999990,
                    "text": "We should use Rust"
                }
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await
        .unwrap();
        match &msg.content {
            ChannelContent::Text(t) => {
                assert!(t.starts_with("[Replying to Bob: We should use Rust]\n\n"));
                assert!(t.ends_with("I agree with that"));
            }
            other => panic!("Expected Text, got {other:?}"),
        }
        // reply_to_message_id should be stored in metadata
        assert_eq!(
            msg.metadata
                .get("reply_to_message_id")
                .and_then(|v| v.as_i64()),
            Some(99)
        );
    }

    #[tokio::test]
    async fn test_reply_to_message_with_caption() {
        // reply_to_message that has a caption (e.g. photo) instead of text.
        let update = serde_json::json!({
            "update_id": 701,
            "message": {
                "message_id": 101,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "text": "Nice photo!",
                "reply_to_message": {
                    "message_id": 98,
                    "from": { "id": 456, "first_name": "Carol" },
                    "chat": { "id": 123, "type": "private" },
                    "date": 1699999980,
                    "photo": [{ "file_id": "x", "file_unique_id": "y", "width": 100, "height": 100 }],
                    "caption": "Sunset view"
                }
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await
        .unwrap();
        match &msg.content {
            ChannelContent::Text(t) => {
                assert!(t.starts_with("[Replying to Carol: Sunset view]\n\n"));
                assert!(t.ends_with("Nice photo!"));
            }
            other => panic!("Expected Text, got {other:?}"),
        }
        assert_eq!(
            msg.metadata
                .get("reply_to_message_id")
                .and_then(|v| v.as_i64()),
            Some(98)
        );
    }

    #[tokio::test]
    async fn test_reply_to_message_no_text_no_prepend() {
        // reply_to_message with no text or caption (e.g. sticker) — no prepend, but
        // reply_to_message_id is still stored in metadata.
        let update = serde_json::json!({
            "update_id": 702,
            "message": {
                "message_id": 102,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "text": "What was that?",
                "reply_to_message": {
                    "message_id": 97,
                    "from": { "id": 456, "first_name": "Dave" },
                    "chat": { "id": 123, "type": "private" },
                    "date": 1699999970,
                    "sticker": { "file_id": "stk", "file_unique_id": "z" }
                }
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await
        .unwrap();
        match &msg.content {
            ChannelContent::Text(t) => {
                assert_eq!(t, "What was that?");
            }
            other => panic!("Expected Text, got {other:?}"),
        }
        assert_eq!(
            msg.metadata
                .get("reply_to_message_id")
                .and_then(|v| v.as_i64()),
            Some(97)
        );
    }

    #[tokio::test]
    async fn test_reply_to_message_unknown_sender() {
        // reply_to_message without a `from` field — sender should default to "Unknown".
        let update = serde_json::json!({
            "update_id": 703,
            "message": {
                "message_id": 103,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "text": "Interesting",
                "reply_to_message": {
                    "message_id": 96,
                    "chat": { "id": 123, "type": "private" },
                    "date": 1699999960,
                    "text": "Anonymous message"
                }
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await
        .unwrap();
        match &msg.content {
            ChannelContent::Text(t) => {
                assert!(t.starts_with("[Replying to Unknown: Anonymous message]\n\n"));
                assert!(t.ends_with("Interesting"));
            }
            other => panic!("Expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_no_reply_to_message_unchanged() {
        // Messages without reply_to_message should be unaffected.
        let update = serde_json::json!({
            "update_id": 704,
            "message": {
                "message_id": 104,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "text": "Just a normal message"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(
            &update,
            open_to_all(),
            "fake:token",
            &client,
            DEFAULT_API_URL,
            None,
        )
        .await
        .unwrap();
        match &msg.content {
            ChannelContent::Text(t) => {
                assert_eq!(t, "Just a normal message");
            }
            other => panic!("Expected Text, got {other:?}"),
        }
        assert!(!msg.metadata.contains_key("reply_to_message_id"));
    }

    #[test]
    fn send_message_retry_after_helper_matches_telegram_shape() {
        let body = r#"{"ok":false,"error_code":429,"description":"Too Many Requests: retry after 11","parameters":{"retry_after":11}}"#;
        assert_eq!(telegram_retry_after_seconds(429, body), Some(11));
        assert_eq!(telegram_retry_after_seconds(400, body), None);
    }

    // Regression: build_unhandled_callback_message used to set
    // platform_message_id to the callback_query id instead of the original
    // keyboard message id, breaking reply threading — same bug class fixed
    // earlier for ask_user_answer_callback_message.
    #[test]
    fn unhandled_callback_message_uses_original_message_id_not_callback_id() {
        let callback_query = serde_json::json!({
            "message": {
                "message_id": 999,
                "text": "Pick an option"
            }
        });
        let callback = TelegramCallbackContext {
            id: "cbq-should-not-be-used".to_string(),
            data: "legacy:unknown:payload".to_string(),
            from_id: 456,
            from_name: "Alex".to_string(),
            chat_id: -100123,
            thread_id: Some("topic-7".to_string()),
        };

        let msg = build_unhandled_callback_message(&callback_query, &callback);

        assert_eq!(msg.platform_message_id, "999");
        assert_eq!(msg.metadata["original_message_id"], serde_json::json!(999));
    }

    #[test]
    fn unhandled_callback_message_handles_missing_original_message_id() {
        let callback_query = serde_json::json!({ "message": {} });
        let callback = TelegramCallbackContext {
            id: "cbq-1".to_string(),
            data: "legacy:unknown:payload".to_string(),
            from_id: 456,
            from_name: "Alex".to_string(),
            chat_id: -100123,
            thread_id: None,
        };

        let msg = build_unhandled_callback_message(&callback_query, &callback);

        assert_eq!(msg.platform_message_id, "");
    }
}
