//! Preconditions before running an inbound agent turn.

use super::inbound_authorization::authorize_inbound_chat;
use super::inbound_auto_reply::handle_auto_reply;
use super::inbound_session_agent::mark_inbound_session_agent;
use super::ChannelBridgeHandle;
use crate::inbound_queue::InboundSessionQueue;
use crate::types::{ChannelAdapter, ChannelMessage};
use captain_types::agent::AgentId;
use captain_types::config::OutputFormat;
use std::sync::Arc;

pub(super) struct InboundAgentPreflightContext<'a> {
    pub(super) handle: &'a Arc<dyn ChannelBridgeHandle>,
    pub(super) adapter: &'a dyn ChannelAdapter,
    pub(super) message: &'a ChannelMessage,
    pub(super) agent_id: AgentId,
    pub(super) text: &'a str,
    pub(super) active_session: Option<(&'a InboundSessionQueue, &'a str)>,
    pub(super) sender_user_id: &'a str,
    pub(super) channel_type: &'a str,
    pub(super) thread_id: Option<&'a str>,
    pub(super) output_format: OutputFormat,
}

pub(super) enum InboundAgentPreflight<'a> {
    Continue { active_session_key: Option<&'a str> },
    Stop,
}

pub(super) async fn prepare_inbound_agent_preflight(
    ctx: InboundAgentPreflightContext<'_>,
) -> InboundAgentPreflight<'_> {
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
        return InboundAgentPreflight::Stop;
    }

    let active_session_key = mark_inbound_session_agent(ctx.active_session, ctx.agent_id);

    if handle_auto_reply(
        ctx.handle,
        ctx.adapter,
        ctx.message,
        ctx.agent_id,
        ctx.text,
        ctx.channel_type,
        ctx.thread_id,
        ctx.output_format,
    )
    .await
    {
        return InboundAgentPreflight::Stop;
    }

    InboundAgentPreflight::Continue { active_session_key }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbound_queue_types::InboundStart;
    use crate::types::{ChannelContent, ChannelType, ChannelUser};
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

    struct MockPreflightHandle {
        authorization: Mutex<Result<(), String>>,
        auto_reply: Mutex<Option<String>>,
        deliveries: Mutex<Vec<DeliveryRecord>>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for MockPreflightHandle {
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

        async fn authorize_channel_user(
            &self,
            _channel: &str,
            _user_id: &str,
            _capability: &str,
        ) -> Result<(), String> {
            self.authorization.lock().unwrap().clone()
        }

        async fn check_auto_reply(&self, _agent_id: AgentId, _message: &str) -> Option<String> {
            self.auto_reply.lock().unwrap().clone()
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

    fn handle(
        authorization: Result<(), &str>,
        auto_reply: Option<&str>,
    ) -> Arc<MockPreflightHandle> {
        Arc::new(MockPreflightHandle {
            authorization: Mutex::new(authorization.map_err(str::to_string)),
            auto_reply: Mutex::new(auto_reply.map(str::to_string)),
            deliveries: Mutex::new(Vec::new()),
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

    fn active_session(sessions: &InboundSessionQueue, message: ChannelMessage) -> String {
        let session_key = "telegram|chat:chat-1|user:user-1|captain:-|thread:topic-1".to_string();
        assert!(matches!(
            sessions.start_or_queue(session_key.clone(), message),
            InboundStart::Started { .. }
        ));
        session_key
    }

    #[tokio::test]
    async fn preflight_denial_stops_without_marking_session() {
        let mock_handle = handle(Err("owner only"), None);
        let handle: Arc<dyn ChannelBridgeHandle> = mock_handle.clone();
        let adapter = adapter();
        let message = message();
        let sessions = InboundSessionQueue::default();
        let session_key = active_session(&sessions, message.clone());
        let agent_id = AgentId::new();

        let result = prepare_inbound_agent_preflight(InboundAgentPreflightContext {
            handle: &handle,
            adapter: &adapter,
            message: &message,
            agent_id,
            text: "hello",
            active_session: Some((&sessions, &session_key)),
            sender_user_id: "user-1",
            channel_type: "telegram",
            thread_id: Some("topic-1"),
            output_format: OutputFormat::PlainText,
        })
        .await;

        assert!(matches!(result, InboundAgentPreflight::Stop));
        assert_eq!(sessions.active_agent(&session_key), None);
        assert_eq!(
            adapter.sent.lock().unwrap().as_slice(),
            &[(
                "Access denied: owner only".to_string(),
                Some("topic-1".to_string())
            )]
        );
    }

    #[tokio::test]
    async fn preflight_continue_marks_session_when_no_auto_reply() {
        let mock_handle = handle(Ok(()), None);
        let handle: Arc<dyn ChannelBridgeHandle> = mock_handle.clone();
        let adapter = adapter();
        let message = message();
        let sessions = InboundSessionQueue::default();
        let session_key = active_session(&sessions, message.clone());
        let agent_id = AgentId::new();

        let result = prepare_inbound_agent_preflight(InboundAgentPreflightContext {
            handle: &handle,
            adapter: &adapter,
            message: &message,
            agent_id,
            text: "hello",
            active_session: Some((&sessions, &session_key)),
            sender_user_id: "user-1",
            channel_type: "telegram",
            thread_id: Some("topic-1"),
            output_format: OutputFormat::PlainText,
        })
        .await;

        assert!(matches!(
            result,
            InboundAgentPreflight::Continue {
                active_session_key: Some(
                    "telegram|chat:chat-1|user:user-1|captain:-|thread:topic-1"
                )
            }
        ));
        assert_eq!(sessions.active_agent(&session_key), Some(agent_id));
        assert!(adapter.sent.lock().unwrap().is_empty());
        assert!(mock_handle.deliveries.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn preflight_auto_reply_stops_after_marking_session() {
        let mock_handle = handle(Ok(()), Some("Auto response"));
        let handle: Arc<dyn ChannelBridgeHandle> = mock_handle.clone();
        let adapter = adapter();
        let message = message();
        let sessions = InboundSessionQueue::default();
        let session_key = active_session(&sessions, message.clone());
        let agent_id = AgentId::new();

        let result = prepare_inbound_agent_preflight(InboundAgentPreflightContext {
            handle: &handle,
            adapter: &adapter,
            message: &message,
            agent_id,
            text: "hello",
            active_session: Some((&sessions, &session_key)),
            sender_user_id: "user-1",
            channel_type: "telegram",
            thread_id: Some("topic-1"),
            output_format: OutputFormat::PlainText,
        })
        .await;

        assert!(matches!(result, InboundAgentPreflight::Stop));
        assert_eq!(sessions.active_agent(&session_key), Some(agent_id));
        assert_eq!(
            adapter.sent.lock().unwrap().as_slice(),
            &[("Auto response".to_string(), Some("topic-1".to_string()))]
        );
        assert_eq!(
            mock_handle.deliveries.lock().unwrap().as_slice(),
            &[(
                agent_id,
                "telegram".to_string(),
                "chat-1".to_string(),
                true,
                None,
                Some("topic-1".to_string())
            )]
        );
    }
}
