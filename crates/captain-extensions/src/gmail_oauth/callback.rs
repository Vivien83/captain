use crate::{ExtensionError, ExtensionResult};
use axum::extract::Query;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, Mutex};
use zeroize::Zeroizing;

const OAUTH_FLOW_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const CALLBACK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const CALLBACK_SUCCESS_HTML: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Captain Gmail</title></head><body><main><h1>Gmail connected</h1><p>You can close this tab and return to Captain.</p></main></body></html>";
const CALLBACK_ERROR_HTML: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Captain Gmail</title></head><body><main><h1>Connection refused</h1><p>Return to Captain for the actionable error.</p></main></body></html>";

pub(super) struct CallbackServer {
    result_rx: oneshot::Receiver<CallbackResult>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    server_task: Option<tokio::task::JoinHandle<()>>,
}

impl CallbackServer {
    pub(super) async fn wait_for_code(mut self) -> ExtensionResult<Zeroizing<String>> {
        let outcome = tokio::time::timeout(OAUTH_FLOW_TIMEOUT, &mut self.result_rx).await;
        self.stop().await;
        match outcome {
            Err(_) => Err(ExtensionError::OAuth(
                "Gmail authorization timed out after 10 minutes".to_string(),
            )),
            Ok(Err(_)) => Err(ExtensionError::OAuth(
                "Gmail callback server stopped before authorization completed".to_string(),
            )),
            Ok(Ok(CallbackResult::Code(code))) => Ok(Zeroizing::new(code)),
            Ok(Ok(CallbackResult::Denied(code))) => Err(ExtensionError::OAuth(format!(
                "Google authorization was not granted ({code})"
            ))),
            Ok(Ok(CallbackResult::InvalidResponse)) => Err(ExtensionError::OAuth(
                "Google callback did not contain a valid authorization code".to_string(),
            )),
        }
    }

    pub(super) async fn shutdown(mut self) {
        self.stop().await;
    }

    async fn stop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        if let Some(mut task) = self.server_task.take() {
            if tokio::time::timeout(CALLBACK_SHUTDOWN_TIMEOUT, &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
    }
}

enum CallbackResult {
    Code(String),
    Denied(String),
    InvalidResponse,
}

#[derive(Deserialize)]
struct CallbackParams {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: String,
    #[serde(default)]
    error: Option<String>,
}

pub(super) fn start_callback_server(
    listener: tokio::net::TcpListener,
    expected_host: String,
    expected_state: String,
) -> CallbackServer {
    let (result_tx, result_rx) = oneshot::channel();
    let result_tx = Arc::new(Mutex::new(Some(result_tx)));
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let app = Router::new().route(
        "/callback",
        get({
            let result_tx = Arc::clone(&result_tx);
            move |headers: HeaderMap, Query(query): Query<CallbackParams>| {
                let result_tx = Arc::clone(&result_tx);
                let expected_host = expected_host.clone();
                let expected_state = expected_state.clone();
                async move {
                    let host_matches = headers
                        .get(header::HOST)
                        .and_then(|host| host.to_str().ok())
                        .is_some_and(|host| host == expected_host);
                    if !host_matches || !state_matches(&expected_state, &query.state) {
                        return callback_response(StatusCode::BAD_REQUEST, CALLBACK_ERROR_HTML);
                    }
                    let outcome = if let Some(error) = query.error {
                        CallbackResult::Denied(sanitize_oauth_code(&error))
                    } else if let Some(code) = query.code {
                        if code.is_empty() || code.len() > 8192 || code.contains(['\n', '\r']) {
                            CallbackResult::InvalidResponse
                        } else {
                            CallbackResult::Code(code)
                        }
                    } else {
                        CallbackResult::InvalidResponse
                    };
                    let is_code = matches!(outcome, CallbackResult::Code(_));
                    let delivered = match result_tx.lock().await.take() {
                        Some(sender) => sender.send(outcome).is_ok(),
                        None => false,
                    };
                    let success = is_code && delivered;
                    callback_response(
                        if success {
                            StatusCode::OK
                        } else {
                            StatusCode::BAD_REQUEST
                        },
                        if success {
                            CALLBACK_SUCCESS_HTML
                        } else {
                            CALLBACK_ERROR_HTML
                        },
                    )
                }
            }
        }),
    );
    let server_task = tokio::spawn(async move {
        let server = axum::serve(listener, app).with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        });
        let _ = server.await;
    });
    CallbackServer {
        result_rx,
        shutdown_tx: Some(shutdown_tx),
        server_task: Some(server_task),
    }
}

fn callback_response(status: StatusCode, html: &'static str) -> Response {
    let mut response = (status, Html(html)).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'",
        ),
    );
    response
}

fn state_matches(expected: &str, received: &str) -> bool {
    let expected_hash = Sha256::digest(expected.as_bytes());
    let received_hash = Sha256::digest(received.as_bytes());
    expected_hash == received_hash
}

fn sanitize_oauth_code(value: &str) -> String {
    let code: String = value
        .chars()
        .take(64)
        .filter(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '-' | '.')
        })
        .collect();
    if code.is_empty() {
        "oauth_denied".to_string()
    } else {
        code
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_material_is_static_and_state_is_bound() {
        assert!(state_matches("expected", "expected"));
        assert!(!state_matches("expected", "attacker"));
        assert!(!CALLBACK_SUCCESS_HTML.contains("<script"));
        assert!(!CALLBACK_ERROR_HTML.contains("{error}"));
        let response = callback_response(StatusCode::OK, CALLBACK_SUCCESS_HTML);
        assert_eq!(
            response
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some("no-store")
        );
        assert!(response
            .headers()
            .contains_key(header::CONTENT_SECURITY_POLICY));
    }

    #[tokio::test]
    async fn callback_server_rejects_bad_state_and_accepts_exactly_one_code() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let expected_host = format!("127.0.0.1:{port}");
        let callback = start_callback_server(
            listener,
            expected_host.clone(),
            "expected-state".to_string(),
        );
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();

        let rejected = http
            .get(format!(
                "http://{expected_host}/callback?state=wrong-state&code=ignored"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

        let accepted = http
            .get(format!(
                "http://{expected_host}/callback?state=expected-state&code=first-code"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(accepted.status(), StatusCode::OK);

        let replay = http
            .get(format!(
                "http://{expected_host}/callback?state=expected-state&code=replayed-code"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(replay.status(), StatusCode::BAD_REQUEST);

        let code = callback.wait_for_code().await.unwrap();
        assert_eq!(code.as_str(), "first-code");
    }

    #[test]
    fn oauth_error_codes_are_bounded_and_non_reflective() {
        assert_eq!(sanitize_oauth_code("access_denied"), "access_denied");
        assert_eq!(
            sanitize_oauth_code("<script>alert(1)</script>"),
            "scriptalert1script"
        );
        assert_eq!(sanitize_oauth_code("!!!"), "oauth_denied");
        assert!(sanitize_oauth_code(&"a".repeat(100)).len() <= 64);
    }
}
