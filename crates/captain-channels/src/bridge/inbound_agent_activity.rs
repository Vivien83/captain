//! Typing and visible-progress activity for inbound agent turns.

use super::progress::{
    abort_visible_progress_task, spawn_typing_loop, spawn_visible_progress_loop_for_channel,
};
use crate::types::{ChannelAdapter, ChannelType, ChannelUser};
use captain_types::config::OutputFormat;
use std::sync::Arc;

pub(super) struct InboundAgentActivity {
    typing_task: Option<tokio::task::JoinHandle<()>>,
    progress_task: Option<tokio::task::JoinHandle<()>>,
}

impl InboundAgentActivity {
    pub(super) async fn start(
        channel: &ChannelType,
        adapter: &dyn ChannelAdapter,
        adapter_arc: Arc<dyn ChannelAdapter>,
        sender: &ChannelUser,
        thread_id: Option<&str>,
        output_format: OutputFormat,
    ) -> Self {
        let _ = adapter.send_typing(sender).await;
        let typing_task = spawn_typing_loop(adapter_arc.clone(), sender.clone());
        let progress_task = spawn_visible_progress_loop_for_channel(
            channel,
            adapter_arc,
            sender.clone(),
            thread_id.map(str::to_string),
            output_format,
        );

        Self {
            typing_task: Some(typing_task),
            progress_task,
        }
    }

    pub(super) fn stop(mut self) {
        self.abort_all();
    }

    fn abort_all(&mut self) {
        if let Some(task) = self.typing_task.take() {
            task.abort();
        }
        abort_visible_progress_task(self.progress_task.take());
    }

    #[cfg(test)]
    fn has_visible_progress(&self) -> bool {
        self.progress_task.is_some()
    }
}

impl Drop for InboundAgentActivity {
    fn drop(&mut self) {
        self.abort_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelContent, ChannelMessage};
    use async_trait::async_trait;
    use futures::{stream, Stream};
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RecordingAdapter {
        channel: ChannelType,
        typing_count: AtomicUsize,
    }

    impl RecordingAdapter {
        fn new(channel: ChannelType) -> Self {
            Self {
                channel,
                typing_count: AtomicUsize::new(0),
            }
        }

        fn typing_count(&self) -> usize {
            self.typing_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ChannelAdapter for RecordingAdapter {
        fn name(&self) -> &str {
            "recording"
        }

        fn channel_type(&self) -> ChannelType {
            self.channel.clone()
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

        async fn send_typing(&self, _user: &ChannelUser) -> Result<(), Box<dyn std::error::Error>> {
            self.typing_count.fetch_add(1, Ordering::SeqCst);
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
    async fn activity_sends_immediate_typing_and_starts_progress_for_non_telegram() {
        let adapter = Arc::new(RecordingAdapter::new(ChannelType::Discord));
        let adapter_arc: Arc<dyn ChannelAdapter> = adapter.clone();
        let sender = sender();

        let activity = InboundAgentActivity::start(
            &ChannelType::Discord,
            adapter.as_ref(),
            adapter_arc,
            &sender,
            Some("thread-7"),
            OutputFormat::Markdown,
        )
        .await;

        assert_eq!(adapter.typing_count(), 1);
        assert!(activity.has_visible_progress());
        activity.stop();
    }

    #[tokio::test]
    async fn activity_skips_visible_progress_for_telegram_streaming() {
        let adapter = Arc::new(RecordingAdapter::new(ChannelType::Telegram));
        let adapter_arc: Arc<dyn ChannelAdapter> = adapter.clone();
        let sender = sender();

        let activity = InboundAgentActivity::start(
            &ChannelType::Telegram,
            adapter.as_ref(),
            adapter_arc,
            &sender,
            None,
            OutputFormat::TelegramHtml,
        )
        .await;

        assert_eq!(adapter.typing_count(), 1);
        assert!(!activity.has_visible_progress());
        activity.stop();
    }
}
