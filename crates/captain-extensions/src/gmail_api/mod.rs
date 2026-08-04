//! Bounded native Gmail REST client used by the kernel email service.

mod mime;
mod wire;

use base64::Engine as _;
use captain_types::email::{
    GmailAttachmentData, GmailLabel, GmailMessageSummary, GmailSearchRequest, GmailUpdateResult,
};
use futures::{stream, StreamExt, TryStreamExt};
use reqwest::{header, Method, RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use std::collections::HashSet;
use std::time::Duration;
use zeroize::Zeroizing;

pub use mime::{build_gmail_mime, GmailThreadingHeaders};
use wire::{
    AttachmentResponse, DraftResponse, GoogleErrorEnvelope, HistoryListResponse, LabelListResponse,
    MessageListResponse, MessageResponse, ProfileResponse,
};

const GMAIL_API_BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
const MAX_METADATA_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_MESSAGE_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_LABEL_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_HISTORY_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_ERROR_RESPONSE_BYTES: usize = 8 * 1024;
const MAX_QUERY_BYTES: usize = 1_024;
const MAX_PAGE_TOKEN_BYTES: usize = 2_048;
const MAX_IDENTIFIER_BYTES: usize = 256;
const SUMMARY_CONCURRENCY: usize = 4;
const MAX_HISTORY_MESSAGES_PER_PAGE: usize = 2_000;

pub type GmailApiResult<T> = Result<T, GmailApiError>;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum GmailApiError {
    #[error("Gmail authorization expired or was revoked; reconnect the account")]
    Unauthorized,
    #[error("The connected Gmail grant does not permit this operation")]
    PermissionDenied,
    #[error("Gmail API rate limit reached; retry later")]
    RateLimited { retry_after_seconds: Option<u64> },
    #[error("Gmail API is temporarily unavailable (HTTP {status})")]
    Transient { status: u16 },
    #[error("Gmail API rejected the request (HTTP {status}, reason: {reason})")]
    Rejected { status: u16, reason: String },
    #[error("Gmail history cursor expired; a bounded full synchronization is required")]
    HistoryExpired,
    #[error("Gmail API transport failed")]
    Transport,
    #[error("Gmail API returned invalid data: {0}")]
    InvalidResponse(String),
    #[error("Invalid Gmail request: {0}")]
    InvalidInput(String),
    #[error("Gmail response exceeded the {limit} byte safety limit")]
    ResponseTooLarge { limit: usize },
}

impl GmailApiError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unauthorized => "gmail_unauthorized",
            Self::PermissionDenied => "gmail_permission_denied",
            Self::RateLimited { .. } => "gmail_rate_limited",
            Self::Transient { .. } | Self::Transport => "gmail_transient",
            Self::Rejected { .. } => "gmail_rejected",
            Self::HistoryExpired => "gmail_history_expired",
            Self::InvalidResponse(_) => "gmail_invalid_response",
            Self::InvalidInput(_) => "gmail_invalid_input",
            Self::ResponseTooLarge { .. } => "gmail_response_too_large",
        }
    }

    pub fn requires_reauthorization(&self) -> bool {
        matches!(self, Self::Unauthorized)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailSearchPage {
    pub messages: Vec<GmailMessageSummary>,
    pub next_page_token: Option<String>,
    pub result_size_estimate: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailMessageIdPage {
    pub message_ids: Vec<String>,
    pub next_page_token: Option<String>,
    pub result_size_estimate: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailHistoryRequest {
    pub start_history_id: String,
    pub page_token: Option<String>,
    pub max_results: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailHistoryMessageAdded {
    pub history_id: String,
    pub message_id: String,
    pub thread_id: String,
    pub label_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailHistoryPage {
    pub messages_added: Vec<GmailHistoryMessageAdded>,
    pub next_page_token: Option<String>,
    pub history_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailMailboxProfile {
    pub email_address: String,
    pub messages_total: u64,
    pub threads_total: u64,
    pub history_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailDecodedMessage {
    pub summary: GmailMessageSummary,
    pub message_id_header: Option<String>,
    pub references: Option<String>,
    pub reply_to: Option<String>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub body_truncated: bool,
    pub attachments: Vec<captain_types::email::GmailAttachmentSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailReplyMetadata {
    pub summary: GmailMessageSummary,
    pub message_id_header: Option<String>,
    pub references: Option<String>,
    pub reply_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailDeliveryReceipt {
    pub id: String,
    pub message_id: String,
    pub thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailModifyReceipt {
    pub id: String,
    pub thread_id: String,
    pub label_ids: Vec<String>,
}

/// HTTPS-only Gmail API client. The bearer token is zeroized on drop and the
/// production base URL cannot be redirected or overridden.
pub struct GmailApiClient {
    http: reqwest::Client,
    access_token: Zeroizing<String>,
    base_url: String,
}

impl GmailApiClient {
    pub fn new(access_token: &str) -> GmailApiResult<Self> {
        Self::with_base_url(access_token, GMAIL_API_BASE)
    }

    fn with_base_url(access_token: &str, base_url: &str) -> GmailApiResult<Self> {
        if access_token.is_empty() || access_token.len() > 16 * 1024 {
            return Err(GmailApiError::InvalidInput(
                "access token is empty or oversized".to_string(),
            ));
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(45))
            .user_agent(concat!("Captain/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| GmailApiError::Transport)?;
        Ok(Self {
            http,
            access_token: Zeroizing::new(access_token.to_string()),
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    #[cfg(test)]
    fn for_test(access_token: &str, base_url: &str) -> GmailApiResult<Self> {
        Self::with_base_url(access_token, base_url)
    }

    pub async fn search_messages(
        &self,
        request: &GmailSearchRequest,
    ) -> GmailApiResult<GmailSearchPage> {
        let GmailMessageIdPage {
            message_ids,
            next_page_token,
            result_size_estimate,
        } = self.list_message_ids(request).await?;
        let messages = stream::iter(
            message_ids
                .into_iter()
                .map(|message_id| async move { self.message_summary(&message_id).await }),
        )
        .buffered(SUMMARY_CONCURRENCY)
        .try_collect::<Vec<_>>()
        .await?;
        Ok(GmailSearchPage {
            messages,
            next_page_token,
            result_size_estimate,
        })
    }

    /// List only immutable Gmail message identifiers. This lets durable sync
    /// callers checkpoint the page independently from metadata fetches.
    pub async fn list_message_ids(
        &self,
        request: &GmailSearchRequest,
    ) -> GmailApiResult<GmailMessageIdPage> {
        validate_search_request(request)?;
        let mut builder = self.authorized(Method::GET, "/messages").query(&[
            ("maxResults", request.max_results.to_string()),
            ("includeSpamTrash", request.include_spam_trash.to_string()),
        ]);
        if !request.query.is_empty() {
            builder = builder.query(&[("q", request.query.as_str())]);
        }
        if let Some(page_token) = request.page_token.as_deref() {
            builder = builder.query(&[("pageToken", page_token)]);
        }
        for label_id in &request.label_ids {
            builder = builder.query(&[("labelIds", label_id)]);
        }
        let listed: MessageListResponse = self
            .execute_json(builder, MAX_METADATA_RESPONSE_BYTES)
            .await?;
        let mut seen = HashSet::new();
        let mut message_ids = Vec::with_capacity(listed.messages.len());
        for message in listed.messages {
            let id = required_identifier(message.id, "listed message id")?;
            if seen.insert(id.clone()) {
                message_ids.push(id);
            }
        }
        if message_ids.len() > usize::from(request.max_results) {
            return Err(GmailApiError::InvalidResponse(
                "message list exceeded the requested page size".to_string(),
            ));
        }
        let next_page_token =
            validate_page_token_response(listed.next_page_token, "message list page token")?;
        Ok(GmailMessageIdPage {
            message_ids,
            next_page_token,
            result_size_estimate: listed.result_size_estimate,
        })
    }

    pub async fn mailbox_profile(&self) -> GmailApiResult<GmailMailboxProfile> {
        let response: ProfileResponse = self
            .execute_json(
                self.authorized(Method::GET, "/profile"),
                MAX_METADATA_RESPONSE_BYTES,
            )
            .await?;
        validate_text("profile email address", &response.email_address, 320, false).map_err(
            |_| GmailApiError::InvalidResponse("profile email address was malformed".to_string()),
        )?;
        if !response.email_address.contains('@') {
            return Err(GmailApiError::InvalidResponse(
                "profile email address was malformed".to_string(),
            ));
        }
        Ok(GmailMailboxProfile {
            email_address: response.email_address,
            messages_total: response.messages_total,
            threads_total: response.threads_total,
            history_id: validate_history_id_response(&response.history_id, "mailbox history id")?,
        })
    }

    pub async fn list_history(
        &self,
        request: &GmailHistoryRequest,
    ) -> GmailApiResult<GmailHistoryPage> {
        validate_history_request(request)?;
        let mut builder = self.authorized(Method::GET, "/history").query(&[
            ("startHistoryId", request.start_history_id.clone()),
            ("maxResults", request.max_results.to_string()),
            ("historyTypes", "messageAdded".to_string()),
        ]);
        if let Some(page_token) = request.page_token.as_deref() {
            builder = builder.query(&[("pageToken", page_token)]);
        }
        let listed: HistoryListResponse =
            match self.execute_json(builder, MAX_HISTORY_RESPONSE_BYTES).await {
                Err(GmailApiError::Rejected { status: 404, .. }) => {
                    return Err(GmailApiError::HistoryExpired);
                }
                result => result?,
            };
        decode_history_page(listed)
    }

    pub async fn read_message(
        &self,
        message_id: &str,
        max_body_bytes: usize,
    ) -> GmailApiResult<GmailDecodedMessage> {
        validate_identifier("message_id", message_id)?;
        if !(1..=256 * 1024).contains(&max_body_bytes) {
            return Err(GmailApiError::InvalidInput(
                "max_body_bytes must be between 1 and 262144".to_string(),
            ));
        }
        let path = format!("/messages/{message_id}");
        let response: MessageResponse = self
            .execute_json(
                self.authorized(Method::GET, &path)
                    .query(&[("format", "full")]),
                MAX_MESSAGE_RESPONSE_BYTES,
            )
            .await?;
        let mut decoded = wire::decode_message(response, max_body_bytes)?;
        for deferred in std::mem::take(&mut decoded.deferred_body_parts) {
            if decoded.body.total_len() >= max_body_bytes {
                decoded.body.truncated = true;
                break;
            }
            let remaining = max_body_bytes - decoded.body.total_len();
            let attachment = self
                .download_attachment(message_id, &deferred.attachment_id, remaining)
                .await?;
            decoded
                .body
                .append(&deferred.mime_type, &attachment.data, max_body_bytes);
        }
        Ok(GmailDecodedMessage {
            summary: decoded.summary,
            message_id_header: decoded.message_id_header,
            references: decoded.references,
            reply_to: decoded.reply_to,
            body_text: decoded.body.text,
            body_html: decoded.body.html,
            body_truncated: decoded.body.truncated,
            attachments: decoded.attachments,
        })
    }

    /// Fetch only the headers required to build a threaded reply. This never
    /// downloads body parts or attachment bytes.
    pub async fn read_reply_metadata(
        &self,
        message_id: &str,
    ) -> GmailApiResult<GmailReplyMetadata> {
        validate_identifier("message_id", message_id)?;
        let path = format!("/messages/{message_id}");
        let mut builder = self
            .authorized(Method::GET, &path)
            .query(&[("format", "metadata")]);
        for header_name in [
            "From",
            "To",
            "Cc",
            "Subject",
            "Message-ID",
            "References",
            "Reply-To",
        ] {
            builder = builder.query(&[("metadataHeaders", header_name)]);
        }
        let response: MessageResponse = self
            .execute_json(builder, MAX_METADATA_RESPONSE_BYTES)
            .await?;
        let decoded = wire::decode_message(response, 1)?;
        Ok(GmailReplyMetadata {
            summary: decoded.summary,
            message_id_header: decoded.message_id_header,
            references: decoded.references,
            reply_to: decoded.reply_to,
        })
    }

    pub async fn list_labels(&self) -> GmailApiResult<Vec<GmailLabel>> {
        let response: LabelListResponse = self
            .execute_json(
                self.authorized(Method::GET, "/labels"),
                MAX_LABEL_RESPONSE_BYTES,
            )
            .await?;
        Ok(response
            .labels
            .into_iter()
            .map(wire::label_from_response)
            .collect())
    }

    pub async fn send_message(
        &self,
        raw: &str,
        thread_id: Option<&str>,
    ) -> GmailApiResult<GmailDeliveryReceipt> {
        let response: MessageResponse = self
            .execute_json(
                self.authorized(Method::POST, "/messages/send")
                    .json(&wire::RawMessageRequest { raw, thread_id }),
                MAX_METADATA_RESPONSE_BYTES,
            )
            .await?;
        receipt_from_message(response)
    }

    pub async fn create_draft(
        &self,
        raw: &str,
        thread_id: Option<&str>,
    ) -> GmailApiResult<GmailDeliveryReceipt> {
        let response: DraftResponse = self
            .execute_json(
                self.authorized(Method::POST, "/drafts")
                    .json(&wire::DraftCreateRequest::new(raw, thread_id)),
                MAX_METADATA_RESPONSE_BYTES,
            )
            .await?;
        let message = response.message.ok_or_else(|| {
            GmailApiError::InvalidResponse("draft response omitted message".to_string())
        })?;
        let message_id = required_identifier(message.id, "draft message id")?;
        let thread_id = required_identifier(message.thread_id, "draft thread id")?;
        Ok(GmailDeliveryReceipt {
            id: required_identifier(response.id, "draft id")?,
            message_id,
            thread_id,
        })
    }

    pub async fn modify_message(
        &self,
        message_id: &str,
        add_label_ids: &[String],
        remove_label_ids: &[String],
    ) -> GmailApiResult<GmailModifyReceipt> {
        validate_identifier("message_id", message_id)?;
        validate_label_changes(add_label_ids, remove_label_ids)?;
        let path = format!("/messages/{message_id}/modify");
        let response: MessageResponse = self
            .execute_json(
                self.authorized(Method::POST, &path)
                    .json(&wire::ModifyMessageRequest {
                        add_label_ids,
                        remove_label_ids,
                    }),
                MAX_METADATA_RESPONSE_BYTES,
            )
            .await?;
        modify_receipt(response)
    }

    pub async fn set_trashed(
        &self,
        message_id: &str,
        trashed: bool,
    ) -> GmailApiResult<GmailModifyReceipt> {
        validate_identifier("message_id", message_id)?;
        let action = if trashed { "trash" } else { "untrash" };
        let path = format!("/messages/{message_id}/{action}");
        let response: MessageResponse = self
            .execute_json(
                self.authorized(Method::POST, &path)
                    .json(&serde_json::json!({})),
                MAX_METADATA_RESPONSE_BYTES,
            )
            .await?;
        modify_receipt(response)
    }

    pub async fn download_attachment(
        &self,
        message_id: &str,
        attachment_id: &str,
        max_bytes: usize,
    ) -> GmailApiResult<GmailAttachmentData> {
        validate_identifier("message_id", message_id)?;
        validate_identifier("attachment_id", attachment_id)?;
        if max_bytes == 0 || max_bytes > 20 * 1024 * 1024 {
            return Err(GmailApiError::InvalidInput(
                "attachment limit must be between 1 byte and 20 MiB".to_string(),
            ));
        }
        let path = format!("/messages/{message_id}/attachments/{attachment_id}");
        let encoded_limit = max_bytes.saturating_mul(4).saturating_div(3) + 64 * 1024;
        let response: AttachmentResponse = self
            .execute_json(self.authorized(Method::GET, &path), encoded_limit)
            .await?;
        let data = decode_base64url(&response.data)?;
        if data.len() > max_bytes {
            return Err(GmailApiError::ResponseTooLarge { limit: max_bytes });
        }
        Ok(GmailAttachmentData { data })
    }

    /// Fetch bounded message metadata without downloading body or attachment
    /// bytes. Used by deterministic automation matching.
    pub async fn message_summary(&self, message_id: &str) -> GmailApiResult<GmailMessageSummary> {
        validate_identifier("message_id", message_id)?;
        let path = format!("/messages/{message_id}");
        let mut builder = self
            .authorized(Method::GET, &path)
            .query(&[("format", "metadata")]);
        for header_name in ["From", "To", "Cc", "Subject"] {
            builder = builder.query(&[("metadataHeaders", header_name)]);
        }
        let response: MessageResponse = self
            .execute_json(builder, MAX_METADATA_RESPONSE_BYTES)
            .await?;
        wire::summary_from_response(&response)
    }

    fn authorized(&self, method: Method, path: &str) -> RequestBuilder {
        self.http
            .request(method, format!("{}{}", self.base_url, path))
            .bearer_auth(self.access_token.as_str())
            .header(header::ACCEPT, "application/json")
    }

    async fn execute_json<T: DeserializeOwned>(
        &self,
        builder: RequestBuilder,
        max_bytes: usize,
    ) -> GmailApiResult<T> {
        let response = builder.send().await.map_err(|_| GmailApiError::Transport)?;
        let status = response.status();
        let retry_after_seconds = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        let body_limit = if status.is_success() {
            max_bytes
        } else {
            MAX_ERROR_RESPONSE_BYTES
        };
        let bytes = collect_response_body(response, body_limit).await?;
        if !status.is_success() {
            return Err(classify_http_error(status, retry_after_seconds, &bytes));
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| GmailApiError::InvalidResponse("response was not valid JSON".to_string()))
    }
}

fn validate_search_request(request: &GmailSearchRequest) -> GmailApiResult<()> {
    if !(1..=50).contains(&request.max_results) {
        return Err(GmailApiError::InvalidInput(
            "max_results must be between 1 and 50".to_string(),
        ));
    }
    validate_text("query", &request.query, MAX_QUERY_BYTES, true)?;
    if request.label_ids.len() > 20 {
        return Err(GmailApiError::InvalidInput(
            "at most 20 label_ids may be supplied".to_string(),
        ));
    }
    for label_id in &request.label_ids {
        validate_identifier("label_id", label_id)?;
    }
    if let Some(page_token) = request.page_token.as_deref() {
        validate_text("page_token", page_token, MAX_PAGE_TOKEN_BYTES, false)?;
    }
    Ok(())
}

fn validate_history_request(request: &GmailHistoryRequest) -> GmailApiResult<()> {
    validate_history_id_input(&request.start_history_id)?;
    if !(1..=500).contains(&request.max_results) {
        return Err(GmailApiError::InvalidInput(
            "history max_results must be between 1 and 500".to_string(),
        ));
    }
    if let Some(page_token) = request.page_token.as_deref() {
        validate_text("page_token", page_token, MAX_PAGE_TOKEN_BYTES, false)?;
    }
    Ok(())
}

fn decode_history_page(listed: HistoryListResponse) -> GmailApiResult<GmailHistoryPage> {
    let history_id = validate_history_id_response(&listed.history_id, "mailbox history id")?;
    let next_page_token =
        validate_page_token_response(listed.next_page_token, "history page token")?;
    let mut seen = HashSet::new();
    let mut messages_added = Vec::new();
    for record in listed.history {
        let record_id = validate_history_id_response(&record.id, "history record id")?;
        for added in record.messages_added {
            let message_id = required_identifier(added.message.id, "history message id")?;
            if !seen.insert(message_id.clone()) {
                continue;
            }
            if messages_added.len() >= MAX_HISTORY_MESSAGES_PER_PAGE {
                return Err(GmailApiError::InvalidResponse(
                    "history page exceeded the message safety limit".to_string(),
                ));
            }
            let thread_id = required_identifier(added.message.thread_id, "history thread id")?;
            let mut label_ids = Vec::new();
            for label in added.message.label_ids.into_iter().take(100) {
                validate_identifier("history label id", &label).map_err(|_| {
                    GmailApiError::InvalidResponse(
                        "history message contained a malformed label id".to_string(),
                    )
                })?;
                if !label_ids.contains(&label) {
                    label_ids.push(label);
                }
            }
            messages_added.push(GmailHistoryMessageAdded {
                history_id: record_id.clone(),
                message_id,
                thread_id,
                label_ids,
            });
        }
    }
    Ok(GmailHistoryPage {
        messages_added,
        next_page_token,
        history_id,
    })
}

fn validate_page_token_response(
    value: Option<String>,
    name: &str,
) -> GmailApiResult<Option<String>> {
    if value.as_deref().is_some_and(|token| {
        token.is_empty()
            || token.len() > MAX_PAGE_TOKEN_BYTES
            || token.chars().any(char::is_control)
    }) {
        return Err(GmailApiError::InvalidResponse(format!(
            "{name} was malformed"
        )));
    }
    Ok(value)
}

fn validate_history_id_input(value: &str) -> GmailApiResult<()> {
    if value.is_empty() || value.len() > 128 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(GmailApiError::InvalidInput(
            "history id must contain 1 to 128 digits".to_string(),
        ));
    }
    Ok(())
}

fn validate_history_id_response(value: &str, name: &str) -> GmailApiResult<String> {
    if value.is_empty() || value.len() > 128 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(GmailApiError::InvalidResponse(format!(
            "{name} was missing or malformed"
        )));
    }
    Ok(value.to_string())
}

fn validate_label_changes(add: &[String], remove: &[String]) -> GmailApiResult<()> {
    if add.is_empty() && remove.is_empty() {
        return Err(GmailApiError::InvalidInput(
            "at least one label change is required".to_string(),
        ));
    }
    if add.len() > 100 || remove.len() > 100 {
        return Err(GmailApiError::InvalidInput(
            "at most 100 labels may be changed at once".to_string(),
        ));
    }
    for label in add.iter().chain(remove) {
        validate_identifier("label_id", label)?;
    }
    if add.iter().any(|label| remove.contains(label)) {
        return Err(GmailApiError::InvalidInput(
            "a label cannot be added and removed in the same request".to_string(),
        ));
    }
    Ok(())
}

fn validate_identifier(name: &str, value: &str) -> GmailApiResult<()> {
    validate_text(name, value, MAX_IDENTIFIER_BYTES, false)?;
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(GmailApiError::InvalidInput(format!(
            "{name} contains unsupported characters"
        )));
    }
    Ok(())
}

fn validate_text(name: &str, value: &str, max: usize, allow_empty: bool) -> GmailApiResult<()> {
    if (!allow_empty && value.is_empty())
        || value.len() > max
        || value.chars().any(char::is_control)
    {
        return Err(GmailApiError::InvalidInput(format!(
            "{name} is empty, oversized, or contains control characters"
        )));
    }
    Ok(())
}

async fn collect_response_body(
    response: reqwest::Response,
    limit: usize,
) -> GmailApiResult<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|content_length| content_length > limit as u64)
    {
        return Err(GmailApiError::ResponseTooLarge { limit });
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| GmailApiError::Transport)?;
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(GmailApiError::ResponseTooLarge { limit });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn classify_http_error(
    status: StatusCode,
    retry_after_seconds: Option<u64>,
    body: &[u8],
) -> GmailApiError {
    if status == StatusCode::UNAUTHORIZED {
        return GmailApiError::Unauthorized;
    }
    let reason = serde_json::from_slice::<GoogleErrorEnvelope>(body)
        .ok()
        .and_then(GoogleErrorEnvelope::primary_reason)
        .map(sanitize_reason)
        .unwrap_or_else(|| "request_rejected".to_string());
    if status == StatusCode::TOO_MANY_REQUESTS
        || matches!(
            reason.as_str(),
            "rateLimitExceeded" | "userRateLimitExceeded" | "dailyLimitExceeded"
        )
    {
        return GmailApiError::RateLimited {
            retry_after_seconds,
        };
    }
    if status == StatusCode::FORBIDDEN {
        return GmailApiError::PermissionDenied;
    }
    if status == StatusCode::REQUEST_TIMEOUT || status.as_u16() == 425 || status.is_server_error() {
        return GmailApiError::Transient {
            status: status.as_u16(),
        };
    }
    GmailApiError::Rejected {
        status: status.as_u16(),
        reason,
    }
}

fn sanitize_reason(reason: String) -> String {
    let filtered = reason
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '_')
        .take(64)
        .collect::<String>();
    if filtered.is_empty() {
        "request_rejected".to_string()
    } else {
        filtered
    }
}

fn decode_base64url(value: &str) -> GmailApiResult<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(value))
        .map_err(|_| GmailApiError::InvalidResponse("invalid base64url body".to_string()))
}

fn receipt_from_message(response: MessageResponse) -> GmailApiResult<GmailDeliveryReceipt> {
    let id = required_identifier(response.id, "message id")?;
    Ok(GmailDeliveryReceipt {
        id: id.clone(),
        message_id: id,
        thread_id: required_identifier(response.thread_id, "thread id")?,
    })
}

fn modify_receipt(response: MessageResponse) -> GmailApiResult<GmailModifyReceipt> {
    Ok(GmailModifyReceipt {
        id: required_identifier(response.id, "message id")?,
        thread_id: required_identifier(response.thread_id, "thread id")?,
        label_ids: response.label_ids,
    })
}

fn required_identifier(value: String, name: &str) -> GmailApiResult<String> {
    validate_identifier(name, &value)?;
    Ok(value)
}

pub fn update_result_from_receipt(
    account_alias: captain_types::email::GmailAccountAlias,
    email_address: String,
    receipt: GmailModifyReceipt,
) -> GmailUpdateResult {
    GmailUpdateResult {
        account_alias,
        email_address,
        id: receipt.id,
        thread_id: receipt.thread_id,
        label_ids: receipt.label_ids,
    }
}

#[cfg(test)]
mod tests;
