//! Bridge between the runtime's `StreamEvent` and chat channels (HS.3).
//!
//! `captain-runtime` emits a rich event vocabulary (TextDelta, ToolUseStart,
//! ToolInputDelta, ToolUseEnd, ThinkingDelta, ContentComplete,
//! IntermediateMessage, AskUser, …). Chat channels only need a small
//! subset to drive a live-rendered transcript. This module
//! is the translation layer.
//!
//! Layering:
//!   captain-runtime --(StreamEvent)--> captain-api/streaming_channels
//!                                          |
//!                                          v
//!                                    ChannelStreamEvent
//!                                          |
//!                                          v
//!                                    captain-channels::StreamConsumer
//!                                          |
//!                                          v
//!                                    StreamingChannelAdapter
//!                                          (Telegram, Discord, …)
//!
//! HS.3a (this commit) ships the mapping + the pump loop with unit tests.
//! Wiring the pump into `KernelBridgeAdapter::send_message` lands in HS.3b
//! so the runtime change is reversible on its own.

use captain_channels::stream_consumer::{
    ChannelStreamEvent, StreamConsumer, StreamingChannelAdapter,
};
use captain_runtime::llm_driver::StreamEvent;
use tokio::sync::mpsc::Receiver;

/// Maximum bytes of `result_preview` carried into a tool bubble.
/// The full result lives in the kernel session — the channel surface
/// just needs a glance.
pub const TOOL_RESULT_PREVIEW_BYTES: usize = 280;

/// Maximum bytes of the JSON input preview rendered next to a tool name
/// when the tool starts.
pub const TOOL_INPUT_PREVIEW_BYTES: usize = 200;

/// Translate one runtime `StreamEvent` to a `ChannelStreamEvent`.
///
/// Returns `None` for events the channel surface deliberately ignores
/// (thinking, phase changes, ask_user, raw stdout/stderr chunks, the per-
/// token tool input deltas, AND the bare ToolUseStart — see HS.6).
///
/// HS.6 — tool bubbles defer to `ToolUseEnd` instead of opening on
/// `ToolUseStart`. ToolUseStart fires before the input JSON is fully
/// streamed, so we wouldn't have a useful preview yet. Waiting for
/// ToolUseEnd lets us pick a meaningful slice (`shell_exec.command`,
/// `file_read.path`, etc.) via `tool_input_preview`. The visual cost
/// is negligible: ToolInputDelta arrives within a few hundred ms of
/// the start.
pub fn map_runtime_event(event: &StreamEvent) -> Option<ChannelStreamEvent> {
    match event {
        StreamEvent::TextDelta { text } => {
            if text.is_empty() {
                None
            } else {
                Some(ChannelStreamEvent::TextDelta(text.clone()))
            }
        }
        StreamEvent::ToolUseEnd { id, name, input } => Some(ChannelStreamEvent::ToolStart {
            tool_use_id: id.clone(),
            emoji: captain_runtime::agent_loop::tool_emoji(name).to_string(),
            name: name.clone(),
            input_preview: clip(
                &captain_runtime::agent_loop::tool_input_preview(name, input),
                TOOL_INPUT_PREVIEW_BYTES,
            ),
        }),
        StreamEvent::ToolExecutionResult {
            tool_use_id,
            name,
            result_preview,
            is_error,
        } => Some(ChannelStreamEvent::ToolEnd {
            tool_use_id: tool_use_id.clone(),
            emoji: captain_runtime::agent_loop::tool_emoji(name).to_string(),
            name: name.clone(),
            result_preview: clip(result_preview, TOOL_RESULT_PREVIEW_BYTES),
            is_error: *is_error,
        }),
        StreamEvent::IntermediateMessage { content } => {
            if content.trim().is_empty() {
                None
            } else {
                Some(ChannelStreamEvent::Commentary(content.clone()))
            }
        }
        StreamEvent::ToolOutputDelta {
            tool_use_id,
            stream: "progress",
            chunk,
        } => {
            let progress = chunk.trim();
            if progress.is_empty() {
                None
            } else {
                Some(ChannelStreamEvent::ToolProgress {
                    tool_use_id: tool_use_id.clone(),
                    chunk: progress.to_string(),
                })
            }
        }
        // Ignored on purpose:
        //   - ContentComplete: see HS.3a fix — the agent loop runs ≥2
        //     iterations and we'd close the pump too early.
        //   - ToolUseStart: see HS.6 — wait for ToolUseEnd to have the
        //     full input for a useful preview.
        //   - ToolInputDelta: per-token input deltas would flood edits.
        //   - ThinkingDelta: TUI-only; channels shouldn't leak the CoT.
        //   - ToolOutputDelta(stdout/stderr): raw terminal streams are a TUI
        //     feature. Only semantic "progress" ticks are surfaced here.
        StreamEvent::PhaseChange { phase, detail } if phase == "model_fallback" => detail
            .as_ref()
            .map(|text| text.trim())
            .filter(|text| !text.is_empty())
            .map(|text| ChannelStreamEvent::Commentary(text.to_string())),
        //   - PhaseChange / AskUser / UserResponse: orthogonal control
        //     events handled outside the streaming surface. model_fallback is
        //     the exception because fallback must never be silent.
        StreamEvent::ContentComplete { .. }
        | StreamEvent::ToolUseStart { .. }
        | StreamEvent::ToolInputDelta { .. }
        | StreamEvent::ThinkingDelta { .. }
        | StreamEvent::ToolOutputDelta { .. }
        | StreamEvent::PhaseChange { .. }
        | StreamEvent::AskUser { .. }
        | StreamEvent::UserResponse { .. } => None,
    }
}

/// Drain a runtime `StreamEvent` channel and drive a `StreamConsumer`
/// until the producer closes the receiver. The pump does NOT terminate
/// on `ContentComplete` — that event fires once per LLM iteration, and
/// a typical agent loop runs ≥2 iterations (one to call the tool,
/// another to interpret the result). Stopping early would lose every
/// `ToolExecutionResult` and the final assistant text.
///
/// Always issues a `Done` once the channel closes so the cursor never
/// lingers on the last edited message.
pub async fn pump_stream_to_channel<A: StreamingChannelAdapter>(
    rx: Receiver<StreamEvent>,
    consumer: &mut StreamConsumer<A>,
) -> Result<(), String> {
    pump_stream_to_channel_with_observer(rx, consumer, |_| {}).await
}

/// Same as `pump_stream_to_channel`, with a synchronous observer for
/// operator metrics. The observer sees every runtime event before channel
/// filtering, but it must never perform I/O or block the pump.
pub async fn pump_stream_to_channel_with_observer<A, F>(
    mut rx: Receiver<StreamEvent>,
    consumer: &mut StreamConsumer<A>,
    mut observe: F,
) -> Result<(), String>
where
    A: StreamingChannelAdapter,
    F: FnMut(&StreamEvent),
{
    while let Some(event) = rx.recv().await {
        observe(&event);
        if let Some(channel_event) = map_runtime_event(&event) {
            consumer.handle_event(channel_event).await?;
        }
    }
    consumer
        .handle_event(ChannelStreamEvent::Done)
        .await
        .map_err(|e| format!("pump_stream_to_channel: final Done failed: {e}"))
}

/// Truncate `s` to at most `max_bytes`, on a UTF-8 char boundary,
/// appending `…` when something was dropped. Used to keep tool
/// previews short.
fn clip(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut cut = max_bytes;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut out = String::with_capacity(cut + 1);
    out.push_str(&s[..cut]);
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use captain_types::message::{StopReason, TokenUsage};
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    // ---------- map_runtime_event ----------

    #[test]
    fn text_delta_passes_through() {
        let mapped = map_runtime_event(&StreamEvent::TextDelta { text: "hi".into() });
        assert_eq!(mapped, Some(ChannelStreamEvent::TextDelta("hi".into())));
    }

    #[test]
    fn empty_text_delta_is_dropped() {
        // Some drivers emit empty deltas at boundaries — they would
        // open an empty live message and immediately edit it. Drop them.
        let mapped = map_runtime_event(&StreamEvent::TextDelta { text: "".into() });
        assert!(mapped.is_none());
    }

    #[test]
    fn tool_use_start_is_dropped_so_tool_use_end_carries_the_input() {
        // HS.6 — bare ToolUseStart fires before the input is fully
        // streamed; we wait for ToolUseEnd to render a useful preview.
        let mapped = map_runtime_event(&StreamEvent::ToolUseStart {
            id: "t1".into(),
            name: "web_search".into(),
        });
        assert!(
            mapped.is_none(),
            "ToolUseStart must be dropped so ToolUseEnd can deliver the full input"
        );
    }

    #[test]
    fn tool_use_end_opens_tool_bubble_with_emoji_and_preview() {
        let mapped = map_runtime_event(&StreamEvent::ToolUseEnd {
            id: "t1".into(),
            name: "shell_exec".into(),
            input: serde_json::json!({"command": "curl -sSf https://example.com"}),
        });
        match mapped {
            Some(ChannelStreamEvent::ToolStart {
                tool_use_id,
                emoji,
                name,
                input_preview,
            }) => {
                assert_eq!(tool_use_id, "t1");
                assert_eq!(name, "shell_exec");
                assert_eq!(emoji, "💻", "shell_exec must map to the laptop emoji");
                assert!(
                    input_preview.starts_with("curl -sSf"),
                    "preview must extract the command field, got {input_preview:?}"
                );
            }
            other => panic!("expected ToolStart with emoji + preview, got {other:?}"),
        }
    }

    #[test]
    fn tool_execution_result_emits_tool_end_with_clip() {
        let big = "x".repeat(TOOL_RESULT_PREVIEW_BYTES + 50);
        let mapped = map_runtime_event(&StreamEvent::ToolExecutionResult {
            tool_use_id: "t1".into(),
            name: "shell_exec".into(),
            result_preview: big,
            is_error: false,
        });
        match mapped {
            Some(ChannelStreamEvent::ToolEnd {
                tool_use_id,
                emoji,
                name,
                result_preview,
                is_error,
            }) => {
                assert_eq!(tool_use_id, "t1");
                assert_eq!(name, "shell_exec");
                assert_eq!(emoji, "💻");
                assert!(!is_error);
                assert!(
                    result_preview.ends_with('…'),
                    "long result must be clipped with an ellipsis: {result_preview:?}"
                );
                assert!(result_preview.len() <= TOOL_RESULT_PREVIEW_BYTES + 4);
            }
            other => panic!("expected ToolEnd, got {other:?}"),
        }
    }

    #[test]
    fn tool_execution_result_preserves_error_flag() {
        let mapped = map_runtime_event(&StreamEvent::ToolExecutionResult {
            tool_use_id: "t1".into(),
            name: "web_fetch".into(),
            result_preview: "404 Not Found".into(),
            is_error: true,
        });
        match mapped {
            Some(ChannelStreamEvent::ToolEnd { is_error, .. }) => assert!(is_error),
            other => panic!("expected ToolEnd with is_error, got {other:?}"),
        }
    }

    #[test]
    fn intermediate_message_becomes_commentary() {
        let mapped = map_runtime_event(&StreamEvent::IntermediateMessage {
            content: "réfléchis : voilà ma piste".into(),
        });
        match mapped {
            Some(ChannelStreamEvent::Commentary(s)) => assert!(s.starts_with("réfléchis")),
            other => panic!("expected Commentary, got {other:?}"),
        }
    }

    #[test]
    fn empty_intermediate_message_is_dropped() {
        let mapped = map_runtime_event(&StreamEvent::IntermediateMessage {
            content: "   \n".into(),
        });
        assert!(mapped.is_none(), "whitespace-only commentary is noise");
    }

    #[test]
    fn content_complete_is_dropped_so_multi_iter_loops_keep_streaming() {
        // ContentComplete fires once per LLM iteration. The agent loop
        // typically runs ≥2 iterations (call tool → interpret result),
        // so treating ContentComplete as `Done` would close the pump
        // after the first turn and lose every subsequent ToolExecution
        // Result + final assistant text. The pump finalises on
        // `rx.recv().await -> None` instead.
        let mapped = map_runtime_event(&StreamEvent::ContentComplete {
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage::default(),
        });
        assert!(
            mapped.is_none(),
            "ContentComplete must NOT translate to Done — multi-iteration \
             agent loops would lose events past the first iteration"
        );
    }

    #[test]
    fn thinking_delta_is_dropped() {
        let mapped = map_runtime_event(&StreamEvent::ThinkingDelta {
            text: "let me think".into(),
        });
        assert!(
            mapped.is_none(),
            "ThinkingDelta is TUI-only; channels must not leak the CoT"
        );
    }

    #[test]
    fn tool_input_delta_is_dropped() {
        let mapped = map_runtime_event(&StreamEvent::ToolInputDelta {
            text: "{\"q\":\"".into(),
        });
        assert!(mapped.is_none());
    }

    #[test]
    fn tool_use_end_carries_input_into_preview() {
        // HS.6 inversion: ToolUseEnd is now THE event that opens the
        // bubble (because it has the full input). Asserts the web_search
        // emoji is picked up and the preview extracts the `query` field.
        let mapped = map_runtime_event(&StreamEvent::ToolUseEnd {
            id: "t1".into(),
            name: "web_search".into(),
            input: serde_json::json!({"query": "rust async tokio"}),
        });
        match mapped {
            Some(ChannelStreamEvent::ToolStart {
                tool_use_id,
                emoji,
                name,
                input_preview,
            }) => {
                assert_eq!(tool_use_id, "t1");
                assert_eq!(emoji, "🌐");
                assert_eq!(name, "web_search");
                assert_eq!(input_preview, "rust async tokio");
            }
            other => panic!("expected ToolStart, got {other:?}"),
        }
    }

    #[test]
    fn raw_tool_output_delta_is_dropped() {
        let mapped = map_runtime_event(&StreamEvent::ToolOutputDelta {
            tool_use_id: "t1".into(),
            stream: "stdout",
            chunk: "compiling…".into(),
        });
        assert!(
            mapped.is_none(),
            "stdout deltas would flood Telegram edits — TUI feature only"
        );
    }

    #[test]
    fn progress_tool_output_delta_becomes_tool_progress() {
        let mapped = map_runtime_event(&StreamEvent::ToolOutputDelta {
            tool_use_id: "t1".into(),
            stream: "progress",
            chunk: "Frame 7/30 décrite\n".into(),
        });
        assert_eq!(
            mapped,
            Some(ChannelStreamEvent::ToolProgress {
                tool_use_id: "t1".into(),
                chunk: "Frame 7/30 décrite".into(),
            })
        );
    }

    #[test]
    fn phase_change_is_dropped() {
        let mapped = map_runtime_event(&StreamEvent::PhaseChange {
            phase: "thinking".into(),
            detail: None,
        });
        assert!(mapped.is_none());
    }

    #[test]
    fn model_fallback_phase_is_visible_commentary() {
        let detail = "⚠️ fallback explicite. Target: openrouter/model. Reason: rate_limited. Timestamp: 2026-05-15T12:00:00Z.";
        let mapped = map_runtime_event(&StreamEvent::PhaseChange {
            phase: "model_fallback".into(),
            detail: Some(detail.into()),
        });
        assert_eq!(mapped, Some(ChannelStreamEvent::Commentary(detail.into())));
    }

    #[test]
    fn empty_model_fallback_phase_is_dropped() {
        let mapped = map_runtime_event(&StreamEvent::PhaseChange {
            phase: "model_fallback".into(),
            detail: Some("   ".into()),
        });
        assert!(mapped.is_none());
    }

    // ---------- pump_stream_to_channel ----------

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum MockCall {
        SendNew(String),
        Edit(String, String),
    }

    #[derive(Clone, Default)]
    struct MockAdapter {
        calls: Arc<Mutex<Vec<MockCall>>>,
        next_id: Arc<Mutex<usize>>,
    }

    impl MockAdapter {
        fn new() -> Self {
            Self::default()
        }
        fn calls(&self) -> Vec<MockCall> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl StreamingChannelAdapter for MockAdapter {
        async fn send_new_message(&self, text: &str) -> Result<String, String> {
            let mut id = self.next_id.lock().unwrap();
            let mid = format!("mid_{id}");
            *id += 1;
            self.calls
                .lock()
                .unwrap()
                .push(MockCall::SendNew(text.to_string()));
            Ok(mid)
        }
        async fn edit_message(&self, message_id: &str, text: &str) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push(MockCall::Edit(message_id.to_string(), text.to_string()));
            Ok(())
        }
    }

    fn fast_consumer(adapter: MockAdapter) -> StreamConsumer<MockAdapter> {
        use captain_channels::stream_consumer::StreamConsumerConfig;
        use std::time::Duration;
        StreamConsumer::new(
            adapter,
            StreamConsumerConfig {
                edit_interval: Duration::from_secs(0),
                buffer_threshold: 1,
                cursor: captain_channels::stream_consumer::STREAM_CURSOR,
                max_message_bytes: 0,
            },
        )
    }

    #[tokio::test]
    async fn pump_full_text_then_done() {
        let adapter = MockAdapter::new();
        let mut consumer = fast_consumer(adapter.clone());
        let (tx, rx) = mpsc::channel(8);
        tx.send(StreamEvent::TextDelta {
            text: "hello ".into(),
        })
        .await
        .unwrap();
        tx.send(StreamEvent::TextDelta {
            text: "world".into(),
        })
        .await
        .unwrap();
        tx.send(StreamEvent::ContentComplete {
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage::default(),
        })
        .await
        .unwrap();
        drop(tx);

        pump_stream_to_channel(rx, &mut consumer).await.unwrap();

        let calls = adapter.calls();
        let final_call = calls.last().expect("at least one call");
        match final_call {
            MockCall::Edit(_, body) => {
                assert_eq!(
                    body, "hello world",
                    "Done must finalise without the streaming cursor"
                );
            }
            other => panic!("expected final Edit, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pump_text_tool_text_renders_three_messages() {
        // Reproduces the headline "human" UX: text → tool bubble → text.
        let adapter = MockAdapter::new();
        let mut consumer = fast_consumer(adapter.clone());
        let (tx, rx) = mpsc::channel(16);
        tx.send(StreamEvent::TextDelta {
            text: "let me check".into(),
        })
        .await
        .unwrap();
        // HS.6: ToolUseStart is filtered; the bubble opens on ToolUseEnd.
        tx.send(StreamEvent::ToolUseEnd {
            id: "t1".into(),
            name: "web_search".into(),
            input: serde_json::json!({"query": "rust"}),
        })
        .await
        .unwrap();
        tx.send(StreamEvent::ToolExecutionResult {
            tool_use_id: "t1".into(),
            name: "web_search".into(),
            result_preview: "5 hits".into(),
            is_error: false,
        })
        .await
        .unwrap();
        tx.send(StreamEvent::TextDelta {
            text: "voilà la réponse".into(),
        })
        .await
        .unwrap();
        tx.send(StreamEvent::ContentComplete {
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage::default(),
        })
        .await
        .unwrap();
        drop(tx);

        pump_stream_to_channel(rx, &mut consumer).await.unwrap();

        let calls = adapter.calls();
        // HS.10 — tool start + tool end now share a single bubble
        // (one send + one edit), so we expect 3 sends total: intro
        // live, tool bubble, post-tool live. The result is appended
        // via edit_message on the same tool bubble.
        let send_count = calls
            .iter()
            .filter(|c| matches!(c, MockCall::SendNew(_)))
            .count();
        assert_eq!(
            send_count, 3,
            "expected 3 sends (intro live, tool bubble, post-tool live) — \
             tool start+end share one bubble post-HS.10. got {calls:?}"
        );
        let tool_edit_present = calls.iter().any(|c| {
            matches!(c, MockCall::Edit(_, body) if body.contains("web_search") && body.contains("5 hits"))
        });
        assert!(
            tool_edit_present,
            "the tool bubble must be edited in place to show the result"
        );
        match calls.last().unwrap() {
            MockCall::Edit(_, body) => {
                assert!(
                    !body.contains('▉'),
                    "final edit must strip the cursor: {body:?}"
                );
                assert!(body.contains("voilà"));
            }
            other => panic!("expected final edit, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pump_forces_done_when_producer_closes_silently() {
        // A producer that drops without emitting ContentComplete must
        // still leave the message in a finalised state — otherwise the
        // cursor lingers forever.
        let adapter = MockAdapter::new();
        let mut consumer = fast_consumer(adapter.clone());
        let (tx, rx) = mpsc::channel(4);
        tx.send(StreamEvent::TextDelta {
            text: "partial".into(),
        })
        .await
        .unwrap();
        drop(tx);

        pump_stream_to_channel(rx, &mut consumer).await.unwrap();

        let calls = adapter.calls();
        let last = calls.last().unwrap();
        match last {
            MockCall::Edit(_, body) => {
                assert_eq!(body, "partial", "synthetic Done must strip cursor")
            }
            other => panic!("expected final edit on synthetic Done, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pump_ignores_thinking_and_phase_events() {
        let adapter = MockAdapter::new();
        let mut consumer = fast_consumer(adapter.clone());
        let (tx, rx) = mpsc::channel(8);
        tx.send(StreamEvent::ThinkingDelta {
            text: "thinking out loud".into(),
        })
        .await
        .unwrap();
        tx.send(StreamEvent::PhaseChange {
            phase: "tool_use".into(),
            detail: Some("web_search".into()),
        })
        .await
        .unwrap();
        tx.send(StreamEvent::ContentComplete {
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage::default(),
        })
        .await
        .unwrap();
        drop(tx);

        pump_stream_to_channel(rx, &mut consumer).await.unwrap();
        assert!(
            adapter.calls().is_empty(),
            "filtered events must not produce channel calls: {:?}",
            adapter.calls()
        );
    }

    #[test]
    fn clip_keeps_short_strings_intact() {
        assert_eq!(clip("hello", 100), "hello");
    }

    #[test]
    fn clip_truncates_with_ellipsis() {
        let s = "a".repeat(50);
        let clipped = clip(&s, 10);
        assert!(clipped.ends_with('…'));
        assert_eq!(&clipped[..10], &"a".repeat(10));
    }

    #[test]
    fn clip_respects_utf8_char_boundary() {
        // 'é' is 2 bytes; cutting at the middle byte must roll back.
        let s = "café"; // 5 bytes total: c(1) a(1) f(1) é(2)
        let clipped = clip(s, 4);
        // Cut would land in the middle of 'é' → roll back to byte 3.
        assert_eq!(clipped, "caf…");
    }
}
