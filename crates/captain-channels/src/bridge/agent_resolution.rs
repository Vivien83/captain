//! Agent fallback and stale-ID re-resolution for inbound channel dispatch.

use super::routing::resolve_inbound_agent;
use super::ChannelBridgeHandle;
use crate::router::AgentRouter;
use crate::types::ChannelMessage;
use captain_types::agent::AgentId;
use std::sync::Arc;
use tracing::{info, warn};

pub(crate) const NO_AGENTS_AVAILABLE_MESSAGE: &str =
    "No agents available. Start the dashboard at http://127.0.0.1:4200 to create one.";

pub(crate) async fn resolve_fallback_agent(
    handle: &Arc<dyn ChannelBridgeHandle>,
    router: &Arc<AgentRouter>,
    message: &ChannelMessage,
    thread_id: Option<&str>,
    preferred_name: &str,
) -> Option<AgentId> {
    let fallback = match handle.find_agent_by_name(preferred_name).await {
        Ok(Some(id)) => Some(id),
        _ => handle
            .list_agents()
            .await
            .ok()
            .and_then(|agents| agents.first().map(|(id, _)| *id)),
    };

    if let Some(id) = fallback {
        if thread_id.is_none() {
            router.set_user_default(message.sender.platform_id.clone(), id);
        }
    }
    fallback
}

pub(crate) async fn resolve_inbound_agent_target(
    handle: &Arc<dyn ChannelBridgeHandle>,
    router: &Arc<AgentRouter>,
    message: &ChannelMessage,
    thread_id: Option<&str>,
    mention_override: Option<AgentId>,
    preferred_fallback_name: &str,
) -> Option<AgentId> {
    let topic_agent = if let Some(tid) = thread_id {
        handle.get_agent_for_topic(tid).await
    } else {
        None
    };

    if let Some(agent_id) =
        resolve_inbound_agent(router, message, thread_id, topic_agent, mention_override)
    {
        return Some(agent_id);
    }

    resolve_fallback_agent(handle, router, message, thread_id, preferred_fallback_name).await
}

/// If an error contains "Agent not found", try to re-resolve the channel's
/// default agent by the name stored at bridge startup.
pub(crate) async fn try_reresolution(
    err: &str,
    channel_key: &str,
    handle: &Arc<dyn ChannelBridgeHandle>,
    router: &Arc<AgentRouter>,
) -> Option<AgentId> {
    if !is_agent_not_found_error(err) {
        return None;
    }
    let name = router.channel_default_name(channel_key)?;
    info!(
        channel = channel_key,
        agent_name = %name,
        "Agent not found - attempting re-resolution by name"
    );
    match handle.find_agent_by_name(&name).await {
        Ok(Some(new_id)) => {
            router.update_channel_default(channel_key, new_id);
            info!(
                channel = channel_key,
                agent_name = %name,
                new_id = %new_id,
                "Re-resolved agent successfully"
            );
            Some(new_id)
        }
        _ => {
            warn!(
                channel = channel_key,
                agent_name = %name,
                "Re-resolution failed - agent not found by name"
            );
            None
        }
    }
}

fn is_agent_not_found_error(err: &str) -> bool {
    err.to_ascii_lowercase().contains("agent not found")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelContent, ChannelType, ChannelUser};
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct MockHandle {
        agents: Mutex<Vec<(AgentId, String)>>,
        topic_agents: Mutex<HashMap<String, AgentId>>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for MockHandle {
        async fn send_message(
            &self,
            _agent_id: AgentId,
            message: &str,
            _channel_type: Option<&str>,
        ) -> Result<String, String> {
            Ok(message.to_string())
        }

        async fn find_agent_by_name(&self, name: &str) -> Result<Option<AgentId>, String> {
            Ok(self
                .agents
                .lock()
                .unwrap()
                .iter()
                .find(|(_, agent_name)| agent_name == name)
                .map(|(id, _)| *id))
        }

        async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
            Ok(self.agents.lock().unwrap().clone())
        }

        async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
            Err("spawn not implemented".to_string())
        }

        async fn get_agent_for_topic(&self, thread_id: &str) -> Option<AgentId> {
            self.topic_agents.lock().unwrap().get(thread_id).copied()
        }
    }

    fn mock_handle(agents: Vec<(AgentId, String)>) -> Arc<MockHandle> {
        Arc::new(MockHandle {
            agents: Mutex::new(agents),
            topic_agents: Mutex::new(HashMap::new()),
        })
    }

    fn test_message() -> ChannelMessage {
        ChannelMessage {
            channel: ChannelType::Telegram,
            platform_message_id: "m1".to_string(),
            sender: ChannelUser {
                platform_id: "chat-1".to_string(),
                display_name: "Ada".to_string(),
                captain_user: None,
            },
            content: ChannelContent::Text("hello".to_string()),
            target_agent: None,
            timestamp: chrono::Utc::now(),
            is_group: false,
            thread_id: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    #[tokio::test]
    async fn fallback_prefers_named_agent_and_sets_user_default_outside_topic() {
        let named = AgentId::new();
        let first = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = mock_handle(vec![
            (first, "other".to_string()),
            (named, "captain".to_string()),
        ]);
        let router = Arc::new(AgentRouter::new());
        let message = test_message();

        let resolved = resolve_fallback_agent(&handle, &router, &message, None, "captain").await;

        assert_eq!(resolved, Some(named));
        assert_eq!(
            router.resolve(&ChannelType::Telegram, "chat-1", None),
            Some(named)
        );
    }

    #[tokio::test]
    async fn fallback_uses_first_listed_agent_when_named_agent_is_absent() {
        let first = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> =
            mock_handle(vec![(first, "researcher".to_string())]);
        let router = Arc::new(AgentRouter::new());

        let resolved =
            resolve_fallback_agent(&handle, &router, &test_message(), None, "captain").await;

        assert_eq!(resolved, Some(first));
    }

    #[tokio::test]
    async fn fallback_does_not_set_user_default_for_threaded_messages() {
        let first = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> =
            mock_handle(vec![(first, "captain".to_string())]);
        let router = Arc::new(AgentRouter::new());

        let resolved = resolve_fallback_agent(
            &handle,
            &router,
            &test_message(),
            Some("topic-1"),
            "captain",
        )
        .await;

        assert_eq!(resolved, Some(first));
        assert_eq!(router.resolve(&ChannelType::Telegram, "chat-1", None), None);
    }

    #[tokio::test]
    async fn inbound_target_prefers_topic_agent_over_mention_and_defaults() {
        let topic = AgentId::new();
        let mention = AgentId::new();
        let user_default = AgentId::new();
        let handle = mock_handle(Vec::new());
        handle
            .topic_agents
            .lock()
            .unwrap()
            .insert("topic-1".to_string(), topic);
        let handle_trait: Arc<dyn ChannelBridgeHandle> = handle;
        let router = Arc::new(AgentRouter::new());
        router.set_user_default("chat-1".to_string(), user_default);

        let resolved = resolve_inbound_agent_target(
            &handle_trait,
            &router,
            &test_message(),
            Some("topic-1"),
            Some(mention),
            "captain",
        )
        .await;

        assert_eq!(resolved, Some(topic));
    }

    #[tokio::test]
    async fn inbound_target_falls_back_to_named_agent_when_unrouted() {
        let named = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> =
            mock_handle(vec![(named, "captain".to_string())]);
        let router = Arc::new(AgentRouter::new());
        let message = test_message();

        let resolved =
            resolve_inbound_agent_target(&handle, &router, &message, None, None, "captain").await;

        assert_eq!(resolved, Some(named));
        assert_eq!(
            router.resolve(&ChannelType::Telegram, "chat-1", None),
            Some(named)
        );
    }

    #[tokio::test]
    async fn reresolution_updates_channel_default_by_stored_name() {
        let stale = AgentId::new();
        let fresh = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> =
            mock_handle(vec![(fresh, "captain".to_string())]);
        let router = Arc::new(AgentRouter::new());
        router.set_channel_default_with_name("Telegram".to_string(), stale, "captain".to_string());

        let resolved =
            try_reresolution("Agent not found: stale id", "Telegram", &handle, &router).await;

        assert_eq!(resolved, Some(fresh));
        assert_eq!(
            router.resolve_channel_default(&ChannelType::Telegram),
            Some(fresh)
        );
    }

    #[tokio::test]
    async fn reresolution_ignores_unrelated_errors() {
        let fresh = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> =
            mock_handle(vec![(fresh, "captain".to_string())]);
        let router = Arc::new(AgentRouter::new());
        router.set_channel_default_with_name("Telegram".to_string(), fresh, "captain".to_string());

        let resolved = try_reresolution("rate limit", "Telegram", &handle, &router).await;

        assert_eq!(resolved, None);
    }
}
