//! Agent fallback and stale-ID re-resolution for inbound channel dispatch.

use super::routing::resolve_inbound_agent;
use super::ChannelBridgeHandle;
use crate::router::AgentRouter;
use crate::types::{ChannelMessage, INTERNAL_TARGET_AGENT_NAME_METADATA_KEY};
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
    if let Some(requested_name) = message
        .metadata
        .get(INTERNAL_TARGET_AGENT_NAME_METADATA_KEY)
        .and_then(serde_json::Value::as_str)
        .filter(|name| !name.is_empty())
    {
        match handle.find_agent_by_name(requested_name).await {
            Ok(Some(agent_id)) => return Some(agent_id),
            Ok(None) => warn!(
                agent_name = requested_name,
                "Requested inbound agent no longer exists; using normal routing"
            ),
            Err(error) => warn!(
                agent_name = requested_name,
                %error,
                "Requested inbound agent lookup failed; using normal routing"
            ),
        }
    }

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
    account_id: Option<&str>,
    handle: &Arc<dyn ChannelBridgeHandle>,
    router: &Arc<AgentRouter>,
) -> Option<AgentId> {
    if !is_agent_not_found_error(err) {
        return None;
    }
    let account_name = account_id.and_then(|account| {
        router
            .account_default_name(channel_key, account)
            .map(|name| (account, name))
    });
    let (resolved_account, name) = match account_name {
        Some((account, name)) => (Some(account), name),
        None => (None, router.channel_default_name(channel_key)?),
    };
    info!(
        channel = channel_key,
        account = resolved_account.unwrap_or("-"),
        agent_name = %name,
        "Agent not found - attempting re-resolution by name"
    );
    match handle.find_agent_by_name(&name).await {
        Ok(Some(new_id)) => {
            if let Some(account) = resolved_account {
                router.update_account_default(channel_key, account, new_id);
            } else {
                router.update_channel_default(channel_key, new_id);
            }
            info!(
                channel = channel_key,
                account = resolved_account.unwrap_or("-"),
                agent_name = %name,
                new_id = %new_id,
                "Re-resolved agent successfully"
            );
            Some(new_id)
        }
        _ => {
            warn!(
                channel = channel_key,
                account = resolved_account.unwrap_or("-"),
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
    async fn inbound_target_resolves_reserved_requested_agent_name_first() {
        let requested = AgentId::new();
        let topic = AgentId::new();
        let handle = mock_handle(vec![(requested, "researcher".to_string())]);
        handle
            .topic_agents
            .lock()
            .unwrap()
            .insert("topic-1".to_string(), topic);
        let handle_trait: Arc<dyn ChannelBridgeHandle> = handle;
        let router = Arc::new(AgentRouter::new());
        let mut message = test_message();
        message.metadata.insert(
            INTERNAL_TARGET_AGENT_NAME_METADATA_KEY.to_string(),
            serde_json::json!("researcher"),
        );

        let resolved = resolve_inbound_agent_target(
            &handle_trait,
            &router,
            &message,
            Some("topic-1"),
            None,
            "captain",
        )
        .await;

        assert_eq!(resolved, Some(requested));
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

        let resolved = try_reresolution(
            "Agent not found: stale id",
            "Telegram",
            None,
            &handle,
            &router,
        )
        .await;

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

        let resolved = try_reresolution("rate limit", "Telegram", None, &handle, &router).await;

        assert_eq!(resolved, None);
    }

    #[tokio::test]
    async fn reresolution_refreshes_the_scoped_account_default() {
        let stale = AgentId::new();
        let fresh = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> =
            mock_handle(vec![(fresh, "work-agent".to_string())]);
        let router = Arc::new(AgentRouter::new());
        router.set_account_default_with_name(
            "Email".to_string(),
            "work".to_string(),
            stale,
            "work-agent".to_string(),
        );

        let resolved = try_reresolution(
            "Agent not found: stale id",
            "Email",
            Some("work"),
            &handle,
            &router,
        )
        .await;

        assert_eq!(resolved, Some(fresh));
        assert_eq!(
            router.resolve_channel_default_for_account(&ChannelType::Email, Some("work")),
            Some(fresh)
        );
    }
}
