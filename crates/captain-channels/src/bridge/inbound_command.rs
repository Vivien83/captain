//! Inbound command execution shared by native and text slash commands.

use super::command_dispatch::{handle_command, CommandContext};
use super::command_response::send_command_response;
use super::inbound_control::parse_known_text_command;
use super::model_switch_pending::PendingModelSwitchStore;
use super::ChannelBridgeHandle;
use crate::router::AgentRouter;
use crate::types::{ChannelAdapter, ChannelUser};
use captain_types::config::OutputFormat;
use std::sync::Arc;

pub(super) struct InboundCommandExecutionContext<'a> {
    pub(super) handle: &'a Arc<dyn ChannelBridgeHandle>,
    pub(super) router: &'a Arc<AgentRouter>,
    pub(super) adapter: &'a dyn ChannelAdapter,
    pub(super) sender: &'a ChannelUser,
    pub(super) sender_user_id: &'a str,
    pub(super) channel: &'a str,
    pub(super) thread_id: Option<&'a str>,
    pub(super) source_message_id: Option<&'a str>,
    pub(super) output_format: OutputFormat,
    pub(super) pending_model_switches: &'a PendingModelSwitchStore,
}

pub(super) async fn handle_inbound_command(
    name: &str,
    args: &[String],
    ctx: InboundCommandExecutionContext<'_>,
) {
    let result = handle_command(
        name,
        args,
        CommandContext {
            handle: ctx.handle,
            router: ctx.router,
            sender: ctx.sender,
            sender_user_id: ctx.sender_user_id,
            channel: ctx.channel,
            thread_id: ctx.thread_id,
            source_message_id: ctx.source_message_id,
            pending_model_switches: ctx.pending_model_switches,
        },
    )
    .await;
    send_command_response(
        ctx.adapter,
        ctx.sender,
        result,
        ctx.thread_id,
        ctx.output_format,
    )
    .await;
}

pub(super) async fn try_handle_inbound_text_command(
    text: &str,
    ctx: InboundCommandExecutionContext<'_>,
) -> bool {
    let Some(command) = parse_known_text_command(text) else {
        return false;
    };

    handle_inbound_command(&command.name, &command.args, ctx).await;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelContent, ChannelMessage, ChannelStatus, ChannelType};
    use async_trait::async_trait;
    use captain_types::agent::AgentId;
    use futures::stream;
    use std::collections::HashMap;
    use std::pin::Pin;
    use std::sync::Mutex;

    struct MockCommandHandle;

    #[async_trait]
    impl ChannelBridgeHandle for MockCommandHandle {
        async fn send_message(
            &self,
            _agent_id: AgentId,
            message: &str,
            _channel_type: Option<&str>,
        ) -> Result<String, String> {
            Ok(message.to_string())
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

    struct MockCommandAdapter {
        sent: Mutex<Vec<(String, Option<String>)>>,
    }

    #[async_trait]
    impl ChannelAdapter for MockCommandAdapter {
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

        async fn send_rich(
            &self,
            _user: &ChannelUser,
            content: ChannelContent,
            metadata: &HashMap<String, serde_json::Value>,
        ) -> Result<Option<String>, Box<dyn std::error::Error>> {
            if let ChannelContent::Text(text) = content {
                let thread_id = metadata
                    .get("thread_id")
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string);
                self.sent.lock().unwrap().push((text, thread_id));
            }
            Ok(None)
        }

        async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }

        fn status(&self) -> ChannelStatus {
            ChannelStatus::default()
        }
    }

    fn context<'a>(
        handle: &'a Arc<dyn ChannelBridgeHandle>,
        router: &'a Arc<AgentRouter>,
        adapter: &'a dyn ChannelAdapter,
        sender: &'a ChannelUser,
        pending_model_switches: &'a PendingModelSwitchStore,
    ) -> InboundCommandExecutionContext<'a> {
        InboundCommandExecutionContext {
            handle,
            router,
            adapter,
            sender,
            sender_user_id: "user-1",
            channel: "telegram",
            thread_id: Some("topic-1"),
            source_message_id: None,
            output_format: OutputFormat::PlainText,
            pending_model_switches,
        }
    }

    #[tokio::test]
    async fn text_command_executes_and_sends_threaded_response() {
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockCommandHandle);
        let router = Arc::new(AgentRouter::new());
        let adapter = MockCommandAdapter {
            sent: Mutex::new(Vec::new()),
        };
        let pending_model_switches = Arc::new(dashmap::DashMap::new());
        let sender = ChannelUser {
            platform_id: "chat-1".to_string(),
            display_name: "Ada".to_string(),
            captain_user: Some("user-1".to_string()),
        };

        let handled = try_handle_inbound_text_command(
            "/help",
            context(&handle, &router, &adapter, &sender, &pending_model_switches),
        )
        .await;

        assert!(handled);
        let sent = adapter.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].0.contains("Captain Bot Commands:"));
        assert_eq!(sent[0].1.as_deref(), Some("topic-1"));
    }

    #[tokio::test]
    async fn unknown_text_command_is_left_for_agent() {
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockCommandHandle);
        let router = Arc::new(AgentRouter::new());
        let adapter = MockCommandAdapter {
            sent: Mutex::new(Vec::new()),
        };
        let pending_model_switches = Arc::new(dashmap::DashMap::new());
        let sender = ChannelUser {
            platform_id: "chat-1".to_string(),
            display_name: "Ada".to_string(),
            captain_user: Some("user-1".to_string()),
        };

        let handled = try_handle_inbound_text_command(
            "/unknown reach agent",
            context(&handle, &router, &adapter, &sender, &pending_model_switches),
        )
        .await;

        assert!(!handled);
        assert!(adapter.sent.lock().unwrap().is_empty());
    }
}
