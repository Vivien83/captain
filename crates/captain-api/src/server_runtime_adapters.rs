use crate::channel_bridge;
use crate::routes::AppState;
use captain_kernel::CaptainKernel;
use std::sync::Arc;
use tracing::{info, warn};

pub(crate) fn spawn_peer_discovery(kernel: Arc<CaptainKernel>, api_port: u16) {
    let ops: Arc<dyn captain_runtime::peer_discovery::PeerDiscoveryOps> =
        Arc::new(KernelPeerOps { kernel, api_port });

    match captain_runtime::peer_discovery::spawn_peer_discovery(ops) {
        Ok(handle) => {
            // Keep the handle alive for the daemon lifetime.
            std::mem::forget(handle);
        }
        Err(e) => warn!("Peer discovery init failed: {e} - federation disabled"),
    }
}

struct KernelPeerOps {
    kernel: Arc<CaptainKernel>,
    api_port: u16,
}

#[async_trait::async_trait]
impl captain_runtime::peer_discovery::PeerDiscoveryOps for KernelPeerOps {
    fn instance_id(&self) -> String {
        self.kernel.instance_id.clone()
    }

    fn api_listen_port(&self) -> u16 {
        self.api_port
    }

    fn instance_name(&self) -> String {
        std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .ok()
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| format!("captain-{}", &self.kernel.instance_id[..8]))
    }

    fn has_external_agent(&self, name: &str) -> bool {
        let store = match self.kernel.a2a_external_agents.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        store.iter().any(|(n, _)| n == name)
    }

    async fn add_external_agent(&self, name: &str, base_url: &str) -> Result<bool, String> {
        if self.has_external_agent(name) {
            return Ok(false);
        }
        let client = captain_runtime::a2a::A2aClient::new();
        let card = client
            .discover(base_url)
            .await
            .map_err(|e| format!("a2a discover {base_url}: {e}"))?;
        let mut store = self
            .kernel
            .a2a_external_agents
            .lock()
            .map_err(|e| format!("a2a store poisoned: {e}"))?;
        if store.iter().any(|(n, _)| n == name) {
            return Ok(false);
        }
        store.push((name.to_string(), card));
        Ok(true)
    }
}

pub(crate) fn spawn_integration_hot_reload(state: Arc<AppState>) {
    use captain_types::event::{EventPayload, SystemEvent};

    let mut rx = state.kernel.event_bus.subscribe_all();
    let kernel = state.kernel.clone();
    let bridge_mutex = Arc::new(tokio::sync::Mutex::new(()));

    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let name = match event.payload {
                        EventPayload::System(SystemEvent::IntegrationConfigured { name }) => name,
                        _ => continue,
                    };
                    info!(integration = %name, "Hot-reload triggered");

                    let _g = bridge_mutex.lock().await;

                    channel_bridge::reload_secrets_into_env(&kernel.config.home_dir);

                    if let Err(e) = kernel.reload_config() {
                        warn!("reload_config failed during hot-reload: {e}");
                    }

                    let config_path = kernel.config.home_dir.join("config.toml");
                    let fresh = if config_path.exists() {
                        captain_kernel::config::load_config(Some(&config_path))
                    } else {
                        warn!("config.toml missing during hot-reload");
                        continue;
                    };

                    {
                        let mut guard = state.bridge_manager.lock().await;
                        if let Some(b) = guard.as_mut() {
                            b.stop().await;
                        }
                        let (new_bridge, names) = channel_bridge::start_channel_bridge_with_config(
                            kernel.clone(),
                            &fresh.channels,
                        )
                        .await;
                        *guard = new_bridge;
                        info!(
                            integration = %name,
                            adapters = ?names,
                            "Channel bridge re-spawned"
                        );
                    }
                    *state.channels_config.write().await = fresh.channels;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    warn!(
                        skipped,
                        "Integration hot-reload listener lagged on event bus"
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
