use crate::llm_driver::StreamEvent;
use captain_types::error::{CaptainError, CaptainResult};
use captain_types::message::{StopReason, TokenUsage};
use tokio::sync::mpsc;

const MAX_HELD_STREAM_EVENTS: usize = 131_072;
const MAX_HELD_STREAM_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StreamDeliveryCheckpoint {
    event_count: usize,
    estimated_bytes: usize,
    validated_event_count: usize,
}

#[derive(Debug, Default)]
pub(crate) struct StreamDeliveryBuffer {
    events: Vec<StreamEvent>,
    estimated_bytes: usize,
    validated_event_count: usize,
}

impl StreamDeliveryBuffer {
    pub(crate) fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub(crate) fn checkpoint(&self) -> StreamDeliveryCheckpoint {
        StreamDeliveryCheckpoint {
            event_count: self.events.len(),
            estimated_bytes: self.estimated_bytes,
            validated_event_count: self.validated_event_count,
        }
    }

    pub(crate) fn validate_segment(
        &mut self,
        checkpoint: StreamDeliveryCheckpoint,
        text: &str,
        stop_reason: StopReason,
    ) -> CaptainResult<()> {
        if checkpoint.event_count != checkpoint.validated_event_count
            || self.validated_event_count != checkpoint.event_count
        {
            return Err(CaptainError::LlmDriver(
                "held stream validation checkpoint does not follow the last certified segment"
                    .to_string(),
            ));
        }
        let segment = self.events.get(checkpoint.event_count..).ok_or_else(|| {
            CaptainError::LlmDriver(
                "held stream validation checkpoint is outside the delivery buffer".to_string(),
            )
        })?;
        let streamed_text = segment
            .iter()
            .filter_map(|event| match event {
                StreamEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        let terminal_events = segment
            .iter()
            .filter_map(|event| match event {
                StreamEvent::ContentComplete { stop_reason, .. } => Some(*stop_reason),
                _ => None,
            })
            .collect::<Vec<_>>();
        if streamed_text != text || terminal_events.as_slice() != [stop_reason] {
            return Err(CaptainError::LlmDriver(
                "held stream did not conserve the provider response exactly; delivery refused"
                    .to_string(),
            ));
        }
        self.validated_event_count = self.events.len();
        Ok(())
    }

    pub(crate) fn all_events_validated(&self) -> bool {
        self.validated_event_count == self.events.len()
    }

    pub(crate) fn push(&mut self, event: StreamEvent) -> CaptainResult<()> {
        let event_bytes = estimated_event_bytes(&event);
        let next_count = self.events.len().saturating_add(1);
        let next_bytes = self.estimated_bytes.saturating_add(event_bytes);
        if next_count > MAX_HELD_STREAM_EVENTS || next_bytes > MAX_HELD_STREAM_BYTES {
            return Err(CaptainError::LlmDriver(format!(
                "held stream exceeded its bounded delivery buffer ({next_count} events, {next_bytes} estimated bytes)"
            )));
        }
        self.events.push(event);
        self.estimated_bytes = next_bytes;
        Ok(())
    }

    pub(crate) fn append(&mut self, mut other: Self) -> CaptainResult<()> {
        let checkpoint = self.checkpoint();
        for event in other.events.drain(..) {
            if let Err(error) = self.push(event) {
                self.rollback(checkpoint);
                return Err(error);
            }
        }
        Ok(())
    }

    pub(crate) fn rollback(&mut self, checkpoint: StreamDeliveryCheckpoint) {
        self.events.truncate(checkpoint.event_count);
        self.estimated_bytes = checkpoint.estimated_bytes;
        self.validated_event_count = checkpoint.validated_event_count;
    }

    pub(crate) fn discard(&mut self) {
        self.events.clear();
        self.estimated_bytes = 0;
        self.validated_event_count = 0;
    }

    pub(crate) async fn release(&mut self, tx: &mpsc::Sender<StreamEvent>) -> CaptainResult<()> {
        if !self.all_events_validated() {
            return Err(CaptainError::LlmDriver(
                "held stream release refused because one or more segments are uncertified"
                    .to_string(),
            ));
        }
        let events = std::mem::take(&mut self.events);
        self.estimated_bytes = 0;
        self.validated_event_count = 0;
        for event in events {
            if tx.send(event).await.is_err() {
                break;
            }
        }
        Ok(())
    }

    pub(crate) async fn replace_with_final(
        &mut self,
        tx: &mpsc::Sender<StreamEvent>,
        text: &str,
        stop_reason: StopReason,
        usage: TokenUsage,
    ) {
        self.discard();
        if !text.is_empty()
            && tx
                .send(StreamEvent::TextDelta {
                    text: text.to_string(),
                })
                .await
                .is_err()
        {
            return;
        }
        let _ = tx
            .send(StreamEvent::ContentComplete { stop_reason, usage })
            .await;
    }
}

fn estimated_event_bytes(event: &StreamEvent) -> usize {
    const EVENT_OVERHEAD: usize = 64;
    EVENT_OVERHEAD
        + match event {
            StreamEvent::TextDelta { text }
            | StreamEvent::ToolInputDelta { text }
            | StreamEvent::ThinkingDelta { text } => text.len(),
            StreamEvent::ToolUseStart { id, name } => id.len().saturating_add(name.len()),
            StreamEvent::ToolUseEnd { id, name, input } => id
                .len()
                .saturating_add(name.len())
                .saturating_add(input.to_string().len()),
            StreamEvent::ContentComplete { .. } => 64,
            StreamEvent::PhaseChange { phase, detail } => phase
                .len()
                .saturating_add(detail.as_ref().map_or(0, String::len)),
            StreamEvent::CompactionProgress { .. } => 256,
            StreamEvent::ToolExecutionResult {
                tool_use_id,
                name,
                result_preview,
                ..
            } => tool_use_id
                .len()
                .saturating_add(name.len())
                .saturating_add(result_preview.len()),
            StreamEvent::ToolOutputDelta {
                tool_use_id,
                stream,
                chunk,
            } => tool_use_id
                .len()
                .saturating_add(stream.len())
                .saturating_add(chunk.len()),
            StreamEvent::IntermediateMessage { content } => content.len(),
            StreamEvent::SuggestedReplies { options } => {
                options.iter().map(String::len).sum::<usize>()
            }
            StreamEvent::AskUser { question, options } => question.len().saturating_add(
                options
                    .as_ref()
                    .map_or(0, |items| items.iter().map(String::len).sum()),
            ),
            StreamEvent::UserResponse { content } => content.len(),
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn drain(mut rx: mpsc::Receiver<StreamEvent>) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }
        events
    }

    #[tokio::test]
    async fn release_preserves_every_event_in_order() {
        let mut buffer = StreamDeliveryBuffer::default();
        let checkpoint = buffer.checkpoint();
        buffer
            .push(StreamEvent::TextDelta {
                text: "one".to_string(),
            })
            .unwrap();
        buffer
            .push(StreamEvent::ThinkingDelta {
                text: "two".to_string(),
            })
            .unwrap();
        buffer
            .push(StreamEvent::TextDelta {
                text: "three".to_string(),
            })
            .unwrap();
        buffer
            .push(StreamEvent::ContentComplete {
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage::default(),
            })
            .unwrap();
        buffer
            .validate_segment(checkpoint, "onethree", StopReason::EndTurn)
            .unwrap();
        let (tx, rx) = mpsc::channel(5);

        buffer.release(&tx).await.unwrap();
        drop(tx);
        let events = drain(rx).await;

        assert_eq!(events.len(), 4);
        assert!(matches!(&events[0], StreamEvent::TextDelta { text } if text == "one"));
        assert!(matches!(&events[1], StreamEvent::ThinkingDelta { text } if text == "two"));
        assert!(matches!(&events[2], StreamEvent::TextDelta { text } if text == "three"));
        assert!(matches!(
            &events[3],
            StreamEvent::ContentComplete {
                stop_reason: StopReason::EndTurn,
                ..
            }
        ));
        assert!(buffer.is_empty());
    }

    #[test]
    fn rollback_rejects_only_events_after_the_checkpoint() {
        let mut buffer = StreamDeliveryBuffer::default();
        buffer
            .push(StreamEvent::TextDelta {
                text: "keep".to_string(),
            })
            .unwrap();
        let checkpoint = buffer.checkpoint();
        buffer
            .push(StreamEvent::TextDelta {
                text: "reject".to_string(),
            })
            .unwrap();

        buffer.rollback(checkpoint);

        assert_eq!(buffer.events.len(), 1);
        assert!(matches!(&buffer.events[0], StreamEvent::TextDelta { text } if text == "keep"));
    }

    #[tokio::test]
    async fn replacement_discards_draft_and_emits_one_terminal_sequence() {
        let mut buffer = StreamDeliveryBuffer::default();
        buffer
            .push(StreamEvent::TextDelta {
                text: "unverified draft".to_string(),
            })
            .unwrap();
        buffer
            .push(StreamEvent::ContentComplete {
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage::default(),
            })
            .unwrap();
        let (tx, rx) = mpsc::channel(4);

        buffer
            .replace_with_final(
                &tx,
                "verification incomplete",
                StopReason::EndTurn,
                TokenUsage {
                    output_tokens: 7,
                    ..Default::default()
                },
            )
            .await;
        drop(tx);
        let events = drain(rx).await;

        assert_eq!(events.len(), 2);
        assert!(
            matches!(&events[0], StreamEvent::TextDelta { text } if text == "verification incomplete")
        );
        assert!(matches!(
            &events[1],
            StreamEvent::ContentComplete { stop_reason: StopReason::EndTurn, usage }
                if usage.output_tokens == 7
        ));
    }

    #[test]
    fn bounded_buffer_fails_closed_without_retaining_the_overflow_event() {
        let mut buffer = StreamDeliveryBuffer::default();
        let oversized = "x".repeat(MAX_HELD_STREAM_BYTES);

        let error = buffer
            .push(StreamEvent::TextDelta { text: oversized })
            .unwrap_err();

        assert!(error.to_string().contains("bounded delivery buffer"));
        assert!(buffer.is_empty());
    }

    #[test]
    fn continuation_segments_require_exact_text_and_one_terminal_each() {
        let mut buffer = StreamDeliveryBuffer::default();
        let first = buffer.checkpoint();
        buffer
            .push(StreamEvent::TextDelta {
                text: "partial".to_string(),
            })
            .unwrap();
        buffer
            .push(StreamEvent::ContentComplete {
                stop_reason: StopReason::MaxTokens,
                usage: TokenUsage::default(),
            })
            .unwrap();
        buffer
            .validate_segment(first, "partial", StopReason::MaxTokens)
            .unwrap();

        let second = buffer.checkpoint();
        buffer
            .push(StreamEvent::TextDelta {
                text: " final".to_string(),
            })
            .unwrap();
        buffer
            .push(StreamEvent::ContentComplete {
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage::default(),
            })
            .unwrap();
        buffer
            .validate_segment(second, " final", StopReason::EndTurn)
            .unwrap();

        assert!(buffer.all_events_validated());
    }

    #[test]
    fn duplicate_terminal_event_is_rejected() {
        let mut buffer = StreamDeliveryBuffer::default();
        let checkpoint = buffer.checkpoint();
        for _ in 0..2 {
            buffer
                .push(StreamEvent::ContentComplete {
                    stop_reason: StopReason::EndTurn,
                    usage: TokenUsage::default(),
                })
                .unwrap();
        }

        let error = buffer
            .validate_segment(checkpoint, "", StopReason::EndTurn)
            .unwrap_err();

        assert!(error.to_string().contains("delivery refused"));
        assert!(!buffer.all_events_validated());
    }

    #[tokio::test]
    async fn disconnected_receiver_does_not_block_or_retain_delivery() {
        let mut buffer = StreamDeliveryBuffer::default();
        let checkpoint = buffer.checkpoint();
        buffer
            .push(StreamEvent::TextDelta {
                text: "done".to_string(),
            })
            .unwrap();
        buffer
            .push(StreamEvent::ContentComplete {
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage::default(),
            })
            .unwrap();
        buffer
            .validate_segment(checkpoint, "done", StopReason::EndTurn)
            .unwrap();
        let (tx, rx) = mpsc::channel(2);
        drop(rx);

        buffer.release(&tx).await.unwrap();

        assert!(buffer.is_empty());
    }

    #[tokio::test]
    async fn uncertified_segment_can_never_be_released() {
        let mut buffer = StreamDeliveryBuffer::default();
        buffer
            .push(StreamEvent::TextDelta {
                text: "draft".to_string(),
            })
            .unwrap();
        let (tx, mut rx) = mpsc::channel(2);

        let error = buffer.release(&tx).await.unwrap_err();

        assert!(error.to_string().contains("uncertified"));
        assert!(matches!(
            rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        assert!(!buffer.is_empty());
    }
}
