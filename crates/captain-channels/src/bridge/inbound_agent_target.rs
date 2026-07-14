//! Agent target resolution for inbound channel dispatch.

use super::agent_resolution::{resolve_inbound_agent_target, NO_AGENTS_AVAILABLE_MESSAGE};
use super::command_response::send_response;
use super::inbound_mention::resolve_inbound_mention_override;
use super::ChannelBridgeHandle;
use crate::router::AgentRouter;
use crate::types::{ChannelAdapter, ChannelMessage};
use captain_types::agent::AgentId;
use captain_types::config::OutputFormat;
use std::sync::Arc;

pub(super) struct InboundAgentTargetContext<'a> {
    pub(super) handle: &'a Arc<dyn ChannelBridgeHandle>,
    pub(super) router: &'a Arc<AgentRouter>,
    pub(super) adapter: &'a dyn ChannelAdapter,
    pub(super) message: &'a ChannelMessage,
    pub(super) text: &'a str,
    pub(super) thread_id: Option<&'a str>,
    pub(super) output_format: OutputFormat,
    pub(super) preferred_fallback_name: &'a str,
}

pub(super) struct InboundAgentDispatchTarget {
    pub(super) agent_id: AgentId,
    pub(super) text_for_agent: String,
}

pub(super) async fn resolve_inbound_agent_dispatch_target(
    ctx: InboundAgentTargetContext<'_>,
) -> Option<InboundAgentDispatchTarget> {
    let mention = resolve_inbound_mention_override(ctx.handle, ctx.text).await;

    let Some(agent_id) = resolve_inbound_agent_target(
        ctx.handle,
        ctx.router,
        ctx.message,
        ctx.thread_id,
        mention.agent_override,
        ctx.preferred_fallback_name,
    )
    .await
    else {
        send_response(
            ctx.adapter,
            &ctx.message.sender,
            NO_AGENTS_AVAILABLE_MESSAGE.to_string(),
            ctx.thread_id,
            ctx.output_format,
        )
        .await;
        return None;
    };

    Some(InboundAgentDispatchTarget {
        agent_id,
        text_for_agent: mention.text_for_agent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelContent, ChannelType, ChannelUser};
    use async_trait::async_trait;
    use futures::{stream, Stream};
    use std::collections::HashMap;
    use std::pin::Pin;
    use std::sync::Mutex;

    struct MockTargetHandle {
        agents: Mutex<Vec<(AgentId, String)>>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for MockTargetHandle {
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
            Err("not implemented".to_string())
        }
    }

    struct RecordingAdapter {
        sent: Mutex<Vec<(String, Option<String>)>>,
    }

    #[async_trait]
    impl ChannelAdapter for RecordingAdapter {
        fn name(&self) -> &str {
            "recording"
        }

        fn channel_type(&self) -> ChannelType {
            ChannelType::Telegram
        }

        async fn start(
            &self,
        ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, Box<dyn std::error::Error>>
        {
            Ok(Box::pin(stream::empty()))
        }

        async fn send(
            &self,
            _user: &ChannelUser,
            content: ChannelContent,
        ) -> Result<(), Box<dyn std::error::Error>> {
            if let ChannelContent::Text(text) = content {
                self.sent.lock().unwrap().push((text, None));
            }
            Ok(())
        }

        async fn send_in_thread(
            &self,
            _user: &ChannelUser,
            content: ChannelContent,
            thread_id: &str,
        ) -> Result<(), Box<dyn std::error::Error>> {
            if let ChannelContent::Text(text) = content {
                self.sent
                    .lock()
                    .unwrap()
                    .push((text, Some(thread_id.to_string())));
            }
            Ok(())
        }

        async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }
    }

    fn handle(agents: Vec<(AgentId, String)>) -> Arc<dyn ChannelBridgeHandle> {
        Arc::new(MockTargetHandle {
            agents: Mutex::new(agents),
        })
    }

    fn adapter() -> RecordingAdapter {
        RecordingAdapter {
            sent: Mutex::new(Vec::new()),
        }
    }

    fn message() -> ChannelMessage {
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
            thread_id: Some("topic-1".to_string()),
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn agent_target_routes_known_mention_and_strips_prefix() {
        let agent_id = AgentId::new();
        let handle = handle(vec![(agent_id, "vision".to_string())]);
        let router = Arc::new(AgentRouter::new());
        let adapter = adapter();
        let message = message();

        let target = resolve_inbound_agent_dispatch_target(InboundAgentTargetContext {
            handle: &handle,
            router: &router,
            adapter: &adapter,
            message: &message,
            text: "@vision inspect this",
            thread_id: message.thread_id.as_deref(),
            output_format: OutputFormat::PlainText,
            preferred_fallback_name: "captain",
        })
        .await
        .expect("mention target resolves");

        assert_eq!(target.agent_id, agent_id);
        assert_eq!(target.text_for_agent, "inspect this");
        assert!(adapter.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn agent_target_sends_no_agent_response_when_unresolved() {
        let handle = handle(Vec::new());
        let router = Arc::new(AgentRouter::new());
        let adapter = adapter();
        let message = message();

        let target = resolve_inbound_agent_dispatch_target(InboundAgentTargetContext {
            handle: &handle,
            router: &router,
            adapter: &adapter,
            message: &message,
            text: "hello",
            thread_id: message.thread_id.as_deref(),
            output_format: OutputFormat::PlainText,
            preferred_fallback_name: "captain",
        })
        .await;

        assert!(target.is_none());
        assert_eq!(
            adapter.sent.lock().unwrap().as_slice(),
            &[(
                NO_AGENTS_AVAILABLE_MESSAGE.to_string(),
                Some("topic-1".to_string())
            )]
        );
    }
}
