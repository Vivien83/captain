use super::CaptainKernel;
use std::sync::Arc;
use tracing::info;

impl CaptainKernel {
    pub(super) fn log_network_status_from_config(&self) {
        if self.config.network_enabled {
            info!("OFP network enabled — peer discovery will use shared_secret from config");
        }
    }

    pub(super) fn spawn_a2a_discovery_if_configured(self: &Arc<Self>) {
        let Some(a2a_config) = self.config.a2a.as_ref() else {
            return;
        };
        if !a2a_discovery_should_start(a2a_config.enabled, a2a_config.external_agents.len()) {
            return;
        }

        let kernel = Arc::clone(self);
        let agents = a2a_config.external_agents.clone();
        tokio::spawn(async move {
            let discovered = captain_runtime::a2a::discover_external_agents(&agents).await;
            if let Ok(mut store) = kernel.a2a_external_agents.lock() {
                *store = discovered;
            }
        });
    }

    pub(super) fn spawn_whatsapp_gateway_if_configured(self: &Arc<Self>) {
        if !whatsapp_gateway_should_start(self.config.channels.whatsapp.is_some()) {
            return;
        }

        let kernel = Arc::clone(self);
        tokio::spawn(async move {
            crate::whatsapp_gateway::start_whatsapp_gateway(&kernel).await;
        });
    }
}

fn a2a_discovery_should_start(enabled: bool, external_agents_count: usize) -> bool {
    enabled && external_agents_count > 0
}

fn whatsapp_gateway_should_start(configured: bool) -> bool {
    configured
}

#[cfg(test)]
mod tests {
    use super::{a2a_discovery_should_start, whatsapp_gateway_should_start};

    #[test]
    fn a2a_discovery_requires_enabled_config_and_external_agents() {
        assert!(a2a_discovery_should_start(true, 1));
        assert!(!a2a_discovery_should_start(false, 1));
        assert!(!a2a_discovery_should_start(true, 0));
    }

    #[test]
    fn whatsapp_gateway_starts_only_when_configured() {
        assert!(whatsapp_gateway_should_start(true));
        assert!(!whatsapp_gateway_should_start(false));
    }
}
