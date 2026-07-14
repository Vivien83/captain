use serde::{Deserialize, Serialize};

/// Config hot-reload mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReloadMode {
    /// No automatic reloading.
    Off,
    /// Full restart on config change.
    Restart,
    /// Hot-reload safe sections only (channels, skills, heartbeat).
    Hot,
    /// Hot-reload where possible, flag restart-required otherwise.
    #[default]
    Hybrid,
}

/// Configuration for config file watching and hot-reload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReloadConfig {
    /// Reload mode. Default: hybrid.
    pub mode: ReloadMode,
    /// Debounce window in milliseconds. Default: 500.
    pub debounce_ms: u64,
}

impl Default for ReloadConfig {
    fn default() -> Self {
        Self {
            mode: ReloadMode::default(),
            debounce_ms: 500,
        }
    }
}

/// Webhook trigger authentication configuration.
///
/// Controls the `/hooks/wake` and `/hooks/agent` endpoints for external
/// systems to trigger agent actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebhookTriggerConfig {
    /// Enable webhook trigger endpoints. Default: false.
    pub enabled: bool,
    /// Env var name holding the bearer token (NOT the token itself).
    /// MUST be set if enabled=true. Token must be >= 32 chars.
    pub token_env: String,
    /// Max payload size in bytes. Default: 65536.
    pub max_payload_bytes: usize,
    /// Rate limit: max requests per minute per IP. Default: 30.
    pub rate_limit_per_minute: u32,
}

impl Default for WebhookTriggerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            token_env: "CAPTAIN_WEBHOOK_TOKEN".to_string(),
            max_payload_bytes: 65536,
            rate_limit_per_minute: 30,
        }
    }
}

/// Native outbound webhook dispatcher configuration.
///
/// These hooks subscribe to Captain lifecycle events directly. They are not
/// model tool calls: when enabled, the daemon emits matching internal events to
/// the configured endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OutboundWebhooksConfig {
    /// Enable the dispatcher. Default: false.
    pub enabled: bool,
    /// Per-endpoint delivery timeout. Default: 10 seconds.
    pub timeout_secs: u64,
    /// Max number of attempts per delivery. Default: 3.
    pub max_attempts: u8,
    /// Registered endpoints.
    pub endpoints: Vec<OutboundWebhookEndpoint>,
}

impl Default for OutboundWebhooksConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_secs: 10,
            max_attempts: 3,
            endpoints: Vec::new(),
        }
    }
}

/// One outbound webhook endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OutboundWebhookEndpoint {
    /// Human-readable stable name.
    pub name: String,
    /// Public http(s) destination URL.
    pub url: String,
    /// Env var holding the HMAC secret. If empty, no signature is sent.
    pub secret_env: String,
    /// Event kinds to emit. Supports "*" wildcard.
    pub events: Vec<String>,
    /// Enable this endpoint.
    pub enabled: bool,
}

impl Default for OutboundWebhookEndpoint {
    fn default() -> Self {
        Self {
            name: String::new(),
            url: String::new(),
            secret_env: String::new(),
            events: vec!["*".to_string()],
            enabled: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        OutboundWebhookEndpoint, OutboundWebhooksConfig, ReloadConfig, ReloadMode,
        WebhookTriggerConfig,
    };
    use crate::config::KernelConfig;
    use serde::Deserialize;

    #[test]
    fn reload_config_defaults_to_hybrid_with_short_debounce() {
        let config = ReloadConfig::default();

        assert_eq!(config.mode, ReloadMode::Hybrid);
        assert_eq!(config.debounce_ms, 500);
    }

    #[test]
    fn reload_mode_serde_accepts_snake_case_values() {
        #[derive(Deserialize)]
        struct Wrapper {
            mode: ReloadMode,
        }

        let hot: Wrapper = toml::from_str("mode = \"hot\"").unwrap();
        let restart: Wrapper = toml::from_str("mode = \"restart\"").unwrap();

        assert_eq!(hot.mode, ReloadMode::Hot);
        assert_eq!(restart.mode, ReloadMode::Restart);
    }

    #[test]
    fn webhook_trigger_defaults_keep_endpoint_locked() {
        let config = WebhookTriggerConfig::default();

        assert!(!config.enabled);
        assert_eq!(config.token_env, "CAPTAIN_WEBHOOK_TOKEN");
        assert_eq!(config.max_payload_bytes, 65536);
        assert_eq!(config.rate_limit_per_minute, 30);
    }

    #[test]
    fn outbound_webhook_defaults_keep_dispatcher_disabled() {
        let config = OutboundWebhooksConfig::default();
        let endpoint = OutboundWebhookEndpoint::default();

        assert!(!config.enabled);
        assert_eq!(config.timeout_secs, 10);
        assert_eq!(config.max_attempts, 3);
        assert!(config.endpoints.is_empty());
        assert!(endpoint.enabled);
        assert_eq!(endpoint.events, vec!["*"]);
        assert!(endpoint.secret_env.is_empty());
    }

    #[test]
    fn automation_sections_deserialize_from_kernel_toml() {
        let config: KernelConfig = toml::from_str(
            r#"
            [reload]
            mode = "hot"
            debounce_ms = 750

            [webhook_triggers]
            enabled = true
            token_env = "CAPTAIN_HOOK_TOKEN"
            max_payload_bytes = 4096
            rate_limit_per_minute = 12

            [outbound_webhooks]
            enabled = true
            timeout_secs = 7
            max_attempts = 4

            [[outbound_webhooks.endpoints]]
            name = "audit"
            url = "https://example.com/hook"
            secret_env = "CAPTAIN_OUTBOUND_WEBHOOK_SECRET"
            events = ["project.completed"]
            enabled = false
            "#,
        )
        .unwrap();

        assert_eq!(config.reload.mode, ReloadMode::Hot);
        assert_eq!(config.reload.debounce_ms, 750);

        let trigger = config.webhook_triggers.unwrap();
        assert!(trigger.enabled);
        assert_eq!(trigger.token_env, "CAPTAIN_HOOK_TOKEN");
        assert_eq!(trigger.max_payload_bytes, 4096);
        assert_eq!(trigger.rate_limit_per_minute, 12);

        assert!(config.outbound_webhooks.enabled);
        assert_eq!(config.outbound_webhooks.timeout_secs, 7);
        assert_eq!(config.outbound_webhooks.max_attempts, 4);
        assert_eq!(config.outbound_webhooks.endpoints.len(), 1);
        assert_eq!(config.outbound_webhooks.endpoints[0].name, "audit");
        assert!(!config.outbound_webhooks.endpoints[0].enabled);
        assert_eq!(
            config.outbound_webhooks.endpoints[0].events,
            vec!["project.completed"]
        );
    }
}
