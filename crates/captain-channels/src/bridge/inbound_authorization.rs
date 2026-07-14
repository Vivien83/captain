//! RBAC checks for inbound channel chat dispatch.

use super::command_response::send_response;
use super::ChannelBridgeHandle;
use crate::types::{ChannelAdapter, ChannelUser};
use captain_types::config::OutputFormat;
use std::sync::Arc;

pub(super) async fn authorize_inbound_chat(
    handle: &Arc<dyn ChannelBridgeHandle>,
    adapter: &dyn ChannelAdapter,
    sender: &ChannelUser,
    sender_user_id: &str,
    channel_type: &str,
    thread_id: Option<&str>,
    output_format: OutputFormat,
) -> bool {
    match handle
        .authorize_channel_user(channel_type, sender_user_id, "chat")
        .await
    {
        Ok(()) => true,
        Err(denied) => {
            send_response(
                adapter,
                sender,
                format!("Access denied: {denied}"),
                thread_id,
                output_format,
            )
            .await;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelContent, ChannelMessage, ChannelStatus, ChannelType};
    use async_trait::async_trait;
    use captain_types::agent::AgentId;
    use futures::stream;
    use std::pin::Pin;
    use std::sync::Mutex;

    struct MockAuthorizationHandle {
        result: Mutex<Result<(), String>>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for MockAuthorizationHandle {
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
            self.result.lock().unwrap().clone()
        }
    }

    struct MockAuthorizationAdapter {
        sent: Mutex<Vec<(String, Option<String>)>>,
    }

    #[async_trait]
    impl ChannelAdapter for MockAuthorizationAdapter {
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

        async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }

        fn status(&self) -> ChannelStatus {
            ChannelStatus::default()
        }
    }

    fn sender() -> ChannelUser {
        ChannelUser {
            platform_id: "chat-1".to_string(),
            display_name: "Ada".to_string(),
            captain_user: Some("user-1".to_string()),
        }
    }

    #[tokio::test]
    async fn authorization_allows_without_sending_response() {
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockAuthorizationHandle {
            result: Mutex::new(Ok(())),
        });
        let adapter = MockAuthorizationAdapter {
            sent: Mutex::new(Vec::new()),
        };

        let allowed = authorize_inbound_chat(
            &handle,
            &adapter,
            &sender(),
            "user-1",
            "telegram",
            Some("topic-1"),
            OutputFormat::PlainText,
        )
        .await;

        assert!(allowed);
        assert!(adapter.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn authorization_denial_sends_threaded_access_denied() {
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockAuthorizationHandle {
            result: Mutex::new(Err("owner only".to_string())),
        });
        let adapter = MockAuthorizationAdapter {
            sent: Mutex::new(Vec::new()),
        };

        let allowed = authorize_inbound_chat(
            &handle,
            &adapter,
            &sender(),
            "user-1",
            "telegram",
            Some("topic-1"),
            OutputFormat::PlainText,
        )
        .await;

        assert!(!allowed);
        assert_eq!(
            adapter.sent.lock().unwrap().as_slice(),
            &[(
                "Access denied: owner only".to_string(),
                Some("topic-1".to_string())
            )]
        );
    }
}
