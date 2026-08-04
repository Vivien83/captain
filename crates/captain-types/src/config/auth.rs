use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

/// User configuration for RBAC multi-user support.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    /// User display name.
    pub name: String,
    /// User role (owner, admin, user, viewer).
    #[serde(default = "default_role")]
    pub role: String,
    /// Channel bindings: maps channel platform IDs to this user.
    /// e.g., {"telegram": "123456", "discord": "987654"}
    #[serde(default)]
    pub channel_bindings: HashMap<String, String>,
    /// Optional API key hash for API authentication.
    #[serde(default)]
    pub api_key_hash: Option<String>,
}

fn default_role() -> String {
    "user".to_string()
}

/// Credential vault configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VaultConfig {
    /// Whether the vault is enabled (auto-detected if vault.enc exists).
    pub enabled: bool,
    /// Custom vault file path (default: ~/.captain/vault.enc).
    pub path: Option<PathBuf>,
}

impl Default for VaultConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            path: None,
        }
    }
}

/// A named authentication profile for a provider.
///
/// Multiple profiles can be configured per provider to enable key rotation
/// when one key gets rate-limited or has billing issues.
#[derive(Clone, Serialize, Deserialize)]
pub struct AuthProfile {
    /// Profile name (e.g., "primary", "secondary").
    pub name: String,
    /// Environment variable holding the API key.
    pub api_key_env: String,
    /// Priority (lower = preferred). Default: 0.
    #[serde(default)]
    pub priority: u32,
}

/// SECURITY: Custom Debug impl redacts env var name.
impl std::fmt::Debug for AuthProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthProfile")
            .field("name", &self.name)
            .field("api_key_env", &"<redacted>")
            .field("priority", &self.priority)
            .finish()
    }
}

/// Web authentication (username/password login).
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    /// Enable username/password authentication for browser surfaces.
    pub enabled: bool,
    /// Explicitly permit credentialless access from a direct loopback client.
    ///
    /// This is a local-development escape hatch, not a deployment profile.
    /// New installations fail closed unless setup provisions credentials.
    pub allow_unauthenticated_loopback: bool,
    /// Admin username.
    pub username: String,
    /// Argon2id PHC password hash. Legacy SHA-256 values migrate at login.
    pub password_hash: String,
    /// Captain-managed base64 encoding of a 32-byte session signing key.
    pub session_secret: String,
    /// Monotonic credential revision embedded in every browser session token.
    pub session_epoch: u64,
    /// Session token lifetime in hours (default: 72 = 3 days).
    pub session_ttl_hours: u64,
    /// Secure-cookie policy: automatic HTTPS detection, always, or never.
    pub session_cookie_secure: SessionCookieSecurePolicy,
}

impl std::fmt::Debug for AuthConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthConfig")
            .field("enabled", &self.enabled)
            .field(
                "allow_unauthenticated_loopback",
                &self.allow_unauthenticated_loopback,
            )
            .field("username", &self.username)
            .field(
                "password_hash",
                &if self.password_hash.is_empty() {
                    "<unset>"
                } else {
                    "<redacted>"
                },
            )
            .field(
                "session_secret",
                &if self.session_secret.is_empty() {
                    "<unset>"
                } else {
                    "<redacted>"
                },
            )
            .field("session_epoch", &self.session_epoch)
            .field("session_ttl_hours", &self.session_ttl_hours)
            .field("session_cookie_secure", &self.session_cookie_secure)
            .finish()
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_unauthenticated_loopback: false,
            username: "admin".to_string(),
            password_hash: String::new(),
            session_secret: String::new(),
            session_epoch: 0,
            session_ttl_hours: 72,
            session_cookie_secure: SessionCookieSecurePolicy::Auto,
        }
    }
}

/// Policy used when emitting the browser session cookie.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionCookieSecurePolicy {
    /// Add `Secure` when the configured public URL or a trusted proxy says HTTPS.
    #[default]
    Auto,
    /// Always add `Secure`.
    Always,
    /// Never add `Secure`. Intended only for explicit local HTTP development.
    Never,
}

/// Result of verifying a configured web password hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebPasswordVerification {
    Invalid,
    Argon2id,
    LegacySha256,
}

impl WebPasswordVerification {
    pub fn is_valid(self) -> bool {
        !matches!(self, Self::Invalid)
    }
}

/// Whether a dotted config path is owned by Captain's web-auth lifecycle.
pub fn is_managed_auth_config_path(path: &str) -> bool {
    matches!(
        path,
        "auth.password_hash" | "auth.session_secret" | "auth.session_epoch"
    )
}

/// Whether reading a dotted config path would disclose authentication material.
pub fn is_secret_auth_config_path(path: &str) -> bool {
    matches!(path, "auth.password_hash" | "auth.session_secret")
}

/// Generate a base64 session key from 32 bytes supplied by the OS CSPRNG.
pub fn generate_session_secret() -> io::Result<String> {
    let mut key = [0u8; 32];
    fill_random_bytes(&mut key)?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key))
}

fn fill_random_bytes(output: &mut [u8]) -> io::Result<()> {
    rand::rngs::OsRng
        .try_fill_bytes(output)
        .map_err(|error| io::Error::other(format!("OS CSPRNG unavailable: {error}")))
}

/// Hash a browser password with Argon2id and a fresh CSPRNG salt.
pub fn hash_web_password(password: &str) -> io::Result<String> {
    let mut salt_bytes = [0u8; 16];
    fill_random_bytes(&mut salt_bytes)?;
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|error| io::Error::other(format!("encode Argon2 salt: {error}")))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| io::Error::other(format!("hash password with Argon2id: {error}")))
}

/// Verify Argon2id PHC hashes and the legacy unsalted SHA-256 format.
pub fn verify_web_password(password: &str, stored_hash: &str) -> WebPasswordVerification {
    if stored_hash.starts_with("$argon2") {
        let Ok(parsed) = PasswordHash::new(stored_hash) else {
            return WebPasswordVerification::Invalid;
        };
        if parsed.algorithm.as_str() != "argon2id" {
            return WebPasswordVerification::Invalid;
        }
        return if Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
        {
            WebPasswordVerification::Argon2id
        } else {
            WebPasswordVerification::Invalid
        };
    }

    let computed = hex::encode(Sha256::digest(password.as_bytes()));
    use subtle::ConstantTimeEq;
    if stored_hash.len() == computed.len()
        && bool::from(stored_hash.as_bytes().ct_eq(computed.as_bytes()))
    {
        WebPasswordVerification::LegacySha256
    } else {
        WebPasswordVerification::Invalid
    }
}

/// Decode a configured session key without deriving it from any credential.
pub fn decode_session_secret(encoded: &str) -> Option<[u8; 32]> {
    let encoded = encoded.trim();
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(encoded))
        .ok()?;
    decoded.try_into().ok()
}

/// Ensure that one durable, private signing state exists before Kernel boot.
///
/// The root `config.toml` is patched rather than reserialized so user comments
/// and unrelated formatting survive an upgrade. Existing non-empty malformed
/// keys fail closed instead of being silently rotated.
pub fn ensure_session_signing_state(config_path: &Path, auth: &mut AuthConfig) -> io::Result<bool> {
    let raw = match std::fs::read_to_string(config_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let mut document = if raw.trim().is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        raw.parse::<toml_edit::DocumentMut>()
            .map_err(|error| invalid_auth_state(format!("parse config.toml: {error}")))?
    };

    if !document.as_table().contains_key("auth") {
        document["auth"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let auth_table = document
        .get_mut("auth")
        .and_then(toml_edit::Item::as_table_like_mut)
        .ok_or_else(|| invalid_auth_state("[auth] must be a TOML table"))?;

    let persisted_enabled = match auth_table.get("enabled") {
        Some(item) => Some(
            item.as_bool()
                .ok_or_else(|| invalid_auth_state("auth.enabled must be a boolean"))?,
        ),
        None => None,
    };
    let persisted_loopback_opt_out = match auth_table.get("allow_unauthenticated_loopback") {
        Some(item) => Some(item.as_bool().ok_or_else(|| {
            invalid_auth_state("auth.allow_unauthenticated_loopback must be a boolean")
        })?),
        None => None,
    };
    let persisted_secret = match auth_table.get("session_secret") {
        Some(item) => Some(
            item.as_str()
                .ok_or_else(|| invalid_auth_state("auth.session_secret must be a string"))?
                .trim()
                .to_string(),
        ),
        None => None,
    };
    let persisted_epoch = match auth_table.get("session_epoch") {
        Some(item) => {
            let value = item
                .as_integer()
                .ok_or_else(|| invalid_auth_state("auth.session_epoch must be an integer"))?;
            Some(
                u64::try_from(value)
                    .map_err(|_| invalid_auth_state("auth.session_epoch must be non-negative"))?,
            )
        }
        None => None,
    };

    if let Some(secret) = persisted_secret
        .as_deref()
        .filter(|secret| !secret.is_empty())
    {
        if decode_session_secret(secret).is_none() {
            return Err(invalid_auth_state(
                "auth.session_secret must encode exactly 32 bytes",
            ));
        }
        auth.session_secret = secret.to_string();
    } else if auth.session_secret.trim().is_empty() {
        auth.session_secret = generate_session_secret()?;
    } else if decode_session_secret(&auth.session_secret).is_none() {
        return Err(invalid_auth_state(
            "auth.session_secret must encode exactly 32 bytes",
        ));
    }
    if let Some(epoch) = persisted_epoch {
        auth.session_epoch = epoch;
    }
    auth.allow_unauthenticated_loopback =
        persisted_loopback_opt_out.unwrap_or(matches!(persisted_enabled, Some(false)));

    let changed = persisted_secret.as_deref() != Some(auth.session_secret.as_str())
        || persisted_epoch != Some(auth.session_epoch)
        || persisted_loopback_opt_out != Some(auth.allow_unauthenticated_loopback);
    if changed {
        auth_table.insert(
            "session_secret",
            toml_edit::value(auth.session_secret.as_str()),
        );
        auth_table.insert(
            "session_epoch",
            toml_edit::value(
                i64::try_from(auth.session_epoch)
                    .map_err(|_| invalid_auth_state("auth.session_epoch exceeds TOML range"))?,
            ),
        );
        auth_table.insert(
            "allow_unauthenticated_loopback",
            toml_edit::value(auth.allow_unauthenticated_loopback),
        );
        crate::durable_fs::atomic_write(config_path, document.to_string().as_bytes())?;
    }
    Ok(changed)
}

/// Remove web-auth key material before config text crosses a display boundary.
pub fn redact_auth_secrets(source: &str) -> Result<String, toml_edit::TomlError> {
    let mut document = source.parse::<toml_edit::DocumentMut>()?;
    if let Some(auth) = document
        .get_mut("auth")
        .and_then(toml_edit::Item::as_table_like_mut)
    {
        auth.remove("password_hash");
        auth.remove("session_secret");
    }
    Ok(document.to_string())
}

fn invalid_auth_state(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// OAuth client ID overrides for PKCE flows.
///
/// Configure in config.toml:
/// ```toml
/// [oauth]
/// google_client_id = "your-google-client-id"
/// github_client_id = "your-github-client-id"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OAuthConfig {
    /// Google OAuth2 client ID for PKCE flow.
    pub google_client_id: Option<String>,
    /// GitHub OAuth client ID for PKCE flow.
    pub github_client_id: Option<String>,
    /// Microsoft (Entra ID) OAuth client ID.
    pub microsoft_client_id: Option<String>,
    /// Slack OAuth client ID.
    pub slack_client_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_config_missing_optional_fields_uses_safe_defaults() {
        let user: UserConfig = toml::from_str(
            r#"
            name = "Alice"
            "#,
        )
        .unwrap();

        assert_eq!(user.name, "Alice");
        assert_eq!(user.role, "user");
        assert!(user.channel_bindings.is_empty());
        assert!(user.api_key_hash.is_none());
    }

    #[test]
    fn vault_config_defaults_to_enabled_without_path() {
        let vault = VaultConfig::default();

        assert!(vault.enabled);
        assert!(vault.path.is_none());
    }

    #[test]
    fn auth_profile_debug_redacts_api_key_env() {
        let profile = AuthProfile {
            name: "primary".to_string(),
            api_key_env: "SUPER_SECRET_KEY_ENV".to_string(),
            priority: 7,
        };

        let debug = format!("{profile:?}");
        assert!(debug.contains("primary"));
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("SUPER_SECRET_KEY_ENV"));
    }

    #[test]
    fn auth_profile_priority_defaults_to_zero() {
        let profile: AuthProfile = toml::from_str(
            r#"
            name = "primary"
            api_key_env = "OPENAI_API_KEY"
            "#,
        )
        .unwrap();

        assert_eq!(profile.priority, 0);
    }

    #[test]
    fn auth_config_defaults_keep_web_login_disabled() {
        let auth = AuthConfig::default();

        assert!(!auth.enabled);
        assert!(!auth.allow_unauthenticated_loopback);
        assert_eq!(auth.username, "admin");
        assert!(auth.password_hash.is_empty());
        assert!(auth.session_secret.is_empty());
        assert_eq!(auth.session_epoch, 0);
        assert_eq!(auth.session_ttl_hours, 72);
        assert_eq!(auth.session_cookie_secure, SessionCookieSecurePolicy::Auto);
    }

    #[test]
    fn managed_auth_paths_are_classified_without_hiding_the_epoch() {
        assert!(is_managed_auth_config_path("auth.password_hash"));
        assert!(is_managed_auth_config_path("auth.session_secret"));
        assert!(is_managed_auth_config_path("auth.session_epoch"));
        assert!(!is_managed_auth_config_path("auth.session_ttl_hours"));

        assert!(is_secret_auth_config_path("auth.password_hash"));
        assert!(is_secret_auth_config_path("auth.session_secret"));
        assert!(!is_secret_auth_config_path("auth.session_epoch"));
    }

    #[test]
    fn session_signing_state_is_unique_and_durable_per_instance() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let shared_config =
            "api_key = \"same-api-key\"\n\n[auth]\npassword_hash = \"same-password-hash\"\n";
        std::fs::write(first.path().join("config.toml"), shared_config).unwrap();
        std::fs::write(second.path().join("config.toml"), shared_config).unwrap();
        let mut first_auth = AuthConfig {
            password_hash: "same-password-hash".to_string(),
            ..AuthConfig::default()
        };
        let mut second_auth = first_auth.clone();

        assert!(
            ensure_session_signing_state(&first.path().join("config.toml"), &mut first_auth)
                .unwrap()
        );
        assert!(
            ensure_session_signing_state(&second.path().join("config.toml"), &mut second_auth)
                .unwrap()
        );

        assert_ne!(first_auth.session_secret, second_auth.session_secret);
        assert!(decode_session_secret(&first_auth.session_secret).is_some());
        assert!(decode_session_secret(&second_auth.session_secret).is_some());

        let first_secret = first_auth.session_secret.clone();
        assert!(
            !ensure_session_signing_state(&first.path().join("config.toml"), &mut first_auth)
                .unwrap()
        );
        assert_eq!(first_auth.session_secret, first_secret);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(first.path().join("config.toml"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn malformed_persisted_session_secret_fails_closed() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("config.toml");
        std::fs::write(
            &path,
            "[auth]\nsession_secret = \"not-a-32-byte-key\"\nsession_epoch = 0\n",
        )
        .unwrap();
        let mut auth = AuthConfig::default();

        let error = ensure_session_signing_state(&path, &mut auth).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(auth.session_secret.is_empty());
    }

    #[test]
    fn missing_auth_configuration_persists_fail_closed_loopback_policy() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("config.toml");
        std::fs::write(&path, "api_listen = \"127.0.0.1:50051\"\n").unwrap();
        let mut auth = AuthConfig::default();

        assert!(ensure_session_signing_state(&path, &mut auth).unwrap());

        assert!(!auth.allow_unauthenticated_loopback);
        let persisted = std::fs::read_to_string(path).unwrap();
        assert!(persisted.contains("allow_unauthenticated_loopback = false"));
    }

    #[test]
    fn legacy_explicitly_disabled_auth_migrates_to_explicit_loopback_opt_out() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("config.toml");
        std::fs::write(&path, "[auth]\nenabled = false\n").unwrap();
        let mut auth = AuthConfig::default();

        assert!(ensure_session_signing_state(&path, &mut auth).unwrap());

        assert!(auth.allow_unauthenticated_loopback);
        let persisted = std::fs::read_to_string(path).unwrap();
        assert!(persisted.contains("allow_unauthenticated_loopback = true"));
    }

    #[test]
    fn explicit_loopback_policy_is_preserved_without_reinterpretation() {
        for allowed in [false, true] {
            let temporary = tempfile::tempdir().unwrap();
            let path = temporary.path().join("config.toml");
            std::fs::write(
                &path,
                format!("[auth]\nenabled = false\nallow_unauthenticated_loopback = {allowed}\n"),
            )
            .unwrap();
            let mut auth = AuthConfig::default();

            ensure_session_signing_state(&path, &mut auth).unwrap();

            assert_eq!(auth.allow_unauthenticated_loopback, allowed);
        }
    }

    #[test]
    fn auth_debug_and_config_redaction_hide_session_key() {
        let secret = generate_session_secret().unwrap();
        let auth = AuthConfig {
            password_hash: "password-hash".to_string(),
            session_secret: secret.clone(),
            session_epoch: 4,
            ..AuthConfig::default()
        };
        let debug = format!("{auth:?}");
        assert!(!debug.contains("password-hash"));
        assert!(!debug.contains(&secret));

        let raw = format!(
            "[auth]\nsession_secret = \"{secret}\"\nsession_epoch = 4\nsession_ttl_hours = 72\n"
        );
        let redacted = redact_auth_secrets(&raw).unwrap();
        assert!(!redacted.contains(&secret));
        assert!(!redacted.contains("session_secret"));
        assert!(redacted.contains("session_epoch = 4"));
    }

    #[test]
    fn argon2id_password_hashes_are_salted_and_verifiable() {
        let first = hash_web_password("correct horse battery staple").unwrap();
        let second = hash_web_password("correct horse battery staple").unwrap();

        assert!(first.starts_with("$argon2id$"));
        assert!(second.starts_with("$argon2id$"));
        assert_ne!(first, second);
        assert_eq!(
            verify_web_password("correct horse battery staple", &first),
            WebPasswordVerification::Argon2id
        );
        assert_eq!(
            verify_web_password("wrong", &first),
            WebPasswordVerification::Invalid
        );
    }

    #[test]
    fn legacy_sha256_password_hash_is_recognized_only_for_migration() {
        let legacy = hex::encode(Sha256::digest(b"legacy-password"));
        assert_eq!(
            verify_web_password("legacy-password", &legacy),
            WebPasswordVerification::LegacySha256
        );
        assert_eq!(
            verify_web_password("wrong", &legacy),
            WebPasswordVerification::Invalid
        );
        assert_eq!(
            verify_web_password("legacy-password", "$argon2i$v=19$m=4096,t=3,p=1$bad$bad"),
            WebPasswordVerification::Invalid
        );
    }

    #[test]
    fn oauth_config_accepts_partial_pkce_clients() {
        let oauth: OAuthConfig = toml::from_str(
            r#"
            github_client_id = "github-client"
            "#,
        )
        .unwrap();

        assert!(oauth.google_client_id.is_none());
        assert_eq!(oauth.github_client_id.as_deref(), Some("github-client"));
        assert!(oauth.microsoft_client_id.is_none());
        assert!(oauth.slack_client_id.is_none());
    }
}
