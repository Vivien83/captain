//! Initial inbound agent turn orchestration.

use super::inbound_agent_activity::InboundAgentActivity;
use super::inbound_agent_send::send_inbound_agent_message;
use super::inbound_lifecycle::send_inbound_lifecycle_started;
use super::ChannelBridgeHandle;
use crate::types::{ChannelAdapter, ChannelMessage};
use captain_types::agent::AgentId;
use captain_types::config::OutputFormat;
use captain_types::message::ContentBlock;
use std::sync::Arc;

pub(super) struct InboundAgentTurnOutcome {
    pub(super) message_id: String,
    pub(super) posted_inline: bool,
    pub(super) result: Result<String, String>,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_inbound_agent_turn(
    handle: &Arc<dyn ChannelBridgeHandle>,
    adapter: &dyn ChannelAdapter,
    adapter_arc: &Arc<dyn ChannelAdapter>,
    message: &ChannelMessage,
    agent_id: AgentId,
    image_blocks_for_agent: Option<&[ContentBlock]>,
    final_text: &str,
    channel_type: &str,
    thread_id: Option<&str>,
    output_format: OutputFormat,
    lifecycle_reactions: bool,
    active_session_key: Option<&str>,
) -> InboundAgentTurnOutcome {
    let message_id = message.platform_message_id.clone();
    send_inbound_lifecycle_started(adapter, &message.sender, &message_id, lifecycle_reactions)
        .await;

    let activity = InboundAgentActivity::start(
        &message.channel,
        adapter,
        adapter_arc.clone(),
        &message.sender,
        thread_id,
        output_format,
    )
    .await;

    let send_outcome = send_inbound_agent_message(
        handle,
        adapter_arc,
        message,
        agent_id,
        image_blocks_for_agent,
        final_text,
        channel_type,
        thread_id,
        active_session_key,
    )
    .await;

    activity.stop();

    InboundAgentTurnOutcome {
        message_id,
        posted_inline: send_outcome.posted_inline,
        result: send_outcome.result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        AgentPhase, ChannelContent, ChannelMessage, ChannelType, ChannelUser, LifecycleReaction,
    };
    use async_trait::async_trait;
    use futures::{stream, Stream};
    use std::collections::HashMap;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    struct MockTurnHandle {
        text_calls: Mutex<Vec<(AgentId, String, Option<String>)>>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for MockTurnHandle {
        async fn send_message(
            &self,
            agent_id: AgentId,
            message: &str,
            channel_type: Option<&str>,
        ) -> Result<String, String> {
            self.text_calls.lock().unwrap().push((
                agent_id,
                message.to_string(),
                channel_type.map(str::to_string),
            ));
            Ok(format!("agent:{message}"))
        }

        async fn find_agent_by_name(&self, _name: &str) -> Result<Option<AgentId>, String> {
            Ok(None)
        }

        async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
            Ok(Vec::new())
        }

        async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
            Ok(AgentId::new())
        }
    }

    struct RecordingAdapter {
        typing_count: AtomicUsize,
        reactions: Mutex<Vec<(String, AgentPhase)>>,
    }

    impl RecordingAdapter {
        fn new() -> Self {
            Self {
                typing_count: AtomicUsize::new(0),
                reactions: Mutex::new(Vec::new()),
            }
        }

        fn typing_count(&self) -> usize {
            self.typing_count.load(Ordering::SeqCst)
        }

        fn reactions(&self) -> Vec<(String, AgentPhase)> {
            self.reactions.lock().unwrap().clone()
        }
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
            _content: ChannelContent,
        ) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }

        async fn send_typing(&self, _user: &ChannelUser) -> Result<(), Box<dyn std::error::Error>> {
            self.typing_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn send_reaction(
            &self,
            _user: &ChannelUser,
            message_id: &str,
            reaction: &LifecycleReaction,
        ) -> Result<(), Box<dyn std::error::Error>> {
            self.reactions
                .lock()
                .unwrap()
                .push((message_id.to_string(), reaction.phase.clone()));
            Ok(())
        }

        async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }
    }

    fn message() -> ChannelMessage {
        ChannelMessage {
            channel: ChannelType::Telegram,
            platform_message_id: "42".to_string(),
            sender: ChannelUser {
                platform_id: "chat-1".to_string(),
                display_name: "Alex".to_string(),
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
    async fn turn_starts_lifecycle_activity_and_sends_text() {
        let handle = Arc::new(MockTurnHandle {
            text_calls: Mutex::new(Vec::new()),
        });
        let handle_arc: Arc<dyn ChannelBridgeHandle> = handle.clone();
        let adapter = Arc::new(RecordingAdapter::new());
        let adapter_arc: Arc<dyn ChannelAdapter> = adapter.clone();
        let agent_id = AgentId::new();
        let message = message();

        let outcome = run_inbound_agent_turn(
            &handle_arc,
            adapter.as_ref(),
            &adapter_arc,
            &message,
            agent_id,
            None,
            "hello",
            "telegram",
            message.thread_id.as_deref(),
            OutputFormat::PlainText,
            true,
            Some("session-key"),
        )
        .await;

        assert_eq!(outcome.message_id, "42");
        assert!(!outcome.posted_inline);
        assert_eq!(outcome.result, Ok("agent:hello".to_string()));
        assert_eq!(adapter.typing_count(), 1);
        assert_eq!(
            adapter.reactions(),
            vec![
                ("42".to_string(), AgentPhase::Queued),
                ("42".to_string(), AgentPhase::Thinking),
            ]
        );
        assert_eq!(
            handle.text_calls.lock().unwrap().as_slice(),
            &[(agent_id, "hello".to_string(), Some("telegram".to_string()))]
        );
    }
}
