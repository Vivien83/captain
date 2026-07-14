//! Pending Telegram model-switch choices for channel callbacks.

use captain_types::agent::AgentId;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long an unconfirmed Telegram model-switch plan stays in the bridge
/// cache before it is dropped. After this window the user has to relaunch
/// `/model <name>` to get a fresh plan.
const PENDING_MODEL_SWITCH_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub(super) struct PendingModelSwitch {
    pub(super) agent_id: AgentId,
    pub(super) target_model: String,
    pub(super) target_provider: Option<String>,
    pub(super) created_at: Instant,
}

impl PendingModelSwitch {
    pub(super) fn is_expired(&self) -> bool {
        self.created_at.elapsed() >= PENDING_MODEL_SWITCH_TTL
    }
}

pub(super) type PendingModelSwitchStore = Arc<DashMap<String, PendingModelSwitch>>;

pub(super) fn remember_pending_model_switch(
    pending_model_switches: &PendingModelSwitchStore,
    plan_id: String,
    pending: PendingModelSwitch,
) {
    // Last-wins concurrency: if the same agent issues two `/model X` commands
    // before clicking, the newer plan invalidates older buttons for that agent.
    // Opportunistic GC: also drop any expired entry we run across so the map
    // doesn't grow unbounded for agents that never click again.
    let stale_keys: Vec<String> = pending_model_switches
        .iter()
        .filter(|entry| entry.agent_id == pending.agent_id || entry.is_expired())
        .map(|entry| entry.key().clone())
        .collect();
    for key in stale_keys {
        pending_model_switches.remove(&key);
    }
    pending_model_switches.insert(plan_id, pending);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_pending_model_switches() -> PendingModelSwitchStore {
        Arc::new(DashMap::new())
    }

    #[test]
    fn pending_model_switch_expires_after_ttl() {
        let agent_id = AgentId::new();
        let old = Instant::now()
            .checked_sub(PENDING_MODEL_SWITCH_TTL + Duration::from_secs(1))
            .expect("instant must support sub for the TTL window");
        let stale = PendingModelSwitch {
            agent_id,
            target_model: "anthropic/claude-haiku-4-5".to_string(),
            target_provider: None,
            created_at: old,
        };
        assert!(stale.is_expired());

        let fresh = PendingModelSwitch {
            agent_id,
            target_model: "anthropic/claude-sonnet-4-6".to_string(),
            target_provider: None,
            created_at: Instant::now(),
        };
        assert!(!fresh.is_expired());
    }

    #[test]
    fn remember_pending_model_switch_drops_expired_entries() {
        let store = test_pending_model_switches();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();
        let old = Instant::now()
            .checked_sub(PENDING_MODEL_SWITCH_TTL + Duration::from_secs(1))
            .expect("instant must support sub for the TTL window");

        store.insert(
            "stale-other-agent".to_string(),
            PendingModelSwitch {
                agent_id: agent_b,
                target_model: "x".to_string(),
                target_provider: None,
                created_at: old,
            },
        );
        store.insert(
            "stale-same-agent".to_string(),
            PendingModelSwitch {
                agent_id: agent_a,
                target_model: "y".to_string(),
                target_provider: None,
                created_at: Instant::now(),
            },
        );
        assert_eq!(store.len(), 2);

        remember_pending_model_switch(
            &store,
            "fresh".to_string(),
            PendingModelSwitch {
                agent_id: agent_a,
                target_model: "z".to_string(),
                target_provider: None,
                created_at: Instant::now(),
            },
        );

        assert_eq!(store.len(), 1, "stale + same-agent entries removed");
        assert!(store.contains_key("fresh"));
        assert!(!store.contains_key("stale-other-agent"));
        assert!(!store.contains_key("stale-same-agent"));
    }
}
