//! R.1.1 — A2A peer discovery via mDNS (zero-config LAN federation).
//!
//! Each Captain advertises a `_captain._tcp.local.` service on boot and
//! concurrently browses the LAN for other Captains. When a peer is
//! resolved, we fetch its agent card via the existing A2A flow and
//! append it to the kernel's `a2a_external_agents` store, after which
//! the `a2a_send` tool can target it by name without manual `a2a_discover`.
//!
//! Self-filter: every daemon process generates a UUID at boot and writes
//! it to the mDNS TXT record under the `instance_id` key. The browser
//! ignores any resolved service whose `instance_id` matches our own.
//!
//! Decoupling: the module talks to the kernel through [`PeerDiscoveryOps`],
//! a narrow trait that exposes only what discovery needs (instance_id,
//! listen addr, dedup helper, store-append). Tests substitute a stub
//! that records calls without touching the network.

use async_trait::async_trait;
use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// mDNS service type all Captain daemons announce themselves under.
pub const SERVICE_TYPE: &str = "_captain._tcp.local.";

/// TXT record key used to detect our own broadcast and ignore it.
pub const TXT_INSTANCE_ID: &str = "instance_id";

/// TXT record key carrying the human-friendly instance name.
pub const TXT_INSTANCE_NAME: &str = "name";

/// Narrow operations the discovery loop needs from the kernel.
#[async_trait]
pub trait PeerDiscoveryOps: Send + Sync {
    /// UUID generated at boot. Used in the TXT record + self-filter.
    fn instance_id(&self) -> String;
    /// Hostname/IP and port the API server is listening on. Sent in the
    /// service info so peers can resolve us back via HTTP.
    fn api_listen_port(&self) -> u16;
    /// Friendly label (e.g. `captain-prod-server`). Falls back to hostname.
    fn instance_name(&self) -> String;
    /// True when the agent name is already in our external store. Avoids
    /// re-fetching the card every time mDNS re-broadcasts.
    fn has_external_agent(&self, name: &str) -> bool;
    /// Resolve `base_url`, fetch its agent card, append to the store.
    /// Returns `Ok(true)` if newly added, `Ok(false)` if already known.
    async fn add_external_agent(&self, name: &str, base_url: &str) -> Result<bool, String>;
}

/// Decision returned by [`process_event`] for one mDNS event. Captured
/// as data so tests can assert behaviour without fake networks.
#[derive(Debug, PartialEq, Eq)]
pub enum DiscoveryDecision {
    /// We saw our own broadcast; ignore.
    Self_,
    /// Already in the store; nothing to do.
    Already,
    /// Will fetch this peer's card next.
    WillFetch { name: String, base_url: String },
    /// TXT record missing/invalid; ignore.
    Skipped(&'static str),
    /// Event we don't react to (e.g. ServiceFound/Removed/SearchStarted).
    Ignored,
}

/// Pure decision logic for one *resolved* mDNS service. Extracted so we
/// can unit-test the filtering rules without spinning up a real daemon
/// (the caller dispatches events; this fn is pure).
pub fn process_resolved(
    info: &ResolvedService,
    our_instance_id: &str,
    has_agent: &dyn Fn(&str) -> bool,
) -> DiscoveryDecision {
    let props: HashMap<String, String> = info
        .txt_properties
        .iter()
        .map(|p| (p.key().to_string(), p.val_str().to_string()))
        .collect();

    let peer_id = match props.get(TXT_INSTANCE_ID) {
        Some(s) if !s.is_empty() => s,
        _ => return DiscoveryDecision::Skipped("missing instance_id"),
    };
    if peer_id == our_instance_id {
        return DiscoveryDecision::Self_;
    }

    let name = props
        .get(TXT_INSTANCE_NAME)
        .cloned()
        .unwrap_or_else(|| info.fullname.clone());

    if has_agent(&name) {
        return DiscoveryDecision::Already;
    }

    let addr = info.addresses.iter().next().map(|s| s.to_ip_addr());
    let Some(addr) = addr else {
        return DiscoveryDecision::Skipped("no IP address");
    };
    let base_url = format!("http://{addr}:{}", info.port);
    DiscoveryDecision::WillFetch { name, base_url }
}

/// Handle returned by [`spawn_peer_discovery`]. Dropping it shuts down
/// the underlying mDNS daemon (advertising + browsing both stop).
pub struct PeerDiscoveryHandle {
    daemon: ServiceDaemon,
    fullname: String,
}

impl PeerDiscoveryHandle {
    /// Best-effort goodbye: send a "service unregister" message so peers
    /// know we're going away cleanly, then shutdown.
    pub fn shutdown(self) {
        if let Err(e) = self.daemon.unregister(&self.fullname) {
            debug!("mdns unregister failed: {e}");
        }
        let _ = self.daemon.shutdown();
    }
}

/// Spawn the mDNS advertiser + browser for this Captain. Returns an
/// opaque handle that, when dropped, shuts both halves down. On any
/// init failure (no network, sandboxed env, …) returns an error and
/// the caller should log + continue without federation.
pub fn spawn_peer_discovery(ops: Arc<dyn PeerDiscoveryOps>) -> Result<PeerDiscoveryHandle, String> {
    let daemon = ServiceDaemon::new().map_err(|e| format!("ServiceDaemon init: {e}"))?;

    let instance_id = ops.instance_id();
    let instance_name = ops.instance_name();
    let port = ops.api_listen_port();

    let host_ipv4 = local_host_label();
    let mut props = HashMap::new();
    props.insert(TXT_INSTANCE_ID.to_string(), instance_id.clone());
    props.insert(TXT_INSTANCE_NAME.to_string(), instance_name.clone());

    let service_info = ServiceInfo::new(
        SERVICE_TYPE,
        &instance_name,
        &host_ipv4,
        "",
        port,
        Some(props),
    )
    .map_err(|e| format!("ServiceInfo build: {e}"))?
    .enable_addr_auto();

    let fullname = service_info.get_fullname().to_string();
    daemon
        .register(service_info)
        .map_err(|e| format!("mdns register: {e}"))?;

    let receiver = daemon
        .browse(SERVICE_TYPE)
        .map_err(|e| format!("mdns browse: {e}"))?;

    info!(
        instance_id = %instance_id,
        instance_name = %instance_name,
        port,
        "Peer discovery active (mDNS _captain._tcp.local.)"
    );

    let ops_for_loop = ops.clone();
    let our_id = instance_id.clone();
    tokio::spawn(async move {
        while let Ok(event) = receiver.recv_async().await {
            let info = match event {
                ServiceEvent::ServiceResolved(boxed) => boxed,
                _ => continue,
            };
            let has_agent_fn = |n: &str| ops_for_loop.has_external_agent(n);
            match process_resolved(&info, &our_id, &has_agent_fn) {
                DiscoveryDecision::WillFetch { name, base_url } => {
                    match ops_for_loop.add_external_agent(&name, &base_url).await {
                        Ok(true) => {
                            info!(name = %name, base_url = %base_url, "Discovered new Captain peer")
                        }
                        Ok(false) => debug!(name = %name, "Peer already known, skipped"),
                        Err(e) => {
                            warn!(name = %name, base_url = %base_url, "Peer fetch failed: {e}")
                        }
                    }
                }
                DiscoveryDecision::Self_ => debug!("ignored own mDNS broadcast"),
                DiscoveryDecision::Already => {}
                DiscoveryDecision::Skipped(reason) => {
                    debug!(reason, "skipped mDNS event")
                }
                DiscoveryDecision::Ignored => {}
            }
        }
        debug!("mdns receiver closed; discovery loop exiting");
    });

    Ok(PeerDiscoveryHandle { daemon, fullname })
}

/// Best-effort hostname label used as the mDNS host field. mdns-sd
/// expects a `<host>.local.` form; on failure we fall back to a static
/// label so registration still succeeds in containers without a real
/// hostname.
fn local_host_label() -> String {
    let host = hostname_or_fallback();
    if host.ends_with(".local.") {
        host
    } else {
        format!("{host}.local.")
    }
}

fn hostname_or_fallback() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "captain".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::net::IpAddr;

    fn build_resolved(
        fullname: &str,
        instance: &str,
        port: u16,
        ip: IpAddr,
        props: &[(&str, &str)],
    ) -> ResolvedService {
        let mut hm = HashMap::new();
        for (k, v) in props {
            hm.insert(k.to_string(), v.to_string());
        }
        let info = ServiceInfo::new(SERVICE_TYPE, instance, fullname, ip, port, Some(hm))
            .expect("test ServiceInfo");
        info.as_resolved_service()
    }

    #[test]
    fn process_resolved_filters_self_via_instance_id() {
        let svc = build_resolved(
            "alice.local.",
            "alice",
            50051,
            "10.0.0.1".parse().unwrap(),
            &[("instance_id", "MY-OWN-ID"), ("name", "alice")],
        );
        let d = process_resolved(&svc, "MY-OWN-ID", &|_| false);
        assert_eq!(d, DiscoveryDecision::Self_);
    }

    #[test]
    fn process_resolved_will_fetch_unknown_peer() {
        let svc = build_resolved(
            "bob.local.",
            "bob",
            50051,
            "10.0.0.2".parse().unwrap(),
            &[("instance_id", "PEER-B"), ("name", "bob")],
        );
        let d = process_resolved(&svc, "MY-OWN-ID", &|_| false);
        match d {
            DiscoveryDecision::WillFetch { name, base_url } => {
                assert_eq!(name, "bob");
                assert!(base_url.contains("10.0.0.2"));
                assert!(base_url.contains("50051"));
            }
            other => panic!("expected WillFetch, got {other:?}"),
        }
    }

    #[test]
    fn process_resolved_skips_already_known() {
        let svc = build_resolved(
            "bob.local.",
            "bob",
            50051,
            "10.0.0.2".parse().unwrap(),
            &[("instance_id", "PEER-B"), ("name", "bob")],
        );
        let known = RefCell::new(true);
        let has = |_: &str| *known.borrow();
        let d = process_resolved(&svc, "MY-OWN-ID", &has);
        assert_eq!(d, DiscoveryDecision::Already);
    }

    #[test]
    fn process_resolved_skips_when_instance_id_missing() {
        let svc = build_resolved(
            "carol.local.",
            "carol",
            50051,
            "10.0.0.3".parse().unwrap(),
            &[("name", "carol")],
        );
        let d = process_resolved(&svc, "MY-OWN-ID", &|_| false);
        assert!(matches!(d, DiscoveryDecision::Skipped(_)));
    }

    #[test]
    fn process_resolved_falls_back_to_fullname_when_no_name_txt() {
        let svc = build_resolved(
            "host.local.",
            "no-name-txt",
            50051,
            "10.0.0.4".parse().unwrap(),
            &[("instance_id", "PEER-X")],
        );
        let d = process_resolved(&svc, "MY-OWN-ID", &|_| false);
        match d {
            DiscoveryDecision::WillFetch { name, .. } => {
                assert!(!name.is_empty());
            }
            other => panic!("expected WillFetch, got {other:?}"),
        }
    }

    #[test]
    fn hostname_fallback_returns_non_empty_string() {
        let h = hostname_or_fallback();
        assert!(!h.is_empty());
    }

    #[test]
    fn local_host_label_always_ends_with_local_dot() {
        let h = local_host_label();
        assert!(h.ends_with(".local."), "got: {h}");
    }
}
