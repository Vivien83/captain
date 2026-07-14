//! Text mention parsing for inbound channel routing.

use super::ChannelBridgeHandle;
use captain_types::agent::AgentId;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MentionOverride<'a> {
    pub(super) agent_name: &'a str,
    pub(super) text_for_agent: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InboundMentionResolution {
    pub(super) text_for_agent: String,
    pub(super) agent_override: Option<AgentId>,
}

pub(super) async fn resolve_inbound_mention_override(
    handle: &Arc<dyn ChannelBridgeHandle>,
    text: &str,
) -> InboundMentionResolution {
    let Some(mention) = parse_mention_override(text) else {
        return InboundMentionResolution {
            text_for_agent: text.to_string(),
            agent_override: None,
        };
    };

    match handle.find_agent_by_name(mention.agent_name).await {
        Ok(Some(id)) => InboundMentionResolution {
            text_for_agent: mention.text_for_agent.to_string(),
            agent_override: Some(id),
        },
        _ => InboundMentionResolution {
            text_for_agent: text.to_string(),
            agent_override: None,
        },
    }
}

pub(super) fn parse_mention_override(text: &str) -> Option<MentionOverride<'_>> {
    if !text.starts_with('@') {
        return None;
    }
    let space_pos = text.find(' ')?;
    Some(MentionOverride {
        agent_name: &text[1..space_pos],
        text_for_agent: &text[space_pos + 1..],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct MockMentionHandle {
        agents: Mutex<Vec<(AgentId, String)>>,
        lookups: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for MockMentionHandle {
        async fn send_message(
            &self,
            _agent_id: AgentId,
            message: &str,
            _channel_type: Option<&str>,
        ) -> Result<String, String> {
            Ok(message.to_string())
        }

        async fn find_agent_by_name(&self, name: &str) -> Result<Option<AgentId>, String> {
            self.lookups.lock().unwrap().push(name.to_string());
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
            Err("not implemented".to_string())
        }
    }

    fn handle(agents: Vec<(AgentId, String)>) -> Arc<MockMentionHandle> {
        Arc::new(MockMentionHandle {
            agents: Mutex::new(agents),
            lookups: Mutex::new(Vec::new()),
        })
    }

    #[test]
    fn mention_override_extracts_agent_and_message() {
        assert_eq!(
            parse_mention_override("@coder fix this"),
            Some(MentionOverride {
                agent_name: "coder",
                text_for_agent: "fix this"
            })
        );
    }

    #[test]
    fn mention_override_preserves_text_after_first_space() {
        assert_eq!(
            parse_mention_override("@coder  keep spacing"),
            Some(MentionOverride {
                agent_name: "coder",
                text_for_agent: " keep spacing"
            })
        );
    }

    #[test]
    fn mention_override_ignores_non_mentions_or_incomplete_mentions() {
        assert_eq!(parse_mention_override(" @coder fix this"), None);
        assert_eq!(parse_mention_override("@coder"), None);
        assert_eq!(parse_mention_override("hello @coder"), None);
    }

    #[tokio::test]
    async fn mention_resolution_routes_to_known_agent_and_strips_prefix() {
        let agent_id = AgentId::new();
        let handle = handle(vec![(agent_id, "coder".to_string())]);
        let handle_trait: Arc<dyn ChannelBridgeHandle> = handle.clone();

        let resolved = resolve_inbound_mention_override(&handle_trait, "@coder fix this").await;

        assert_eq!(
            resolved,
            InboundMentionResolution {
                text_for_agent: "fix this".to_string(),
                agent_override: Some(agent_id),
            }
        );
        assert_eq!(handle.lookups.lock().unwrap().as_slice(), &["coder"]);
    }

    #[tokio::test]
    async fn mention_resolution_keeps_original_text_when_agent_is_unknown() {
        let handle = handle(Vec::new());
        let handle_trait: Arc<dyn ChannelBridgeHandle> = handle.clone();

        let resolved = resolve_inbound_mention_override(&handle_trait, "@coder fix this").await;

        assert_eq!(
            resolved,
            InboundMentionResolution {
                text_for_agent: "@coder fix this".to_string(),
                agent_override: None,
            }
        );
        assert_eq!(handle.lookups.lock().unwrap().as_slice(), &["coder"]);
    }

    #[tokio::test]
    async fn mention_resolution_ignores_plain_text_without_lookup() {
        let handle = handle(Vec::new());
        let handle_trait: Arc<dyn ChannelBridgeHandle> = handle.clone();

        let resolved = resolve_inbound_mention_override(&handle_trait, "hello captain").await;

        assert_eq!(
            resolved,
            InboundMentionResolution {
                text_for_agent: "hello captain".to_string(),
                agent_override: None,
            }
        );
        assert!(handle.lookups.lock().unwrap().is_empty());
    }
}
