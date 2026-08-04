use base64::Engine as _;
use captain_types::email::{GmailComposeRequest, GmailOutgoingAttachment};
use lettre::message::{header::ContentType, Attachment, Mailbox, MultiPart, SinglePart};
use zeroize::{Zeroize, Zeroizing};

use super::{GmailApiError, GmailApiResult};

const MAX_RECIPIENTS: usize = 50;
const MAX_SUBJECT_BYTES: usize = 998;
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_ATTACHMENTS: usize = 10;
const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailThreadingHeaders {
    pub thread_id: String,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
}

pub fn build_gmail_mime(
    request: &GmailComposeRequest,
    from: &str,
    threading: Option<&GmailThreadingHeaders>,
) -> GmailApiResult<Zeroizing<String>> {
    validate_compose(request)?;
    let from_mailbox = parse_mailbox("from", from)?;
    let mut builder = lettre::Message::builder()
        .from(from_mailbox)
        .subject(request.subject.clone());
    for recipient in &request.to {
        builder = builder.to(parse_mailbox("to", recipient)?);
    }
    for recipient in &request.cc {
        builder = builder.cc(parse_mailbox("cc", recipient)?);
    }
    for recipient in &request.bcc {
        builder = builder.bcc(parse_mailbox("bcc", recipient)?);
    }
    if !request.bcc.is_empty() {
        builder = builder.keep_bcc();
    }
    if let Some(reply_to) = request.reply_to.as_deref() {
        builder = builder.reply_to(parse_mailbox("reply_to", reply_to)?);
    }
    if let Some(threading) = threading {
        if let Some(in_reply_to) = threading.in_reply_to.as_ref() {
            builder = builder.in_reply_to(validate_message_header(in_reply_to, "In-Reply-To")?);
        }
        if let Some(references) = threading.references.as_ref() {
            builder = builder.references(validate_message_header(references, "References")?);
        }
    }

    let message = if request.attachments.is_empty() {
        if let Some(html) = request.html_body.clone() {
            builder
                .multipart(MultiPart::alternative_plain_html(
                    request.text_body.clone(),
                    html,
                ))
                .map_err(|_| GmailApiError::InvalidInput("email MIME could not be built".into()))?
        } else {
            builder
                .singlepart(SinglePart::plain(request.text_body.clone()))
                .map_err(|_| GmailApiError::InvalidInput("email MIME could not be built".into()))?
        }
    } else {
        let content = if let Some(html) = request.html_body.clone() {
            MultiPart::alternative_plain_html(request.text_body.clone(), html)
        } else {
            MultiPart::alternative().singlepart(SinglePart::plain(request.text_body.clone()))
        };
        let mut mixed = MultiPart::mixed().multipart(content);
        for attachment in &request.attachments {
            mixed = mixed.singlepart(attachment_part(attachment)?);
        }
        builder
            .multipart(mixed)
            .map_err(|_| GmailApiError::InvalidInput("email MIME could not be built".into()))?
    };

    let mut formatted = Zeroizing::new(message.formatted());
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(formatted.as_slice());
    formatted.zeroize();
    Ok(Zeroizing::new(encoded))
}

fn validate_compose(request: &GmailComposeRequest) -> GmailApiResult<()> {
    let recipients = request.to.len() + request.cc.len() + request.bcc.len();
    if recipients == 0 || recipients > MAX_RECIPIENTS {
        return Err(GmailApiError::InvalidInput(format!(
            "email requires 1 to {MAX_RECIPIENTS} recipients"
        )));
    }
    if request.subject.len() > MAX_SUBJECT_BYTES
        || request.subject.contains(['\r', '\n'])
        || request.subject.chars().any(|character| character == '\0')
    {
        return Err(GmailApiError::InvalidInput(
            "subject is oversized or contains header control characters".to_string(),
        ));
    }
    let body_bytes = request.text_body.len() + request.html_body.as_ref().map_or(0, String::len);
    if body_bytes == 0 || body_bytes > MAX_BODY_BYTES {
        return Err(GmailApiError::InvalidInput(format!(
            "email body must contain 1 to {MAX_BODY_BYTES} bytes"
        )));
    }
    if request.attachments.len() > MAX_ATTACHMENTS {
        return Err(GmailApiError::InvalidInput(format!(
            "at most {MAX_ATTACHMENTS} attachments may be sent"
        )));
    }
    let total_attachment_bytes =
        request
            .attachments
            .iter()
            .try_fold(0usize, |total, attachment| {
                total.checked_add(attachment.data.len()).ok_or_else(|| {
                    GmailApiError::InvalidInput("attachment size overflow".to_string())
                })
            })?;
    if total_attachment_bytes > MAX_ATTACHMENT_BYTES {
        return Err(GmailApiError::InvalidInput(format!(
            "attachments exceed the {MAX_ATTACHMENT_BYTES} byte Gmail safety limit"
        )));
    }
    Ok(())
}

fn attachment_part(attachment: &GmailOutgoingAttachment) -> GmailApiResult<SinglePart> {
    if attachment.filename.is_empty()
        || attachment.filename.len() > 255
        || attachment
            .filename
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
    {
        return Err(GmailApiError::InvalidInput(
            "attachment filename is empty, oversized, or unsafe".to_string(),
        ));
    }
    let content_type = ContentType::parse(&attachment.mime_type).map_err(|_| {
        GmailApiError::InvalidInput(format!(
            "attachment '{}' has an invalid MIME type",
            attachment.filename
        ))
    })?;
    Ok(Attachment::new(attachment.filename.clone()).body(attachment.data.clone(), content_type))
}

fn parse_mailbox(field: &str, value: &str) -> GmailApiResult<Mailbox> {
    if value.len() > 512 || value.contains(['\r', '\n']) {
        return Err(GmailApiError::InvalidInput(format!(
            "{field} address is oversized or contains control characters"
        )));
    }
    value
        .parse::<Mailbox>()
        .map_err(|_| GmailApiError::InvalidInput(format!("{field} address is invalid")))
}

fn validate_message_header(value: &str, name: &str) -> GmailApiResult<String> {
    if value.is_empty()
        || value.len() > 8 * 1024
        || value.contains(['\r', '\n'])
        || value.chars().any(|character| character == '\0')
    {
        return Err(GmailApiError::InvalidInput(format!(
            "{name} header is malformed"
        )));
    }
    Ok(value.to_string())
}
