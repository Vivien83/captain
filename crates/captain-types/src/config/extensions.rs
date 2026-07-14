use serde::{Deserialize, Serialize};

/// Extensions & integrations configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtensionsConfig {
    /// Enable auto-reconnect for MCP integrations.
    pub auto_reconnect: bool,
    /// Maximum reconnect attempts before giving up.
    pub reconnect_max_attempts: u32,
    /// Maximum backoff duration in seconds.
    pub reconnect_max_backoff_secs: u64,
    /// Health check interval in seconds.
    pub health_check_interval_secs: u64,
}

impl Default for ExtensionsConfig {
    fn default() -> Self {
        Self {
            auto_reconnect: true,
            reconnect_max_attempts: 10,
            reconnect_max_backoff_secs: 300,
            health_check_interval_secs: 60,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ExtensionsConfig;
    use crate::config::KernelConfig;

    #[test]
    fn extensions_defaults_keep_reconnect_enabled() {
        let config = ExtensionsConfig::default();

        assert!(config.auto_reconnect);
        assert_eq!(config.reconnect_max_attempts, 10);
        assert_eq!(config.reconnect_max_backoff_secs, 300);
        assert_eq!(config.health_check_interval_secs, 60);
    }

    #[test]
    fn extensions_deserialize_partial_kernel_toml_with_defaults() {
        let config: KernelConfig = toml::from_str(
            r#"
            [extensions]
            auto_reconnect = false
            reconnect_max_attempts = 2
            "#,
        )
        .unwrap();

        assert!(!config.extensions.auto_reconnect);
        assert_eq!(config.extensions.reconnect_max_attempts, 2);
        assert_eq!(config.extensions.reconnect_max_backoff_secs, 300);
        assert_eq!(config.extensions.health_check_interval_secs, 60);
    }
}
