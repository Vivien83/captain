//! Shared types for native Gmail account access.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

pub const GMAIL_SCOPE_SEND: &str = "https://www.googleapis.com/auth/gmail.send";
pub const GMAIL_SCOPE_READONLY: &str = "https://www.googleapis.com/auth/gmail.readonly";
pub const GMAIL_SCOPE_MODIFY: &str = "https://www.googleapis.com/auth/gmail.modify";

const SEND_SCOPES: &[&str] = &["openid", "email", GMAIL_SCOPE_SEND];
const READ_SCOPES: &[&str] = &["openid", "email", GMAIL_SCOPE_READONLY];
const ASSISTANT_SCOPES: &[&str] = &["openid", "email", GMAIL_SCOPE_MODIFY];

/// Canonical operator-facing name for one Gmail account.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GmailAccountAlias(String);

impl GmailAccountAlias {
    pub const MAX_LEN: usize = 48;

    /// Parse a user alias into its lowercase canonical form.
    pub fn parse(value: &str) -> Result<Self, GmailAliasError> {
        let canonical = value.trim().to_ascii_lowercase();
        if canonical.is_empty() {
            return Err(GmailAliasError::Empty);
        }
        if canonical.len() > Self::MAX_LEN {
            return Err(GmailAliasError::TooLong(Self::MAX_LEN));
        }
        let mut chars = canonical.chars();
        let first = chars.next().expect("non-empty alias");
        if !first.is_ascii_alphanumeric() {
            return Err(GmailAliasError::InvalidStart);
        }
        if !chars.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        }) {
            return Err(GmailAliasError::InvalidCharacter);
        }
        Ok(Self(canonical))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for GmailAccountAlias {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for GmailAccountAlias {
    type Err = GmailAliasError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for GmailAccountAlias {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for GmailAccountAlias {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum GmailAliasError {
    #[error("Gmail account alias cannot be empty")]
    Empty,
    #[error("Gmail account alias cannot exceed {0} characters")]
    TooLong(usize),
    #[error("Gmail account alias must start with a letter or digit")]
    InvalidStart,
    #[error("Gmail account alias may contain only letters, digits, '.', '_' and '-'")]
    InvalidCharacter,
}

/// Least-privilege OAuth profile selected by the operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GmailAccessProfile {
    /// Send mail only. Does not permit mailbox reads.
    Send,
    /// Read mail without changing labels or sending.
    Read,
    /// Read, compose, send and modify labels.
    Assistant,
}

impl GmailAccessProfile {
    pub fn required_scopes(self) -> &'static [&'static str] {
        match self {
            Self::Send => SEND_SCOPES,
            Self::Read => READ_SCOPES,
            Self::Assistant => ASSISTANT_SCOPES,
        }
    }

    pub fn required_gmail_scope(self) -> &'static str {
        match self {
            Self::Send => GMAIL_SCOPE_SEND,
            Self::Read => GMAIL_SCOPE_READONLY,
            Self::Assistant => GMAIL_SCOPE_MODIFY,
        }
    }

    pub fn can_read(self) -> bool {
        matches!(self, Self::Read | Self::Assistant)
    }

    pub fn can_send(self) -> bool {
        matches!(self, Self::Send | Self::Assistant)
    }

    pub fn can_modify(self) -> bool {
        matches!(self, Self::Assistant)
    }
}

impl fmt::Display for GmailAccessProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Send => "send",
            Self::Read => "read",
            Self::Assistant => "assistant",
        })
    }
}

impl FromStr for GmailAccessProfile {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "send" => Ok(Self::Send),
            "read" | "readonly" | "read_only" => Ok(Self::Read),
            "assistant" | "modify" => Ok(Self::Assistant),
            _ => Err("access must be one of: send, read, assistant".to_string()),
        }
    }
}

/// Durable readiness state. Error details remain in bounded structured logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GmailAccountStatus {
    Ready,
    ReauthRequired,
    Disabled,
}

impl fmt::Display for GmailAccountStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Ready => "ready",
            Self::ReauthRequired => "reauth_required",
            Self::Disabled => "disabled",
        })
    }
}

impl FromStr for GmailAccountStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ready" => Ok(Self::Ready),
            "reauth_required" => Ok(Self::ReauthRequired),
            "disabled" => Ok(Self::Disabled),
            _ => Err(format!("unsupported Gmail account status '{value}'")),
        }
    }
}

/// Public-safe projection of a connected Gmail account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailAccountSummary {
    pub alias: GmailAccountAlias,
    pub email_address: String,
    pub access_profile: GmailAccessProfile,
    pub granted_scopes: Vec<String>,
    pub history_id: Option<String>,
    pub status: GmailAccountStatus,
    pub enabled: bool,
    pub is_default: bool,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub last_error_code: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Bounded search request for one connected Gmail mailbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailSearchRequest {
    pub account_alias: Option<GmailAccountAlias>,
    pub query: String,
    pub label_ids: Vec<String>,
    pub max_results: u16,
    pub page_token: Option<String>,
    pub include_spam_trash: bool,
}

/// Public-safe Gmail message metadata used by search and read operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailMessageSummary {
    pub id: String,
    pub thread_id: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub cc: Option<String>,
    pub subject: Option<String>,
    pub received_at: Option<DateTime<Utc>>,
    pub snippet: String,
    pub label_ids: Vec<String>,
    pub size_estimate: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailSearchResult {
    pub account_alias: GmailAccountAlias,
    pub email_address: String,
    pub messages: Vec<GmailMessageSummary>,
    pub next_page_token: Option<String>,
    pub result_size_estimate: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailReadRequest {
    pub account_alias: Option<GmailAccountAlias>,
    pub message_id: String,
    pub max_body_bytes: usize,
}

/// Attachment metadata. The attachment body is never injected into the agent
/// implicitly; it must be saved through the dedicated bounded tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailAttachmentSummary {
    pub attachment_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: u64,
}

/// Bounded, decoded content for one Gmail message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailMessageContent {
    pub account_alias: GmailAccountAlias,
    pub email_address: String,
    pub summary: GmailMessageSummary,
    pub message_id_header: Option<String>,
    pub references: Option<String>,
    pub reply_to: Option<String>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub body_truncated: bool,
    pub attachments: Vec<GmailAttachmentSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GmailDeliveryMode {
    Draft,
    Send,
}

impl fmt::Display for GmailDeliveryMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Draft => "draft",
            Self::Send => "send",
        })
    }
}

/// In-memory attachment supplied by a workspace-bounded runtime tool. Bytes
/// are intentionally not serializable and never become part of tool output.
#[derive(Clone, PartialEq, Eq)]
pub struct GmailOutgoingAttachment {
    pub filename: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}

impl fmt::Debug for GmailOutgoingAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GmailOutgoingAttachment")
            .field("filename", &self.filename)
            .field("mime_type", &self.mime_type)
            .field("size", &self.data.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailComposeRequest {
    pub account_alias: Option<GmailAccountAlias>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub reply_to: Option<String>,
    pub subject: String,
    pub text_body: String,
    pub html_body: Option<String>,
    pub attachments: Vec<GmailOutgoingAttachment>,
    pub delivery: GmailDeliveryMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailReplyRequest {
    pub account_alias: Option<GmailAccountAlias>,
    pub message_id: String,
    pub text_body: String,
    pub html_body: Option<String>,
    pub attachments: Vec<GmailOutgoingAttachment>,
    pub delivery: GmailDeliveryMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailComposeResult {
    pub account_alias: GmailAccountAlias,
    pub email_address: String,
    pub delivery: GmailDeliveryMode,
    pub id: String,
    pub message_id: String,
    pub thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailLabel {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub messages_total: u64,
    pub messages_unread: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailLabelListRequest {
    pub account_alias: Option<GmailAccountAlias>,
    pub query: Option<String>,
    pub max_results: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailLabelListResult {
    pub account_alias: GmailAccountAlias,
    pub email_address: String,
    pub labels: Vec<GmailLabel>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailUpdateRequest {
    pub account_alias: Option<GmailAccountAlias>,
    pub message_id: String,
    pub add_label_ids: Vec<String>,
    pub remove_label_ids: Vec<String>,
    pub trash: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailUpdateResult {
    pub account_alias: GmailAccountAlias,
    pub email_address: String,
    pub id: String,
    pub thread_id: String,
    pub label_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailAttachmentRequest {
    pub account_alias: Option<GmailAccountAlias>,
    pub message_id: String,
    pub attachment_id: String,
}

/// Secret-free binary result handed directly to the workspace writer.
#[derive(Clone, PartialEq, Eq)]
pub struct GmailAttachmentData {
    pub data: Vec<u8>,
}

impl fmt::Debug for GmailAttachmentData {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GmailAttachmentData")
            .field("size", &self.data.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_are_canonical_and_reject_ambiguous_input() {
        assert_eq!(
            GmailAccountAlias::parse("  Pro.Work ").unwrap().as_str(),
            "pro.work"
        );
        assert!(GmailAccountAlias::parse("-invalid").is_err());
        assert!(GmailAccountAlias::parse("not valid").is_err());
        assert!(GmailAccountAlias::parse(&"a".repeat(49)).is_err());
    }

    #[test]
    fn alias_deserialization_cannot_bypass_validation() {
        assert!(serde_json::from_str::<GmailAccountAlias>("\"bad alias\"").is_err());
        let alias = serde_json::from_str::<GmailAccountAlias>("\"Personal\"").unwrap();
        assert_eq!(alias.as_str(), "personal");
    }

    #[test]
    fn profiles_expose_exact_least_privilege_capabilities() {
        assert_eq!(
            GmailAccessProfile::Send.required_gmail_scope(),
            GMAIL_SCOPE_SEND
        );
        assert!(!GmailAccessProfile::Send.can_read());
        assert!(GmailAccessProfile::Send.can_send());
        assert!(GmailAccessProfile::Read.can_read());
        assert!(!GmailAccessProfile::Read.can_send());
        assert!(GmailAccessProfile::Assistant.can_modify());
        assert!(GmailAccessProfile::Assistant
            .required_scopes()
            .contains(&GMAIL_SCOPE_MODIFY));
    }
}
