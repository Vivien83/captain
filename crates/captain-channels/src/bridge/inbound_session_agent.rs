//! Active-agent bookkeeping for inbound channel sessions.

use crate::inbound_queue::InboundSessionQueue;
use captain_types::agent::AgentId;

pub(super) fn mark_inbound_session_agent<'a>(
    active_session: Option<(&InboundSessionQueue, &'a str)>,
    agent_id: AgentId,
) -> Option<&'a str> {
    let (sessions, session_key) = active_session?;
    sessions.set_active_agent(session_key, agent_id);
    Some(session_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbound_queue_types::InboundStart;
    use crate::types::{ChannelContent, ChannelMessage, ChannelType, ChannelUser};
    use chrono::Utc;
    use std::collections::HashMap;

    fn message() -> ChannelMessage {
        ChannelMessage {
            channel: ChannelType::Telegram,
            platform_message_id: "m1".to_string(),
            sender: ChannelUser {
                platform_id: "chat-1".to_string(),
                display_name: "Alex".to_string(),
                captain_user: None,
            },
            content: ChannelContent::Text("hello".to_string()),
            target_agent: None,
            timestamp: Utc::now(),
            is_group: false,
            thread_id: Some("topic-1".to_string()),
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn marks_active_session_agent_and_returns_key() {
        let sessions = InboundSessionQueue::default();
        let session_key = "telegram|chat:chat-1|user:user-1|captain:-|thread:topic-1".to_string();
        let agent_id = AgentId::new();

        assert!(matches!(
            sessions.start_or_queue(session_key.clone(), message()),
            InboundStart::Started { .. }
        ));

        let returned_key = mark_inbound_session_agent(Some((&sessions, &session_key)), agent_id);

        assert_eq!(returned_key, Some(session_key.as_str()));
        assert_eq!(sessions.active_agent(&session_key), Some(agent_id));
    }

    #[test]
    fn skips_when_no_active_session() {
        let agent_id = AgentId::new();

        assert_eq!(mark_inbound_session_agent(None, agent_id), None);
    }
}
