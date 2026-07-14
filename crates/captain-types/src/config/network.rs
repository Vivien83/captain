use serde::{Deserialize, Serialize};

/// Network layer configuration.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    /// libp2p listen addresses.
    pub listen_addresses: Vec<String>,
    /// Bootstrap peers for DHT.
    pub bootstrap_peers: Vec<String>,
    /// Enable mDNS for local discovery.
    pub mdns_enabled: bool,
    /// Maximum number of connected peers.
    pub max_peers: u32,
    /// Pre-shared secret for OFP HMAC authentication (required when network is enabled).
    pub shared_secret: String,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            listen_addresses: vec!["/ip4/0.0.0.0/tcp/0".to_string()],
            bootstrap_peers: vec![],
            mdns_enabled: true,
            max_peers: 50,
            shared_secret: String::new(),
        }
    }
}

/// SECURITY: Custom Debug impl redacts sensitive fields (shared_secret).
impl std::fmt::Debug for NetworkConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkConfig")
            .field("listen_addresses", &self.listen_addresses)
            .field("bootstrap_peers", &self.bootstrap_peers)
            .field("mdns_enabled", &self.mdns_enabled)
            .field("max_peers", &self.max_peers)
            .field(
                "shared_secret",
                &if self.shared_secret.is_empty() {
                    "<empty>"
                } else {
                    "<redacted>"
                },
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::NetworkConfig;
    use crate::config::KernelConfig;

    #[test]
    fn network_config_defaults_keep_local_discovery_ready() {
        let config = NetworkConfig::default();

        assert_eq!(config.listen_addresses, vec!["/ip4/0.0.0.0/tcp/0"]);
        assert!(config.bootstrap_peers.is_empty());
        assert!(config.mdns_enabled);
        assert_eq!(config.max_peers, 50);
        assert!(config.shared_secret.is_empty());
    }

    #[test]
    fn network_debug_redacts_shared_secret() {
        let config = NetworkConfig {
            shared_secret: "super-secret".to_string(),
            ..Default::default()
        };

        let debug = format!("{config:?}");

        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("super-secret"));
    }

    #[test]
    fn network_debug_marks_empty_shared_secret() {
        let debug = format!("{:?}", NetworkConfig::default());

        assert!(debug.contains("<empty>"));
    }

    #[test]
    fn network_section_deserializes_from_kernel_toml() {
        let config: KernelConfig = toml::from_str(
            r#"
            network_enabled = true

            [network]
            listen_addresses = ["/ip4/127.0.0.1/tcp/9000"]
            bootstrap_peers = ["/ip4/127.0.0.1/tcp/9001/p2p/peer"]
            mdns_enabled = false
            max_peers = 7
            shared_secret = "ofp-secret"
            "#,
        )
        .unwrap();

        assert!(config.network_enabled);
        assert_eq!(config.network.listen_addresses.len(), 1);
        assert_eq!(config.network.bootstrap_peers.len(), 1);
        assert!(!config.network.mdns_enabled);
        assert_eq!(config.network.max_peers, 7);
        assert_eq!(config.network.shared_secret, "ofp-secret");
    }
}
