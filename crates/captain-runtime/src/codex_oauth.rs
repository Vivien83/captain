//! OpenAI Codex OAuth — device code flow against `auth.openai.com`.
//!
//! Mirrors the flow used by the OpenAI Codex CLI so users on
//! a ChatGPT Plus/Pro subscription can drive Codex without an API key:
//!
//! 1. POST `/api/accounts/deviceauth/usercode` → `{user_code, device_auth_id, interval}`
//! 2. Show the user `https://auth.openai.com/codex/device` + the user_code.
//! 3. Poll `/api/accounts/deviceauth/token` until 200 with `{authorization_code, code_verifier}`.
//! 4. POST `/oauth/token` (form-urlencoded, `grant_type=authorization_code`) → `{access_token, refresh_token}`.
//! 5. Refresh later via `grant_type=refresh_token`.
//!
//! All HTTP timeouts and errors are surfaced explicitly — no silent failure.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use zeroize::Zeroizing;

/// Public OAuth client_id used by the official Codex CLI.
pub const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// OpenAI auth issuer.
pub const CODEX_ISSUER: &str = "https://auth.openai.com";

/// Device code initiation endpoint (returns user_code + device_auth_id).
pub const CODEX_DEVICE_USERCODE_URL: &str =
    "https://auth.openai.com/api/accounts/deviceauth/usercode";

/// Device code polling endpoint (returns authorization_code once user signed in).
pub const CODEX_DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";

/// OAuth token exchange / refresh endpoint.
pub const CODEX_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

/// User-facing verification URL (where they paste the code).
pub const CODEX_DEVICE_VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";

/// Redirect URI baked into the device flow (must match exactly on token exchange).
pub const CODEX_DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";

#[derive(Debug, Deserialize)]
pub struct DeviceCodeResponse {
    pub user_code: String,
    pub device_auth_id: String,
    /// Suggested polling interval in seconds. Defaults to 5 if missing.
    /// OpenAI's auth endpoint returns this field as a JSON string ("5")
    /// — `interval_from_str_or_int` accepts both string and number to
    /// stay forward-compatible.
    #[serde(
        default = "default_interval",
        deserialize_with = "interval_from_str_or_int"
    )]
    pub interval: u64,
}

fn default_interval() -> u64 {
    5
}

fn interval_from_str_or_int<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let v = serde_json::Value::deserialize(deserializer)?;
    match v {
        serde_json::Value::Number(n) => n
            .as_u64()
            .ok_or_else(|| Error::custom("interval not a positive integer")),
        serde_json::Value::String(s) => s
            .trim()
            .parse::<u64>()
            .map_err(|e| Error::custom(format!("interval string parse: {e}"))),
        other => Err(Error::custom(format!(
            "interval must be number or string, got {other:?}"
        ))),
    }
}

#[derive(Debug, Deserialize)]
struct DeviceTokenResponse {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

/// Result of a successful login or refresh.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexCredentials {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub last_refresh: String,
    pub auth_mode: String,
    pub source: String,
}

/// Outcome of a single polling attempt.
pub enum PollOutcome {
    /// User hasn't completed the flow yet — caller should sleep `interval` and retry.
    Pending,
    /// User completed; ready to exchange the authorization_code for tokens.
    Authorized {
        authorization_code: String,
        code_verifier: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum CodexOAuthError {
    #[error("device code request failed: {0}")]
    DeviceCodeRequest(String),
    #[error("device code response missing required fields")]
    DeviceCodeIncomplete,
    #[error("polling endpoint returned status {0}")]
    PollError(u16),
    #[error("polling timed out after {0:?}")]
    PollTimeout(Duration),
    #[error("token exchange failed: {0}")]
    TokenExchange(String),
    #[error("refresh failed: {0}")]
    RefreshFailed(String),
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("server returned no access_token")]
    NoAccessToken,
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Step 1 — request a device code. The returned `user_code` is what the user
/// types on `CODEX_DEVICE_VERIFICATION_URL`.
pub async fn request_device_code() -> Result<DeviceCodeResponse, CodexOAuthError> {
    let body = serde_json::json!({ "client_id": CODEX_CLIENT_ID });
    let resp = http_client()
        .post(CODEX_DEVICE_USERCODE_URL)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| CodexOAuthError::DeviceCodeRequest(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(CodexOAuthError::DeviceCodeRequest(format!(
            "HTTP {}",
            resp.status()
        )));
    }
    let parsed: DeviceCodeResponse = resp
        .json()
        .await
        .map_err(|e| CodexOAuthError::DeviceCodeRequest(e.to_string()))?;
    if parsed.user_code.is_empty() || parsed.device_auth_id.is_empty() {
        return Err(CodexOAuthError::DeviceCodeIncomplete);
    }
    Ok(parsed)
}

/// Step 3 — poll once. Returns `Pending` (user not done yet) or `Authorized`.
pub async fn poll_authorization(
    device_auth_id: &str,
    user_code: &str,
) -> Result<PollOutcome, CodexOAuthError> {
    let body = serde_json::json!({
        "device_auth_id": device_auth_id,
        "user_code": user_code,
    });
    let resp = http_client()
        .post(CODEX_DEVICE_TOKEN_URL)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| CodexOAuthError::Http(e.to_string()))?;
    let status = resp.status().as_u16();
    match status {
        200 => {
            let parsed: DeviceTokenResponse = resp
                .json()
                .await
                .map_err(|e| CodexOAuthError::Http(e.to_string()))?;
            if parsed.authorization_code.is_empty() || parsed.code_verifier.is_empty() {
                return Err(CodexOAuthError::TokenExchange(
                    "missing authorization_code or code_verifier".into(),
                ));
            }
            Ok(PollOutcome::Authorized {
                authorization_code: parsed.authorization_code,
                code_verifier: parsed.code_verifier,
            })
        }
        403 | 404 => Ok(PollOutcome::Pending),
        other => Err(CodexOAuthError::PollError(other)),
    }
}

/// Step 4 — exchange the authorization_code for an access_token + refresh_token.
pub async fn exchange_code(
    authorization_code: &str,
    code_verifier: &str,
) -> Result<CodexCredentials, CodexOAuthError> {
    let resp = http_client()
        .post(CODEX_OAUTH_TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", authorization_code),
            ("redirect_uri", CODEX_DEVICE_REDIRECT_URI),
            ("client_id", CODEX_CLIENT_ID),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await
        .map_err(|e| CodexOAuthError::TokenExchange(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(CodexOAuthError::TokenExchange(format!(
            "HTTP {}",
            resp.status()
        )));
    }
    let parsed: OAuthTokenResponse = resp
        .json()
        .await
        .map_err(|e| CodexOAuthError::TokenExchange(e.to_string()))?;
    let access = Zeroizing::new(parsed.access_token);
    if access.is_empty() {
        return Err(CodexOAuthError::NoAccessToken);
    }
    Ok(CodexCredentials {
        access_token: (*access).clone(),
        refresh_token: parsed.refresh_token.unwrap_or_default(),
        expires_at: now_secs().saturating_add(parsed.expires_in.unwrap_or(3600) as i64),
        last_refresh: rfc3339_now(),
        auth_mode: "chatgpt".into(),
        source: "device-code".into(),
    })
}

/// Step 5 — refresh an expired access_token using a stored refresh_token.
pub async fn refresh_tokens(refresh_token: &str) -> Result<CodexCredentials, CodexOAuthError> {
    if refresh_token.is_empty() {
        return Err(CodexOAuthError::RefreshFailed(
            "missing refresh_token — re-run `captain login codex`".into(),
        ));
    }
    let resp = http_client()
        .post(CODEX_OAUTH_TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CODEX_CLIENT_ID),
        ])
        .send()
        .await
        .map_err(|e| CodexOAuthError::RefreshFailed(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(CodexOAuthError::RefreshFailed(format!(
            "HTTP {}",
            resp.status()
        )));
    }
    let parsed: OAuthTokenResponse = resp
        .json()
        .await
        .map_err(|e| CodexOAuthError::RefreshFailed(e.to_string()))?;
    let access = Zeroizing::new(parsed.access_token);
    if access.is_empty() {
        return Err(CodexOAuthError::NoAccessToken);
    }
    Ok(CodexCredentials {
        access_token: (*access).clone(),
        // OpenAI may rotate refresh tokens; fall back to the original one when
        // the response omits it (some flows only refresh the access token).
        refresh_token: parsed
            .refresh_token
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| refresh_token.to_string()),
        expires_at: now_secs().saturating_add(parsed.expires_in.unwrap_or(3600) as i64),
        last_refresh: rfc3339_now(),
        auth_mode: "chatgpt".into(),
        source: "device-code".into(),
    })
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn rfc3339_now() -> String {
    // Avoid pulling chrono into this crate's hot path — produce an RFC3339-ish
    // UTC string with std time. Refresh logic only checks `expires_at` (i64);
    // the human-readable string is for the persisted JSON file.
    let secs = now_secs();
    format!("{secs}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_codex_cli() {
        assert_eq!(CODEX_CLIENT_ID, "app_EMoamEEZ73f0CkXaXp7hrann");
        assert!(CODEX_OAUTH_TOKEN_URL.starts_with("https://auth.openai.com"));
        assert!(CODEX_DEVICE_VERIFICATION_URL.ends_with("/codex/device"));
        assert!(CODEX_DEVICE_REDIRECT_URI.ends_with("/deviceauth/callback"));
    }

    #[tokio::test]
    async fn refresh_with_empty_token_errors() {
        let err = refresh_tokens("").await.unwrap_err();
        assert!(matches!(err, CodexOAuthError::RefreshFailed(_)));
    }

    #[test]
    fn device_code_response_decodes_interval_as_number() {
        let body = r#"{"user_code":"ABC","device_auth_id":"id1","interval":5}"#;
        let parsed: DeviceCodeResponse = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.user_code, "ABC");
        assert_eq!(parsed.interval, 5);
    }

    #[test]
    fn device_code_response_decodes_interval_as_string() {
        // Real OpenAI auth.openai.com returns interval as a JSON string
        // — we must accept both shapes.
        let body = r#"{"user_code":"XYZ","device_auth_id":"id2","interval":"5"}"#;
        let parsed: DeviceCodeResponse = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.user_code, "XYZ");
        assert_eq!(parsed.interval, 5);
    }

    #[test]
    fn device_code_response_uses_default_interval_when_missing() {
        let body = r#"{"user_code":"DEF","device_auth_id":"id3"}"#;
        let parsed: DeviceCodeResponse = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.interval, 5);
    }
}
