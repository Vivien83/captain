//! Per-agent API outbound callback delivery.

use crate::ssrf_pin::resolve_pinned_socket_addr;
use captain_types::{
    agent::AgentId,
    agent_api::{
        agent_api_callback_secret_env as shared_agent_api_callback_secret_env,
        agent_api_callback_url_env as shared_agent_api_callback_url_env,
        agent_api_egress_configure_url, agent_api_egress_queue_url,
        agent_api_egress_retry_url_template, agent_api_egress_test_url,
    },
};
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use std::time::Duration;

const MAX_AGENT_API_CALLBACK_BODY_SIZE: usize = 256 * 1024;
const MAX_AGENT_API_CALLBACK_TEXT_SIZE: usize = 64 * 1024;
const MIN_AGENT_API_CALLBACK_SECRET_LEN: usize = 16;
const AGENT_API_CALLBACK_TIMEOUT_SECS: u64 = 10;
const AGENT_API_CALLBACK_MAX_ATTEMPTS: u8 = 2;
const ALLOW_LOCAL_AGENT_API_CALLBACKS_ENV: &str = "CAPTAIN_AGENT_API_ALLOW_LOCAL_CALLBACKS";

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Serialize)]
pub struct AgentApiEgressDescriptor {
    pub callback_url_env: String,
    pub callback_secret_env: String,
    pub callback_configured: bool,
    pub config_status: AgentApiCallbackConfigStatus,
    pub event_header: &'static str,
    pub signature_header: &'static str,
    pub max_payload_bytes: usize,
    pub timeout_secs: u64,
    pub max_attempts: u8,
    pub configure_url: String,
    pub test_url: String,
    pub queue_status_url: String,
    pub retry_url_template: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentApiCallbackConfigStatus {
    pub callback_url_env: String,
    pub callback_secret_env: String,
    pub callback_url_configured: bool,
    pub callback_url_valid: bool,
    pub callback_secret_configured: bool,
    pub ready: bool,
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AgentApiCallbackDelivery {
    attempted: bool,
    delivered: bool,
    attempts: u8,
    retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    queued_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    queue_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl AgentApiCallbackDelivery {
    pub(crate) fn delivered(&self) -> bool {
        self.delivered
    }

    pub(crate) fn should_queue(&self) -> bool {
        self.attempted && !self.delivered && self.retryable
    }

    pub(crate) fn error_message(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(crate) fn mark_queued(&mut self, id: String) {
        self.queued_id = Some(id);
    }

    pub(crate) fn mark_queue_error(&mut self, err: String) {
        self.queue_error = Some(err);
    }

    pub(crate) fn audit_outcome(&self) -> String {
        if !self.attempted {
            return "skipped:not_configured".to_string();
        }
        if self.delivered {
            return format!("delivered attempts={}", self.attempts);
        }
        if let Some(queued_id) = &self.queued_id {
            return format!("queued id={queued_id} attempts={}", self.attempts);
        }
        if let Some(queue_error) = &self.queue_error {
            return format!(
                "queue_error attempts={} retryable={} error={}",
                self.attempts,
                self.retryable,
                clip_audit_text(queue_error)
            );
        }
        format!(
            "failed attempts={} retryable={} error={}",
            self.attempts,
            self.retryable,
            clip_audit_text(self.error.as_deref().unwrap_or("callback failed"))
        )
    }
}

pub fn agent_api_egress_descriptor(agent_id: &AgentId) -> AgentApiEgressDescriptor {
    let config_status = agent_api_callback_config_status(agent_id);
    AgentApiEgressDescriptor {
        callback_url_env: agent_api_callback_url_env(agent_id),
        callback_secret_env: agent_api_callback_secret_env(agent_id),
        callback_configured: config_status.ready,
        config_status,
        event_header: "x-captain-event",
        signature_header: "x-captain-signature",
        max_payload_bytes: MAX_AGENT_API_CALLBACK_BODY_SIZE,
        timeout_secs: AGENT_API_CALLBACK_TIMEOUT_SECS,
        max_attempts: AGENT_API_CALLBACK_MAX_ATTEMPTS,
        configure_url: agent_api_egress_configure_url(agent_id),
        test_url: agent_api_egress_test_url(agent_id),
        queue_status_url: agent_api_egress_queue_url(agent_id),
        retry_url_template: agent_api_egress_retry_url_template(agent_id),
    }
}

pub fn agent_api_callback_config_status(agent_id: &AgentId) -> AgentApiCallbackConfigStatus {
    let url_env = agent_api_callback_url_env(agent_id);
    let secret_env = agent_api_callback_secret_env(agent_id);
    let url = std::env::var(&url_env)
        .ok()
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty());
    let callback_url_configured = url.is_some();
    let url_issue = url
        .as_deref()
        .and_then(|url| validate_agent_api_callback_url(url).err());
    let callback_url_valid = callback_url_configured && url_issue.is_none();
    let callback_secret_configured = std::env::var(&secret_env)
        .map(|secret| secret.len() >= MIN_AGENT_API_CALLBACK_SECRET_LEN)
        .unwrap_or(false);

    let (state, issue) = if !callback_url_configured && !callback_secret_configured {
        ("disabled", None)
    } else if callback_url_valid && callback_secret_configured {
        ("ready", None)
    } else if callback_url_configured && !callback_url_valid {
        (
            "misconfigured",
            Some(format!(
                "{url_env}: {}",
                url_issue.unwrap_or_else(|| "invalid callback URL".to_string())
            )),
        )
    } else if callback_url_configured {
        (
            "misconfigured",
            Some(format!("{secret_env} is missing or too short")),
        )
    } else {
        ("misconfigured", Some(format!("{url_env} is missing")))
    };

    AgentApiCallbackConfigStatus {
        callback_url_env: url_env,
        callback_secret_env: secret_env,
        callback_url_configured,
        callback_url_valid,
        callback_secret_configured,
        ready: state == "ready",
        state,
        issue,
    }
}

pub(crate) async fn deliver_agent_api_callback(
    agent_id: &AgentId,
    payload: &serde_json::Value,
) -> AgentApiCallbackDelivery {
    deliver_agent_api_callback_with_local_policy(
        agent_id,
        payload,
        local_agent_api_callbacks_allowed(),
    )
    .await
}

/// Testable core of `deliver_agent_api_callback`, taking the local-callback
/// escape hatch as a parameter instead of reading the global env — so tests
/// don't race each other over process-wide state (mirrors the existing
/// split for `validate_agent_api_callback_url`).
async fn deliver_agent_api_callback_with_local_policy(
    agent_id: &AgentId,
    payload: &serde_json::Value,
    allow_local: bool,
) -> AgentApiCallbackDelivery {
    let url_env = agent_api_callback_url_env(agent_id);
    let url = match std::env::var(&url_env) {
        Ok(url) if !url.trim().is_empty() => url.trim().to_string(),
        _ => return callback_not_attempted(),
    };
    if let Err(err) = validate_agent_api_callback_url_with_local_policy(&url, allow_local) {
        return callback_failed(0, format!("{url_env}: {err}"));
    }

    let secret_env = agent_api_callback_secret_env(agent_id);
    let secret = match std::env::var(&secret_env) {
        Ok(secret) if secret.len() >= MIN_AGENT_API_CALLBACK_SECRET_LEN => secret,
        _ => return callback_failed(0, format!("{secret_env} is missing or too short")),
    };

    let body = match serde_json::to_vec(payload) {
        Ok(body) if body.len() <= MAX_AGENT_API_CALLBACK_BODY_SIZE => body,
        Ok(_) => {
            return callback_failed(
                0,
                format!(
                    "callback payload too large (max {} bytes)",
                    MAX_AGENT_API_CALLBACK_BODY_SIZE
                ),
            )
        }
        Err(err) => return callback_failed(0, format!("callback payload encode failed: {err}")),
    };

    let signature = match callback_signature(&secret, &body) {
        Ok(signature) => signature,
        Err(err) => return callback_failed(0, err),
    };

    // The URL check above inspects the host as given, but never resolves
    // DNS — a callback URL using a domain that simply resolves to an
    // internal address (169.254.169.254, a private IP...) passes it
    // untouched, since nothing in the URL string itself is suspicious.
    // Resolve once here, reject if no candidate address is public, and
    // pin the connection to the validated address via `.resolve()` so a
    // second, different resolution at connect time can't retarget the
    // request either.
    let pinned = match resolve_pinned_socket_addr(&url, allow_local).await {
        Ok(addr) => addr,
        Err(err) => return callback_failed(0, format!("{url_env}: {err}")),
    };

    // redirect::Policy::none() is load-bearing: the checks above validate
    // the callback host, but a 3xx response from an otherwise-valid host
    // can point anywhere — including cloud metadata endpoints or internal
    // services. Following it would silently re-target the signed callback
    // payload past every check above.
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(AGENT_API_CALLBACK_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none())
        .resolve(&pinned.host, pinned.addr)
        .build()
    {
        Ok(client) => client,
        Err(err) => return callback_failed(0, format!("callback client init failed: {err}")),
    };

    let mut attempts = 0;
    let mut last_error = None;
    while attempts < AGENT_API_CALLBACK_MAX_ATTEMPTS {
        attempts += 1;
        let result = client
            .post(&url)
            .header("content-type", "application/json")
            .header("x-captain-agent-id", agent_id.to_string())
            .header(
                "x-captain-event",
                payload
                    .get("event")
                    .and_then(|value| value.as_str())
                    .unwrap_or("agent_api.completed"),
            )
            .header("x-captain-signature", signature.clone())
            .body(body.clone())
            .send()
            .await;

        match result {
            Ok(response) if response.status().is_success() => {
                return AgentApiCallbackDelivery {
                    attempted: true,
                    delivered: true,
                    attempts,
                    retryable: false,
                    queued_id: None,
                    queue_error: None,
                    error: None,
                }
            }
            Ok(response) => {
                let status = response.status();
                last_error = Some(format!("callback returned HTTP {}", status.as_u16()));
                if !status.is_server_error() && status.as_u16() != 429 {
                    return callback_failed_with_retryable(
                        attempts,
                        last_error.unwrap_or_else(|| "callback failed".to_string()),
                        false,
                    );
                }
            }
            Err(err) => last_error = Some(format!("callback request failed: {err}")),
        }
        tokio::time::sleep(Duration::from_millis(200 * attempts as u64)).await;
    }

    callback_failed_with_retryable(
        attempts,
        last_error.unwrap_or_else(|| "callback failed".to_string()),
        true,
    )
}

pub(crate) fn clip_for_callback(text: &str) -> String {
    if text.len() <= MAX_AGENT_API_CALLBACK_TEXT_SIZE {
        return text.to_string();
    }
    let mut boundary = MAX_AGENT_API_CALLBACK_TEXT_SIZE;
    while !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}...", &text[..boundary])
}

pub(crate) fn agent_api_callback_url_env(agent_id: &AgentId) -> String {
    shared_agent_api_callback_url_env(agent_id)
}

pub(crate) fn agent_api_callback_secret_env(agent_id: &AgentId) -> String {
    shared_agent_api_callback_secret_env(agent_id)
}

pub(crate) fn validate_agent_api_callback_url(url: &str) -> Result<(), String> {
    validate_agent_api_callback_url_with_local_policy(url, local_agent_api_callbacks_allowed())
}

/// See `captain_types::ssrf_guard` — the shared SSRF check outbound event
/// webhooks, agent-API egress callbacks, and agent-API provisioning-time
/// validation all delegate to.
fn validate_agent_api_callback_url_with_local_policy(
    url: &str,
    allow_local: bool,
) -> Result<(), String> {
    captain_types::ssrf_guard::validate_outbound_callback_url(url, allow_local)
}

fn local_agent_api_callbacks_allowed() -> bool {
    std::env::var(ALLOW_LOCAL_AGENT_API_CALLBACKS_ENV)
        .map(|value| agent_api_local_callback_flag_enabled(&value))
        .unwrap_or(false)
}

fn agent_api_local_callback_flag_enabled(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn callback_signature(secret: &str, body: &[u8]) -> Result<String, String> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|err| err.to_string())?;
    mac.update(body);
    Ok(format!(
        "sha256={}",
        hex::encode(mac.finalize().into_bytes())
    ))
}

fn callback_not_attempted() -> AgentApiCallbackDelivery {
    AgentApiCallbackDelivery {
        attempted: false,
        delivered: false,
        attempts: 0,
        retryable: false,
        queued_id: None,
        queue_error: None,
        error: None,
    }
}

fn callback_failed(attempts: u8, error: String) -> AgentApiCallbackDelivery {
    callback_failed_with_retryable(attempts, error, attempts > 0)
}

fn callback_failed_with_retryable(
    attempts: u8,
    error: String,
    retryable: bool,
) -> AgentApiCallbackDelivery {
    AgentApiCallbackDelivery {
        attempted: true,
        delivered: false,
        attempts,
        retryable,
        queued_id: None,
        queue_error: None,
        error: Some(error),
    }
}

fn clip_audit_text(text: &str) -> String {
    const MAX: usize = 180;
    if text.len() <= MAX {
        return text.to_string();
    }
    let mut boundary = MAX;
    while !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}...", &text[..boundary])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_agent_id() -> AgentId {
        "01234567-89ab-cdef-0123-456789abcdef".parse().unwrap()
    }

    #[test]
    fn callback_envs_are_deterministic_per_agent() {
        assert_eq!(
            agent_api_callback_url_env(&sample_agent_id()),
            "CAPTAIN_AGENT_API_CALLBACK_URL_01234567_89AB_CDEF_0123_456789ABCDEF"
        );
        assert_eq!(
            agent_api_callback_secret_env(&sample_agent_id()),
            "CAPTAIN_AGENT_API_CALLBACK_SECRET_01234567_89AB_CDEF_0123_456789ABCDEF"
        );
    }

    #[test]
    fn callback_config_requires_valid_url_and_secret() {
        let agent_id = sample_agent_id();
        std::env::remove_var(agent_api_callback_url_env(&agent_id));
        std::env::remove_var(agent_api_callback_secret_env(&agent_id));
        assert!(!agent_api_callback_config_status(&agent_id).ready);

        std::env::set_var(
            agent_api_callback_url_env(&agent_id),
            "https://example.com/hook",
        );
        assert!(!agent_api_callback_config_status(&agent_id).ready);

        std::env::set_var(agent_api_callback_secret_env(&agent_id), "short");
        assert!(!agent_api_callback_config_status(&agent_id).ready);

        std::env::set_var(agent_api_callback_secret_env(&agent_id), "0123456789abcdef");
        assert!(agent_api_callback_config_status(&agent_id).ready);
        std::env::remove_var(agent_api_callback_url_env(&agent_id));
        std::env::remove_var(agent_api_callback_secret_env(&agent_id));
    }

    #[test]
    fn local_callback_urls_require_explicit_smoke_escape_hatch() {
        assert!(validate_agent_api_callback_url_with_local_policy(
            "http://127.0.0.1:48888/hook",
            false
        )
        .is_err());
        assert!(validate_agent_api_callback_url_with_local_policy(
            "http://localhost:48888/hook",
            false
        )
        .is_err());

        assert!(validate_agent_api_callback_url_with_local_policy(
            "http://127.0.0.1:48888/hook",
            true
        )
        .is_ok());
        assert!(validate_agent_api_callback_url_with_local_policy(
            "http://localhost:48888/hook",
            true
        )
        .is_ok());
        assert!(
            validate_agent_api_callback_url_with_local_policy("http://192.168.1.5/hook", true)
                .is_err()
        );
    }

    #[test]
    fn local_callback_escape_hatch_accepts_only_explicit_truthy_values() {
        for value in ["1", "true", "TRUE", " yes ", "on"] {
            assert!(agent_api_local_callback_flag_enabled(value));
        }
        for value in ["", "0", "false", "off", "maybe", "localhost"] {
            assert!(!agent_api_local_callback_flag_enabled(value));
        }
    }

    #[test]
    fn callback_signature_uses_hmac_sha256_prefix() {
        let signature = callback_signature("0123456789abcdef", br#"{"ok":true}"#).unwrap();
        assert!(signature.starts_with("sha256="));
        assert_eq!(signature.len(), "sha256=".len() + 64);
    }

    #[test]
    fn callback_clip_preserves_utf8_boundary() {
        let clipped = clip_for_callback(&"é".repeat(MAX_AGENT_API_CALLBACK_TEXT_SIZE));
        assert!(clipped.ends_with("..."));
        assert!(clipped.is_char_boundary(clipped.len()));
    }

    #[test]
    fn callback_audit_outcome_reflects_queue_status() {
        let mut delivery = callback_failed_with_retryable(2, "HTTP 503".to_string(), true);
        delivery.mark_queued("queue-1".to_string());
        assert_eq!(delivery.audit_outcome(), "queued id=queue-1 attempts=2");
    }

    /// Regression for the redirect-based SSRF bypass: a callback URL that
    /// passes `validate_agent_api_callback_url` can still respond with a
    /// 3xx pointing anywhere (cloud metadata, an internal service...). If
    /// the client followed it, the signed payload would be forwarded past
    /// the check entirely. `Policy::none()` must make delivery fail clean
    /// instead — and, decisively, the redirect target must never see a
    /// request.
    #[tokio::test]
    async fn callback_delivery_does_not_follow_redirects() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let redirect_target = MockServer::start().await;

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/internal", redirect_target.uri())),
            )
            .mount(&server)
            .await;

        // Each test uses a fresh AgentId, so the per-agent url/secret envs
        // below don't collide with other tests running in parallel — but
        // the local-callback escape hatch is process-global, so it's
        // passed as a parameter instead of touching that env var (see
        // deliver_agent_api_callback_with_local_policy).
        let agent_id = AgentId::new();
        std::env::set_var(
            agent_api_callback_url_env(&agent_id),
            format!("{}/hook", server.uri()),
        );
        std::env::set_var(agent_api_callback_secret_env(&agent_id), "0123456789abcdef");

        let payload = serde_json::json!({ "event": "agent_api.completed" });
        let delivery =
            deliver_agent_api_callback_with_local_policy(&agent_id, &payload, true).await;

        std::env::remove_var(agent_api_callback_url_env(&agent_id));
        std::env::remove_var(agent_api_callback_secret_env(&agent_id));

        assert!(
            !delivery.delivered,
            "a 3xx response must not count as delivered"
        );
        assert!(
            redirect_target
                .received_requests()
                .await
                .unwrap()
                .is_empty(),
            "the redirect target must never receive a forwarded request"
        );
    }
}
