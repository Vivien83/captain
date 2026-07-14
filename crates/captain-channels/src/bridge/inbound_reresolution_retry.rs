//! Re-resolve stale inbound agents and retry the channel turn.

use super::agent_resolution::try_reresolution;
use super::inbound_agent_activity::InboundAgentActivity;
use super::inbound_agent_send::retry_inbound_agent_message;
use super::inbound_retry_result::relay_inbound_retry_result;
use super::ChannelBridgeHandle;
use crate::router::AgentRouter;
use crate::types::{ChannelAdapter, ChannelMessage};
use captain_types::config::OutputFormat;
use captain_types::message::ContentBlock;
use std::sync::Arc;

pub(super) struct InboundReresolutionRetryContext<'a> {
    pub(super) handle: &'a Arc<dyn ChannelBridgeHandle>,
    pub(super) router: &'a Arc<AgentRouter>,
    pub(super) message: &'a ChannelMessage,
    pub(super) adapter: &'a dyn ChannelAdapter,
    pub(super) adapter_arc: &'a Arc<dyn ChannelAdapter>,
    pub(super) channel_key: &'a str,
    pub(super) channel_type: &'a str,
    pub(super) image_blocks_for_agent: Option<&'a [ContentBlock]>,
    pub(super) text: &'a str,
    pub(super) message_id: &'a str,
    pub(super) lifecycle_reactions: bool,
    pub(super) thread_id: Option<&'a str>,
    pub(super) output_format: OutputFormat,
}

pub(super) async fn try_handle_inbound_reresolution_retry(
    ctx: InboundReresolutionRetryContext<'_>,
    initial_error: &str,
) -> bool {
    let Some(new_id) =
        try_reresolution(initial_error, ctx.channel_key, ctx.handle, ctx.router).await
    else {
        return false;
    };

    let retry_activity = InboundAgentActivity::start(
        &ctx.message.channel,
        ctx.adapter,
        ctx.adapter_arc.clone(),
        &ctx.message.sender,
        ctx.thread_id,
        ctx.output_format,
    )
    .await;
    let retry = retry_inbound_agent_message(
        ctx.handle,
        new_id,
        ctx.image_blocks_for_agent,
        ctx.text,
        ctx.channel_type,
    )
    .await;
    retry_activity.stop();
    relay_inbound_retry_result(
        ctx.handle,
        ctx.adapter,
        ctx.message,
        new_id,
        ctx.channel_type,
        retry,
        ctx.message_id,
        ctx.lifecycle_reactions,
        ctx.thread_id,
        ctx.output_format,
    )
    .await;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentPhase, ChannelContent, ChannelType, ChannelUser, LifecycleReaction};
    use async_trait::async_trait;
    use captain_types::agent::AgentId;
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

    struct MockRetryHandle {
        agents: Mutex<Vec<(String, AgentId)>>,
        text_calls: Mutex<Vec<(AgentId, String, Option<String>)>>,
        deliveries: Mutex<Vec<DeliveryRecord>>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for MockRetryHandle {
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
            Ok("retry ok".to_string())
        }

        async fn find_agent_by_name(&self, name: &str) -> Result<Option<AgentId>, String> {
            Ok(self
                .agents
                .lock()
                .unwrap()
                .iter()
                .find(|(agent_name, _)| agent_name == name)
                .map(|(_, id)| *id))
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
        typing_count: AtomicUsize,
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

    fn handle(agents: Vec<(String, AgentId)>) -> Arc<MockRetryHandle> {
        Arc::new(MockRetryHandle {
            agents: Mutex::new(agents),
            text_calls: Mutex::new(Vec::new()),
            deliveries: Mutex::new(Vec::new()),
        })
    }

    fn adapter() -> Arc<RecordingAdapter> {
        Arc::new(RecordingAdapter {
            sent: Mutex::new(Vec::new()),
            reactions: Mutex::new(Vec::new()),
            typing_count: AtomicUsize::new(0),
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
    async fn reresolution_retry_returns_false_without_stored_default_name() {
        let handle = handle(Vec::new());
        let handle_trait: Arc<dyn ChannelBridgeHandle> = handle.clone();
        let router = Arc::new(AgentRouter::new());
        let adapter = adapter();
        let adapter_trait: Arc<dyn ChannelAdapter> = adapter.clone();
        let message = message();

        let handled = try_handle_inbound_reresolution_retry(
            InboundReresolutionRetryContext {
                handle: &handle_trait,
                router: &router,
                message: &message,
                adapter: adapter.as_ref(),
                adapter_arc: &adapter_trait,
                channel_key: "Telegram",
                channel_type: "telegram",
                image_blocks_for_agent: None,
                text: "hello",
                message_id: "msg-42",
                lifecycle_reactions: true,
                thread_id: Some("topic-7"),
                output_format: OutputFormat::PlainText,
            },
            "Agent not found",
        )
        .await;

        assert!(!handled);
        assert_eq!(adapter.typing_count.load(Ordering::SeqCst), 0);
        assert!(handle.text_calls.lock().unwrap().is_empty());
        assert!(handle.deliveries.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn reresolution_retry_relaunches_agent_and_relays_success() {
        let stale_id = AgentId::new();
        let fresh_id = AgentId::new();
        let handle = handle(vec![("captain".to_string(), fresh_id)]);
        let handle_trait: Arc<dyn ChannelBridgeHandle> = handle.clone();
        let router = Arc::new(AgentRouter::new());
        router.set_channel_default_with_name(
            "Telegram".to_string(),
            stale_id,
            "captain".to_string(),
        );
        let adapter = adapter();
        let adapter_trait: Arc<dyn ChannelAdapter> = adapter.clone();
        let message = message();

        let handled = try_handle_inbound_reresolution_retry(
            InboundReresolutionRetryContext {
                handle: &handle_trait,
                router: &router,
                message: &message,
                adapter: adapter.as_ref(),
                adapter_arc: &adapter_trait,
                channel_key: "Telegram",
                channel_type: "telegram",
                image_blocks_for_agent: None,
                text: "hello",
                message_id: "msg-42",
                lifecycle_reactions: true,
                thread_id: Some("topic-7"),
                output_format: OutputFormat::PlainText,
            },
            "Agent not found",
        )
        .await;

        assert!(handled);
        assert_eq!(
            router.resolve_channel_default(&ChannelType::Telegram),
            Some(fresh_id)
        );
        assert_eq!(adapter.typing_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            handle.text_calls.lock().unwrap().as_slice(),
            &[(fresh_id, "hello".to_string(), Some("telegram".to_string()))]
        );
        assert_eq!(
            adapter.reactions.lock().unwrap().as_slice(),
            &[("msg-42".to_string(), AgentPhase::Done)]
        );
        assert_eq!(
            adapter.sent.lock().unwrap().as_slice(),
            &[("retry ok".to_string(), Some("topic-7".to_string()))]
        );
        assert_eq!(
            handle.deliveries.lock().unwrap().as_slice(),
            &[(
                fresh_id,
                "telegram".to_string(),
                "1001".to_string(),
                true,
                None,
                Some("topic-7".to_string())
            )]
        );
    }
}
