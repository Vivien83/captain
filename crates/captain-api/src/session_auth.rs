//! Stateless session token authentication for browser surfaces.
//! Tokens are HMAC-SHA256 signed and contain username + expiry.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::path::Path;

type HmacSha256 = Hmac<Sha256>;

/// Live web auth snapshot loaded from `config.toml`.
#[derive(Clone, Debug)]
pub struct WebAuthSnapshot {
    pub api_key: String,
    pub auth: captain_types::config::AuthConfig,
}

impl WebAuthSnapshot {
    pub fn api_key_configured(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    pub fn session_secret(&self) -> String {
        derive_session_secret(&self.api_key, &self.auth.password_hash)
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
        return fallback;
    };
    let mut snapshot = fallback;

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
        if let Some(ttl) = auth.get("session_ttl_hours").and_then(|v| v.as_integer()) {
            if ttl > 0 {
                snapshot.auth.session_ttl_hours = ttl as u64;
            }
        }
    }
    snapshot
}

/// Keep sessions bound to both the API key and the password hash. Rotating the
/// web password invalidates old browser sessions even when api_key is stable.
pub fn derive_session_secret(api_key: &str, password_hash: &str) -> String {
    let api_key = api_key.trim();
    if !api_key.is_empty() && !password_hash.is_empty() {
        format!("{api_key}:{password_hash}")
    } else if !api_key.is_empty() {
        api_key.to_string()
    } else {
        password_hash.to_string()
    }
}

pub fn username_matches(provided: &str, expected: &str) -> bool {
    use subtle::ConstantTimeEq;
    if provided.len() != expected.len() {
        return false;
    }
    provided.as_bytes().ct_eq(expected.as_bytes()).into()
}

pub fn verify_session_token_for_auth(token: &str, snapshot: &WebAuthSnapshot) -> Option<String> {
    let username = verify_session_token(token, &snapshot.session_secret())?;
    if username_matches(&username, &snapshot.auth.username) {
        Some(username)
    } else {
        None
    }
}

/// Create a session token: base64(username:expiry_unix:hmac_hex)
pub fn create_session_token(username: &str, secret: &str, ttl_hours: u64) -> String {
    use base64::Engine;
    let expiry = chrono::Utc::now().timestamp() + (ttl_hours as i64 * 3600);
    let payload = format!("{username}:{expiry}");
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC key");
    mac.update(payload.as_bytes());
    let signature = hex::encode(mac.finalize().into_bytes());
    base64::engine::general_purpose::STANDARD.encode(format!("{payload}:{signature}"))
}

/// Verify a session token. Returns the username if valid and not expired.
pub fn verify_session_token(token: &str, secret: &str) -> Option<String> {
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(token)
        .ok()?;
    let decoded_str = String::from_utf8(decoded).ok()?;
    let parts: Vec<&str> = decoded_str.splitn(3, ':').collect();
    if parts.len() != 3 {
        return None;
    }
    let (username, expiry_str, provided_sig) = (parts[0], parts[1], parts[2]);

    let expiry: i64 = expiry_str.parse().ok()?;
    if chrono::Utc::now().timestamp() > expiry {
        return None;
    }

    let payload = format!("{username}:{expiry_str}");
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).ok()?;
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

/// Hash a password with SHA256 for config storage.
pub fn hash_password(password: &str) -> String {
    use sha2::Digest;
    hex::encode(Sha256::digest(password.as_bytes()))
}

/// Verify a password against a stored SHA256 hash (constant-time).
pub fn verify_password(password: &str, stored_hash: &str) -> bool {
    let computed = hash_password(password);
    use subtle::ConstantTimeEq;
    if computed.len() != stored_hash.len() {
        return false;
    }
    computed.as_bytes().ct_eq(stored_hash.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_and_verify_password() {
        let hash = hash_password("secret123");
        assert!(verify_password("secret123", &hash));
        assert!(!verify_password("wrong", &hash));
    }

    #[test]
    fn test_create_and_verify_token() {
        let token = create_session_token("admin", "my-secret", 1);
        let user = verify_session_token(&token, "my-secret");
        assert_eq!(user, Some("admin".to_string()));
    }

    #[test]
    fn test_derive_session_secret_binds_api_key_and_password_hash() {
        assert_eq!(derive_session_secret("api", "hash"), "api:hash");
        assert_eq!(derive_session_secret("api", ""), "api");
        assert_eq!(derive_session_secret("", "hash"), "hash");
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
"#,
        )
        .unwrap();
        let fallback = captain_types::config::AuthConfig::default();
        let snapshot = load_web_auth_snapshot(dir.path(), "secret-from-store", &fallback);
        assert_eq!(snapshot.api_key, "secret-from-store");
        assert!(snapshot.auth.enabled);
        assert_eq!(snapshot.auth.username, "admin");
    }

    #[test]
    fn test_verify_session_token_rejects_old_username() {
        let mut auth = captain_types::config::AuthConfig {
            enabled: true,
            username: "new-admin".to_string(),
            password_hash: "hash".to_string(),
            session_ttl_hours: 1,
        };
        let snapshot = WebAuthSnapshot {
            api_key: "api".to_string(),
            auth: auth.clone(),
        };
        let token = create_session_token("admin", &snapshot.session_secret(), 1);
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
        let token = create_session_token("admin", "my-secret", 1);
        let user = verify_session_token(&token, "wrong-secret");
        assert_eq!(user, None);
    }

    #[test]
    fn test_token_invalid_base64() {
        let user = verify_session_token("not-valid-base64!!!", "secret");
        assert_eq!(user, None);
    }

    #[test]
    fn test_password_hash_length_mismatch() {
        assert!(!verify_password("x", "short"));
    }
}
