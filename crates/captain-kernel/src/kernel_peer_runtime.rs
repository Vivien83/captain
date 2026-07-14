use super::{kernel_config_support::gethostname, CaptainKernel};
use captain_wire::{PeerConfig, PeerNode, PeerRegistry};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{info, warn};

fn parse_multiaddr_socket(raw: &str, default_ip: &str, default_port: &str) -> Option<SocketAddr> {
    let parts: Vec<&str> = raw.split('/').collect();
    let ip = parts.get(2).unwrap_or(&default_ip);
    let port = parts.get(4).unwrap_or(&default_port);
    format!("{ip}:{port}").parse().ok()
}

fn parse_listen_addr(raw: &str) -> SocketAddr {
    let fallback: SocketAddr = "0.0.0.0:9090"
        .parse()
        .expect("static fallback address must parse");
    if raw.starts_with('/') {
        parse_multiaddr_socket(raw, "0.0.0.0", "9090").unwrap_or(fallback)
    } else {
        raw.parse().unwrap_or(fallback)
    }
}

fn parse_bootstrap_addr(raw: &str) -> Option<SocketAddr> {
    if raw.starts_with('/') {
        parse_multiaddr_socket(raw, "127.0.0.1", "9090")
    } else {
        raw.parse().ok()
    }
}

impl CaptainKernel {
    /// Start the OFP peer networking node.
    ///
    /// Binds a TCP listener, registers with the peer registry, and connects
    /// to bootstrap peers from config.
    pub(crate) async fn start_ofp_node(self: &Arc<Self>) {
        let listen_addr_str = self
            .config
            .network
            .listen_addresses
            .first()
            .cloned()
            .unwrap_or_else(|| "0.0.0.0:9090".to_string());
        let listen_addr = parse_listen_addr(&listen_addr_str);

        let node_id = uuid::Uuid::new_v4().to_string();
        let node_name = gethostname().unwrap_or_else(|| "captain-node".to_string());

        let peer_config = PeerConfig {
            listen_addr,
            node_id: node_id.clone(),
            node_name: node_name.clone(),
            shared_secret: self.config.network.shared_secret.clone(),
        };

        let registry = PeerRegistry::new();

        let handle: Arc<dyn captain_wire::peer::PeerHandle> = self.self_arc();

        match PeerNode::start(peer_config, registry.clone(), handle.clone()).await {
            Ok((node, _accept_task)) => {
                let addr = node.local_addr();
                info!(
                    node_id = %node_id,
                    listen = %addr,
                    "OFP peer node started"
                );

                let _ = self.peer_registry.set(registry.clone());
                let _ = self.peer_node.set(node.clone());

                for peer_addr_str in &self.config.network.bootstrap_peers {
                    if let Some(addr) = parse_bootstrap_addr(peer_addr_str) {
                        match node.connect_to_peer(addr, handle.clone()).await {
                            Ok(()) => {
                                info!(peer = %addr, "OFP: connected to bootstrap peer");
                            }
                            Err(e) => {
                                warn!(peer = %addr, error = %e, "OFP: failed to connect to bootstrap peer");
                            }
                        }
                    } else {
                        warn!(addr = %peer_addr_str, "OFP: invalid bootstrap peer address");
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "OFP: failed to start peer node");
            }
        }
    }

    /// Get the kernel's strong Arc reference from the stored weak handle.
    fn self_arc(self: &Arc<Self>) -> Arc<Self> {
        Arc::clone(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listen_addr_parser_matches_hermes_defaults() {
        assert_eq!(
            parse_listen_addr("/ip4/127.0.0.1/tcp/9091"),
            "127.0.0.1:9091".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(
            parse_listen_addr("127.0.0.1:8080"),
            "127.0.0.1:8080".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(
            parse_listen_addr("not-an-address"),
            "0.0.0.0:9090".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn bootstrap_addr_parser_keeps_invalid_addresses_optional() {
        assert_eq!(
            parse_bootstrap_addr("/ip4/10.0.0.4/tcp/9090"),
            Some("10.0.0.4:9090".parse::<SocketAddr>().unwrap())
        );
        assert_eq!(
            parse_bootstrap_addr("/ip4/10.0.0.4"),
            Some("10.0.0.4:9090".parse::<SocketAddr>().unwrap())
        );
        assert_eq!(
            parse_bootstrap_addr("localhost:9090"),
            "localhost:9090".parse::<SocketAddr>().ok()
        );
        assert_eq!(parse_bootstrap_addr("bad"), None);
    }
}
