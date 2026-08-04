//! Public-safe authentication probe for the xAI inference API.
//!
//! xAI exposes `/v1/me` for both API keys and OAuth bearer tokens. API keys
//! additionally expose their inference ACLs through `/v1/api-key`. These
//! endpoints validate credentials without spending inference tokens.

use futures::StreamExt;
use reqwest::{Client, Response, StatusCode, Url};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::time::{Duration, Instant};
use thiserror::Error;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum XaiAuthMethod {
    ApiKey,
    OAuth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct XaiAuthProbe {
    pub auth_method: XaiAuthMethod,
    pub active: bool,
    pub inference_ready: bool,
    pub latency_ms: u64,
    pub zdr_status: String,
    pub team_blocked: bool,
    pub credential_blocked: bool,
    pub credential_disabled: bool,
    pub permissions_verified: bool,
    pub chat_endpoint_allowed: Option<bool>,
    pub selected_model_allowed: Option<bool>,
}

impl XaiAuthProbe {
    pub fn readiness_error(&self, required_model: Option<&str>) -> Option<String> {
        if self.team_blocked {
            return Some("xAI team is blocked".to_string());
        }
        if self.credential_blocked {
            return Some("xAI credential is blocked".to_string());
        }
        if self.credential_disabled {
            return Some("xAI credential is disabled".to_string());
        }
        if self.chat_endpoint_allowed == Some(false) {
            return Some("xAI API key lacks the chat endpoint ACL".to_string());
        }
        if self.selected_model_allowed == Some(false) {
            return Some(
                match required_model.filter(|model| !model.trim().is_empty()) {
                    Some(model) => format!("xAI API key lacks a model ACL for {model}"),
                    None => "xAI API key lacks a model ACL".to_string(),
                },
            );
        }
        if !self.inference_ready {
            return Some("xAI credential is not ready for inference".to_string());
        }
        None
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum XaiAuthProbeError {
    #[error("xAI base URL is invalid or unsafe")]
    InvalidBaseUrl,
    #[error("xAI authentication request timed out")]
    Timeout,
    #[error("xAI authentication endpoint is unreachable")]
    Unreachable,
    #[error("xAI authentication request failed")]
    Transport,
    #[error("xAI rejected the credential")]
    Rejected,
    #[error("xAI denied access to the authentication endpoint")]
    Forbidden,
    #[error("xAI authentication endpoint is rate limited")]
    RateLimited,
    #[error("xAI authentication endpoint is unavailable")]
    ProviderUnavailable,
    #[error("xAI authentication endpoint returned HTTP {0}")]
    UnexpectedStatus(u16),
    #[error("xAI authentication response exceeded the safety limit")]
    ResponseTooLarge,
    #[error("xAI authentication response was invalid")]
    InvalidResponse,
}

#[derive(Debug, Deserialize)]
struct MeResponse {
    zdr_status: String,
    team_blocked: bool,
    api_key: Option<MeApiKeyInfo>,
    oauth: Option<MeOAuthInfo>,
}

#[derive(Debug, Deserialize)]
struct MeApiKeyInfo {
    blocked: bool,
    disabled: bool,
}

#[derive(Debug, Deserialize)]
struct MeOAuthInfo {
    client_id: String,
}

#[derive(Debug, Deserialize)]
struct ApiKeyInfo {
    acls: Vec<String>,
    team_blocked: bool,
    api_key_blocked: bool,
    api_key_disabled: bool,
}

/// Validate an xAI API key or externally issued OAuth bearer without making
/// a billable model request.
pub async fn probe_xai_auth(
    base_url: &str,
    bearer: &str,
    required_model: Option<&str>,
) -> Result<XaiAuthProbe, XaiAuthProbeError> {
    if bearer.trim().is_empty() {
        return Err(XaiAuthProbeError::Rejected);
    }

    let me_url = endpoint_url(base_url, "me")?;
    let api_key_url = endpoint_url(base_url, "api-key")?;
    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(crate::USER_AGENT)
        .build()
        .map_err(|_| XaiAuthProbeError::Transport)?;

    let started = Instant::now();
    let me: MeResponse = get_json(&client, me_url, bearer).await?;
    let zdr_status = normalized_zdr_status(&me.zdr_status);

    match (me.api_key, me.oauth) {
        (Some(api_key), None) => {
            let active = !me.team_blocked && !api_key.blocked && !api_key.disabled;
            if !active {
                return Ok(XaiAuthProbe {
                    auth_method: XaiAuthMethod::ApiKey,
                    active,
                    inference_ready: false,
                    latency_ms: started.elapsed().as_millis() as u64,
                    zdr_status,
                    team_blocked: me.team_blocked,
                    credential_blocked: api_key.blocked,
                    credential_disabled: api_key.disabled,
                    permissions_verified: false,
                    chat_endpoint_allowed: None,
                    selected_model_allowed: None,
                });
            }

            let details: ApiKeyInfo = get_json(&client, api_key_url, bearer).await?;
            let team_blocked = me.team_blocked || details.team_blocked;
            let credential_blocked = api_key.blocked || details.api_key_blocked;
            let credential_disabled = api_key.disabled || details.api_key_disabled;
            let chat_endpoint_allowed = acl_allows(&details.acls, "endpoint", Some("chat"));
            let selected_model_allowed = acl_allows(
                &details.acls,
                "model",
                normalized_required_model(required_model),
            );
            let active = !team_blocked && !credential_blocked && !credential_disabled;

            Ok(XaiAuthProbe {
                auth_method: XaiAuthMethod::ApiKey,
                active,
                inference_ready: active && chat_endpoint_allowed && selected_model_allowed,
                latency_ms: started.elapsed().as_millis() as u64,
                zdr_status,
                team_blocked,
                credential_blocked,
                credential_disabled,
                permissions_verified: true,
                chat_endpoint_allowed: Some(chat_endpoint_allowed),
                selected_model_allowed: Some(selected_model_allowed),
            })
        }
        (None, Some(oauth)) if !oauth.client_id.trim().is_empty() => {
            let active = !me.team_blocked;
            Ok(XaiAuthProbe {
                auth_method: XaiAuthMethod::OAuth,
                active,
                inference_ready: active,
                latency_ms: started.elapsed().as_millis() as u64,
                zdr_status,
                team_blocked: me.team_blocked,
                credential_blocked: false,
                credential_disabled: false,
                permissions_verified: false,
                chat_endpoint_allowed: None,
                selected_model_allowed: None,
            })
        }
        _ => Err(XaiAuthProbeError::InvalidResponse),
    }
}

fn endpoint_url(base_url: &str, endpoint: &str) -> Result<Url, XaiAuthProbeError> {
    let mut url = Url::parse(base_url).map_err(|_| XaiAuthProbeError::InvalidBaseUrl)?;
    if !safe_transport(&url)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(XaiAuthProbeError::InvalidBaseUrl);
    }

    let base_path = url.path().trim_end_matches('/');
    let versioned_path = if base_path.ends_with("/v1") {
        base_path.to_string()
    } else if base_path.is_empty() {
        "/v1".to_string()
    } else {
        format!("{base_path}/v1")
    };
    url.set_path(&format!("{versioned_path}/{endpoint}"));
    Ok(url)
}

fn safe_transport(url: &Url) -> bool {
    match url.scheme() {
        "https" => true,
        "http" => url.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<IpAddr>()
                    .map(|address| address.is_loopback())
                    .unwrap_or(false)
        }),
        _ => false,
    }
}

async fn get_json<T: for<'de> Deserialize<'de>>(
    client: &Client,
    url: Url,
    bearer: &str,
) -> Result<T, XaiAuthProbeError> {
    let response = client
        .get(url)
        .bearer_auth(bearer)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(map_transport_error)?;
    ensure_success(response.status())?;
    let body = read_limited(response).await?;
    serde_json::from_slice(&body).map_err(|_| XaiAuthProbeError::InvalidResponse)
}

fn ensure_success(status: StatusCode) -> Result<(), XaiAuthProbeError> {
    if status.is_success() {
        return Ok(());
    }
    Err(match status.as_u16() {
        400 | 401 => XaiAuthProbeError::Rejected,
        403 => XaiAuthProbeError::Forbidden,
        429 => XaiAuthProbeError::RateLimited,
        500..=599 => XaiAuthProbeError::ProviderUnavailable,
        status => XaiAuthProbeError::UnexpectedStatus(status),
    })
}

async fn read_limited(response: Response) -> Result<Vec<u8>, XaiAuthProbeError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(XaiAuthProbeError::ResponseTooLarge);
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(map_transport_error)?;
        if chunk.len() > MAX_RESPONSE_BYTES.saturating_sub(body.len()) {
            return Err(XaiAuthProbeError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn map_transport_error(error: reqwest::Error) -> XaiAuthProbeError {
    if error.is_timeout() {
        XaiAuthProbeError::Timeout
    } else if error.is_connect() {
        XaiAuthProbeError::Unreachable
    } else {
        XaiAuthProbeError::Transport
    }
}

fn normalized_zdr_status(status: &str) -> String {
    match status {
        "no_zdr" | "zdr" | "pii_scrubbing" => status.to_string(),
        _ => "unknown".to_string(),
    }
}

fn normalized_required_model(model: Option<&str>) -> Option<&str> {
    model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(|model| model.strip_prefix("xai/").unwrap_or(model))
}

fn acl_allows(acls: &[String], family: &str, required: Option<&str>) -> bool {
    let prefix = format!("api-key:{family}:");
    let wildcard = format!("{prefix}*");
    if acls.iter().any(|acl| acl == &wildcard) {
        return true;
    }
    match required {
        Some(value) => {
            let exact = format!("{prefix}{value}");
            acls.iter().any(|acl| acl == &exact)
        }
        None => acls.iter().any(|acl| {
            acl.strip_prefix(&prefix)
                .is_some_and(|value| !value.is_empty())
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn me_api_key() -> serde_json::Value {
        serde_json::json!({
            "user_id": "user-private",
            "team_id": "team-private",
            "zdr_status": "zdr",
            "team_blocked": false,
            "api_key": {
                "redacted_api_key": "xai-...safe",
                "api_key_id": "key-private",
                "blocked": false,
                "disabled": false
            },
            "oauth": null
        })
    }

    fn api_key_details(acls: Vec<&str>) -> serde_json::Value {
        serde_json::json!({
            "redacted_api_key": "xai-...safe",
            "user_id": "user-private",
            "team_id": "team-private",
            "api_key_id": "key-private",
            "acls": acls,
            "team_blocked": false,
            "api_key_blocked": false,
            "api_key_disabled": false
        })
    }

    #[tokio::test]
    async fn api_key_probe_verifies_status_and_inference_acls() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/me"))
            .and(header("authorization", "Bearer xai-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(me_api_key()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/api-key"))
            .and(header("authorization", "Bearer xai-secret"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(api_key_details(vec![
                    "api-key:endpoint:chat",
                    "api-key:model:grok-4.5",
                ])),
            )
            .expect(1)
            .mount(&server)
            .await;

        let result = probe_xai_auth(&server.uri(), "xai-secret", Some("xai/grok-4.5"))
            .await
            .expect("probe should pass");

        assert_eq!(result.auth_method, XaiAuthMethod::ApiKey);
        assert!(result.active);
        assert!(result.inference_ready);
        assert!(result.permissions_verified);
        assert_eq!(result.chat_endpoint_allowed, Some(true));
        assert_eq!(result.selected_model_allowed, Some(true));
        assert_eq!(result.zdr_status, "zdr");
    }

    #[tokio::test]
    async fn oauth_bearer_uses_me_without_api_key_introspection() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "user_id": "user-private",
                "team_id": "team-private",
                "zdr_status": "no_zdr",
                "team_blocked": false,
                "api_key": null,
                "oauth": {"client_id": "official-client"}
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/api-key"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let result = probe_xai_auth(&server.uri(), "oauth-secret", Some("grok-4.5"))
            .await
            .expect("OAuth probe should pass");

        assert_eq!(result.auth_method, XaiAuthMethod::OAuth);
        assert!(result.inference_ready);
        assert!(!result.permissions_verified);
        assert_eq!(result.chat_endpoint_allowed, None);
    }

    #[tokio::test]
    async fn api_key_without_model_acl_is_authenticated_but_not_ready() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(me_api_key()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/api-key"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(api_key_details(vec![
                    "api-key:endpoint:chat",
                    "api-key:model:grok-4.3",
                ])),
            )
            .mount(&server)
            .await;

        let result = probe_xai_auth(&server.uri(), "xai-secret", Some("grok-4.5"))
            .await
            .expect("credential itself is valid");

        assert!(result.active);
        assert!(!result.inference_ready);
        assert_eq!(result.selected_model_allowed, Some(false));
        assert_eq!(
            result.readiness_error(Some("grok-4.5")).as_deref(),
            Some("xAI API key lacks a model ACL for grok-4.5")
        );
    }

    #[tokio::test]
    async fn redirects_are_not_followed_with_a_bearer() {
        let source = MockServer::start().await;
        let destination = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/me"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/v1/me", destination.uri())),
            )
            .expect(1)
            .mount(&source)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/me"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&destination)
            .await;

        let error = probe_xai_auth(&source.uri(), "never-forward", None)
            .await
            .expect_err("redirect must be rejected");

        assert_eq!(error, XaiAuthProbeError::UnexpectedStatus(302));
    }

    #[tokio::test]
    async fn hostile_error_bodies_and_large_successes_are_never_exposed() {
        let rejected = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/me"))
            .respond_with(ResponseTemplate::new(401).set_body_string("leaked-xai-secret"))
            .mount(&rejected)
            .await;

        let error = probe_xai_auth(&rejected.uri(), "leaked-xai-secret", None)
            .await
            .expect_err("credential should be rejected");
        assert_eq!(error, XaiAuthProbeError::Rejected);
        assert!(!error.to_string().contains("leaked-xai-secret"));

        let oversized = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/me"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("x".repeat(MAX_RESPONSE_BYTES + 1)),
            )
            .mount(&oversized)
            .await;
        let error = probe_xai_auth(&oversized.uri(), "xai-secret", None)
            .await
            .expect_err("oversized response must fail");
        assert_eq!(error, XaiAuthProbeError::ResponseTooLarge);
    }

    #[test]
    fn endpoint_builder_accepts_https_and_loopback_only() {
        assert_eq!(
            endpoint_url("https://api.x.ai/v1", "me")
                .expect("official URL")
                .as_str(),
            "https://api.x.ai/v1/me"
        );
        assert_eq!(
            endpoint_url("http://127.0.0.1:8080/proxy", "me")
                .expect("loopback proxy")
                .as_str(),
            "http://127.0.0.1:8080/proxy/v1/me"
        );
        assert_eq!(
            endpoint_url("http://example.com/v1", "me"),
            Err(XaiAuthProbeError::InvalidBaseUrl)
        );
    }
}
