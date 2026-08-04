//! Native Gmail operations exposed through the runtime kernel boundary.

use std::path::PathBuf;

use captain_extensions::gmail_api::{
    build_gmail_mime, GmailApiClient, GmailApiError, GmailThreadingHeaders,
};
use captain_memory::gmail_accounts::{GmailAccountRecord, GmailAccountStore};
use captain_types::email::{
    GmailAttachmentData, GmailAttachmentRequest, GmailComposeRequest, GmailComposeResult,
    GmailDeliveryMode, GmailLabelListRequest, GmailLabelListResult, GmailMessageContent,
    GmailReadRequest, GmailReplyRequest, GmailSearchRequest, GmailSearchResult, GmailUpdateRequest,
    GmailUpdateResult,
};

use super::kernel_email_credentials::{
    GmailCredentialContext, GmailCredentialManager, GmailRequiredAccess,
};
use super::CaptainKernel;

const MAX_LABEL_RESULTS: u16 = 200;
const MAX_LABEL_QUERY_BYTES: usize = 256;
const ATTACHMENT_DOWNLOAD_LIMIT: usize = 20 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct GmailRuntimeService {
    credentials: GmailCredentialManager,
    store: GmailAccountStore,
}

impl GmailRuntimeService {
    pub(crate) fn new(home: PathBuf, store: GmailAccountStore) -> Self {
        Self {
            credentials: GmailCredentialManager::new(home, store.clone()),
            store,
        }
    }

    pub(crate) fn accounts(
        &self,
    ) -> Result<Vec<captain_types::email::GmailAccountSummary>, String> {
        self.store
            .list()
            .map(|records| records.into_iter().map(|record| record.summary).collect())
            .map_err(|error| error.to_string())
    }

    pub(crate) async fn search(
        &self,
        request: GmailSearchRequest,
    ) -> Result<GmailSearchResult, String> {
        let context = self
            .credentials
            .authorize(request.account_alias.clone(), GmailRequiredAccess::Read)
            .await?;
        let client = api_client(&context)?;
        match client.search_messages(&request).await {
            Ok(page) => {
                self.credentials.record_success(&context.record).await;
                Ok(GmailSearchResult {
                    account_alias: context.record.summary.alias,
                    email_address: context.record.summary.email_address,
                    messages: page.messages,
                    next_page_token: page.next_page_token,
                    result_size_estimate: page.result_size_estimate,
                })
            }
            Err(error) => self.fail_read(&context.record, error).await,
        }
    }

    pub(crate) async fn read(
        &self,
        request: GmailReadRequest,
    ) -> Result<GmailMessageContent, String> {
        let context = self
            .credentials
            .authorize(request.account_alias, GmailRequiredAccess::Read)
            .await?;
        let client = api_client(&context)?;
        match client
            .read_message(&request.message_id, request.max_body_bytes)
            .await
        {
            Ok(message) => {
                self.credentials.record_success(&context.record).await;
                Ok(GmailMessageContent {
                    account_alias: context.record.summary.alias,
                    email_address: context.record.summary.email_address,
                    summary: message.summary,
                    message_id_header: message.message_id_header,
                    references: message.references,
                    reply_to: message.reply_to,
                    body_text: message.body_text,
                    body_html: message.body_html,
                    body_truncated: message.body_truncated,
                    attachments: message.attachments,
                })
            }
            Err(error) => self.fail_read(&context.record, error).await,
        }
    }

    pub(crate) async fn compose(
        &self,
        request: GmailComposeRequest,
    ) -> Result<GmailComposeResult, String> {
        let required = match request.delivery {
            GmailDeliveryMode::Draft => GmailRequiredAccess::Modify,
            GmailDeliveryMode::Send => GmailRequiredAccess::Send,
        };
        let context = self
            .credentials
            .authorize(request.account_alias.clone(), required)
            .await?;
        self.deliver(&context, &request, None).await
    }

    pub(crate) async fn reply(
        &self,
        request: GmailReplyRequest,
    ) -> Result<GmailComposeResult, String> {
        let context = self
            .credentials
            .authorize(request.account_alias, GmailRequiredAccess::Modify)
            .await?;
        let client = api_client(&context)?;
        let original = match client.read_reply_metadata(&request.message_id).await {
            Ok(message) => message,
            Err(error) => return self.fail_read(&context.record, error).await,
        };
        let recipient = original
            .reply_to
            .clone()
            .or_else(|| original.summary.from.clone())
            .ok_or_else(|| "The original Gmail message has no reply recipient".to_string())?;
        let subject = reply_subject(original.summary.subject.as_deref());
        let threading = GmailThreadingHeaders {
            thread_id: original.summary.thread_id,
            in_reply_to: original.message_id_header.clone(),
            references: reply_references(
                original.references.as_deref(),
                original.message_id_header.as_deref(),
            )?,
        };
        let compose = GmailComposeRequest {
            account_alias: Some(context.record.summary.alias.clone()),
            to: vec![recipient],
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: None,
            subject,
            text_body: request.text_body,
            html_body: request.html_body,
            attachments: request.attachments,
            delivery: request.delivery,
        };
        self.deliver(&context, &compose, Some(&threading)).await
    }

    pub(crate) async fn labels(
        &self,
        request: GmailLabelListRequest,
    ) -> Result<GmailLabelListResult, String> {
        validate_label_request(&request)?;
        let context = self
            .credentials
            .authorize(request.account_alias, GmailRequiredAccess::Read)
            .await?;
        let client = api_client(&context)?;
        let mut labels = match client.list_labels().await {
            Ok(labels) => labels,
            Err(error) => return self.fail_read(&context.record, error).await,
        };
        if let Some(query) = request.query.as_deref() {
            let query = query.to_ascii_lowercase();
            labels.retain(|label| {
                label.name.to_ascii_lowercase().contains(&query)
                    || label.id.to_ascii_lowercase().contains(&query)
            });
        }
        labels.sort_by(|left, right| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        let truncated = labels.len() > usize::from(request.max_results);
        labels.truncate(usize::from(request.max_results));
        self.credentials.record_success(&context.record).await;
        Ok(GmailLabelListResult {
            account_alias: context.record.summary.alias,
            email_address: context.record.summary.email_address,
            labels,
            truncated,
        })
    }

    pub(crate) async fn update(
        &self,
        request: GmailUpdateRequest,
    ) -> Result<GmailUpdateResult, String> {
        validate_update_request(&request)?;
        let context = self
            .credentials
            .authorize(request.account_alias, GmailRequiredAccess::Modify)
            .await?;
        let client = api_client(&context)?;
        let receipt = if let Some(trashed) = request.trash {
            client.set_trashed(&request.message_id, trashed).await
        } else {
            client
                .modify_message(
                    &request.message_id,
                    &request.add_label_ids,
                    &request.remove_label_ids,
                )
                .await
        };
        match receipt {
            Ok(receipt) => {
                self.credentials.record_success(&context.record).await;
                Ok(GmailUpdateResult {
                    account_alias: context.record.summary.alias,
                    email_address: context.record.summary.email_address,
                    id: receipt.id,
                    thread_id: receipt.thread_id,
                    label_ids: receipt.label_ids,
                })
            }
            Err(error) => {
                self.fail_write(
                    &context.record,
                    "update",
                    "Inspect the message labels or trash state first.",
                    error,
                )
                .await
            }
        }
    }

    pub(crate) async fn attachment(
        &self,
        request: GmailAttachmentRequest,
    ) -> Result<GmailAttachmentData, String> {
        let context = self
            .credentials
            .authorize(request.account_alias, GmailRequiredAccess::Read)
            .await?;
        let client = api_client(&context)?;
        match client
            .download_attachment(
                &request.message_id,
                &request.attachment_id,
                ATTACHMENT_DOWNLOAD_LIMIT,
            )
            .await
        {
            Ok(data) => {
                self.credentials.record_success(&context.record).await;
                Ok(data)
            }
            Err(error) => self.fail_read(&context.record, error).await,
        }
    }

    async fn deliver(
        &self,
        context: &GmailCredentialContext,
        request: &GmailComposeRequest,
        threading: Option<&GmailThreadingHeaders>,
    ) -> Result<GmailComposeResult, String> {
        let raw = build_gmail_mime(request, &context.record.summary.email_address, threading)
            .map_err(|error| error.to_string())?;
        let client = api_client(context)?;
        let thread_id = threading.map(|headers| headers.thread_id.as_str());
        let receipt = match request.delivery {
            GmailDeliveryMode::Draft => client.create_draft(&raw, thread_id).await,
            GmailDeliveryMode::Send => client.send_message(&raw, thread_id).await,
        };
        match receipt {
            Ok(receipt) => {
                self.credentials.record_success(&context.record).await;
                Ok(GmailComposeResult {
                    account_alias: context.record.summary.alias.clone(),
                    email_address: context.record.summary.email_address.clone(),
                    delivery: request.delivery,
                    id: receipt.id,
                    message_id: receipt.message_id,
                    thread_id: receipt.thread_id,
                })
            }
            Err(error) => {
                let action = match request.delivery {
                    GmailDeliveryMode::Draft => "draft",
                    GmailDeliveryMode::Send => "send",
                };
                self.fail_write(
                    &context.record,
                    action,
                    "Inspect Sent or Drafts first.",
                    error,
                )
                .await
            }
        }
    }

    async fn fail_read<T>(
        &self,
        record: &GmailAccountRecord,
        error: GmailApiError,
    ) -> Result<T, String> {
        self.credentials.record_api_failure(record, &error).await;
        Err(error.to_string())
    }

    async fn fail_write<T>(
        &self,
        record: &GmailAccountRecord,
        action: &str,
        inspection_hint: &str,
        error: GmailApiError,
    ) -> Result<T, String> {
        self.credentials.record_api_failure(record, &error).await;
        Err(write_failure_message(action, inspection_hint, &error))
    }
}

fn write_failure_message(action: &str, inspection_hint: &str, error: &GmailApiError) -> String {
    format!(
        "Gmail {action} failed: {error}. Operation state may be uncertain; do not retry automatically. {inspection_hint}"
    )
}

impl CaptainKernel {
    fn gmail_runtime_service(&self) -> GmailRuntimeService {
        GmailRuntimeService::new(
            self.config.home_dir.clone(),
            self.memory.gmail_accounts().clone(),
        )
    }

    pub(super) fn handle_email_accounts(
        &self,
    ) -> Result<Vec<captain_types::email::GmailAccountSummary>, String> {
        self.gmail_runtime_service().accounts()
    }

    pub(super) async fn handle_email_search(
        &self,
        request: GmailSearchRequest,
    ) -> Result<GmailSearchResult, String> {
        self.gmail_runtime_service().search(request).await
    }

    pub(super) async fn handle_email_read(
        &self,
        request: GmailReadRequest,
    ) -> Result<GmailMessageContent, String> {
        self.gmail_runtime_service().read(request).await
    }

    pub(super) async fn handle_email_compose(
        &self,
        request: GmailComposeRequest,
    ) -> Result<GmailComposeResult, String> {
        self.gmail_runtime_service().compose(request).await
    }

    pub(super) async fn handle_email_reply(
        &self,
        request: GmailReplyRequest,
    ) -> Result<GmailComposeResult, String> {
        self.gmail_runtime_service().reply(request).await
    }

    pub(super) async fn handle_email_labels(
        &self,
        request: GmailLabelListRequest,
    ) -> Result<GmailLabelListResult, String> {
        self.gmail_runtime_service().labels(request).await
    }

    pub(super) async fn handle_email_update(
        &self,
        request: GmailUpdateRequest,
    ) -> Result<GmailUpdateResult, String> {
        self.gmail_runtime_service().update(request).await
    }

    pub(super) async fn handle_email_attachment(
        &self,
        request: GmailAttachmentRequest,
    ) -> Result<GmailAttachmentData, String> {
        self.gmail_runtime_service().attachment(request).await
    }
}

fn api_client(context: &GmailCredentialContext) -> Result<GmailApiClient, String> {
    GmailApiClient::new(context.tokens.access_token()).map_err(|error| error.to_string())
}

fn validate_label_request(request: &GmailLabelListRequest) -> Result<(), String> {
    if !(1..=MAX_LABEL_RESULTS).contains(&request.max_results) {
        return Err(format!(
            "Gmail label max_results must be between 1 and {MAX_LABEL_RESULTS}"
        ));
    }
    if request.query.as_ref().is_some_and(|query| {
        query.len() > MAX_LABEL_QUERY_BYTES || query.chars().any(char::is_control)
    }) {
        return Err("Gmail label query is oversized or contains control characters".to_string());
    }
    Ok(())
}

fn validate_update_request(request: &GmailUpdateRequest) -> Result<(), String> {
    let has_labels = !request.add_label_ids.is_empty() || !request.remove_label_ids.is_empty();
    if request.trash.is_some() && has_labels {
        return Err(
            "Trash/restore cannot be combined with label changes in one Gmail operation"
                .to_string(),
        );
    }
    if request.trash.is_none() && !has_labels {
        return Err("A Gmail update requires one reversible action".to_string());
    }
    Ok(())
}

fn reply_subject(original: Option<&str>) -> String {
    let subject = original.unwrap_or("").trim();
    if subject
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("re:"))
    {
        subject.to_string()
    } else if subject.is_empty() {
        "Re:".to_string()
    } else {
        format!("Re: {subject}")
    }
}

fn reply_references(
    existing: Option<&str>,
    message_id: Option<&str>,
) -> Result<Option<String>, String> {
    let mut references = existing.unwrap_or("").trim().to_string();
    if let Some(message_id) = message_id.map(str::trim).filter(|value| !value.is_empty()) {
        if !references
            .split_whitespace()
            .any(|value| value == message_id)
        {
            if !references.is_empty() {
                references.push(' ');
            }
            references.push_str(message_id);
        }
    }
    if references.len() > 8 * 1024 {
        return Err("Gmail reply References header exceeds the safety limit".to_string());
    }
    Ok((!references.is_empty()).then_some(references))
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::email::GmailAccountAlias;

    #[test]
    fn reply_subject_is_stable_and_never_stacks_prefixes() {
        assert_eq!(
            reply_subject(Some("Quarterly report")),
            "Re: Quarterly report"
        );
        assert_eq!(
            reply_subject(Some("RE: Quarterly report")),
            "RE: Quarterly report"
        );
        assert_eq!(reply_subject(None), "Re:");
    }

    #[test]
    fn reply_references_append_message_id_once() {
        assert_eq!(
            reply_references(Some("<a@example>"), Some("<b@example>")).unwrap(),
            Some("<a@example> <b@example>".to_string())
        );
        assert_eq!(
            reply_references(Some("<a@example> <b@example>"), Some("<b@example>")).unwrap(),
            Some("<a@example> <b@example>".to_string())
        );
    }

    #[test]
    fn update_rejects_partial_multi_action_semantics() {
        let request = GmailUpdateRequest {
            account_alias: Some(GmailAccountAlias::parse("work").unwrap()),
            message_id: "message".to_string(),
            add_label_ids: vec!["STARRED".to_string()],
            remove_label_ids: Vec::new(),
            trash: Some(true),
        };
        assert!(validate_update_request(&request).is_err());
    }

    #[test]
    fn uncertain_write_guidance_matches_the_operation() {
        let error = GmailApiError::Transport;
        let update = write_failure_message(
            "update",
            "Inspect the message labels or trash state first.",
            &error,
        );
        let send = write_failure_message("send", "Inspect Sent or Drafts first.", &error);

        assert!(update.contains("do not retry automatically"));
        assert!(update.contains("message labels or trash state"));
        assert!(!update.contains("Sent or Drafts"));
        assert!(send.contains("Sent or Drafts"));
    }

    #[test]
    fn label_query_is_bounded() {
        let request = GmailLabelListRequest {
            account_alias: None,
            query: Some("x".repeat(MAX_LABEL_QUERY_BYTES + 1)),
            max_results: 20,
        };
        assert!(validate_label_request(&request).is_err());
    }
}
