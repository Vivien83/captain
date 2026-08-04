use super::*;
use axum::body::{to_bytes, Body};
use axum::extract::State;
use axum::http::{Request, Response};
use axum::Router;
use captain_types::email::{
    GmailComposeRequest, GmailDeliveryMode, GmailOutgoingAttachment, GmailSearchRequest,
};
use serde_json::json;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
enum Scenario {
    Search,
    Profile,
    History,
    HistoryExpired,
    Read,
    Deliver,
    MailboxOps,
    RateLimit,
    Unauthorized,
}

#[derive(Clone)]
struct MockState {
    scenario: Scenario,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

#[derive(Debug)]
struct RecordedRequest {
    method: String,
    uri: String,
    authorization: Option<String>,
    body: String,
}

async fn spawn_mock(scenario: Scenario) -> (GmailApiClient, Arc<Mutex<Vec<RecordedRequest>>>) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let state = MockState {
        scenario,
        requests: Arc::clone(&requests),
    };
    let app = Router::new().fallback(mock_handler).with_state(state);
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{address}/gmail/v1/users/me");
    (
        GmailApiClient::for_test("top-secret-access-token", &base).unwrap(),
        requests,
    )
}

async fn mock_handler(State(state): State<MockState>, request: Request<Body>) -> Response<Body> {
    let method = request.method().to_string();
    let uri = request.uri().to_string();
    let path = request.uri().path().to_string();
    let authorization = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = to_bytes(request.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    state.requests.lock().unwrap().push(RecordedRequest {
        method: method.clone(),
        uri,
        authorization,
        body: String::from_utf8_lossy(&body).into_owned(),
    });

    let (status, headers, value) = match state.scenario {
        Scenario::Search => search_response(&path),
        Scenario::Profile => profile_response(&path),
        Scenario::History => history_response(&path),
        Scenario::HistoryExpired => (
            StatusCode::NOT_FOUND,
            Vec::new(),
            json!({"error": {"errors": [{"reason": "notFound"}]}}),
        ),
        Scenario::Read => read_response(&path),
        Scenario::Deliver => delivery_response(&path),
        Scenario::MailboxOps => mailbox_ops_response(&path, &method),
        Scenario::RateLimit => (
            StatusCode::TOO_MANY_REQUESTS,
            vec![(header::RETRY_AFTER, "17")],
            json!({
                "error": {
                    "errors": [{"reason": "userRateLimitExceeded"}],
                    "message": "private-message-body-must-not-leak"
                }
            }),
        ),
        Scenario::Unauthorized => (
            StatusCode::UNAUTHORIZED,
            Vec::new(),
            json!({"error": {"message": "revoked private grant"}}),
        ),
    };
    let mut response = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json");
    for (name, value) in headers {
        response = response.header(name, value);
    }
    response.body(Body::from(value.to_string())).unwrap()
}

fn profile_response(
    path: &str,
) -> (
    StatusCode,
    Vec<(header::HeaderName, &'static str)>,
    serde_json::Value,
) {
    let value = match path {
        "/gmail/v1/users/me/profile" => json!({
            "emailAddress": "me@example.com",
            "messagesTotal": 42,
            "threadsTotal": 17,
            "historyId": "105"
        }),
        _ => return (StatusCode::NOT_FOUND, Vec::new(), json!({})),
    };
    (StatusCode::OK, Vec::new(), value)
}

fn search_response(
    path: &str,
) -> (
    StatusCode,
    Vec<(header::HeaderName, &'static str)>,
    serde_json::Value,
) {
    let value = match path {
        "/gmail/v1/users/me/messages" => json!({
            "messages": [{"id": "abc"}, {"id": "def"}],
            "nextPageToken": "next_1",
            "resultSizeEstimate": 23
        }),
        "/gmail/v1/users/me/messages/abc" => metadata_message("abc", "First"),
        "/gmail/v1/users/me/messages/def" => metadata_message("def", "Second"),
        _ => return (StatusCode::NOT_FOUND, Vec::new(), json!({})),
    };
    (StatusCode::OK, Vec::new(), value)
}

fn metadata_message(id: &str, subject: &str) -> serde_json::Value {
    json!({
        "id": id,
        "threadId": format!("thread_{id}"),
        "labelIds": ["INBOX", "UNREAD"],
        "snippet": format!("Snippet {id}"),
        "internalDate": "1785736800000",
        "sizeEstimate": 1234,
        "payload": {
            "headers": [
                {"name": "From", "value": "Sender <sender@example.com>"},
                {"name": "To", "value": "me@example.com"},
                {"name": "Subject", "value": subject}
            ]
        }
    })
}

fn history_response(
    path: &str,
) -> (
    StatusCode,
    Vec<(header::HeaderName, &'static str)>,
    serde_json::Value,
) {
    let value = match path {
        "/gmail/v1/users/me/history" => json!({
            "history": [
                {
                    "id": "101",
                    "messagesAdded": [{
                        "message": {
                            "id": "abc",
                            "threadId": "thread_abc",
                            "labelIds": ["INBOX", "UNREAD"]
                        }
                    }]
                },
                {
                    "id": "102",
                    "messagesAdded": [
                        {"message": {"id": "abc", "threadId": "thread_abc"}},
                        {"message": {"id": "def", "threadId": "thread_def", "labelIds": ["INBOX"]}}
                    ]
                }
            ],
            "nextPageToken": "history_next",
            "historyId": "105"
        }),
        _ => return (StatusCode::NOT_FOUND, Vec::new(), json!({})),
    };
    (StatusCode::OK, Vec::new(), value)
}

fn read_response(
    path: &str,
) -> (
    StatusCode,
    Vec<(header::HeaderName, &'static str)>,
    serde_json::Value,
) {
    let plain = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("Plain body");
    let deferred_html = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("<b>HTML body</b>");
    let value = match path {
        "/gmail/v1/users/me/messages/abc" => json!({
            "id": "abc",
            "threadId": "thread_abc",
            "labelIds": ["INBOX"],
            "snippet": "Plain body",
            "internalDate": "1785736800000",
            "sizeEstimate": 4567,
            "payload": {
                "mimeType": "multipart/mixed",
                "headers": [
                    {"name": "From", "value": "Sender <sender@example.com>"},
                    {"name": "To", "value": "me@example.com"},
                    {"name": "Subject", "value": "A thread"},
                    {"name": "Message-ID", "value": "<origin@example.com>"},
                    {"name": "References", "value": "<older@example.com>"},
                    {"name": "Reply-To", "value": "reply@example.com"}
                ],
                "parts": [
                    {"mimeType": "text/plain", "body": {"data": plain, "size": 10}},
                    {"mimeType": "text/html", "body": {"attachmentId": "html_1", "size": 16}},
                    {"mimeType": "application/pdf", "filename": "invoice.pdf", "body": {"attachmentId": "pdf_1", "size": 900}}
                ]
            }
        }),
        "/gmail/v1/users/me/messages/abc/attachments/html_1" => {
            json!({"data": deferred_html, "size": 16})
        }
        _ => return (StatusCode::NOT_FOUND, Vec::new(), json!({})),
    };
    (StatusCode::OK, Vec::new(), value)
}

fn delivery_response(
    path: &str,
) -> (
    StatusCode,
    Vec<(header::HeaderName, &'static str)>,
    serde_json::Value,
) {
    let value = match path {
        "/gmail/v1/users/me/messages/send" => {
            json!({"id": "sent_1", "threadId": "thread_sent"})
        }
        "/gmail/v1/users/me/drafts" => json!({
            "id": "draft_1",
            "message": {"id": "message_1", "threadId": "thread_draft"}
        }),
        _ => return (StatusCode::NOT_FOUND, Vec::new(), json!({})),
    };
    (StatusCode::OK, Vec::new(), value)
}

fn mailbox_ops_response(
    path: &str,
    method: &str,
) -> (
    StatusCode,
    Vec<(header::HeaderName, &'static str)>,
    serde_json::Value,
) {
    let attachment = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("attachment body");
    let value = match (method, path) {
        ("GET", "/gmail/v1/users/me/labels") => json!({
            "labels": [
                {"id": "INBOX", "name": "INBOX", "type": "system", "messagesTotal": 9, "messagesUnread": 2},
                {"id": "Label_1", "name": "Customers", "type": "user", "messagesTotal": 3, "messagesUnread": 1}
            ]
        }),
        ("POST", "/gmail/v1/users/me/messages/abc/modify") => {
            json!({"id": "abc", "threadId": "thread_abc", "labelIds": ["INBOX", "STARRED"]})
        }
        ("POST", "/gmail/v1/users/me/messages/abc/trash") => {
            json!({"id": "abc", "threadId": "thread_abc", "labelIds": ["TRASH"]})
        }
        ("POST", "/gmail/v1/users/me/messages/abc/untrash") => {
            json!({"id": "abc", "threadId": "thread_abc", "labelIds": ["INBOX"]})
        }
        ("GET", "/gmail/v1/users/me/messages/abc/attachments/pdf_1") => {
            json!({"data": attachment, "size": 15})
        }
        _ => return (StatusCode::NOT_FOUND, Vec::new(), json!({})),
    };
    (StatusCode::OK, Vec::new(), value)
}

fn compose(delivery: GmailDeliveryMode) -> GmailComposeRequest {
    GmailComposeRequest {
        account_alias: None,
        to: vec!["Recipient <recipient@example.com>".to_string()],
        cc: vec!["copy@example.com".to_string()],
        bcc: vec!["hidden@example.com".to_string()],
        reply_to: Some("replies@example.com".to_string()),
        subject: "Status report".to_string(),
        text_body: "Plain report".to_string(),
        html_body: Some("<strong>Plain report</strong>".to_string()),
        attachments: vec![GmailOutgoingAttachment {
            filename: "report.txt".to_string(),
            mime_type: "text/plain".to_string(),
            data: b"attachment".to_vec(),
        }],
        delivery,
    }
}

#[tokio::test]
async fn search_uses_bearer_headers_and_returns_bounded_metadata() {
    let (client, requests) = spawn_mock(Scenario::Search).await;
    let result = client
        .search_messages(&GmailSearchRequest {
            account_alias: None,
            query: "from:sender@example.com is:unread".to_string(),
            label_ids: vec!["INBOX".to_string()],
            max_results: 2,
            page_token: None,
            include_spam_trash: false,
        })
        .await
        .unwrap();

    assert_eq!(result.messages.len(), 2);
    assert_eq!(result.messages[0].subject.as_deref(), Some("First"));
    assert_eq!(result.next_page_token.as_deref(), Some("next_1"));
    assert_eq!(result.result_size_estimate, 23);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests.iter().all(|request| {
        request.authorization.as_deref() == Some("Bearer top-secret-access-token")
    }));
    assert!(requests
        .iter()
        .all(|request| !request.uri.contains("top-secret-access-token")));
    assert!(requests[0].uri.contains("includeSpamTrash=false"));
    assert!(requests[0].uri.contains("labelIds=INBOX"));
}

#[tokio::test]
async fn message_id_listing_is_a_single_bounded_request() {
    let (client, requests) = spawn_mock(Scenario::Search).await;
    let result = client
        .list_message_ids(&GmailSearchRequest {
            account_alias: None,
            query: "after:1785736800".to_string(),
            label_ids: Vec::new(),
            max_results: 2,
            page_token: None,
            include_spam_trash: true,
        })
        .await
        .unwrap();

    assert_eq!(result.message_ids, vec!["abc", "def"]);
    assert_eq!(result.next_page_token.as_deref(), Some("next_1"));
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].uri.contains("after%3A1785736800"));
    assert!(requests[0].uri.contains("includeSpamTrash=true"));
}

#[test]
fn response_page_tokens_are_bounded_before_they_reenter_requests() {
    assert_eq!(
        validate_page_token_response(Some("next_1".to_string()), "test token").unwrap(),
        Some("next_1".to_string())
    );
    for malformed in [
        String::new(),
        "next\nheader".to_string(),
        "x".repeat(MAX_PAGE_TOKEN_BYTES + 1),
    ] {
        let error = validate_page_token_response(Some(malformed), "test token").unwrap_err();
        assert!(matches!(error, GmailApiError::InvalidResponse(_)));
    }
}

#[tokio::test]
async fn profile_returns_the_live_cursor_without_exposing_the_access_token() {
    let (client, requests) = spawn_mock(Scenario::Profile).await;
    let profile = client.mailbox_profile().await.unwrap();

    assert_eq!(profile.email_address, "me@example.com");
    assert_eq!(profile.messages_total, 42);
    assert_eq!(profile.threads_total, 17);
    assert_eq!(profile.history_id, "105");
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert!(requests[0].uri.ends_with("/profile"));
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer top-secret-access-token")
    );
    assert!(!requests[0].uri.contains("top-secret-access-token"));
}

#[tokio::test]
async fn public_message_summary_fetches_metadata_without_body_or_attachments() {
    let (client, requests) = spawn_mock(Scenario::Search).await;
    let summary = client.message_summary("abc").await.unwrap();

    assert_eq!(summary.subject.as_deref(), Some("First"));
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].uri.contains("format=metadata"));
    assert!(!requests[0].uri.contains("attachments"));
}

#[tokio::test]
async fn history_lists_only_added_messages_and_deduplicates_repeated_records() {
    let (client, requests) = spawn_mock(Scenario::History).await;
    let page = client
        .list_history(&GmailHistoryRequest {
            start_history_id: "100".to_string(),
            page_token: None,
            max_results: 200,
        })
        .await
        .unwrap();

    assert_eq!(page.history_id, "105");
    assert_eq!(page.next_page_token.as_deref(), Some("history_next"));
    assert_eq!(page.messages_added.len(), 2);
    assert_eq!(page.messages_added[0].message_id, "abc");
    assert_eq!(page.messages_added[0].history_id, "101");
    assert_eq!(page.messages_added[1].message_id, "def");
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].uri.contains("startHistoryId=100"));
    assert!(requests[0].uri.contains("maxResults=200"));
    assert!(requests[0].uri.contains("historyTypes=messageAdded"));
}

#[tokio::test]
async fn expired_history_cursor_has_a_dedicated_full_sync_signal() {
    let (client, _) = spawn_mock(Scenario::HistoryExpired).await;
    let error = client
        .list_history(&GmailHistoryRequest {
            start_history_id: "100".to_string(),
            page_token: None,
            max_results: 100,
        })
        .await
        .unwrap_err();

    assert_eq!(error, GmailApiError::HistoryExpired);
    assert_eq!(error.code(), "gmail_history_expired");
    assert!(!error.requires_reauthorization());
}

#[tokio::test]
async fn read_decodes_inline_and_deferred_body_without_implicit_attachment_bytes() {
    let (client, requests) = spawn_mock(Scenario::Read).await;
    let message = client.read_message("abc", 4096).await.unwrap();

    assert_eq!(message.body_text.as_deref(), Some("Plain body"));
    assert_eq!(message.body_html.as_deref(), Some("<b>HTML body</b>"));
    assert!(!message.body_truncated);
    assert_eq!(message.reply_to.as_deref(), Some("reply@example.com"));
    assert_eq!(message.attachments.len(), 1);
    assert_eq!(message.attachments[0].filename, "invoice.pdf");
    assert_eq!(message.attachments[0].attachment_id, "pdf_1");
    assert_eq!(requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn reply_metadata_never_downloads_body_parts_or_attachments() {
    let (client, requests) = spawn_mock(Scenario::Read).await;
    let message = client.read_reply_metadata("abc").await.unwrap();

    assert_eq!(message.reply_to.as_deref(), Some("reply@example.com"));
    assert_eq!(
        message.message_id_header.as_deref(),
        Some("<origin@example.com>")
    );
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].uri.contains("format=metadata"));
    assert!(!requests[0].uri.contains("attachments"));
}

#[tokio::test]
async fn send_and_draft_use_rfc_message_base64url_payloads() {
    let (client, requests) = spawn_mock(Scenario::Deliver).await;
    let raw = build_gmail_mime(&compose(GmailDeliveryMode::Send), "me@example.com", None).unwrap();
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(raw.as_bytes())
        .unwrap();
    let formatted = String::from_utf8_lossy(&decoded);
    assert!(formatted.contains("To: Recipient <recipient@example.com>"));
    assert!(formatted.contains("Bcc: hidden@example.com"));
    assert!(formatted.contains("filename=\"report.txt\""));

    let sent = client.send_message(&raw, None).await.unwrap();
    assert_eq!(sent.message_id, "sent_1");
    let draft = client
        .create_draft(&raw, Some("thread_draft"))
        .await
        .unwrap();
    assert_eq!(draft.id, "draft_1");
    assert_eq!(draft.message_id, "message_1");

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| request.method == "POST"));
    assert!(requests[0].body.contains("\"raw\""));
    assert!(requests[1].body.contains("\"threadId\":\"thread_draft\""));
}

#[tokio::test]
async fn labels_mutations_and_attachment_download_use_exact_gmail_methods() {
    let (client, requests) = spawn_mock(Scenario::MailboxOps).await;
    let labels = client.list_labels().await.unwrap();
    assert_eq!(labels.len(), 2);
    assert_eq!(labels[1].name, "Customers");

    let modified = client
        .modify_message("abc", &["STARRED".to_string()], &["UNREAD".to_string()])
        .await
        .unwrap();
    assert_eq!(modified.label_ids, vec!["INBOX", "STARRED"]);
    assert_eq!(
        client.set_trashed("abc", true).await.unwrap().label_ids,
        vec!["TRASH"]
    );
    assert_eq!(
        client.set_trashed("abc", false).await.unwrap().label_ids,
        vec!["INBOX"]
    );
    assert_eq!(
        client
            .download_attachment("abc", "pdf_1", 1024)
            .await
            .unwrap()
            .data,
        b"attachment body"
    );

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 5);
    assert!(requests[1].body.contains("\"addLabelIds\":[\"STARRED\"]"));
    assert!(requests[1].body.contains("\"removeLabelIds\":[\"UNREAD\"]"));
    assert_eq!(requests[2].method, "POST");
    assert!(requests[2].uri.ends_with("/trash"));
    assert!(requests[3].uri.ends_with("/untrash"));
}

#[tokio::test]
async fn google_errors_are_categorized_without_reflecting_private_bodies() {
    let (client, _) = spawn_mock(Scenario::RateLimit).await;
    let error = client.list_labels().await.unwrap_err();

    assert_eq!(
        error,
        GmailApiError::RateLimited {
            retry_after_seconds: Some(17)
        }
    );
    assert_eq!(error.code(), "gmail_rate_limited");
    assert!(!error.to_string().contains("private-message-body"));
    assert!(!error.to_string().contains("top-secret-access-token"));
}

#[tokio::test]
async fn unauthorized_is_the_only_api_failure_that_requires_reauthentication() {
    let (client, _) = spawn_mock(Scenario::Unauthorized).await;
    let error = client.list_labels().await.unwrap_err();
    assert_eq!(error, GmailApiError::Unauthorized);
    assert!(error.requires_reauthorization());

    let rate_limit = GmailApiError::RateLimited {
        retry_after_seconds: Some(5),
    };
    assert!(!rate_limit.requires_reauthorization());
}

#[test]
fn mime_threading_headers_are_preserved_without_header_injection() {
    let raw = build_gmail_mime(
        &compose(GmailDeliveryMode::Draft),
        "me@example.com",
        Some(&GmailThreadingHeaders {
            thread_id: "thread_abc".to_string(),
            in_reply_to: Some("<origin@example.com>".to_string()),
            references: Some("<older@example.com> <origin@example.com>".to_string()),
        }),
    )
    .unwrap();
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(raw.as_bytes())
        .unwrap();
    let formatted = String::from_utf8_lossy(&decoded);
    assert!(formatted.contains("In-Reply-To: <origin@example.com>"));
    assert!(formatted.contains("References: <older@example.com> <origin@example.com>"));
}

#[test]
fn compose_rejects_header_injection_and_oversized_attachment_sets() {
    let mut request = compose(GmailDeliveryMode::Send);
    request.subject = "hello\r\nBcc: attacker@example.com".to_string();
    assert!(matches!(
        build_gmail_mime(&request, "me@example.com", None),
        Err(GmailApiError::InvalidInput(_))
    ));

    let mut request = compose(GmailDeliveryMode::Send);
    request.attachments[0].filename = "../secret.txt".to_string();
    assert!(matches!(
        build_gmail_mime(&request, "me@example.com", None),
        Err(GmailApiError::InvalidInput(_))
    ));
}

#[test]
fn search_validation_blocks_unbounded_or_control_character_inputs() {
    let invalid = GmailSearchRequest {
        account_alias: None,
        query: "subject:test\nlabel:inbox".to_string(),
        label_ids: Vec::new(),
        max_results: 51,
        page_token: None,
        include_spam_trash: false,
    };
    assert!(matches!(
        validate_search_request(&invalid),
        Err(GmailApiError::InvalidInput(_))
    ));
}

#[test]
fn history_validation_rejects_non_numeric_or_unbounded_cursors() {
    assert!(matches!(
        validate_history_request(&GmailHistoryRequest {
            start_history_id: "not-a-history-id".to_string(),
            page_token: None,
            max_results: 501,
        }),
        Err(GmailApiError::InvalidInput(_))
    ));
}
