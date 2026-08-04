//! Email channel adapter (IMAP + SMTP).
//!
//! Polls IMAP for new emails and sends responses via SMTP using `lettre`.
//! Uses the subject line for agent routing (e.g., "\[coder\] Fix this bug").

use crate::inbound_queue_types::DURABLE_INGRESS_ID_METADATA_KEY;
use crate::types::{
    ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
    INTERNAL_TARGET_AGENT_NAME_METADATA_KEY,
};
use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use futures::Stream;
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::Address;
use lettre::AsyncSmtpTransport;
use lettre::AsyncTransport;
use lettre::Tokio1Executor;
use serde::Serialize;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};
use zeroize::Zeroizing;

/// SASL PLAIN authenticator for IMAP servers that reject LOGIN
/// (e.g., Lark/Larksuite which only advertise AUTH=PLAIN).
struct PlainAuthenticator {
    username: String,
    password: String,
}

impl imap::Authenticator for PlainAuthenticator {
    type Response = String;
    fn process(&self, _data: &[u8]) -> Self::Response {
        // SASL PLAIN: \0<username>\0<password>
        format!("\x00{}\x00{}", self.username, self.password)
    }
}

/// Reply context for email threading (In-Reply-To / Subject continuity).
#[derive(Debug, Clone)]
struct ReplyCtx {
    subject: String,
    message_id: String,
}

type EmailImapSession = imap::Session<imap::Connection>;
const EMAIL_IMAP_ACK_TIMEOUT: Duration = Duration::from_secs(15);
const EMAIL_CONNECTIVITY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EmailConnectivityReport {
    pub imap_folders_checked: usize,
    pub smtp_authenticated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EmailImapCursor {
    folder: String,
    uid_validity: u32,
    uid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedEmail {
    from_addr: String,
    subject: String,
    rfc_message_id: String,
    body: String,
}

#[derive(Debug, Clone)]
struct FetchedEmail {
    cursor: EmailImapCursor,
    parsed: Option<ParsedEmail>,
}

struct EmailPollRuntime {
    tx: mpsc::Sender<ChannelMessage>,
    account_alias: String,
    poll_interval: Duration,
    imap_host: String,
    imap_port: u16,
    username: String,
    password: Zeroizing<String>,
    folders: Vec<String>,
    allowed_senders: Vec<String>,
    shutdown_rx: watch::Receiver<bool>,
    reply_ctx: Arc<DashMap<String, ReplyCtx>>,
}

/// Email channel adapter using IMAP for receiving and SMTP for sending.
pub struct EmailAdapter {
    /// Stable bridge registration name (`email` or `email:<alias>`).
    adapter_name: String,
    /// Stable mailbox alias propagated as trusted routing metadata.
    account_alias: String,
    /// IMAP server host.
    imap_host: String,
    /// IMAP port (993 for TLS).
    imap_port: u16,
    /// SMTP server host.
    smtp_host: String,
    /// SMTP port (587 for STARTTLS, 465 for implicit TLS).
    smtp_port: u16,
    /// Email address (used for both IMAP and SMTP).
    username: String,
    /// SECURITY: Password is zeroized on drop.
    password: Zeroizing<String>,
    /// How often to check for new emails.
    poll_interval: Duration,
    /// Which IMAP folders to monitor.
    folders: Vec<String>,
    /// Only process emails from these senders (empty = deny all,
    /// `["*"]` = allow all).
    allowed_senders: Vec<String>,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Tracks reply context per sender for email threading.
    reply_ctx: Arc<DashMap<String, ReplyCtx>>,
}

impl EmailAdapter {
    /// Create a new email adapter.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        imap_host: String,
        imap_port: u16,
        smtp_host: String,
        smtp_port: u16,
        username: String,
        password: String,
        poll_interval_secs: u64,
        folders: Vec<String>,
        allowed_senders: Vec<String>,
    ) -> Self {
        Self::new_named(
            "email".to_string(),
            "default".to_string(),
            imap_host,
            imap_port,
            smtp_host,
            smtp_port,
            username,
            password,
            poll_interval_secs,
            folders,
            allowed_senders,
        )
    }

    /// Create a named adapter for one configured mailbox.
    #[allow(clippy::too_many_arguments)]
    pub fn new_named(
        adapter_name: String,
        account_alias: String,
        imap_host: String,
        imap_port: u16,
        smtp_host: String,
        smtp_port: u16,
        username: String,
        password: String,
        poll_interval_secs: u64,
        folders: Vec<String>,
        allowed_senders: Vec<String>,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            adapter_name,
            account_alias,
            imap_host,
            imap_port,
            smtp_host,
            smtp_port,
            username,
            password: Zeroizing::new(password),
            poll_interval: Duration::from_secs(poll_interval_secs),
            folders: if folders.is_empty() {
                vec!["INBOX".to_string()]
            } else {
                folders
            },
            allowed_senders,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            reply_ctx: Arc::new(DashMap::new()),
        }
    }

    /// Check if a sender is in the allowlist (B.8 contract: empty = deny all,
    /// `["*"]` = allow all, full address = exact match, `@example.org` = exact
    /// domain match). Invalid addresses never match, including under `*`.
    #[allow(dead_code)]
    fn is_allowed_sender(&self, sender: &str) -> bool {
        email_sender_allowed(&self.allowed_senders, sender)
    }

    /// Extract agent name from subject line brackets, e.g., "[coder] Fix the bug" -> Some("coder")
    fn extract_agent_from_subject(subject: &str) -> Option<String> {
        let subject = subject.trim();
        if subject.starts_with('[') {
            if let Some(end) = subject.find(']') {
                let agent = &subject[1..end];
                if !agent.is_empty() {
                    return Some(agent.to_string());
                }
            }
        }
        None
    }

    /// Strip the agent tag from a subject line.
    fn strip_agent_tag(subject: &str) -> String {
        let subject = subject.trim();
        if subject.starts_with('[') {
            if let Some(end) = subject.find(']') {
                return subject[end + 1..].trim().to_string();
            }
        }
        subject.to_string()
    }

    /// Build an async SMTP transport for sending emails.
    async fn build_smtp_transport(
        &self,
    ) -> Result<AsyncSmtpTransport<Tokio1Executor>, Box<dyn std::error::Error>> {
        let creds = Credentials::new(self.username.clone(), self.password.as_str().to_string());

        let transport = if self.smtp_port == 465 {
            // Implicit TLS (port 465)
            AsyncSmtpTransport::<Tokio1Executor>::relay(&self.smtp_host)?
                .port(self.smtp_port)
                .credentials(creds)
                .timeout(Some(EMAIL_CONNECTIVITY_TIMEOUT))
                .build()
        } else {
            // STARTTLS (port 587 or other)
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&self.smtp_host)?
                .port(self.smtp_port)
                .credentials(creds)
                .timeout(Some(EMAIL_CONNECTIVITY_TIMEOUT))
                .build()
        };

        Ok(transport)
    }

    /// Verify IMAP login/folder access and authenticated SMTP without sending.
    pub async fn test_connectivity(&self) -> Result<EmailConnectivityReport, String> {
        let host = self.imap_host.clone();
        let port = self.imap_port;
        let username = self.username.clone();
        let password = self.password.clone();
        let folders = self.folders.clone();
        let folder_count = folders.len();
        let imap = tokio::task::spawn_blocking(move || {
            test_imap_connectivity(&host, port, &username, password.as_str(), &folders)
        });
        match tokio::time::timeout(EMAIL_CONNECTIVITY_TIMEOUT, imap).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(error))) => return Err(error),
            Ok(Err(error)) => return Err(format!("IMAP connectivity worker failed: {error}")),
            Err(_) => {
                return Err(format!(
                    "IMAP connectivity check exceeded {} seconds",
                    EMAIL_CONNECTIVITY_TIMEOUT.as_secs()
                ))
            }
        }

        let transport = self
            .build_smtp_transport()
            .await
            .map_err(|error| format!("SMTP transport setup failed: {error}"))?;
        let smtp_authenticated =
            tokio::time::timeout(EMAIL_CONNECTIVITY_TIMEOUT, transport.test_connection())
                .await
                .map_err(|_| {
                    format!(
                        "SMTP connectivity check exceeded {} seconds",
                        EMAIL_CONNECTIVITY_TIMEOUT.as_secs()
                    )
                })?
                .map_err(|error| format!("SMTP connectivity check failed: {error}"))?;
        if !smtp_authenticated {
            return Err("SMTP server did not confirm the authenticated connection".to_string());
        }
        Ok(EmailConnectivityReport {
            imap_folders_checked: folder_count,
            smtp_authenticated,
        })
    }
}

pub fn email_allowlist_rule_is_valid(rule: &str) -> bool {
    let rule = rule.trim();
    if rule == "*" {
        return true;
    }
    if let Some(domain) = rule.strip_prefix('@') {
        return !domain.is_empty()
            && url::Host::parse(domain).is_ok()
            && Address::new("captain", domain).is_ok();
    }
    parse_email_address(rule).is_some()
}

pub fn email_address_is_valid(address: &str) -> bool {
    parse_email_address(address).is_some()
}

/// Extract `user@domain` from a potentially formatted email string like `"Name <user@domain>"`.
fn extract_email_addr(raw: &str) -> String {
    let raw = raw.trim();
    if let Some(start) = raw.find('<') {
        if let Some(end) = raw.find('>') {
            if end > start {
                return raw[start + 1..end].trim().to_string();
            }
        }
    }
    raw.to_string()
}

/// Get a specific header value from a parsed email.
fn get_header(parsed: &mailparse::ParsedMail<'_>, name: &str) -> Option<String> {
    parsed
        .headers
        .iter()
        .find(|h| h.get_key().eq_ignore_ascii_case(name))
        .map(|h| h.get_value())
}

/// Extract the text/plain body from a parsed email (handles multipart).
fn extract_text_body(parsed: &mailparse::ParsedMail<'_>) -> String {
    if parsed.subparts.is_empty() {
        return parsed.get_body().unwrap_or_default();
    }
    // Walk subparts looking for text/plain
    for part in &parsed.subparts {
        let ct = part.ctype.mimetype.to_lowercase();
        if ct == "text/plain" {
            return part.get_body().unwrap_or_default();
        }
    }
    // Fallback: first subpart body
    parsed
        .subparts
        .first()
        .and_then(|p| p.get_body().ok())
        .unwrap_or_default()
}

/// Fetch unseen emails from IMAP using blocking I/O. Fetching is read-only:
/// platform acknowledgement happens only after the bridge persists acceptance.
fn fetch_unseen_emails(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    folders: &[String],
) -> Result<Vec<FetchedEmail>, String> {
    let client = imap::ClientBuilder::new(host, port)
        .mode(imap::ConnectionMode::Tls)
        .connect()
        .map_err(|e| format!("IMAP connect failed: {e}"))?;
    let mut session = login_imap_session(client, username, password)?;
    let mut results = Vec::new();

    for folder in folders {
        fetch_unseen_folder(&mut session, folder, &mut results);
    }

    let _ = session.logout();
    Ok(results)
}

fn login_imap_session(
    client: imap::Client<imap::Connection>,
    username: &str,
    password: &str,
) -> Result<EmailImapSession, String> {
    let session = match client.login(username, password) {
        Ok(s) => s,
        Err((login_err, client)) => {
            let authenticator = PlainAuthenticator {
                username: username.to_string(),
                password: password.to_string(),
            };
            client
                .authenticate("PLAIN", &authenticator)
                .map_err(|(e, _)| {
                    format!("IMAP login failed: {login_err}; AUTH=PLAIN also failed: {e}")
                })?
        }
    };
    Ok(session)
}

fn test_imap_connectivity(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    folders: &[String],
) -> Result<(), String> {
    let client = imap::ClientBuilder::new(host, port)
        .mode(imap::ConnectionMode::Tls)
        .connect()
        .map_err(|error| format!("IMAP connectivity check failed: {error}"))?;
    let mut session = login_imap_session(client, username, password)?;
    let result = folders.iter().try_for_each(|folder| {
        session
            .examine(folder)
            .map(|_| ())
            .map_err(|error| format!("IMAP folder '{folder}' is unavailable: {error}"))
    });
    let _ = session.logout();
    result
}

fn fetch_unseen_folder(
    session: &mut EmailImapSession,
    folder: &str,
    results: &mut Vec<FetchedEmail>,
) {
    let Some((uid_validity, uid_set)) = unseen_uid_set(session, folder) else {
        return;
    };

    let fetches = match session.uid_fetch(&uid_set, "(UID RFC822)") {
        Ok(f) => f,
        Err(e) => {
            warn!(folder, error = %e, "IMAP FETCH failed");
            return;
        }
    };

    for fetch in fetches.iter() {
        let Some(uid) = fetch.uid else {
            warn!(
                folder,
                "IMAP UID FETCH response omitted UID, leaving message unread"
            );
            continue;
        };
        let Some(body_bytes) = fetch.body() else {
            warn!(folder, uid, "IMAP UID FETCH response omitted RFC822 body");
            continue;
        };
        let parsed = match parse_fetched_email(body_bytes) {
            Ok(parsed) => Some(parsed),
            Err(error) => {
                warn!(folder, uid, %error, "Failed to parse fetched email; rejecting after poll");
                None
            }
        };
        results.push(FetchedEmail {
            cursor: EmailImapCursor {
                folder: folder.to_string(),
                uid_validity,
                uid,
            },
            parsed,
        });
    }
}

fn unseen_uid_set(session: &mut EmailImapSession, folder: &str) -> Option<(u32, String)> {
    let mailbox = match session.select(folder) {
        Ok(mailbox) => mailbox,
        Err(e) => {
            warn!(folder, error = %e, "IMAP SELECT failed, skipping folder");
            return None;
        }
    };
    let Some(uid_validity) = mailbox.uid_validity else {
        warn!(
            folder,
            "IMAP SELECT omitted UIDVALIDITY, leaving mailbox unread"
        );
        return None;
    };

    let mut uids = match session.uid_search("UNSEEN") {
        Ok(uids) => uids,
        Err(e) => {
            warn!(folder, error = %e, "IMAP SEARCH UNSEEN failed");
            return None;
        }
    };

    if uids.is_empty() {
        debug!(folder, "No unseen emails");
        return None;
    }

    let mut uids = uids.drain().collect::<Vec<_>>();
    uids.sort_unstable();
    Some((
        uid_validity,
        uids.into_iter()
            .take(50)
            .map(|uid| uid.to_string())
            .collect::<Vec<_>>()
            .join(","),
    ))
}

fn parse_fetched_email(body_bytes: &[u8]) -> Result<ParsedEmail, String> {
    let parsed = mailparse::parse_mail(body_bytes)
        .map_err(|error| format!("invalid RFC822 message: {error}"))?;

    let from = get_header(&parsed, "From").unwrap_or_default();
    let subject = get_header(&parsed, "Subject").unwrap_or_default();
    let rfc_message_id = get_header(&parsed, "Message-ID").unwrap_or_default();
    let body = extract_text_body(&parsed);
    Ok(ParsedEmail {
        from_addr: extract_email_addr(&from),
        subject,
        rfc_message_id,
        body,
    })
}

fn stable_email_ingress_id(account_alias: &str, cursor: &EmailImapCursor) -> String {
    format!(
        "email:{account_alias}:{}:{}:{}:{}",
        cursor.folder.len(),
        cursor.folder,
        cursor.uid_validity,
        cursor.uid
    )
}

fn email_cursor_from_message(
    message: &ChannelMessage,
    expected_account_alias: &str,
    allowed_folders: &[String],
) -> Result<EmailImapCursor, String> {
    if message.channel != ChannelType::Email {
        return Err("inbound acknowledgement is not an Email message".to_string());
    }
    let account_alias = message
        .metadata
        .get("account_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Email acknowledgement is missing account_id".to_string())?;
    if account_alias != expected_account_alias {
        return Err("Email acknowledgement account does not match adapter".to_string());
    }
    let folder = message
        .metadata
        .get("imap_folder")
        .and_then(serde_json::Value::as_str)
        .filter(|folder| allowed_folders.iter().any(|allowed| allowed == folder))
        .ok_or_else(|| "Email acknowledgement folder is not configured".to_string())?
        .to_string();
    let uid_validity = message
        .metadata
        .get("imap_uidvalidity")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| "Email acknowledgement UIDVALIDITY is invalid".to_string())?;
    let uid = message
        .metadata
        .get("imap_uid")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| "Email acknowledgement UID is invalid".to_string())?;
    let cursor = EmailImapCursor {
        folder,
        uid_validity,
        uid,
    };
    let stable_id = stable_email_ingress_id(account_alias, &cursor);
    let durable_id = message
        .metadata
        .get(DURABLE_INGRESS_ID_METADATA_KEY)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Email acknowledgement is missing durable identity".to_string())?;
    if durable_id != stable_id || message.platform_message_id != stable_id {
        return Err("Email acknowledgement identity does not match IMAP cursor".to_string());
    }
    Ok(cursor)
}

fn mark_email_seen(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    cursor: &EmailImapCursor,
) -> Result<(), String> {
    let client = imap::ClientBuilder::new(host, port)
        .mode(imap::ConnectionMode::Tls)
        .connect()
        .map_err(|error| format!("IMAP acknowledgement connect failed: {error}"))?;
    let mut session = login_imap_session(client, username, password)?;
    let result = (|| {
        let mailbox = session
            .select(&cursor.folder)
            .map_err(|error| format!("IMAP acknowledgement SELECT failed: {error}"))?;
        if mailbox.uid_validity != Some(cursor.uid_validity) {
            return Err("IMAP UIDVALIDITY changed before acknowledgement".to_string());
        }
        session
            .uid_store(cursor.uid.to_string(), "+FLAGS.SILENT (\\Seen)")
            .map_err(|error| format!("IMAP acknowledgement STORE failed: {error}"))?;
        Ok(())
    })();
    let _ = session.logout();
    result
}

async fn mark_email_seen_async(
    host: String,
    port: u16,
    username: String,
    password: Zeroizing<String>,
    cursor: EmailImapCursor,
) -> Result<(), String> {
    let task = tokio::task::spawn_blocking(move || {
        mark_email_seen(&host, port, &username, password.as_str(), &cursor)
    });
    match tokio::time::timeout(EMAIL_IMAP_ACK_TIMEOUT, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(format!("IMAP acknowledgement worker failed: {error}")),
        Err(_) => Err(format!(
            "IMAP acknowledgement exceeded {} seconds",
            EMAIL_IMAP_ACK_TIMEOUT.as_secs()
        )),
    }
}

#[async_trait]
impl ChannelAdapter for EmailAdapter {
    fn name(&self) -> &str {
        &self.adapter_name
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Email
    }

    fn registration_aliases(&self) -> Vec<String> {
        if self.adapter_name == "email" {
            vec![format!("email:{}", self.account_alias)]
        } else {
            Vec::new()
        }
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, Box<dyn std::error::Error>>
    {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let runtime = self.poll_runtime(tx);

        info!(
            adapter = %self.adapter_name,
            account = %self.account_alias,
            imap_host = %runtime.imap_host,
            imap_port = runtime.imap_port,
            smtp_host = %self.smtp_host,
            smtp_port = self.smtp_port,
            poll_interval_secs = runtime.poll_interval.as_secs(),
            "Starting email adapter"
        );

        tokio::spawn(run_email_poll_loop(runtime));

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match content {
            ChannelContent::Text(text) => self.send_text_email(user, text).await?,
            _ => {
                warn!(
                    "Unsupported email content type for {}, only text is supported",
                    user.platform_id
                );
            }
        }
        Ok(())
    }

    async fn acknowledge_inbound(
        &self,
        message: &ChannelMessage,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let cursor = email_cursor_from_message(message, &self.account_alias, &self.folders)
            .map_err(std::io::Error::other)?;
        mark_email_seen_async(
            self.imap_host.clone(),
            self.imap_port,
            self.username.clone(),
            self.password.clone(),
            cursor,
        )
        .await
        .map_err(|error| std::io::Error::other(error).into())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }
}

impl EmailAdapter {
    async fn send_text_email(
        &self,
        user: &ChannelUser,
        text: String,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (to_addr, to_mailbox, from_mailbox) = email_mailboxes(user, &self.username)?;
        let (subject, body) = outgoing_email_subject_body(text, &to_addr, &self.reply_ctx);
        let email = build_outgoing_email(
            to_mailbox,
            from_mailbox,
            &subject,
            body,
            &to_addr,
            &self.reply_ctx,
        )?;

        let transport = self.build_smtp_transport().await?;
        transport
            .send(email)
            .await
            .map_err(|e| format!("SMTP send failed: {e}"))?;

        info!(
            to = %to_addr,
            subject = %subject,
            "Email sent successfully via SMTP"
        );
        Ok(())
    }

    fn poll_runtime(&self, tx: mpsc::Sender<ChannelMessage>) -> EmailPollRuntime {
        EmailPollRuntime {
            tx,
            account_alias: self.account_alias.clone(),
            poll_interval: self.poll_interval,
            imap_host: self.imap_host.clone(),
            imap_port: self.imap_port,
            username: self.username.clone(),
            password: self.password.clone(),
            folders: self.folders.clone(),
            allowed_senders: self.allowed_senders.clone(),
            shutdown_rx: self.shutdown_rx.clone(),
            reply_ctx: self.reply_ctx.clone(),
        }
    }
}

fn email_mailboxes(
    user: &ChannelUser,
    username: &str,
) -> Result<(String, Mailbox, Mailbox), String> {
    let to_addr = extract_email_addr(&user.platform_id);
    let to_mailbox: Mailbox = to_addr
        .parse()
        .map_err(|e| format!("Invalid recipient email '{}': {}", to_addr, e))?;
    let from_mailbox: Mailbox = username
        .parse()
        .map_err(|e| format!("Invalid sender email '{}': {}", username, e))?;
    Ok((to_addr, to_mailbox, from_mailbox))
}

fn outgoing_email_subject_body(
    text: String,
    to_addr: &str,
    reply_ctx: &DashMap<String, ReplyCtx>,
) -> (String, String) {
    if text.starts_with("Subject: ") {
        if let Some(pos) = text.find("\n\n") {
            return (text[9..pos].trim().to_string(), text[pos + 2..].to_string());
        }
        return ("Captain Reply".to_string(), text);
    }

    let subject = reply_ctx
        .get(to_addr)
        .map(|ctx| format!("Re: {}", ctx.subject))
        .unwrap_or_else(|| "Captain Reply".to_string());
    (subject, text)
}

fn build_outgoing_email(
    to_mailbox: Mailbox,
    from_mailbox: Mailbox,
    subject: &str,
    body: String,
    to_addr: &str,
    reply_ctx: &DashMap<String, ReplyCtx>,
) -> Result<lettre::Message, String> {
    let mut builder = lettre::Message::builder()
        .from(from_mailbox)
        .to(to_mailbox)
        .subject(subject);

    if let Some(ctx) = reply_ctx.get(to_addr) {
        if !ctx.message_id.is_empty() {
            builder = builder.in_reply_to(ctx.message_id.clone());
        }
    }

    builder
        .body(body)
        .map_err(|e| format!("Failed to build email: {e}"))
}

async fn run_email_poll_loop(mut runtime: EmailPollRuntime) {
    loop {
        if let Some(emails) = poll_email_messages(&runtime).await {
            if !dispatch_email_messages(&runtime, emails).await {
                break;
            }
        }
        if !wait_for_next_email_poll(&mut runtime.shutdown_rx, runtime.poll_interval).await {
            break;
        }
    }
}

async fn wait_for_next_email_poll(
    shutdown_rx: &mut watch::Receiver<bool>,
    poll_interval: Duration,
) -> bool {
    tokio::select! {
        _ = shutdown_rx.changed() => {
            info!("Email adapter shutting down");
            false
        }
        _ = tokio::time::sleep(poll_interval) => true,
    }
}

async fn poll_email_messages(runtime: &EmailPollRuntime) -> Option<Vec<FetchedEmail>> {
    let host = runtime.imap_host.clone();
    let port = runtime.imap_port;
    let user = runtime.username.clone();
    let pass = runtime.password.clone();
    let folders = runtime.folders.clone();

    let emails = tokio::task::spawn_blocking(move || {
        fetch_unseen_emails(&host, port, &user, pass.as_str(), &folders)
    })
    .await;

    match emails {
        Ok(Ok(emails)) => Some(emails),
        Ok(Err(e)) => {
            error!("IMAP poll error: {e}");
            None
        }
        Err(e) => {
            error!("IMAP spawn_blocking panic: {e}");
            None
        }
    }
}

async fn dispatch_email_messages(runtime: &EmailPollRuntime, emails: Vec<FetchedEmail>) -> bool {
    for email in emails {
        let cursor = email.cursor;
        let Some(parsed) = email.parsed else {
            acknowledge_rejected_email(runtime, cursor, "malformed RFC822 message").await;
            continue;
        };
        if email_addresses_equal(&runtime.username, &parsed.from_addr) {
            debug!(from = %parsed.from_addr, "Email from this mailbox, rejecting reply loop");
            acknowledge_rejected_email(runtime, cursor, "self-sent reply loop prevention").await;
            continue;
        }
        if !email_sender_allowed(&runtime.allowed_senders, &parsed.from_addr) {
            debug!(from = %parsed.from_addr, "Email from non-allowed sender, rejecting");
            acknowledge_rejected_email(runtime, cursor, "sender denied by allowlist").await;
            continue;
        }

        remember_email_reply_context(
            &runtime.reply_ctx,
            &parsed.from_addr,
            &parsed.subject,
            &parsed.rfc_message_id,
        );
        let msg = email_channel_message(&runtime.account_alias, &cursor, parsed);

        if runtime.tx.send(msg).await.is_err() {
            info!("Email channel receiver dropped, stopping poll");
            return false;
        }
    }
    true
}

async fn acknowledge_rejected_email(
    runtime: &EmailPollRuntime,
    cursor: EmailImapCursor,
    reason: &'static str,
) {
    let uid = cursor.uid;
    let folder = cursor.folder.clone();
    match mark_email_seen_async(
        runtime.imap_host.clone(),
        runtime.imap_port,
        runtime.username.clone(),
        runtime.password.clone(),
        cursor,
    )
    .await
    {
        Ok(()) => debug!(folder, uid, reason, "Rejected Email acknowledged as Seen"),
        Err(error) => warn!(
            folder,
            uid,
            reason,
            %error,
            "Rejected Email remains unread because acknowledgement failed"
        ),
    }
}

fn email_sender_allowed(allowed_senders: &[String], from_addr: &str) -> bool {
    let Some(from_addr) = parse_email_address(from_addr) else {
        return false;
    };

    allowed_senders.iter().any(|rule| {
        let rule = rule.trim();
        if rule == "*" {
            return true;
        }
        if let Some(domain) = rule.strip_prefix('@') {
            return !domain.is_empty() && from_addr.domain().eq_ignore_ascii_case(domain);
        }
        parse_email_address(rule).is_some_and(|allowed| email_addresses_match(&from_addr, &allowed))
    })
}

fn parse_email_address(value: &str) -> Option<Address> {
    let address = value.trim().parse::<Address>().ok()?;
    url::Host::parse(address.domain()).ok()?;
    Some(address)
}

fn email_addresses_equal(left: &str, right: &str) -> bool {
    let (Some(left), Some(right)) = (parse_email_address(left), parse_email_address(right)) else {
        return false;
    };
    email_addresses_match(&left, &right)
}

fn email_addresses_match(left: &Address, right: &Address) -> bool {
    left.user().eq_ignore_ascii_case(right.user())
        && left.domain().eq_ignore_ascii_case(right.domain())
}

fn remember_email_reply_context(
    reply_ctx: &DashMap<String, ReplyCtx>,
    from_addr: &str,
    subject: &str,
    message_id: &str,
) {
    if message_id.is_empty() {
        return;
    }
    reply_ctx.insert(
        from_addr.to_string(),
        ReplyCtx {
            subject: subject.to_string(),
            message_id: message_id.to_string(),
        },
    );
}

fn email_channel_message(
    account_alias: &str,
    cursor: &EmailImapCursor,
    parsed: ParsedEmail,
) -> ChannelMessage {
    let stable_id = stable_email_ingress_id(account_alias, cursor);
    let requested_agent = EmailAdapter::extract_agent_from_subject(&parsed.subject);
    let text = email_text_from_subject_body(&parsed.subject, &parsed.body);
    let mut metadata = std::collections::HashMap::new();
    metadata.insert(
        "account_id".to_string(),
        serde_json::Value::String(account_alias.to_string()),
    );
    metadata.insert(
        DURABLE_INGRESS_ID_METADATA_KEY.to_string(),
        serde_json::Value::String(stable_id.clone()),
    );
    metadata.insert(
        "imap_folder".to_string(),
        serde_json::Value::String(cursor.folder.clone()),
    );
    metadata.insert("imap_uid".to_string(), serde_json::json!(cursor.uid));
    metadata.insert(
        "imap_uidvalidity".to_string(),
        serde_json::json!(cursor.uid_validity),
    );
    if !parsed.rfc_message_id.is_empty() {
        metadata.insert(
            "rfc_message_id".to_string(),
            serde_json::Value::String(parsed.rfc_message_id),
        );
    }
    if let Some(requested_agent) = requested_agent {
        metadata.insert(
            INTERNAL_TARGET_AGENT_NAME_METADATA_KEY.to_string(),
            serde_json::Value::String(requested_agent),
        );
    }
    ChannelMessage {
        channel: ChannelType::Email,
        platform_message_id: stable_id,
        sender: ChannelUser {
            platform_id: parsed.from_addr.clone(),
            display_name: parsed.from_addr,
            captain_user: None,
        },
        content: ChannelContent::Text(text),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: false,
        thread_id: None,
        metadata,
    }
}

fn email_text_from_subject_body(subject: &str, body: &str) -> String {
    let clean_subject = EmailAdapter::strip_agent_tag(subject);
    if clean_subject.is_empty() {
        body.trim().to_string()
    } else {
        format!("Subject: {clean_subject}\n\n{}", body.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_email_adapter_creation() {
        let adapter = EmailAdapter::new(
            "imap.gmail.com".to_string(),
            993,
            "smtp.gmail.com".to_string(),
            587,
            "user@gmail.com".to_string(),
            "password".to_string(),
            30,
            vec![],
            vec![],
        );
        assert_eq!(adapter.name(), "email");
        assert_eq!(adapter.folders, vec!["INBOX".to_string()]);
    }

    #[test]
    fn named_email_adapter_preserves_account_identity() {
        let adapter = EmailAdapter::new_named(
            "email:work".to_string(),
            "work".to_string(),
            "imap.example.com".to_string(),
            993,
            "smtp.example.com".to_string(),
            587,
            "captain@example.com".to_string(),
            "secret".to_string(),
            30,
            vec!["INBOX".to_string()],
            vec!["@example.com".to_string()],
        );

        assert_eq!(adapter.name(), "email:work");
        assert_eq!(adapter.account_alias, "work");
        assert!(adapter.registration_aliases().is_empty());
        assert_eq!(
            adapter.poll_runtime(mpsc::channel(1).0).account_alias,
            "work"
        );
    }

    #[test]
    fn default_email_adapter_registers_its_explicit_alias() {
        let adapter = EmailAdapter::new_named(
            "email".to_string(),
            "work".to_string(),
            "imap.example.com".to_string(),
            993,
            "smtp.example.com".to_string(),
            587,
            "captain@example.com".to_string(),
            "secret".to_string(),
            30,
            vec!["INBOX".to_string()],
            vec!["@example.com".to_string()],
        );

        assert_eq!(adapter.registration_aliases(), vec!["email:work"]);
    }

    #[test]
    fn inbound_email_exposes_trusted_account_metadata() {
        let cursor = EmailImapCursor {
            folder: "INBOX".to_string(),
            uid_validity: 77,
            uid: 42,
        };
        let message = email_channel_message(
            "work",
            &cursor,
            ParsedEmail {
                from_addr: "alice@example.com".to_string(),
                subject: "[researcher] Hello".to_string(),
                rfc_message_id: "<message-1>".to_string(),
                body: "Body".to_string(),
            },
        );

        assert_eq!(
            message
                .metadata
                .get("account_id")
                .and_then(|value| value.as_str()),
            Some("work")
        );
        assert_eq!(message.platform_message_id, "email:work:5:INBOX:77:42");
        assert_eq!(
            message
                .metadata
                .get(DURABLE_INGRESS_ID_METADATA_KEY)
                .and_then(|value| value.as_str()),
            Some("email:work:5:INBOX:77:42")
        );
        assert_eq!(message.metadata["imap_folder"], serde_json::json!("INBOX"));
        assert_eq!(message.metadata["imap_uid"], serde_json::json!(42));
        assert_eq!(message.metadata["imap_uidvalidity"], serde_json::json!(77));
        assert_eq!(
            message.metadata["rfc_message_id"],
            serde_json::json!("<message-1>")
        );
        assert_eq!(message.sender.platform_id, "alice@example.com");
        assert_eq!(message.target_agent, None);
        assert_eq!(
            message.metadata[INTERNAL_TARGET_AGENT_NAME_METADATA_KEY],
            serde_json::json!("researcher")
        );
    }

    #[test]
    fn test_allowed_senders() {
        let adapter = EmailAdapter::new(
            "imap.example.com".to_string(),
            993,
            "smtp.example.com".to_string(),
            587,
            "bot@example.com".to_string(),
            "pass".to_string(),
            30,
            vec![],
            vec!["boss@company.com".to_string()],
        );
        assert!(adapter.is_allowed_sender("boss@company.com"));
        assert!(!adapter.is_allowed_sender("random@other.com"));

        // B.8 — empty allowed_senders now DENIES instead of allowing all.
        let denied = EmailAdapter::new(
            "imap.example.com".to_string(),
            993,
            "smtp.example.com".to_string(),
            587,
            "bot@example.com".to_string(),
            "pass".to_string(),
            30,
            vec![],
            vec![],
        );
        assert!(
            !denied.is_allowed_sender("anyone@anywhere.com"),
            "B.8 contract: empty allowed_senders must deny"
        );

        // `["*"]` is the explicit opt-in for permissive intake.
        let permissive = EmailAdapter::new(
            "imap.example.com".to_string(),
            993,
            "smtp.example.com".to_string(),
            587,
            "bot@example.com".to_string(),
            "pass".to_string(),
            30,
            vec![],
            vec!["*".to_string()],
        );
        assert!(permissive.is_allowed_sender("anyone@anywhere.com"));

        // Exact domain matching remains available without substring spoofing.
        let domain = EmailAdapter::new(
            "imap.example.com".to_string(),
            993,
            "smtp.example.com".to_string(),
            587,
            "bot@example.com".to_string(),
            "pass".to_string(),
            30,
            vec![],
            vec!["@company.com".to_string()],
        );
        assert!(domain.is_allowed_sender("alice@company.com"));
        assert!(domain.is_allowed_sender("bob@company.com"));
        assert!(domain.is_allowed_sender("ALICE@COMPANY.COM"));
        assert!(!domain.is_allowed_sender("alice@other.com"));
        assert!(!domain.is_allowed_sender("alice@company.com.attacker.test"));
    }

    #[test]
    fn email_sender_allowed_matches_adapter_contract() {
        assert!(!email_sender_allowed(&[], "alice@example.com"));
        assert!(email_sender_allowed(
            &["*".to_string()],
            "alice@example.com"
        ));
        assert!(email_sender_allowed(
            &["@example.com".to_string()],
            "alice@example.com"
        ));
        assert!(email_sender_allowed(
            &["ALICE@EXAMPLE.COM".to_string()],
            "alice@example.com"
        ));
        assert!(!email_sender_allowed(
            &["@example.com".to_string()],
            "alice@other.test"
        ));
        assert!(!email_sender_allowed(
            &["alice@example.com".to_string()],
            "alice@example.com.attacker.test"
        ));
        assert!(!email_sender_allowed(&["*".to_string()], ""));
        assert!(!email_sender_allowed(
            &["*".to_string()],
            "not-an-email-address"
        ));
    }

    #[test]
    fn public_allowlist_validator_matches_runtime_security_rules() {
        assert!(email_allowlist_rule_is_valid("*"));
        assert!(email_allowlist_rule_is_valid("alice@example.com"));
        assert!(email_allowlist_rule_is_valid("@example.com"));
        assert!(!email_allowlist_rule_is_valid(""));
        assert!(!email_allowlist_rule_is_valid("example.com"));
        assert!(!email_allowlist_rule_is_valid("@example.com/attacker"));
        assert!(email_address_is_valid("alice@example.com"));
        assert!(!email_address_is_valid("alice@example.com/attacker"));
    }

    #[test]
    fn configured_mailbox_identity_blocks_case_insensitive_reply_loops() {
        assert!(email_addresses_equal(
            "Captain@Example.COM",
            "captain@example.com"
        ));
        assert!(!email_addresses_equal(
            "captain@example.com",
            "captain@other.test"
        ));
        assert!(!email_addresses_equal("not-an-address", "not-an-address"));
    }

    #[test]
    fn email_text_from_subject_body_strips_agent_tag_and_trims_body() {
        assert_eq!(
            email_text_from_subject_body("[coder] Fix bug", "  Body text  "),
            "Subject: Fix bug\n\nBody text"
        );
        assert_eq!(
            email_text_from_subject_body("[coder]", "  Body text  "),
            "Body text"
        );
        assert_eq!(
            email_text_from_subject_body("", "  Body text  "),
            "Body text"
        );
    }

    #[test]
    fn outgoing_email_subject_body_uses_explicit_subject_or_reply_context() {
        let reply_ctx = DashMap::new();
        let (subject, body) = outgoing_email_subject_body(
            "Subject: Explicit\n\nBody".to_string(),
            "alice@example.com",
            &reply_ctx,
        );
        assert_eq!(subject, "Explicit");
        assert_eq!(body, "Body");

        remember_email_reply_context(&reply_ctx, "alice@example.com", "Original", "msg-1");
        let (subject, body) =
            outgoing_email_subject_body("Reply body".to_string(), "alice@example.com", &reply_ctx);
        assert_eq!(subject, "Re: Original");
        assert_eq!(body, "Reply body");

        let (subject, body) =
            outgoing_email_subject_body("No context".to_string(), "bob@example.com", &reply_ctx);
        assert_eq!(subject, "Captain Reply");
        assert_eq!(body, "No context");
    }

    #[test]
    fn email_mailboxes_extracts_recipient_and_validates_sender() {
        let user = ChannelUser {
            platform_id: "Alice <alice@example.com>".to_string(),
            display_name: "Alice".to_string(),
            captain_user: None,
        };
        let (to_addr, _, _) =
            email_mailboxes(&user, "captain@example.com").expect("mailboxes should parse");
        assert_eq!(to_addr, "alice@example.com");

        let err =
            email_mailboxes(&user, "not an email").expect_err("invalid sender mailbox should fail");
        assert!(err.contains("Invalid sender email"));
    }

    #[test]
    fn remember_email_reply_context_skips_empty_message_id() {
        let reply_ctx = DashMap::new();
        remember_email_reply_context(&reply_ctx, "alice@example.com", "Subject", "");
        assert!(reply_ctx.get("alice@example.com").is_none());

        remember_email_reply_context(&reply_ctx, "alice@example.com", "Subject", "msg-1");
        let stored = reply_ctx
            .get("alice@example.com")
            .expect("message id should store reply context");
        assert_eq!(stored.subject, "Subject");
        assert_eq!(stored.message_id, "msg-1");
    }

    #[test]
    fn parse_fetched_email_extracts_headers_and_plain_text() {
        let raw = b"From: Alice <alice@example.com>\r\nSubject: Hello\r\nMessage-ID: <msg-1>\r\nContent-Type: text/plain\r\n\r\nBody text";
        let parsed = parse_fetched_email(raw).expect("valid RFC822 message should parse");

        assert_eq!(parsed.from_addr, "alice@example.com");
        assert_eq!(parsed.subject, "Hello");
        assert_eq!(parsed.rfc_message_id, "<msg-1>");
        assert_eq!(parsed.body, "Body text");
    }

    #[test]
    fn imap_cursor_identity_is_injective_and_validated_before_ack() {
        let cursor = EmailImapCursor {
            folder: "Ops:Inbox".to_string(),
            uid_validity: 9,
            uid: 12,
        };
        let mut message = email_channel_message(
            "work",
            &cursor,
            ParsedEmail {
                from_addr: "alice@example.com".to_string(),
                subject: "Hello".to_string(),
                rfc_message_id: String::new(),
                body: "Body".to_string(),
            },
        );

        assert_eq!(message.platform_message_id, "email:work:9:Ops:Inbox:9:12");
        assert_eq!(
            email_cursor_from_message(&message, "work", &["Ops:Inbox".to_string()]),
            Ok(cursor.clone())
        );
        assert!(!message.metadata.contains_key("rfc_message_id"));

        message
            .metadata
            .insert("imap_uid".to_string(), serde_json::json!(13));
        let error = email_cursor_from_message(&message, "work", &["Ops:Inbox".to_string()])
            .expect_err("cursor tampering must be rejected");
        assert!(error.contains("identity does not match"));
    }

    #[test]
    fn test_extract_agent_from_subject() {
        assert_eq!(
            EmailAdapter::extract_agent_from_subject("[coder] Fix the bug"),
            Some("coder".to_string())
        );
        assert_eq!(
            EmailAdapter::extract_agent_from_subject("[researcher] Find papers on AI"),
            Some("researcher".to_string())
        );
        assert_eq!(
            EmailAdapter::extract_agent_from_subject("No brackets here"),
            None
        );
        assert_eq!(
            EmailAdapter::extract_agent_from_subject("[] Empty brackets"),
            None
        );
    }

    #[test]
    fn test_strip_agent_tag() {
        assert_eq!(
            EmailAdapter::strip_agent_tag("[coder] Fix the bug"),
            "Fix the bug"
        );
        assert_eq!(EmailAdapter::strip_agent_tag("No brackets"), "No brackets");
    }

    #[test]
    fn test_extract_email_addr() {
        assert_eq!(
            extract_email_addr("John Doe <john@example.com>"),
            "john@example.com"
        );
        assert_eq!(extract_email_addr("user@example.com"), "user@example.com");
        assert_eq!(extract_email_addr("<user@test.com>"), "user@test.com");
    }

    #[test]
    fn test_subject_extraction_from_body() {
        let text = "Subject: Test Subject\n\nThis is the body.";
        assert!(text.starts_with("Subject: "));
        let pos = text.find("\n\n").unwrap();
        let subject = &text[9..pos];
        let body = &text[pos + 2..];
        assert_eq!(subject, "Test Subject");
        assert_eq!(body, "This is the body.");
    }

    #[test]
    fn test_reply_ctx_threading() {
        let ctx_map: DashMap<String, ReplyCtx> = DashMap::new();
        ctx_map.insert(
            "user@test.com".to_string(),
            ReplyCtx {
                subject: "Original Subject".to_string(),
                message_id: "<msg-123@test.com>".to_string(),
            },
        );
        let ctx = ctx_map.get("user@test.com").unwrap();
        assert_eq!(ctx.subject, "Original Subject");
        assert_eq!(ctx.message_id, "<msg-123@test.com>");
    }
}
