//! Final execution of an inbound agent dispatch.

use super::inbound_agent_result::{
    handle_inbound_agent_turn_result, InboundAgentTurnResultContext,
};
use super::inbound_agent_turn::run_inbound_agent_turn;
use super::ChannelBridgeHandle;
use crate::router::AgentRouter;
use crate::types::{ChannelAdapter, ChannelMessage};
use captain_types::agent::AgentId;
use captain_types::config::OutputFormat;
use captain_types::message::ContentBlock;
use std::sync::Arc;

pub(super) struct InboundAgentDispatchContext<'a> {
    pub(super) handle: &'a Arc<dyn ChannelBridgeHandle>,
    pub(super) router: &'a Arc<AgentRouter>,
    pub(super) message: &'a ChannelMessage,
    pub(super) adapter: &'a dyn ChannelAdapter,
    pub(super) adapter_arc: &'a Arc<dyn ChannelAdapter>,
    pub(super) agent_id: AgentId,
    pub(super) image_blocks_for_agent: Option<&'a [ContentBlock]>,
    pub(super) text: &'a str,
    pub(super) text_for_agent: &'a str,
    pub(super) active_session_key: Option<&'a str>,
    pub(super) channel_type: &'a str,
    pub(super) thread_id: Option<&'a str>,
    pub(super) output_format: OutputFormat,
    pub(super) lifecycle_reactions: bool,
}

pub(super) async fn dispatch_inbound_agent_turn(ctx: InboundAgentDispatchContext<'_>) {
    let channel_key = format!("{:?}", ctx.message.channel);

    let turn = run_inbound_agent_turn(
        ctx.handle,
        ctx.adapter,
        ctx.adapter_arc,
        ctx.message,
        ctx.agent_id,
        ctx.image_blocks_for_agent,
        ctx.text_for_agent,
        ctx.channel_type,
        ctx.thread_id,
        ctx.output_format,
        ctx.lifecycle_reactions,
        ctx.active_session_key,
    )
    .await;

    handle_inbound_agent_turn_result(
        InboundAgentTurnResultContext {
            handle: ctx.handle,
            router: ctx.router,
            message: ctx.message,
            adapter: ctx.adapter,
            adapter_arc: ctx.adapter_arc,
            agent_id: ctx.agent_id,
            channel_key: &channel_key,
            channel_type: ctx.channel_type,
            image_blocks_for_agent: ctx.image_blocks_for_agent,
            text: ctx.text,
            lifecycle_reactions: ctx.lifecycle_reactions,
            thread_id: ctx.thread_id,
            output_format: ctx.output_format,
        },
        turn,
    )
    .await;
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

    type DeliveryRecord = (
        AgentId,
        String,
        String,
        bool,
        Option<String>,
        Option<String>,
    );

    struct MockDispatchHandle {
        text_calls: Mutex<Vec<(AgentId, String, Option<String>)>>,
        deliveries: Mutex<Vec<DeliveryRecord>>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for MockDispatchHandle {
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
            Err("not implemented".to_string())
        }

        async fn record_delivery(
            &self,
            agent_id: AgentId,
            channel: &str,
            recipient: &str,
            success: bool,
            error: Option<&str>,
            thread_id: Option<&str>,
        ) {
            self.deliveries.lock().unwrap().push((
                agent_id,
                channel.to_string(),
                recipient.to_string(),
                success,
                error.map(str::to_string),
                thread_id.map(str::to_string),
            ));
        }
    }

    struct RecordingAdapter {
        typing_count: AtomicUsize,
        reactions: Mutex<Vec<(String, AgentPhase)>>,
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

    fn handle() -> Arc<MockDispatchHandle> {
        Arc::new(MockDispatchHandle {
            text_calls: Mutex::new(Vec::new()),
            deliveries: Mutex::new(Vec::new()),
        })
    }

    fn adapter() -> Arc<RecordingAdapter> {
        Arc::new(RecordingAdapter {
            typing_count: AtomicUsize::new(0),
            reactions: Mutex::new(Vec::new()),
            sent: Mutex::new(Vec::new()),
        })
    }

    fn message() -> ChannelMessage {
        ChannelMessage {
            channel: ChannelType::Telegram,
            platform_message_id: "42".to_string(),
            sender: ChannelUser {
                platform_id: "1001".to_string(),
                display_name: "Ada".to_string(),
                captain_user: None,
            },
            content: ChannelContent::Text("hello".to_string()),
            target_agent: None,
            timestamp: chrono::Utc::now(),
            is_group: false,
            thread_id: Some("topic-7".to_string()),
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn dispatch_turn_runs_agent_and_completes_success() {
        let mock_handle = handle();
        let handle: Arc<dyn ChannelBridgeHandle> = mock_handle.clone();
        let router = Arc::new(AgentRouter::new());
        let adapter = adapter();
        let adapter_arc: Arc<dyn ChannelAdapter> = adapter.clone();
        let agent_id = AgentId::new();
        let message = message();

        dispatch_inbound_agent_turn(InboundAgentDispatchContext {
            handle: &handle,
            router: &router,
            message: &message,
            adapter: adapter.as_ref(),
            adapter_arc: &adapter_arc,
            agent_id,
            image_blocks_for_agent: None,
            text: "original text",
            text_for_agent: "agent text",
            active_session_key: Some("session-key"),
            channel_type: "telegram",
            thread_id: Some("topic-7"),
            output_format: OutputFormat::TelegramHtml,
            lifecycle_reactions: true,
        })
        .await;

        assert_eq!(adapter.typing_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            adapter.reactions.lock().unwrap().as_slice(),
            &[
                ("42".to_string(), AgentPhase::Queued),
                ("42".to_string(), AgentPhase::Thinking),
                ("42".to_string(), AgentPhase::Done)
            ]
        );
        assert_eq!(
            mock_handle.text_calls.lock().unwrap().as_slice(),
            &[(
                agent_id,
                "agent text".to_string(),
                Some("telegram".to_string())
            )]
        );
        assert_eq!(
            adapter.sent.lock().unwrap().as_slice(),
            &[("agent:agent text".to_string(), Some("topic-7".to_string()))]
        );
        assert_eq!(
            mock_handle.deliveries.lock().unwrap().as_slice(),
            &[(
                agent_id,
                "telegram".to_string(),
                "1001".to_string(),
                true,
                None,
                Some("topic-7".to_string())
            )]
        );
    }
}
