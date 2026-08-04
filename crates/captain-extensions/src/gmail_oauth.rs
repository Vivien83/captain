//! Native Google OAuth and token lifecycle for Gmail accounts.

mod callback;

use crate::{ExtensionError, ExtensionResult};
use callback::start_callback_server;
use captain_types::email::GmailAccessProfile;
use oauth2::basic::BasicClient;
use oauth2::{
    AuthType, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, RefreshToken, Scope, TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
const GOOGLE_USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";
const GMAIL_PROFILE_URL: &str = "https://gmail.googleapis.com/gmail/v1/users/me/profile";
const TOKEN_EXPIRY_SKEW_SECONDS: i64 = 60;
const SECRET_SCHEMA_VERSION: u16 = 1;
const BUNDLED_GOOGLE_CLIENT_ID: Option<&str> = option_env!("CAPTAIN_GOOGLE_OAUTH_CLIENT_ID");

/// Imported Google Desktop OAuth client. It is serialized only into the
/// encrypted Captain vault and never included in public account projections.
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct GoogleDesktopClient {
    schema_version: u16,
    client_id: String,
    client_secret: Option<String>,
}

impl GoogleDesktopClient {
    /// Build an installed-app client from public release metadata.
    ///
    /// Google treats desktop applications as public clients: a client secret
    /// embedded in a distributed binary cannot be confidential. Captain still
    /// accepts the optional field because Google includes it in some Desktop
    /// app downloads, but authorization is protected by PKCE and state.
    pub fn from_public_client(
        client_id: &str,
        client_secret: Option<&str>,
    ) -> ExtensionResult<Self> {
        let client = Self {
            schema_version: SECRET_SCHEMA_VERSION,
            client_id: client_id.trim().to_string(),
            client_secret: client_secret
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
        };
        client.validate()?;
        Ok(client)
    }

    /// Import the JSON downloaded for a Google OAuth "Desktop app" client.
    pub fn from_google_client_json(json: &str) -> ExtensionResult<Self> {
        let document: GoogleClientDocument = serde_json::from_str(json).map_err(|error| {
            ExtensionError::OAuth(format!("Invalid Google OAuth client JSON: {error}"))
        })?;
        let installed = document.installed.ok_or_else(|| {
            ExtensionError::OAuth(
                "Google OAuth credentials must use the Desktop app client type".to_string(),
            )
        })?;
        validate_google_client_document(&installed)?;
        let client = Self {
            schema_version: SECRET_SCHEMA_VERSION,
            client_id: installed.client_id,
            client_secret: installed.client_secret.filter(|value| !value.is_empty()),
        };
        client.validate()?;
        Ok(client)
    }

    pub fn from_secret_json(json: &str) -> ExtensionResult<Self> {
        let client: Self = serde_json::from_str(json).map_err(|error| {
            ExtensionError::OAuth(format!("Invalid stored Gmail OAuth client: {error}"))
        })?;
        client.validate()?;
        Ok(client)
    }

    pub fn to_secret_json(&self) -> ExtensionResult<Zeroizing<String>> {
        serde_json::to_string(self)
            .map(Zeroizing::new)
            .map_err(|error| ExtensionError::OAuth(format!("OAuth client encode failed: {error}")))
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    fn validate(&self) -> ExtensionResult<()> {
        if self.schema_version != SECRET_SCHEMA_VERSION {
            return Err(ExtensionError::OAuth(format!(
                "Unsupported stored Gmail OAuth client version {}",
                self.schema_version
            )));
        }
        let valid_id = self.client_id.len() <= 256
            && self.client_id.ends_with(".apps.googleusercontent.com")
            && !self.client_id.contains(char::is_whitespace);
        if !valid_id {
            return Err(ExtensionError::OAuth(
                "Google OAuth client_id is not a valid Desktop app identifier".to_string(),
            ));
        }
        if self.client_secret.as_ref().is_some_and(|secret| {
            secret.len() > 512 || secret.contains('\n') || secret.contains('\r')
        }) {
            return Err(ExtensionError::OAuth(
                "Google OAuth client_secret is malformed".to_string(),
            ));
        }
        Ok(())
    }
}

/// Return the official Google Desktop client compiled into this Captain
/// binary. Community/development builds may intentionally omit it; callers
/// must then require an operator-supplied Desktop client instead of silently
/// borrowing another application's OAuth identity.
pub fn bundled_google_desktop_client() -> ExtensionResult<Option<GoogleDesktopClient>> {
    match BUNDLED_GOOGLE_CLIENT_ID.map(str::trim) {
        Some("") | None => Ok(None),
        Some(client_id) => GoogleDesktopClient::from_public_client(client_id, None).map(Some),
    }
}

/// Encrypted-vault payload for one Gmail OAuth grant.
#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct GmailTokenSet {
    schema_version: u16,
    access_token: String,
    refresh_token: String,
    expires_at: i64,
    granted_scopes: Vec<String>,
}

impl GmailTokenSet {
    pub fn from_secret_json(json: &str) -> ExtensionResult<Self> {
        let tokens: Self = serde_json::from_str(json).map_err(|error| {
            ExtensionError::OAuth(format!("Invalid stored Gmail token set: {error}"))
        })?;
        tokens.validate()?;
        Ok(tokens)
    }

    pub fn to_secret_json(&self) -> ExtensionResult<Zeroizing<String>> {
        serde_json::to_string(self)
            .map(Zeroizing::new)
            .map_err(|error| ExtensionError::OAuth(format!("Gmail token encode failed: {error}")))
    }

    pub fn access_token(&self) -> &str {
        &self.access_token
    }

    pub fn refresh_token(&self) -> &str {
        &self.refresh_token
    }

    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }

    pub fn granted_scopes(&self) -> &[String] {
        &self.granted_scopes
    }

    pub fn needs_refresh(&self, now_unix_seconds: i64) -> bool {
        self.expires_at <= now_unix_seconds.saturating_add(TOKEN_EXPIRY_SKEW_SECONDS)
    }

    fn validate(&self) -> ExtensionResult<()> {
        if self.schema_version != SECRET_SCHEMA_VERSION
            || self.access_token.is_empty()
            || self.refresh_token.is_empty()
            || self.expires_at <= 0
            || self.granted_scopes.is_empty()
        {
            return Err(ExtensionError::OAuth(
                "Stored Gmail token set is incomplete or unsupported".to_string(),
            ));
        }
        Ok(())
    }
}

/// Successful authorization result before vault and SQLite persistence.
pub struct GmailAuthorization {
    pub email_address: String,
    pub history_id: Option<String>,
    pub tokens: GmailTokenSet,
}

/// Public-safe identity returned by a live Google/Gmail grant verification.
pub struct GmailIdentitySnapshot {
    pub email_address: String,
    pub history_id: Option<String>,
}

/// Run the Google Desktop OAuth flow with a one-time loopback callback.
pub async fn authorize_google_desktop<F>(
    client: &GoogleDesktopClient,
    profile: GmailAccessProfile,
    login_hint: Option<&str>,
    callback_port: Option<u16>,
    announce_authorization_url: F,
) -> ExtensionResult<GmailAuthorization>
where
    F: FnOnce(&str) -> Result<(), String>,
{
    client.validate()?;
    if callback_port == Some(0) {
        return Err(ExtensionError::OAuth(
            "Gmail callback port must be between 1 and 65535".to_string(),
        ));
    }
    let listener =
        tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, callback_port.unwrap_or(0)))
            .await
            .map_err(|error| {
                ExtensionError::OAuth(format!("Could not bind Gmail OAuth loopback: {error}"))
            })?;
    let port = listener
        .local_addr()
        .map_err(|error| ExtensionError::OAuth(format!("Loopback address failed: {error}")))?
        .port();
    let expected_host = format!("127.0.0.1:{port}");
    let redirect_uri = format!("http://{expected_host}/callback");
    let prepared = prepare_authorization(client, profile, login_hint, &redirect_uri)?;
    let callback = start_callback_server(listener, expected_host, prepared.expected_state.clone());

    if let Err(error) = announce_authorization_url(prepared.authorization_url.as_str()) {
        callback.shutdown().await;
        return Err(ExtensionError::OAuth(format!(
            "Could not open Gmail authorization URL: {error}"
        )));
    }

    let code = callback.wait_for_code().await?;
    let tokens =
        exchange_authorization_code(client, profile, &redirect_uri, code, prepared.pkce_verifier)
            .await?;
    let identity = verify_google_tokens(&tokens, profile).await?;

    Ok(GmailAuthorization {
        email_address: identity.email_address,
        history_id: identity.history_id,
        tokens,
    })
}

/// Verify that a stored token still maps to the expected Gmail capability.
pub async fn verify_google_tokens(
    tokens: &GmailTokenSet,
    profile: GmailAccessProfile,
) -> ExtensionResult<GmailIdentitySnapshot> {
    tokens.validate()?;
    let identity = fetch_google_identity(tokens.access_token()).await?;
    let history_id = if profile.can_read() {
        let gmail_profile = fetch_gmail_profile(tokens.access_token()).await?;
        if !gmail_profile
            .email_address
            .eq_ignore_ascii_case(&identity.email)
        {
            return Err(ExtensionError::OAuth(
                "Google identity and Gmail profile do not match".to_string(),
            ));
        }
        Some(gmail_profile.history_id)
    } else {
        None
    };
    Ok(GmailIdentitySnapshot {
        email_address: identity.email,
        history_id,
    })
}

/// Refresh an expired access token while preserving the durable refresh token.
pub async fn refresh_google_tokens(
    client: &GoogleDesktopClient,
    current: &GmailTokenSet,
    profile: GmailAccessProfile,
) -> ExtensionResult<GmailTokenSet> {
    client.validate()?;
    current.validate()?;
    let mut oauth_client = BasicClient::new(ClientId::new(client.client_id.clone()))
        .set_auth_uri(parse_auth_url()?)
        .set_token_uri(parse_token_url()?)
        .set_auth_type(AuthType::RequestBody);
    if let Some(secret) = &client.client_secret {
        oauth_client = oauth_client.set_client_secret(ClientSecret::new(secret.clone()));
    }
    let http = secure_http_client()?;
    let response = oauth_client
        .exchange_refresh_token(&RefreshToken::new(current.refresh_token.clone()))
        .request_async(&http)
        .await
        .map_err(|_| {
            ExtensionError::OAuth(
                "Gmail token refresh failed; reconnect the account if consent was revoked"
                    .to_string(),
            )
        })?;
    token_set_from_response(
        &response,
        Some(current.refresh_token.as_str()),
        current.granted_scopes.as_slice(),
        profile,
    )
}

/// Revoke the Google grant. Google revocation affects every scope granted to
/// the same project, so callers must confirm this account-level consequence.
pub async fn revoke_google_tokens(tokens: &GmailTokenSet) -> ExtensionResult<()> {
    tokens.validate()?;
    let http = secure_http_client()?;
    let response = http
        .post(GOOGLE_REVOKE_URL)
        .form(&[("token", tokens.refresh_token())])
        .send()
        .await
        .map_err(|error| ExtensionError::OAuth(format!("Google revoke request failed: {error}")))?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(ExtensionError::OAuth(format!(
            "Google revoke failed with HTTP {}",
            response.status().as_u16()
        )))
    }
}

struct PreparedAuthorization {
    authorization_url: url::Url,
    expected_state: String,
    pkce_verifier: PkceCodeVerifier,
}

fn prepare_authorization(
    client: &GoogleDesktopClient,
    profile: GmailAccessProfile,
    login_hint: Option<&str>,
    redirect_uri: &str,
) -> ExtensionResult<PreparedAuthorization> {
    let mut oauth_client = BasicClient::new(ClientId::new(client.client_id.clone()))
        .set_auth_uri(parse_auth_url()?)
        .set_token_uri(parse_token_url()?)
        .set_redirect_uri(RedirectUrl::new(redirect_uri.to_string()).map_err(|error| {
            ExtensionError::OAuth(format!("Invalid Gmail OAuth redirect URI: {error}"))
        })?)
        .set_auth_type(AuthType::RequestBody);
    if let Some(secret) = &client.client_secret {
        oauth_client = oauth_client.set_client_secret(ClientSecret::new(secret.clone()));
    }
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let mut request = oauth_client
        .authorize_url(CsrfToken::new_random)
        .set_pkce_challenge(challenge)
        .add_extra_param("access_type", "offline")
        .add_extra_param("prompt", "consent")
        .add_extra_param("include_granted_scopes", "false");
    for scope in profile.required_scopes() {
        request = request.add_scope(Scope::new((*scope).to_string()));
    }
    if let Some(login_hint) = login_hint.filter(|value| !value.trim().is_empty()) {
        request = request.add_extra_param("login_hint", login_hint.trim().to_string());
    }
    let (authorization_url, state) = request.url();
    Ok(PreparedAuthorization {
        authorization_url,
        expected_state: state.secret().to_string(),
        pkce_verifier: verifier,
    })
}

async fn exchange_authorization_code(
    client: &GoogleDesktopClient,
    profile: GmailAccessProfile,
    redirect_uri: &str,
    code: Zeroizing<String>,
    verifier: PkceCodeVerifier,
) -> ExtensionResult<GmailTokenSet> {
    let mut oauth_client = BasicClient::new(ClientId::new(client.client_id.clone()))
        .set_auth_uri(parse_auth_url()?)
        .set_token_uri(parse_token_url()?)
        .set_redirect_uri(RedirectUrl::new(redirect_uri.to_string()).map_err(|error| {
            ExtensionError::OAuth(format!("Invalid Gmail OAuth redirect URI: {error}"))
        })?)
        .set_auth_type(AuthType::RequestBody);
    if let Some(secret) = &client.client_secret {
        oauth_client = oauth_client.set_client_secret(ClientSecret::new(secret.clone()));
    }
    let http = secure_http_client()?;
    let response = oauth_client
        .exchange_code(AuthorizationCode::new(code.to_string()))
        .set_pkce_verifier(verifier)
        .request_async(&http)
        .await
        .map_err(|_| {
            ExtensionError::OAuth(
                "Gmail authorization-code exchange failed; retry the connection flow".to_string(),
            )
        })?;
    token_set_from_response(&response, None, &[], profile)
}

fn token_set_from_response<TT>(
    response: &oauth2::StandardTokenResponse<oauth2::EmptyExtraTokenFields, TT>,
    existing_refresh_token: Option<&str>,
    existing_scopes: &[String],
    profile: GmailAccessProfile,
) -> ExtensionResult<GmailTokenSet>
where
    TT: oauth2::TokenType,
{
    let refresh_token = response
        .refresh_token()
        .map(|token| token.secret().to_string())
        .or_else(|| existing_refresh_token.map(ToString::to_string))
        .ok_or_else(|| {
            ExtensionError::OAuth(
                "Google did not issue a refresh token; revoke prior consent and reconnect"
                    .to_string(),
            )
        })?;
    let mut scopes = response
        .scopes()
        .map(|scopes| {
            scopes
                .iter()
                .map(|scope| scope.as_str().to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| existing_scopes.to_vec());
    scopes.sort();
    scopes.dedup();
    if !scopes
        .iter()
        .any(|scope| scope == profile.required_gmail_scope())
    {
        return Err(ExtensionError::OAuth(format!(
            "Google grant is missing required scope {}",
            profile.required_gmail_scope()
        )));
    }
    let expires_in = response.expires_in().ok_or_else(|| {
        ExtensionError::OAuth("Google token response omitted expires_in".to_string())
    })?;
    let expires_at = now_unix_seconds()?.saturating_add(
        i64::try_from(expires_in.as_secs())
            .map_err(|_| ExtensionError::OAuth("Google token lifetime is invalid".to_string()))?,
    );
    let tokens = GmailTokenSet {
        schema_version: SECRET_SCHEMA_VERSION,
        access_token: response.access_token().secret().to_string(),
        refresh_token,
        expires_at,
        granted_scopes: scopes,
    };
    tokens.validate()?;
    Ok(tokens)
}

#[derive(Deserialize)]
struct GoogleClientDocument {
    installed: Option<GoogleInstalledClientDocument>,
}

#[derive(Deserialize)]
struct GoogleInstalledClientDocument {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
    auth_uri: String,
    token_uri: String,
    #[serde(default)]
    redirect_uris: Vec<String>,
}

fn validate_google_client_document(client: &GoogleInstalledClientDocument) -> ExtensionResult<()> {
    let auth_url_allowed = matches!(
        client.auth_uri.as_str(),
        "https://accounts.google.com/o/oauth2/auth" | GOOGLE_AUTH_URL
    );
    if !auth_url_allowed || client.token_uri != GOOGLE_TOKEN_URL {
        return Err(ExtensionError::OAuth(
            "Google OAuth client file contains non-Google authorization endpoints".to_string(),
        ));
    }
    let has_loopback = client.redirect_uris.iter().any(|uri| {
        uri == "http://localhost"
            || uri.starts_with("http://localhost:")
            || uri == "http://127.0.0.1"
            || uri.starts_with("http://127.0.0.1:")
            || uri == "http://[::1]"
            || uri.starts_with("http://[::1]:")
    });
    if !has_loopback {
        return Err(ExtensionError::OAuth(
            "Google OAuth client must be a Desktop app with a loopback redirect".to_string(),
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
struct GoogleUserInfo {
    email: String,
    #[serde(default)]
    email_verified: bool,
}

async fn fetch_google_identity(access_token: &str) -> ExtensionResult<GoogleUserInfo> {
    let http = secure_http_client()?;
    let response = http
        .get(GOOGLE_USERINFO_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| {
            ExtensionError::OAuth(format!("Google identity request failed: {error}"))
        })?;
    if !response.status().is_success() {
        return Err(ExtensionError::OAuth(format!(
            "Google identity request failed with HTTP {}",
            response.status().as_u16()
        )));
    }
    let identity: GoogleUserInfo = response.json().await.map_err(|error| {
        ExtensionError::OAuth(format!("Google identity response was invalid: {error}"))
    })?;
    if identity.email.trim().is_empty() || !identity.email_verified {
        return Err(ExtensionError::OAuth(
            "Google did not return a verified account email".to_string(),
        ));
    }
    Ok(identity)
}

#[derive(Deserialize)]
struct GmailProfileResponse {
    #[serde(rename = "emailAddress")]
    email_address: String,
    #[serde(rename = "historyId")]
    history_id: String,
}

async fn fetch_gmail_profile(access_token: &str) -> ExtensionResult<GmailProfileResponse> {
    let http = secure_http_client()?;
    let response = http
        .get(GMAIL_PROFILE_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| ExtensionError::OAuth(format!("Gmail profile request failed: {error}")))?;
    if !response.status().is_success() {
        return Err(ExtensionError::OAuth(format!(
            "Gmail profile request failed with HTTP {}",
            response.status().as_u16()
        )));
    }
    let profile: GmailProfileResponse = response.json().await.map_err(|error| {
        ExtensionError::OAuth(format!("Gmail profile response was invalid: {error}"))
    })?;
    if profile.email_address.is_empty() || profile.history_id.is_empty() {
        return Err(ExtensionError::OAuth(
            "Gmail profile response omitted identity or history cursor".to_string(),
        ));
    }
    Ok(profile)
}

fn secure_http_client() -> ExtensionResult<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("captain/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| ExtensionError::OAuth(format!("OAuth HTTP client failed: {error}")))
}

fn parse_auth_url() -> ExtensionResult<AuthUrl> {
    AuthUrl::new(GOOGLE_AUTH_URL.to_string())
        .map_err(|error| ExtensionError::OAuth(format!("Google auth URL invalid: {error}")))
}

fn parse_token_url() -> ExtensionResult<TokenUrl> {
    TokenUrl::new(GOOGLE_TOKEN_URL.to_string())
        .map_err(|error| ExtensionError::OAuth(format!("Google token URL invalid: {error}")))
}

fn now_unix_seconds() -> ExtensionResult<i64> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| ExtensionError::OAuth(format!("System clock invalid: {error}")))?
        .as_secs();
    i64::try_from(seconds)
        .map_err(|_| ExtensionError::OAuth("System clock exceeds supported range".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desktop_client_json() -> String {
        serde_json::json!({
            "installed": {
                "client_id": "123456789.apps.googleusercontent.com",
                "project_id": "captain-test",
                "auth_uri": "https://accounts.google.com/o/oauth2/auth",
                "token_uri": GOOGLE_TOKEN_URL,
                "client_secret": "desktop-secret",
                "redirect_uris": ["http://localhost"]
            }
        })
        .to_string()
    }

    #[test]
    fn desktop_client_import_is_strict_and_vault_roundtrips() {
        let client = GoogleDesktopClient::from_google_client_json(&desktop_client_json()).unwrap();
        assert_eq!(client.client_id(), "123456789.apps.googleusercontent.com");
        let secret = client.to_secret_json().unwrap();
        let restored = GoogleDesktopClient::from_secret_json(&secret).unwrap();
        assert_eq!(restored.client_id(), client.client_id());

        let web = desktop_client_json().replace("installed", "web");
        assert!(GoogleDesktopClient::from_google_client_json(&web).is_err());
        let foreign = desktop_client_json().replace(
            "https://oauth2.googleapis.com/token",
            "https://example.com/token",
        );
        assert!(GoogleDesktopClient::from_google_client_json(&foreign).is_err());
    }

    #[test]
    fn public_desktop_client_validates_without_treating_secret_as_confidential() {
        let client = GoogleDesktopClient::from_public_client(
            "captain.apps.googleusercontent.com",
            Some(" public-desktop-secret "),
        )
        .unwrap();
        let stored = client.to_secret_json().unwrap();
        let restored = GoogleDesktopClient::from_secret_json(&stored).unwrap();

        assert_eq!(restored.client_id(), "captain.apps.googleusercontent.com");
        assert!(GoogleDesktopClient::from_public_client("not-google", None).is_err());
        assert!(GoogleDesktopClient::from_public_client(
            "captain.apps.googleusercontent.com",
            Some("bad\nsecret")
        )
        .is_err());
    }

    #[test]
    fn bundled_desktop_client_is_absent_or_strictly_valid() {
        if let Some(client) = bundled_google_desktop_client().unwrap() {
            assert!(client.client_id().ends_with(".apps.googleusercontent.com"));
        }
    }

    #[test]
    fn authorization_url_uses_pkce_offline_consent_and_exact_scope() {
        let client = GoogleDesktopClient::from_google_client_json(&desktop_client_json()).unwrap();
        let prepared = prepare_authorization(
            &client,
            GmailAccessProfile::Assistant,
            Some("person@gmail.com"),
            "http://127.0.0.1:49152/callback",
        )
        .unwrap();
        let query: std::collections::HashMap<_, _> = prepared
            .authorization_url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        assert_eq!(
            query.get("access_type").map(String::as_str),
            Some("offline")
        );
        assert_eq!(query.get("prompt").map(String::as_str), Some("consent"));
        assert_eq!(
            query.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert!(query.get("scope").is_some_and(
            |scope| scope.contains(GmailAccessProfile::Assistant.required_gmail_scope())
        ));
        assert_eq!(
            query.get("redirect_uri").map(String::as_str),
            Some("http://127.0.0.1:49152/callback")
        );
    }

    #[test]
    fn token_payload_validates_expiry_and_never_needs_plaintext_storage() {
        let tokens = GmailTokenSet {
            schema_version: SECRET_SCHEMA_VERSION,
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: 1_000,
            granted_scopes: vec![GmailAccessProfile::Assistant
                .required_gmail_scope()
                .to_string()],
        };
        assert!(!tokens.needs_refresh(900));
        assert!(tokens.needs_refresh(940));
        let json = tokens.to_secret_json().unwrap();
        let restored = GmailTokenSet::from_secret_json(&json).unwrap();
        assert_eq!(restored.refresh_token(), "refresh");
        assert_eq!(restored.granted_scopes(), tokens.granted_scopes());
    }

    #[tokio::test]
    async fn explicit_zero_callback_port_is_rejected_before_binding() {
        let client = GoogleDesktopClient::from_google_client_json(&desktop_client_json()).unwrap();
        let result =
            authorize_google_desktop(&client, GmailAccessProfile::Send, None, Some(0), |_| Ok(()))
                .await;

        let error = match result {
            Ok(_) => panic!("zero callback port unexpectedly accepted"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("between 1 and 65535"));
    }
}
