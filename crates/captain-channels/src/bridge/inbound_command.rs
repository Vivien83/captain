//! Inbound command execution shared by native and text slash commands.

use super::command_agent::resolve_selected_agent;
use super::command_dispatch::{handle_command, CommandContext};
use super::command_response::{
    send_command_response, send_durable_rich_response, DurableResponseContext,
};
use super::inbound_control::parse_known_text_command;
use super::model_switch_pending::PendingModelSwitchStore;
use super::ChannelBridgeHandle;
use crate::render_telegram_compaction_progress;
use crate::router::AgentRouter;
use crate::telegram::TelegramStreamTarget;
use crate::types::{ChannelAdapter, ChannelUser};
use captain_types::compaction::{CompactionPhase, CompactionState};
use captain_types::config::OutputFormat;
use std::sync::Arc;

pub(super) struct InboundCommandExecutionContext<'a> {
    pub(super) handle: &'a Arc<dyn ChannelBridgeHandle>,
    pub(super) router: &'a Arc<AgentRouter>,
    pub(super) adapter: &'a Arc<dyn ChannelAdapter>,
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
    if name == "compact" && ctx.channel == "telegram" {
        handle_telegram_compaction(ctx).await;
        return;
    }
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
        ctx.adapter.as_ref(),
        ctx.sender,
        result,
        ctx.thread_id,
        ctx.output_format,
        Some(DurableResponseContext {
            handle: ctx.handle,
            agent_id: None,
            channel: ctx.channel,
            source_message_id: ctx.source_message_id.unwrap_or("channel-command"),
            purpose: "command_response",
        }),
    )
    .await
    .unwrap_or_else(|error| {
        tracing::error!(%error, channel = %ctx.channel, "failed to deliver durable command response")
    });
}

async fn handle_telegram_compaction(ctx: InboundCommandExecutionContext<'_>) {
    let Some(agent_id) = resolve_selected_agent(ctx.router, ctx.channel, ctx.sender) else {
        let result = handle_command(
            "compact",
            &[],
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
        let _ = send_command_response(
            ctx.adapter.as_ref(),
            ctx.sender,
            result,
            ctx.thread_id,
            ctx.output_format,
            None,
        )
        .await;
        return;
    };

    let telegram_draft = ctx.adapter.clone().as_telegram_arc().and_then(|telegram| {
        let chat_id = ctx.sender.platform_id.parse::<i64>().ok()?;
        let thread_id = ctx.thread_id.and_then(|value| value.parse::<i64>().ok());
        TelegramStreamTarget::new(telegram, chat_id, thread_id).progress_draft()
    });
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(32);
    let mut operation = Box::pin(
        ctx.handle
            .compact_session_with_progress(agent_id, progress_tx),
    );
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut latest = None;
    let mut animation_tick = 0usize;

    let result = loop {
        tokio::select! {
            result = &mut operation => break result,
            update = progress_rx.recv() => {
                let Some(update) = update else { continue };
                if let Some(draft) = telegram_draft.as_ref() {
                    let rendered = render_telegram_compaction_progress(&update, animation_tick);
                    if let Err(error) = draft.refresh(&rendered).await {
                        tracing::debug!(%error, "Telegram compaction draft refresh failed");
                    }
                }
                latest = Some(update);
            }
            _ = interval.tick(), if latest.as_ref().is_some_and(|p: &captain_types::compaction::CompactionProgress| p.state == CompactionState::Running) => {
                animation_tick = animation_tick.wrapping_add(1);
                if let (Some(draft), Some(progress)) = (telegram_draft.as_ref(), latest.as_ref()) {
                    let rendered = render_telegram_compaction_progress(progress, animation_tick);
                    if let Err(error) = draft.refresh(&rendered).await {
                        tracing::debug!(%error, "Telegram compaction draft animation failed");
                    }
                }
            }
        }
    };
    while let Ok(update) = progress_rx.try_recv() {
        latest = Some(update);
    }

    if let Some(progress) = latest.as_mut() {
        if progress.state == CompactionState::Running {
            match &result {
                Ok(message) => {
                    progress.phase = CompactionPhase::Completed;
                    progress.state = CompactionState::Succeeded;
                    progress.detail = message.clone();
                }
                Err(error) => {
                    progress.phase = CompactionPhase::Failed;
                    progress.state = CompactionState::Failed;
                    progress.detail = error.clone();
                }
            }
            progress.completed_units = None;
            progress.total_units = None;
            progress.unit = None;
        }
    }

    let body = latest.as_ref().map_or_else(
        || match &result {
            Ok(message) => format!(
                "### ✓ Compactage du contexte\n\n<blockquote>{}</blockquote>",
                html_escape::encode_text(message)
            ),
            Err(error) => crate::render_telegram_channel_error(error),
        },
        |progress| render_telegram_compaction_progress(progress, animation_tick),
    );
    let mut metadata = std::collections::HashMap::new();
    if let Some(thread_id) = ctx.thread_id {
        metadata.insert("thread_id".to_string(), serde_json::json!(thread_id));
    }
    let source_message_id = ctx.source_message_id.unwrap_or("telegram-compact");
    if let Err(error) = send_durable_rich_response(
        ctx.adapter.as_ref(),
        ctx.sender,
        body,
        metadata,
        DurableResponseContext {
            handle: ctx.handle,
            agent_id: Some(agent_id),
            channel: ctx.channel,
            source_message_id,
            purpose: "compaction_response",
        },
    )
    .await
    {
        tracing::error!(%error, "failed to deliver Telegram compaction result");
    }
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
    use captain_types::agent::{AgentId, SessionId};
    use captain_types::compaction::{CompactionProgress, COMPACTION_PROGRESS_SCHEMA_VERSION};
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

    struct ProgressCommandHandle {
        agent_id: AgentId,
        session_id: SessionId,
    }

    #[async_trait]
    impl ChannelBridgeHandle for ProgressCommandHandle {
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

        async fn compact_session_with_progress(
            &self,
            agent_id: AgentId,
            progress: tokio::sync::mpsc::Sender<CompactionProgress>,
        ) -> Result<String, String> {
            assert_eq!(agent_id, self.agent_id);
            progress
                .send(CompactionProgress {
                    schema_version: COMPACTION_PROGRESS_SCHEMA_VERSION,
                    operation_id: "compact-telegram".to_string(),
                    runtime_instance_id: "runtime-1".to_string(),
                    agent_id,
                    session_id: self.session_id,
                    phase: CompactionPhase::Summarizing,
                    state: CompactionState::Running,
                    detail: "opaque model call".to_string(),
                    message_count: 40,
                    estimated_tokens: 10_000,
                    context_window_tokens: 200_000,
                    completed_units: None,
                    total_units: None,
                    unit: None,
                    started_at_ms: 1,
                    updated_at_ms: 2,
                })
                .await
                .map_err(|error| error.to_string())?;
            Ok("Compacted 32 messages and kept 8".to_string())
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
        adapter: &'a Arc<dyn ChannelAdapter>,
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
        let adapter_impl = Arc::new(MockCommandAdapter {
            sent: Mutex::new(Vec::new()),
        });
        let adapter: Arc<dyn ChannelAdapter> = adapter_impl.clone();
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
        let sent = adapter_impl.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].0.contains("Captain Bot Commands:"));
        assert_eq!(sent[0].1.as_deref(), Some("topic-1"));
    }

    #[tokio::test]
    async fn unknown_text_command_is_left_for_agent() {
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockCommandHandle);
        let router = Arc::new(AgentRouter::new());
        let adapter_impl = Arc::new(MockCommandAdapter {
            sent: Mutex::new(Vec::new()),
        });
        let adapter: Arc<dyn ChannelAdapter> = adapter_impl.clone();
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
        assert!(adapter_impl.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn telegram_compact_synthesizes_a_terminal_rich_card_if_the_last_tick_was_running() {
        let agent_id = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(ProgressCommandHandle {
            agent_id,
            session_id: SessionId::new(),
        });
        let router = Arc::new(AgentRouter::new());
        router.set_user_default("user-1".to_string(), agent_id);
        let adapter_impl = Arc::new(MockCommandAdapter {
            sent: Mutex::new(Vec::new()),
        });
        let adapter: Arc<dyn ChannelAdapter> = adapter_impl.clone();
        let pending_model_switches = Arc::new(dashmap::DashMap::new());
        let sender = ChannelUser {
            platform_id: "chat-1".to_string(),
            display_name: "Ada".to_string(),
            captain_user: Some("user-1".to_string()),
        };

        let handled = try_handle_inbound_text_command(
            "/compact",
            context(&handle, &router, &adapter, &sender, &pending_model_switches),
        )
        .await;

        assert!(handled);
        let sent = adapter_impl.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].0.contains("Compactage du contexte"));
        assert!(sent[0].0.contains("Terminé"));
        assert!(sent[0].0.contains("<pre>[████████████████]</pre>"));
        assert!(sent[0].0.contains("100% · terminé"));
        assert!(sent[0].0.contains("Compacted 32 messages and kept 8"));
        assert!(!sent[0].0.contains("progression indéterminée"));
        assert_eq!(sent[0].1.as_deref(), Some("topic-1"));
    }
}
