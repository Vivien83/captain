use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    /// Enable username/password authentication for browser surfaces.
    pub enabled: bool,
    /// Admin username.
    pub username: String,
    /// SHA256 hash of the password (hex-encoded).
    /// Generate with: captain auth hash-password
    pub password_hash: String,
    /// Session token lifetime in hours (default: 72 = 3 days).
    pub session_ttl_hours: u64,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            username: "admin".to_string(),
            password_hash: String::new(),
            session_ttl_hours: 72,
        }
    }
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
        assert_eq!(auth.username, "admin");
        assert!(auth.password_hash.is_empty());
        assert_eq!(auth.session_ttl_hours, 72);
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
