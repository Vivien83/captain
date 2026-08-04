use base64::Engine as _;
use captain_types::email::{GmailAttachmentSummary, GmailLabel, GmailMessageSummary};
use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};

use super::{GmailApiError, GmailApiResult};

const MAX_HEADER_BYTES: usize = 4 * 1024;
const MAX_SNIPPET_BYTES: usize = 2 * 1024;
const MAX_ATTACHMENTS: usize = 100;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MessageListResponse {
    #[serde(default)]
    pub messages: Vec<MessageReference>,
    pub next_page_token: Option<String>,
    #[serde(default)]
    pub result_size_estimate: u64,
}

#[derive(Debug, Deserialize)]
pub(super) struct MessageReference {
    pub id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProfileResponse {
    pub email_address: String,
    pub messages_total: u64,
    pub threads_total: u64,
    pub history_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HistoryListResponse {
    #[serde(default)]
    pub history: Vec<HistoryRecord>,
    pub next_page_token: Option<String>,
    #[serde(default)]
    pub history_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HistoryRecord {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub messages_added: Vec<HistoryMessageAddedResponse>,
}

#[derive(Debug, Deserialize)]
pub(super) struct HistoryMessageAddedResponse {
    pub message: HistoryMessageResponse,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HistoryMessageResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub thread_id: String,
    #[serde(default)]
    pub label_ids: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MessageResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub thread_id: String,
    #[serde(default)]
    pub label_ids: Vec<String>,
    #[serde(default)]
    pub snippet: String,
    #[serde(default)]
    pub internal_date: String,
    #[serde(default)]
    pub size_estimate: u64,
    pub payload: Option<MessagePart>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MessagePart {
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub headers: Vec<MessageHeader>,
    #[serde(default)]
    pub body: MessagePartBody,
    #[serde(default)]
    pub parts: Vec<MessagePart>,
}

#[derive(Debug, Deserialize)]
pub(super) struct MessageHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MessagePartBody {
    pub attachment_id: Option<String>,
    #[serde(default)]
    pub size: u64,
    pub data: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DraftResponse {
    #[serde(default)]
    pub id: String,
    pub message: Option<MessageResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RawMessageRequest<'a> {
    pub raw: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<&'a str>,
}

#[derive(Debug, Serialize)]
pub(super) struct DraftCreateRequest<'a> {
    pub message: RawMessageRequest<'a>,
}

impl<'a> DraftCreateRequest<'a> {
    pub(super) fn new(raw: &'a str, thread_id: Option<&'a str>) -> Self {
        Self {
            message: RawMessageRequest { raw, thread_id },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ModifyMessageRequest<'a> {
    pub add_label_ids: &'a [String],
    pub remove_label_ids: &'a [String],
}

#[derive(Debug, Deserialize)]
pub(super) struct AttachmentResponse {
    #[serde(default)]
    pub data: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct LabelListResponse {
    #[serde(default)]
    pub labels: Vec<LabelResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LabelResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub messages_total: u64,
    #[serde(default)]
    pub messages_unread: u64,
}

#[derive(Debug, Deserialize)]
pub(super) struct GoogleErrorEnvelope {
    pub error: Option<GoogleError>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GoogleError {
    #[serde(default)]
    pub errors: Vec<GoogleErrorDetail>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GoogleErrorDetail {
    pub reason: Option<String>,
}

impl GoogleErrorEnvelope {
    pub(super) fn primary_reason(self) -> Option<String> {
        self.error.and_then(|error| {
            error
                .errors
                .into_iter()
                .find_map(|detail| detail.reason)
                .or(error.status)
        })
    }
}

pub(super) struct DecodedMessage {
    pub summary: GmailMessageSummary,
    pub message_id_header: Option<String>,
    pub references: Option<String>,
    pub reply_to: Option<String>,
    pub body: DecodedBody,
    pub attachments: Vec<GmailAttachmentSummary>,
    pub deferred_body_parts: Vec<DeferredBodyPart>,
}

pub(super) struct DeferredBodyPart {
    pub mime_type: String,
    pub attachment_id: String,
}

#[derive(Default)]
pub(super) struct DecodedBody {
    pub text: Option<String>,
    pub html: Option<String>,
    pub truncated: bool,
}

impl DecodedBody {
    pub(super) fn total_len(&self) -> usize {
        self.text.as_ref().map_or(0, String::len) + self.html.as_ref().map_or(0, String::len)
    }

    pub(super) fn append(&mut self, mime_type: &str, bytes: &[u8], max_bytes: usize) {
        let remaining = max_bytes.saturating_sub(self.total_len());
        if remaining == 0 {
            self.truncated = true;
            return;
        }
        let mut decoded = String::from_utf8_lossy(bytes).into_owned();
        if decoded.len() > remaining {
            let mut boundary = remaining;
            while boundary > 0 && !decoded.is_char_boundary(boundary) {
                boundary -= 1;
            }
            decoded.truncate(boundary);
            self.truncated = true;
        }
        let target = if mime_type.eq_ignore_ascii_case("text/html") {
            &mut self.html
        } else {
            &mut self.text
        };
        match target {
            Some(existing) if !decoded.is_empty() => {
                existing.push('\n');
                existing.push_str(&decoded);
            }
            None if !decoded.is_empty() => *target = Some(decoded),
            _ => {}
        }
    }
}

pub(super) fn summary_from_response(
    response: &MessageResponse,
) -> GmailApiResult<GmailMessageSummary> {
    let payload = response.payload.as_ref();
    Ok(GmailMessageSummary {
        id: validated_identifier(&response.id, "message id")?,
        thread_id: validated_identifier(&response.thread_id, "thread id")?,
        from: payload.and_then(|part| bounded_header(part, "From")),
        to: payload.and_then(|part| bounded_header(part, "To")),
        cc: payload.and_then(|part| bounded_header(part, "Cc")),
        subject: payload.and_then(|part| bounded_header(part, "Subject")),
        received_at: response
            .internal_date
            .parse::<i64>()
            .ok()
            .and_then(|millis| Utc.timestamp_millis_opt(millis).single()),
        snippet: bounded_string(&response.snippet, MAX_SNIPPET_BYTES),
        label_ids: response
            .label_ids
            .iter()
            .take(100)
            .filter_map(|label| validated_identifier(label, "label id").ok())
            .collect(),
        size_estimate: response.size_estimate,
    })
}

pub(super) fn decode_message(
    response: MessageResponse,
    max_body_bytes: usize,
) -> GmailApiResult<DecodedMessage> {
    let summary = summary_from_response(&response)?;
    let payload = response.payload.ok_or_else(|| {
        GmailApiError::InvalidResponse("message response omitted payload".to_string())
    })?;
    let message_id_header = bounded_header(&payload, "Message-ID");
    let references = bounded_header(&payload, "References");
    let reply_to = bounded_header(&payload, "Reply-To");
    let mut body = DecodedBody::default();
    let mut attachments = Vec::new();
    let mut deferred_body_parts = Vec::new();
    decode_part(
        &payload,
        max_body_bytes,
        &mut body,
        &mut attachments,
        &mut deferred_body_parts,
    )?;
    Ok(DecodedMessage {
        summary,
        message_id_header,
        references,
        reply_to,
        body,
        attachments,
        deferred_body_parts,
    })
}

fn decode_part(
    part: &MessagePart,
    max_body_bytes: usize,
    body: &mut DecodedBody,
    attachments: &mut Vec<GmailAttachmentSummary>,
    deferred: &mut Vec<DeferredBodyPart>,
) -> GmailApiResult<()> {
    let mime = part.mime_type.to_ascii_lowercase();
    let is_text_body =
        part.filename.is_empty() && matches!(mime.as_str(), "text/plain" | "text/html");
    if is_text_body {
        if let Some(data) = part.body.data.as_deref() {
            let decoded = decode_base64url(data)?;
            body.append(&mime, &decoded, max_body_bytes);
        } else if let Some(attachment_id) = part.body.attachment_id.as_deref() {
            deferred.push(DeferredBodyPart {
                mime_type: mime.clone(),
                attachment_id: validated_identifier(attachment_id, "attachment id")?,
            });
        }
    } else if let Some(attachment_id) = part.body.attachment_id.as_deref() {
        if attachments.len() >= MAX_ATTACHMENTS {
            body.truncated = true;
        } else {
            attachments.push(GmailAttachmentSummary {
                attachment_id: validated_identifier(attachment_id, "attachment id")?,
                filename: bounded_filename(&part.filename, attachments.len() + 1),
                mime_type: bounded_mime_type(&mime),
                size: part.body.size,
            });
        }
    }
    for child in &part.parts {
        decode_part(child, max_body_bytes, body, attachments, deferred)?;
    }
    Ok(())
}

pub(super) fn label_from_response(label: LabelResponse) -> GmailLabel {
    GmailLabel {
        id: bounded_string(&label.id, 256),
        name: bounded_string(&label.name, 512),
        kind: bounded_string(&label.kind, 32),
        messages_total: label.messages_total,
        messages_unread: label.messages_unread,
    }
}

fn bounded_header(part: &MessagePart, name: &str) -> Option<String> {
    part.headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case(name))
        .map(|header| bounded_string(&header.value, MAX_HEADER_BYTES))
        .filter(|value| !value.is_empty())
}

fn bounded_string(value: &str, max_bytes: usize) -> String {
    let mut end = value.len().min(max_bytes);
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end]
        .chars()
        .filter(|character| !matches!(character, '\0' | '\u{000b}' | '\u{000c}'))
        .collect()
}

fn bounded_filename(value: &str, sequence: usize) -> String {
    let candidate = if value.trim().is_empty() {
        format!("attachment-{sequence}")
    } else {
        value
            .chars()
            .map(|character| {
                if character.is_control() || matches!(character, '/' | '\\') {
                    '_'
                } else {
                    character
                }
            })
            .collect()
    };
    bounded_string(&candidate, 255)
}

fn bounded_mime_type(value: &str) -> String {
    let candidate = bounded_string(value, 127);
    if candidate.is_empty() {
        "application/octet-stream".to_string()
    } else {
        candidate
    }
}

fn validated_identifier(value: &str, name: &str) -> GmailApiResult<String> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(GmailApiError::InvalidResponse(format!(
            "{name} was missing or malformed"
        )));
    }
    Ok(value.to_string())
}

fn decode_base64url(value: &str) -> GmailApiResult<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(value))
        .map_err(|_| GmailApiError::InvalidResponse("invalid base64url message part".to_string()))
}
