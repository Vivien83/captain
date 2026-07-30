//! Stateless session token authentication for browser surfaces.
//! Tokens are HMAC-SHA256 signed and contain username + expiry + credential epoch.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::path::Path;

type HmacSha256 = Hmac<Sha256>;

/// Live web auth snapshot loaded from `config.toml`.
#[derive(Clone)]
pub struct WebAuthSnapshot {
    pub api_key: String,
    pub auth: captain_types::config::AuthConfig,
}

impl std::fmt::Debug for WebAuthSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WebAuthSnapshot")
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "<unset>"
                } else {
                    "<redacted>"
                },
            )
            .field("auth", &self.auth)
            .finish()
    }
}

impl WebAuthSnapshot {
    pub fn api_key_configured(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    pub fn session_secret(&self) -> Option<[u8; 32]> {
        derive_session_secret(&self.auth.session_secret)
    }
}

/// Load web/API auth from the persisted config file, falling back to the
/// boot-time config only if the file is unavailable. This keeps web login
/// changes made by Captain tools effective without a daemon restart.
pub fn load_web_auth_snapshot(
    home_dir: &Path,
    fallback_api_key: &str,
    fallback_auth: &captain_types::config::AuthConfig,
) -> WebAuthSnapshot {
    let config_path = home_dir.join("config.toml");
    let fallback = WebAuthSnapshot {
        api_key: fallback_api_key.trim().to_string(),
        auth: fallback_auth.clone(),
    };
    let Ok(raw) = std::fs::read_to_string(&config_path) else {
        return fallback;
    };
    let Ok(parsed) = raw.parse::<toml::Value>() else {
        let mut failed_closed = fallback;
        failed_closed.auth.session_secret.clear();
        return failed_closed;
    };
    let mut snapshot = fallback;
    snapshot.auth.password_hash.clear();
    snapshot.auth.session_secret.clear();

    if let Some(api_key) = parsed.get("api_key").and_then(|v| v.as_str()).or_else(|| {
        parsed
            .get("api")
            .and_then(|v| v.get("api_key"))
            .and_then(|v| v.as_str())
    }) {
        let api_key = api_key.trim();
        if !api_key.is_empty() {
            snapshot.api_key = api_key.to_string();
        }
    }
    if let Some(auth) = parsed.get("auth").and_then(|v| v.as_table()) {
        if let Some(enabled) = auth.get("enabled").and_then(|v| v.as_bool()) {
            snapshot.auth.enabled = enabled;
        }
        if let Some(username) = auth.get("username").and_then(|v| v.as_str()) {
            snapshot.auth.username = username.to_string();
        }
        if let Some(password_hash) = auth.get("password_hash").and_then(|v| v.as_str()) {
            snapshot.auth.password_hash = password_hash.to_string();
        }
        let persisted_signing_state = auth
            .get("session_secret")
            .and_then(|v| v.as_str())
            .filter(|secret| derive_session_secret(secret).is_some())
            .zip(
                auth.get("session_epoch")
                    .and_then(|v| v.as_integer())
                    .and_then(|epoch| u64::try_from(epoch).ok()),
            );
        if let Some((session_secret, session_epoch)) = persisted_signing_state {
            snapshot.auth.session_secret = session_secret.to_string();
            snapshot.auth.session_epoch = session_epoch;
        }
        if let Some(ttl) = auth.get("session_ttl_hours").and_then(|v| v.as_integer()) {
            if ttl > 0 {
                snapshot.auth.session_ttl_hours = ttl as u64;
            }
        }
        if let Some(policy) = auth
            .get("session_cookie_secure")
            .and_then(|value| value.as_str())
        {
            snapshot.auth.session_cookie_secure = match policy {
                "always" => captain_types::config::SessionCookieSecurePolicy::Always,
                "never" => captain_types::config::SessionCookieSecurePolicy::Never,
                _ => captain_types::config::SessionCookieSecurePolicy::Auto,
            };
        }
    }
    snapshot
}

/// Decode only the independent Captain-managed signing key.
pub fn derive_session_secret(session_secret: &str) -> Option<[u8; 32]> {
    captain_types::config::decode_session_secret(session_secret)
}

pub fn username_matches(provided: &str, expected: &str) -> bool {
    use subtle::ConstantTimeEq;
    if provided.len() != expected.len() {
        return false;
    }
    provided.as_bytes().ct_eq(expected.as_bytes()).into()
}

pub fn verify_session_token_for_auth(token: &str, snapshot: &WebAuthSnapshot) -> Option<String> {
    let secret = snapshot.session_secret()?;
    let username = verify_session_token(token, &secret, snapshot.auth.session_epoch)?;
    if username_matches(&username, &snapshot.auth.username) {
        Some(username)
    } else {
        None
    }
}

/// Create a session token: base64(username:expiry_unix:epoch:hmac_hex)
pub fn create_session_token(
    username: &str,
    secret: &[u8; 32],
    ttl_hours: u64,
    session_epoch: u64,
) -> String {
    use base64::Engine;
    let ttl_seconds = ttl_hours.saturating_mul(3600).min(i64::MAX as u64) as i64;
    let expiry = chrono::Utc::now().timestamp().saturating_add(ttl_seconds);
    let payload = format!("{username}:{expiry}:{session_epoch}");
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC key");
    mac.update(payload.as_bytes());
    let signature = hex::encode(mac.finalize().into_bytes());
    base64::engine::general_purpose::STANDARD.encode(format!("{payload}:{signature}"))
}

/// Verify a session token. Returns the username if valid and not expired.
pub fn verify_session_token(
    token: &str,
    secret: &[u8; 32],
    expected_session_epoch: u64,
) -> Option<String> {
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(token)
        .ok()?;
    let decoded_str = String::from_utf8(decoded).ok()?;
    let parts: Vec<&str> = decoded_str.splitn(4, ':').collect();
    if parts.len() != 4 {
        return None;
    }
    let (username, expiry_str, epoch_str, provided_sig) = (parts[0], parts[1], parts[2], parts[3]);

    let expiry: i64 = expiry_str.parse().ok()?;
    if chrono::Utc::now().timestamp() > expiry {
        return None;
    }
    let session_epoch: u64 = epoch_str.parse().ok()?;
    if session_epoch != expected_session_epoch {
        return None;
    }

    let payload = format!("{username}:{expiry_str}:{epoch_str}");
    let mut mac = HmacSha256::new_from_slice(secret).ok()?;
    mac.update(payload.as_bytes());
    let expected_sig = hex::encode(mac.finalize().into_bytes());

    use subtle::ConstantTimeEq;
    if provided_sig.len() != expected_sig.len() {
        return None;
    }
    if provided_sig
        .as_bytes()
        .ct_eq(expected_sig.as_bytes())
        .into()
    {
        Some(username.to_string())
    } else {
        None
    }
}

/// Hash a browser password with Argon2id and a fresh salt.
pub fn hash_password(password: &str) -> std::io::Result<String> {
    captain_types::config::hash_web_password(password)
}

/// Verify Argon2id hashes and legacy SHA-256 hashes during migration.
pub fn verify_password(password: &str, stored_hash: &str) -> bool {
    captain_types::config::verify_web_password(password, stored_hash).is_valid()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_secret(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    #[test]
    fn test_hash_and_verify_password() {
        let first = hash_password("secret123").unwrap();
        let second = hash_password("secret123").unwrap();
        assert_ne!(first, second);
        assert!(verify_password("secret123", &first));
        assert!(!verify_password("wrong", &first));
    }

    #[test]
    fn test_create_and_verify_token() {
        let secret = test_secret(7);
        let token = create_session_token("admin", &secret, 1, 3);
        let user = verify_session_token(&token, &secret, 3);
        assert_eq!(user, Some("admin".to_string()));
    }

    #[test]
    fn test_derive_session_secret_uses_only_the_managed_key() {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(test_secret(11));
        assert_eq!(derive_session_secret(&encoded), Some(test_secret(11)));
        assert_eq!(derive_session_secret("api:password-hash"), None);
    }

    #[test]
    fn test_load_auth_snapshot_keeps_fallback_api_key_when_config_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            r#"
api_key = ""

[auth]
enabled = true
username = "admin"
password_hash = "hash"
session_secret = "CwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCws"
session_epoch = 4
"#,
        )
        .unwrap();
        let fallback = captain_types::config::AuthConfig::default();
        let snapshot = load_web_auth_snapshot(dir.path(), "secret-from-store", &fallback);
        assert_eq!(snapshot.api_key, "secret-from-store");
        assert!(snapshot.auth.enabled);
        assert_eq!(snapshot.auth.username, "admin");
        assert_eq!(snapshot.auth.session_epoch, 4);
        assert_eq!(snapshot.session_secret(), Some(test_secret(11)));
    }

    #[test]
    fn persisted_auth_without_complete_signing_state_fails_closed() {
        let fallback = captain_types::config::AuthConfig {
            session_secret: captain_types::config::generate_session_secret().unwrap(),
            session_epoch: 7,
            ..Default::default()
        };

        for config in [
            "[auth]\nsession_epoch = 7\n",
            "[auth]\nsession_secret = \"CwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCws\"\n",
            "[auth]\nsession_secret = \"invalid\"\nsession_epoch = 7\n",
            "this is not toml",
        ] {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("config.toml"), config).unwrap();
            let snapshot = load_web_auth_snapshot(dir.path(), "api-key", &fallback);
            assert!(
                snapshot.session_secret().is_none(),
                "incomplete persisted signing state must fail closed for {config:?}"
            );
        }
    }

    #[test]
    fn persisted_auth_without_password_hash_never_reuses_fallback_credentials() {
        let fallback = captain_types::config::AuthConfig {
            enabled: true,
            password_hash: hash_password("fallback-password").unwrap(),
            session_secret: captain_types::config::generate_session_secret().unwrap(),
            session_epoch: 7,
            ..Default::default()
        };
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            format!(
                "[auth]\nenabled = true\nsession_secret = \"{}\"\nsession_epoch = 7\n",
                fallback.session_secret
            ),
        )
        .unwrap();

        let snapshot = load_web_auth_snapshot(dir.path(), "api-key", &fallback);
        assert!(snapshot.auth.password_hash.is_empty());
        assert!(!verify_password(
            "fallback-password",
            &snapshot.auth.password_hash
        ));
    }

    #[test]
    fn test_verify_session_token_rejects_old_username() {
        let mut auth = captain_types::config::AuthConfig {
            enabled: true,
            username: "new-admin".to_string(),
            password_hash: "hash".to_string(),
            session_secret: captain_types::config::generate_session_secret().unwrap(),
            session_epoch: 0,
            session_ttl_hours: 1,
            ..Default::default()
        };
        let snapshot = WebAuthSnapshot {
            api_key: "api".to_string(),
            auth: auth.clone(),
        };
        let token = create_session_token(
            "admin",
            &snapshot.session_secret().unwrap(),
            1,
            snapshot.auth.session_epoch,
        );
        assert_eq!(verify_session_token_for_auth(&token, &snapshot), None);

        auth.username = "admin".to_string();
        let snapshot = WebAuthSnapshot {
            api_key: "api".to_string(),
            auth,
        };
        assert_eq!(
            verify_session_token_for_auth(&token, &snapshot),
            Some("admin".to_string())
        );
    }

    #[test]
    fn test_token_wrong_secret() {
        let token = create_session_token("admin", &test_secret(1), 1, 0);
        let user = verify_session_token(&token, &test_secret(2), 0);
        assert_eq!(user, None);
    }

    #[test]
    fn test_token_invalid_base64() {
        let user = verify_session_token("not-valid-base64!!!", &test_secret(1), 0);
        assert_eq!(user, None);
    }

    #[test]
    fn forged_password_hash_key_cannot_sign_a_session() {
        use sha2::Digest;
        let password_hash_key: [u8; 32] = Sha256::digest(b"correct horse battery staple").into();
        let token = create_session_token("admin", &password_hash_key, 1, 0);
        let auth = captain_types::config::AuthConfig {
            enabled: true,
            username: "admin".to_string(),
            password_hash: hex::encode(password_hash_key),
            session_secret: captain_types::config::generate_session_secret().unwrap(),
            session_epoch: 0,
            session_ttl_hours: 1,
            ..Default::default()
        };
        let snapshot = WebAuthSnapshot {
            api_key: String::new(),
            auth,
        };

        assert_eq!(verify_session_token_for_auth(&token, &snapshot), None);
    }

    #[test]
    fn password_change_epoch_invalidates_an_existing_session() {
        let secret = test_secret(9);
        let token = create_session_token("admin", &secret, 1, 12);

        assert_eq!(
            verify_session_token(&token, &secret, 13),
            None,
            "a token from the previous credential epoch must be rejected"
        );
    }

    #[test]
    fn auth_snapshot_debug_redacts_all_key_material() {
        let secret = captain_types::config::generate_session_secret().unwrap();
        let snapshot = WebAuthSnapshot {
            api_key: "captain-api-secret".to_string(),
            auth: captain_types::config::AuthConfig {
                password_hash: "password-hash".to_string(),
                session_secret: secret.clone(),
                ..Default::default()
            },
        };

        let rendered = format!("{snapshot:?}");
        assert!(!rendered.contains("captain-api-secret"));
        assert!(!rendered.contains("password-hash"));
        assert!(!rendered.contains(&secret));
    }

    #[test]
    fn test_password_hash_length_mismatch() {
        assert!(!verify_password("x", "short"));
    }
}
