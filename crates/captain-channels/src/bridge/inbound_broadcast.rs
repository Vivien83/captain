//! Broadcast fan-out for inbound channel messages.

use super::command_response::send_response;
use super::inbound_authorization::authorize_inbound_chat;
use super::progress::spawn_typing_loop;
use super::ChannelBridgeHandle;
use crate::router::AgentRouter;
use crate::types::{ChannelAdapter, ChannelMessage};
use captain_types::agent::AgentId;
use captain_types::config::{BroadcastStrategy, OutputFormat};
use std::sync::Arc;

pub(super) struct InboundBroadcastContext<'a> {
    pub(super) handle: &'a Arc<dyn ChannelBridgeHandle>,
    pub(super) router: &'a Arc<AgentRouter>,
    pub(super) adapter: &'a dyn ChannelAdapter,
    pub(super) adapter_arc: Arc<dyn ChannelAdapter>,
    pub(super) message: &'a ChannelMessage,
    pub(super) sender_user_id: &'a str,
    pub(super) text: &'a str,
    pub(super) channel_type: &'a str,
    pub(super) thread_id: Option<&'a str>,
    pub(super) output_format: OutputFormat,
}

pub(super) async fn try_handle_inbound_broadcast(ctx: InboundBroadcastContext<'_>) -> bool {
    if !ctx.router.has_broadcast(&ctx.message.sender.platform_id) {
        return false;
    }

    let targets = ctx
        .router
        .resolve_broadcast(&ctx.message.sender.platform_id);
    if targets.is_empty() {
        return false;
    }

    // RBAC check applies to broadcast too.
    if !authorize_inbound_chat(
        ctx.handle,
        ctx.adapter,
        &ctx.message.sender,
        ctx.sender_user_id,
        ctx.channel_type,
        ctx.thread_id,
        ctx.output_format,
    )
    .await
    {
        return true;
    }

    let _ = ctx.adapter.send_typing(&ctx.message.sender).await;
    let typing_task = spawn_typing_loop(ctx.adapter_arc.clone(), ctx.message.sender.clone());

    let responses = collect_broadcast_responses(
        ctx.handle,
        &targets,
        ctx.router.broadcast_strategy(),
        ctx.text,
        ctx.channel_type,
    )
    .await;

    typing_task.abort();

    send_response(
        ctx.adapter,
        &ctx.message.sender,
        responses.join("\n\n"),
        ctx.thread_id,
        ctx.output_format,
    )
    .await;
    true
}

pub(super) async fn collect_broadcast_responses(
    handle: &Arc<dyn ChannelBridgeHandle>,
    targets: &[(String, Option<AgentId>)],
    strategy: BroadcastStrategy,
    text: &str,
    channel_type: &str,
) -> Vec<String> {
    match strategy {
        BroadcastStrategy::Parallel => collect_parallel(handle, targets, text).await,
        BroadcastStrategy::Sequential => {
            collect_sequential(handle, targets, text, channel_type).await
        }
    }
}

async fn collect_parallel(
    handle: &Arc<dyn ChannelBridgeHandle>,
    targets: &[(String, Option<AgentId>)],
    text: &str,
) -> Vec<String> {
    let mut handles = Vec::new();
    for (name, maybe_id) in targets {
        if let Some(agent_id) = maybe_id {
            let handle = handle.clone();
            let text = text.to_string();
            let agent_id = *agent_id;
            let name = name.clone();
            handles.push(tokio::spawn(async move {
                let result = handle.send_message(agent_id, &text, None).await;
                format_broadcast_response(&name, result)
            }));
        }
    }

    let mut responses = Vec::new();
    for join_handle in handles {
        if let Ok(response) = join_handle.await {
            responses.push(response);
        }
    }
    responses
}

async fn collect_sequential(
    handle: &Arc<dyn ChannelBridgeHandle>,
    targets: &[(String, Option<AgentId>)],
    text: &str,
    channel_type: &str,
) -> Vec<String> {
    let mut responses = Vec::new();
    for (name, maybe_id) in targets {
        if let Some(agent_id) = maybe_id {
            let result = handle
                .send_message(*agent_id, text, Some(channel_type))
                .await;
            responses.push(format_broadcast_response(name, result));
        }
    }
    responses
}

fn format_broadcast_response(name: &str, result: Result<String, String>) -> String {
    match result {
        Ok(response) => format!("[{name}]: {response}"),
        Err(error) => format!("[{name}]: Error: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelContent, ChannelStatus, ChannelType, ChannelUser};
    use async_trait::async_trait;
    use futures::stream;
    use std::collections::HashMap;
    use std::pin::Pin;
    use std::sync::Mutex;

    struct MockBroadcastHandle {
        responses: HashMap<AgentId, Result<String, String>>,
        calls: Mutex<Vec<(AgentId, String, Option<String>)>>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for MockBroadcastHandle {
        async fn send_message(
            &self,
            agent_id: AgentId,
            message: &str,
            channel_type: Option<&str>,
        ) -> Result<String, String> {
            self.calls.lock().unwrap().push((
                agent_id,
                message.to_string(),
                channel_type.map(str::to_string),
            ));
            self.responses
                .get(&agent_id)
                .cloned()
                .unwrap_or_else(|| Ok("missing".to_string()))
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
    }

    fn mock_handle(
        responses: HashMap<AgentId, Result<String, String>>,
    ) -> Arc<dyn ChannelBridgeHandle> {
        Arc::new(MockBroadcastHandle {
            responses,
            calls: Mutex::new(Vec::new()),
        })
    }

    struct MockBroadcastAdapter {
        sent: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ChannelAdapter for MockBroadcastAdapter {
        fn name(&self) -> &str {
            "telegram"
        }

        fn channel_type(&self) -> ChannelType {
            ChannelType::Telegram
        }

        async fn start(
            &self,
        ) -> Result<
            Pin<Box<dyn futures::Stream<Item = ChannelMessage> + Send>>,
            Box<dyn std::error::Error>,
        > {
            Ok(Box::pin(stream::empty()))
        }

        async fn send(
            &self,
            _user: &ChannelUser,
            content: ChannelContent,
        ) -> Result<(), Box<dyn std::error::Error>> {
            if let ChannelContent::Text(text) = content {
                self.sent.lock().unwrap().push(text);
            }
            Ok(())
        }

        async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }

        fn status(&self) -> ChannelStatus {
            ChannelStatus::default()
        }
    }

    fn message(sender: &str) -> ChannelMessage {
        ChannelMessage {
            channel: ChannelType::Telegram,
            platform_message_id: "m1".to_string(),
            sender: ChannelUser {
                platform_id: sender.to_string(),
                display_name: "Ada".to_string(),
                captain_user: Some("user-1".to_string()),
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
    async fn broadcast_collects_sequential_responses_and_errors() {
        let first = AgentId::new();
        let second = AgentId::new();
        let handle = mock_handle(HashMap::from([
            (first, Ok("alpha".to_string())),
            (second, Err("boom".to_string())),
        ]));

        let responses = collect_broadcast_responses(
            &handle,
            &[
                ("one".to_string(), Some(first)),
                ("skip".to_string(), None),
                ("two".to_string(), Some(second)),
            ],
            BroadcastStrategy::Sequential,
            "hello",
            "telegram",
        )
        .await;

        assert_eq!(
            responses,
            vec!["[one]: alpha".to_string(), "[two]: Error: boom".to_string()]
        );
    }

    #[tokio::test]
    async fn broadcast_parallel_skips_unresolved_targets() {
        let first = AgentId::new();
        let handle = mock_handle(HashMap::from([(first, Ok("alpha".to_string()))]));

        let responses = collect_broadcast_responses(
            &handle,
            &[("one".to_string(), Some(first)), ("skip".to_string(), None)],
            BroadcastStrategy::Parallel,
            "hello",
            "telegram",
        )
        .await;

        assert_eq!(responses, vec!["[one]: alpha".to_string()]);
    }

    #[tokio::test]
    async fn inbound_broadcast_skips_when_sender_has_no_broadcast_config() {
        let handle = mock_handle(HashMap::new());
        let router = Arc::new(AgentRouter::new());
        let adapter: Arc<dyn ChannelAdapter> = Arc::new(MockBroadcastAdapter {
            sent: Mutex::new(Vec::new()),
        });
        let message = message("normal_user");

        let handled = try_handle_inbound_broadcast(InboundBroadcastContext {
            handle: &handle,
            router: &router,
            adapter: adapter.as_ref(),
            adapter_arc: adapter.clone(),
            message: &message,
            sender_user_id: "user-1",
            text: "hello",
            channel_type: "telegram",
            thread_id: Some("topic-1"),
            output_format: OutputFormat::PlainText,
        })
        .await;

        assert!(!handled);
    }
}
