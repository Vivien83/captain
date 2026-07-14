//! Relay the outcome of an inbound agent retry after re-resolution.

use super::inbound_failure_response::complete_inbound_agent_failure;
use super::inbound_success_response::complete_inbound_agent_success;
use super::ChannelBridgeHandle;
use crate::types::{ChannelAdapter, ChannelMessage};
use captain_types::agent::AgentId;
use captain_types::config::OutputFormat;
use std::sync::Arc;

#[allow(clippy::too_many_arguments)]
pub(super) async fn relay_inbound_retry_result(
    handle: &Arc<dyn ChannelBridgeHandle>,
    adapter: &dyn ChannelAdapter,
    message: &ChannelMessage,
    agent_id: AgentId,
    channel_type: &str,
    retry: Result<String, String>,
    message_id: &str,
    lifecycle_reactions: bool,
    thread_id: Option<&str>,
    output_format: OutputFormat,
) {
    match retry {
        Ok(response) => {
            complete_inbound_agent_success(
                handle,
                adapter,
                message,
                agent_id,
                channel_type,
                response,
                false,
                message_id,
                lifecycle_reactions,
                thread_id,
                output_format,
            )
            .await;
        }
        Err(error) => {
            complete_inbound_agent_failure(
                handle,
                adapter,
                message,
                agent_id,
                channel_type,
                &error,
                "Agent error after re-resolution",
                message_id,
                lifecycle_reactions,
                thread_id,
                output_format,
            )
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentPhase, ChannelContent, ChannelType, ChannelUser, LifecycleReaction};
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

    struct MockRetryHandle {
        deliveries: Mutex<Vec<DeliveryRecord>>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for MockRetryHandle {
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

    fn handle() -> Arc<MockRetryHandle> {
        Arc::new(MockRetryHandle {
            deliveries: Mutex::new(Vec::new()),
        })
    }

    fn adapter() -> RecordingAdapter {
        RecordingAdapter {
            sent: Mutex::new(Vec::new()),
            reactions: Mutex::new(Vec::new()),
        }
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
    async fn retry_success_marks_done_and_records_success() {
        let handle = handle();
        let handle_trait: Arc<dyn ChannelBridgeHandle> = handle.clone();
        let adapter = adapter();
        let agent_id = AgentId::new();
        let message = message();

        relay_inbound_retry_result(
            &handle_trait,
            &adapter,
            &message,
            agent_id,
            "telegram",
            Ok("retry ok".to_string()),
            "msg-42",
            true,
            Some("topic-7"),
            OutputFormat::PlainText,
        )
        .await;

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
    async fn retry_failure_marks_error_and_records_sanitized_failure() {
        let handle = handle();
        let handle_trait: Arc<dyn ChannelBridgeHandle> = handle.clone();
        let adapter = adapter();
        let agent_id = AgentId::new();
        let message = message();

        relay_inbound_retry_result(
            &handle_trait,
            &adapter,
            &message,
            agent_id,
            "telegram",
            Err("invalid x-goog-api-key".to_string()),
            "msg-42",
            true,
            Some("topic-7"),
            OutputFormat::PlainText,
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
            handle.deliveries.lock().unwrap().as_slice(),
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
