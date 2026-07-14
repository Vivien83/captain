use serde::{Deserialize, Serialize};

/// Device pairing configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PairingConfig {
    /// Enable device pairing. Default: false.
    pub enabled: bool,
    /// Max paired devices. Default: 10.
    pub max_devices: usize,
    /// Pairing token expiry in seconds. Default: 300 (5 min).
    pub token_expiry_secs: u64,
    /// Push notification provider: "none", "ntfy", "gotify".
    pub push_provider: String,
    /// Ntfy server URL (if push_provider = "ntfy").
    pub ntfy_url: Option<String>,
    /// Ntfy topic (if push_provider = "ntfy").
    pub ntfy_topic: Option<String>,
}

impl Default for PairingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_devices: 10,
            token_expiry_secs: 300,
            push_provider: "none".to_string(),
            ntfy_url: None,
            ntfy_topic: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PairingConfig;
    use crate::config::KernelConfig;

    #[test]
    fn pairing_defaults_keep_device_pairing_disabled() {
        let config = PairingConfig::default();

        assert!(!config.enabled);
        assert_eq!(config.max_devices, 10);
        assert_eq!(config.token_expiry_secs, 300);
        assert_eq!(config.push_provider, "none");
        assert!(config.ntfy_url.is_none());
        assert!(config.ntfy_topic.is_none());
    }

    #[test]
    fn pairing_deserializes_partial_kernel_toml_with_defaults() {
        let config: KernelConfig = toml::from_str(
            r#"
            [pairing]
            enabled = true
            push_provider = "ntfy"
            ntfy_url = "https://ntfy.example.com"
            "#,
        )
        .unwrap();

        assert!(config.pairing.enabled);
        assert_eq!(config.pairing.max_devices, 10);
        assert_eq!(config.pairing.token_expiry_secs, 300);
        assert_eq!(config.pairing.push_provider, "ntfy");
        assert_eq!(
            config.pairing.ntfy_url.as_deref(),
            Some("https://ntfy.example.com")
        );
        assert!(config.pairing.ntfy_topic.is_none());
    }
}
