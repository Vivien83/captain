//! Pure agent routing decisions for inbound channel dispatch.

use crate::router::AgentRouter;
use crate::types::ChannelMessage;
use captain_types::agent::AgentId;

pub(crate) fn resolve_inbound_agent(
    router: &AgentRouter,
    message: &ChannelMessage,
    thread_id: Option<&str>,
    topic_agent: Option<AgentId>,
    mention_override: Option<AgentId>,
) -> Option<AgentId> {
    topic_agent.or(mention_override).or_else(|| {
        if thread_id.is_some() {
            // In a forum topic without mapping: skip user_default, use channel default only.
            // This prevents one topic's agent from "leaking" to all topics via user_default.
            router.resolve_channel_default(&message.channel)
        } else {
            router.resolve(
                &message.channel,
                &message.sender.platform_id,
                message.sender.captain_user.as_deref(),
            )
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelContent, ChannelType, ChannelUser};

    fn message(platform_id: &str, captain_user: Option<&str>) -> ChannelMessage {
        ChannelMessage {
            channel: ChannelType::Telegram,
            platform_message_id: "msg-1".to_string(),
            sender: ChannelUser {
                platform_id: platform_id.to_string(),
                display_name: "User".to_string(),
                captain_user: captain_user.map(str::to_string),
            },
            content: ChannelContent::Text("hello".to_string()),
            target_agent: None,
            timestamp: chrono::Utc::now(),
            is_group: false,
            thread_id: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn topic_agent_wins_over_mention_and_defaults() {
        let mut router = AgentRouter::new();
        let topic = AgentId::new();
        let mention = AgentId::new();
        router.set_default(AgentId::new());

        let resolved = resolve_inbound_agent(
            &router,
            &message("user-1", None),
            Some("42"),
            Some(topic),
            Some(mention),
        );

        assert_eq!(resolved, Some(topic));
    }

    #[test]
    fn mention_override_wins_when_topic_is_absent() {
        let mut router = AgentRouter::new();
        let mention = AgentId::new();
        router.set_default(AgentId::new());

        let resolved =
            resolve_inbound_agent(&router, &message("user-1", None), None, None, Some(mention));

        assert_eq!(resolved, Some(mention));
    }

    #[test]
    fn threaded_message_without_topic_skips_user_default() {
        let router = AgentRouter::new();
        let user_agent = AgentId::new();
        let channel_agent = AgentId::new();
        router.set_user_default("captain-user".to_string(), user_agent);
        router.set_channel_default("Telegram".to_string(), channel_agent);

        let resolved = resolve_inbound_agent(
            &router,
            &message("platform-user", Some("captain-user")),
            Some("42"),
            None,
            None,
        );

        assert_eq!(resolved, Some(channel_agent));
    }

    #[test]
    fn unthreaded_message_uses_full_router_resolution() {
        let router = AgentRouter::new();
        let user_agent = AgentId::new();
        let channel_agent = AgentId::new();
        router.set_user_default("captain-user".to_string(), user_agent);
        router.set_channel_default("Telegram".to_string(), channel_agent);

        let resolved = resolve_inbound_agent(
            &router,
            &message("platform-user", Some("captain-user")),
            None,
            None,
            None,
        );

        assert_eq!(resolved, Some(user_agent));
    }
}
