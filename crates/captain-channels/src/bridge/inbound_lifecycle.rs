//! Inbound lifecycle reactions for long agent turns.

use super::progress::send_lifecycle_reaction;
use crate::types::{AgentPhase, ChannelAdapter, ChannelUser};

pub(super) async fn send_inbound_lifecycle_started(
    adapter: &dyn ChannelAdapter,
    sender: &ChannelUser,
    message_id: &str,
    enabled: bool,
) {
    if !enabled {
        return;
    }

    send_lifecycle_reaction(adapter, sender, message_id, AgentPhase::Queued).await;
    send_lifecycle_reaction(adapter, sender, message_id, AgentPhase::Thinking).await;
}

pub(super) async fn send_inbound_lifecycle_done(
    adapter: &dyn ChannelAdapter,
    sender: &ChannelUser,
    message_id: &str,
    enabled: bool,
) {
    send_inbound_lifecycle_terminal(adapter, sender, message_id, enabled, AgentPhase::Done).await;
}

pub(super) async fn send_inbound_lifecycle_error(
    adapter: &dyn ChannelAdapter,
    sender: &ChannelUser,
    message_id: &str,
    enabled: bool,
) {
    send_inbound_lifecycle_terminal(adapter, sender, message_id, enabled, AgentPhase::Error).await;
}

async fn send_inbound_lifecycle_terminal(
    adapter: &dyn ChannelAdapter,
    sender: &ChannelUser,
    message_id: &str,
    enabled: bool,
    phase: AgentPhase,
) {
    if enabled {
        send_lifecycle_reaction(adapter, sender, message_id, phase).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelContent, ChannelMessage, ChannelType, LifecycleReaction};
    use async_trait::async_trait;
    use futures::{stream, Stream};
    use std::pin::Pin;
    use std::sync::Mutex;

    struct RecordingAdapter {
        reactions: Mutex<Vec<(String, AgentPhase, bool)>>,
    }

    impl RecordingAdapter {
        fn new() -> Self {
            Self {
                reactions: Mutex::new(Vec::new()),
            }
        }

        fn reactions(&self) -> Vec<(String, AgentPhase, bool)> {
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

        async fn send_reaction(
            &self,
            _user: &ChannelUser,
            message_id: &str,
            reaction: &LifecycleReaction,
        ) -> Result<(), Box<dyn std::error::Error>> {
            self.reactions.lock().unwrap().push((
                message_id.to_string(),
                reaction.phase.clone(),
                reaction.remove_previous,
            ));
            Ok(())
        }

        async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }
    }

    fn sender() -> ChannelUser {
        ChannelUser {
            platform_id: "user-1".to_string(),
            display_name: "Ada".to_string(),
            captain_user: None,
        }
    }

    #[tokio::test]
    async fn started_lifecycle_sends_queued_then_thinking_when_enabled() {
        let adapter = RecordingAdapter::new();
        let sender = sender();

        send_inbound_lifecycle_started(&adapter, &sender, "msg-42", true).await;

        assert_eq!(
            adapter.reactions(),
            vec![
                ("msg-42".to_string(), AgentPhase::Queued, true),
                ("msg-42".to_string(), AgentPhase::Thinking, true),
            ]
        );
    }

    #[tokio::test]
    async fn terminal_lifecycle_sends_done_and_error_when_enabled() {
        let adapter = RecordingAdapter::new();
        let sender = sender();

        send_inbound_lifecycle_done(&adapter, &sender, "msg-42", true).await;
        send_inbound_lifecycle_error(&adapter, &sender, "msg-43", true).await;

        assert_eq!(
            adapter.reactions(),
            vec![
                ("msg-42".to_string(), AgentPhase::Done, true),
                ("msg-43".to_string(), AgentPhase::Error, true),
            ]
        );
    }

    #[tokio::test]
    async fn lifecycle_is_noop_when_disabled() {
        let adapter = RecordingAdapter::new();
        let sender = sender();

        send_inbound_lifecycle_started(&adapter, &sender, "msg-1", false).await;
        send_inbound_lifecycle_done(&adapter, &sender, "msg-2", false).await;
        send_inbound_lifecycle_error(&adapter, &sender, "msg-3", false).await;

        assert!(adapter.reactions().is_empty());
    }
}
