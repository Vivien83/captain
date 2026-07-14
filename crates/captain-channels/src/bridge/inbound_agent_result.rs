//! Terminal handling for an initial inbound agent turn.

use super::inbound_agent_turn::InboundAgentTurnOutcome;
use super::inbound_failure_response::complete_inbound_agent_failure;
use super::inbound_reresolution_retry::{
    try_handle_inbound_reresolution_retry, InboundReresolutionRetryContext,
};
use super::inbound_success_response::complete_inbound_agent_success;
use super::ChannelBridgeHandle;
use crate::router::AgentRouter;
use crate::types::{ChannelAdapter, ChannelMessage};
use captain_types::agent::AgentId;
use captain_types::config::OutputFormat;
use captain_types::message::ContentBlock;
use std::sync::Arc;

pub(super) struct InboundAgentTurnResultContext<'a> {
    pub(super) handle: &'a Arc<dyn ChannelBridgeHandle>,
    pub(super) router: &'a Arc<AgentRouter>,
    pub(super) message: &'a ChannelMessage,
    pub(super) adapter: &'a dyn ChannelAdapter,
    pub(super) adapter_arc: &'a Arc<dyn ChannelAdapter>,
    pub(super) agent_id: AgentId,
    pub(super) channel_key: &'a str,
    pub(super) channel_type: &'a str,
    pub(super) image_blocks_for_agent: Option<&'a [ContentBlock]>,
    pub(super) text: &'a str,
    pub(super) lifecycle_reactions: bool,
    pub(super) thread_id: Option<&'a str>,
    pub(super) output_format: OutputFormat,
}

pub(super) async fn handle_inbound_agent_turn_result(
    ctx: InboundAgentTurnResultContext<'_>,
    turn: InboundAgentTurnOutcome,
) {
    let message_id = turn.message_id;
    match turn.result {
        Ok(response) => {
            complete_inbound_agent_success(
                ctx.handle,
                ctx.adapter,
                ctx.message,
                ctx.agent_id,
                ctx.channel_type,
                response,
                turn.posted_inline,
                &message_id,
                ctx.lifecycle_reactions,
                ctx.thread_id,
                ctx.output_format,
            )
            .await;
        }
        Err(error) => {
            if try_handle_inbound_reresolution_retry(
                InboundReresolutionRetryContext {
                    handle: ctx.handle,
                    router: ctx.router,
                    message: ctx.message,
                    adapter: ctx.adapter,
                    adapter_arc: ctx.adapter_arc,
                    channel_key: ctx.channel_key,
                    channel_type: ctx.channel_type,
                    image_blocks_for_agent: ctx.image_blocks_for_agent,
                    text: ctx.text,
                    message_id: &message_id,
                    lifecycle_reactions: ctx.lifecycle_reactions,
                    thread_id: ctx.thread_id,
                    output_format: ctx.output_format,
                },
                &error,
            )
            .await
            {
                return;
            }

            complete_inbound_agent_failure(
                ctx.handle,
                ctx.adapter,
                ctx.message,
                ctx.agent_id,
                ctx.channel_type,
                &error,
                "Agent error",
                &message_id,
                ctx.lifecycle_reactions,
                ctx.thread_id,
                ctx.output_format,
            )
            .await;
        }
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
    use std::sync::Mutex;

    type DeliveryRecord = (
        AgentId,
        String,
        String,
        bool,
        Option<String>,
        Option<String>,
    );

    struct MockResultHandle {
        deliveries: Mutex<Vec<DeliveryRecord>>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for MockResultHandle {
        async fn send_message(
            &self,
            _agent_id: AgentId,
            _message: &str,
            _channel_type: Option<&str>,
        ) -> Result<String, String> {
            Ok(String::new())
        }

        async fn find_agent_by_name(&self, _name: &str) -> Result<Option<AgentId>, String> {
            Ok(None)
        }

        async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
            Ok(Vec::new())
        }

        async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
            Err("not available".to_string())
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
        sent: Mutex<Vec<(String, Option<String>)>>,
        reactions: Mutex<Vec<(String, AgentPhase)>>,
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

    fn handle() -> Arc<MockResultHandle> {
        Arc::new(MockResultHandle {
            deliveries: Mutex::new(Vec::new()),
        })
    }

    fn adapter() -> Arc<RecordingAdapter> {
        Arc::new(RecordingAdapter {
            sent: Mutex::new(Vec::new()),
            reactions: Mutex::new(Vec::new()),
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

    fn context<'a>(
        handle: &'a Arc<dyn ChannelBridgeHandle>,
        router: &'a Arc<AgentRouter>,
        message: &'a ChannelMessage,
        adapter: &'a dyn ChannelAdapter,
        adapter_arc: &'a Arc<dyn ChannelAdapter>,
        agent_id: AgentId,
    ) -> InboundAgentTurnResultContext<'a> {
        InboundAgentTurnResultContext {
            handle,
            router,
            message,
            adapter,
            adapter_arc,
            agent_id,
            channel_key: "Telegram",
            channel_type: "telegram",
            image_blocks_for_agent: None,
            text: "hello",
            lifecycle_reactions: true,
            thread_id: Some("topic-7"),
            output_format: OutputFormat::PlainText,
        }
    }

    #[tokio::test]
    async fn turn_result_success_completes_delivery() {
        let mock_handle = handle();
        let handle: Arc<dyn ChannelBridgeHandle> = mock_handle.clone();
        let router = Arc::new(AgentRouter::new());
        let adapter = adapter();
        let adapter_arc: Arc<dyn ChannelAdapter> = adapter.clone();
        let agent_id = AgentId::new();
        let message = message();

        handle_inbound_agent_turn_result(
            context(
                &handle,
                &router,
                &message,
                adapter.as_ref(),
                &adapter_arc,
                agent_id,
            ),
            InboundAgentTurnOutcome {
                message_id: "msg-42".to_string(),
                posted_inline: false,
                result: Ok("done".to_string()),
            },
        )
        .await;

        assert_eq!(
            adapter.reactions.lock().unwrap().as_slice(),
            &[("msg-42".to_string(), AgentPhase::Done)]
        );
        assert_eq!(
            adapter.sent.lock().unwrap().as_slice(),
            &[("done".to_string(), Some("topic-7".to_string()))]
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

    #[tokio::test]
    async fn turn_result_failure_without_reresolution_completes_failure() {
        let mock_handle = handle();
        let handle: Arc<dyn ChannelBridgeHandle> = mock_handle.clone();
        let router = Arc::new(AgentRouter::new());
        let adapter = adapter();
        let adapter_arc: Arc<dyn ChannelAdapter> = adapter.clone();
        let agent_id = AgentId::new();
        let message = message();

        handle_inbound_agent_turn_result(
            context(
                &handle,
                &router,
                &message,
                adapter.as_ref(),
                &adapter_arc,
                agent_id,
            ),
            InboundAgentTurnOutcome {
                message_id: "msg-42".to_string(),
                posted_inline: false,
                result: Err("invalid x-goog-api-key".to_string()),
            },
        )
        .await;

        assert_eq!(
            adapter.reactions.lock().unwrap().as_slice(),
            &[("msg-42".to_string(), AgentPhase::Error)]
        );
        assert_eq!(
            adapter.sent.lock().unwrap().as_slice(),
            &[(
                "Service temporarily unavailable.".to_string(),
                Some("topic-7".to_string())
            )]
        );
        assert_eq!(
            mock_handle.deliveries.lock().unwrap().as_slice(),
            &[(
                agent_id,
                "telegram".to_string(),
                "1001".to_string(),
                false,
                Some("Service temporarily unavailable.".to_string()),
                Some("topic-7".to_string())
            )]
        );
    }
}
