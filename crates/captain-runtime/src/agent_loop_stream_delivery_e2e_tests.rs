use crate::agent_loop::{run_agent_loop_streaming, with_turn_token_budget};
use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent};
use async_trait::async_trait;
use captain_types::agent::AgentManifest;
use captain_types::message::{ContentBlock, StopReason, TokenUsage};
use captain_types::tool::ToolCall;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

struct VerificationSequenceDriver {
    calls: AtomicUsize,
}

struct ContinuationDriver {
    calls: AtomicUsize,
}

impl VerificationSequenceDriver {
    fn response_for_call(call: usize) -> CompletionResponse {
        match call {
            0 => tool_response(
                "write-1",
                "file_write",
                serde_json::json!({"path": "notes.txt", "content": "done"}),
            ),
            1 => text_response("done", StopReason::EndTurn),
            2 => tool_response(
                "read-1",
                "file_read",
                serde_json::json!({"path": "notes.txt"}),
            ),
            _ => text_response("verified delivery", StopReason::EndTurn),
        }
    }
}

#[async_trait]
impl LlmDriver for VerificationSequenceDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(Self::response_for_call(
            self.calls.fetch_add(1, Ordering::SeqCst),
        ))
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let response = self.complete(request).await?;
        for block in &response.content {
            match block {
                ContentBlock::Text { text, .. } if !text.is_empty() => {
                    tx.send(StreamEvent::TextDelta { text: text.clone() })
                        .await
                        .unwrap();
                }
                ContentBlock::ToolUse {
                    id, name, input, ..
                } => {
                    tx.send(StreamEvent::ToolUseStart {
                        id: id.clone(),
                        name: name.clone(),
                    })
                    .await
                    .unwrap();
                    tx.send(StreamEvent::ToolInputDelta {
                        text: input.to_string(),
                    })
                    .await
                    .unwrap();
                    tx.send(StreamEvent::ToolUseEnd {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                    })
                    .await
                    .unwrap();
                }
                _ => {}
            }
        }
        tx.send(StreamEvent::ContentComplete {
            stop_reason: response.stop_reason,
            usage: response.usage,
        })
        .await
        .unwrap();
        Ok(response)
    }
}

#[async_trait]
impl LlmDriver for ContinuationDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(if call == 0 {
            text_response("part-1", StopReason::MaxTokens)
        } else {
            text_response("part-2", StopReason::EndTurn)
        })
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let response = self.complete(request).await?;
        tx.send(StreamEvent::TextDelta {
            text: response.text(),
        })
        .await
        .unwrap();
        tx.send(StreamEvent::ContentComplete {
            stop_reason: response.stop_reason,
            usage: response.usage,
        })
        .await
        .unwrap();
        Ok(response)
    }
}

fn tool_response(id: &str, name: &str, input: serde_json::Value) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::ToolUse {
            id: id.to_string(),
            name: name.to_string(),
            input: input.clone(),
            provider_metadata: None,
        }],
        stop_reason: StopReason::ToolUse,
        tool_calls: vec![ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            input,
        }],
        usage: TokenUsage {
            input_tokens: 10,
            output_tokens: 2,
            ..Default::default()
        },
    }
}

fn text_response(text: &str, stop_reason: StopReason) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::Text {
            text: text.to_string(),
            provider_metadata: None,
        }],
        stop_reason,
        tool_calls: Vec::new(),
        usage: TokenUsage {
            input_tokens: 10,
            output_tokens: 2,
            ..Default::default()
        },
    }
}

#[tokio::test]
async fn unverified_stream_draft_is_rejected_before_corrected_delivery() {
    let workspace = tempfile::tempdir().unwrap();
    let memory = captain_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = captain_memory::session::Session {
        id: captain_types::agent::SessionId::new(),
        agent_id: captain_types::agent::AgentId::new(),
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
    };
    let manifest = AgentManifest {
        name: "stream-verifier".to_string(),
        ..Default::default()
    };
    let tools = crate::tools::file_tool_definitions()
        .into_iter()
        .filter(|tool| matches!(tool.name.as_str(), "file_write" | "file_read"))
        .collect::<Vec<_>>();
    let driver: Arc<dyn LlmDriver> = Arc::new(VerificationSequenceDriver {
        calls: AtomicUsize::new(0),
    });
    let (tx, mut rx) = mpsc::channel(64);

    let result = run_agent_loop_streaming(
        &manifest,
        "Write notes.txt and verify it before delivery.",
        &mut session,
        &memory,
        driver,
        &tools,
        None,
        tx,
        None,
        None,
        None,
        None,
        None,
        Some(workspace.path()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        events.push(event);
    }
    let visible_text = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    let tool_end_names = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::ToolUseEnd { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(result.response, "verified delivery");
    assert_eq!(visible_text, "verified delivery");
    assert_eq!(tool_end_names, ["file_write", "file_read"]);
    assert_eq!(result.tool_calls.len(), 2);
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("notes.txt")).unwrap(),
        "done"
    );
}

#[tokio::test]
async fn held_continuations_are_released_in_order_without_loss_or_duplication() {
    let memory = captain_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = captain_memory::session::Session {
        id: captain_types::agent::SessionId::new(),
        agent_id: captain_types::agent::AgentId::new(),
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
    };
    let manifest = AgentManifest {
        name: "stream-continuation".to_string(),
        ..Default::default()
    };
    let driver: Arc<dyn LlmDriver> = Arc::new(ContinuationDriver {
        calls: AtomicUsize::new(0),
    });
    let (tx, mut rx) = mpsc::channel(16);

    let result = with_turn_token_budget(
        Some(1_000_000),
        run_agent_loop_streaming(
            &manifest,
            "Return a long answer.",
            &mut session,
            &memory,
            driver,
            &[],
            None,
            tx,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ),
    )
    .await
    .unwrap();

    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        events.push(event);
    }
    let visible_text = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    let stops = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::ContentComplete { stop_reason, .. } => Some(*stop_reason),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(result.response, "part-2");
    assert_eq!(visible_text, "part-1part-2");
    assert_eq!(stops, [StopReason::MaxTokens, StopReason::EndTurn]);
}

#[tokio::test]
async fn budget_stop_replaces_held_tool_draft_before_any_side_effect() {
    let workspace = tempfile::tempdir().unwrap();
    let memory = captain_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = captain_memory::session::Session {
        id: captain_types::agent::SessionId::new(),
        agent_id: captain_types::agent::AgentId::new(),
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
    };
    let manifest = AgentManifest {
        name: "stream-budget".to_string(),
        ..Default::default()
    };
    let tools = crate::tools::file_tool_definitions()
        .into_iter()
        .filter(|tool| tool.name == "file_write")
        .collect::<Vec<_>>();
    let driver: Arc<dyn LlmDriver> = Arc::new(VerificationSequenceDriver {
        calls: AtomicUsize::new(0),
    });
    let (tx, mut rx) = mpsc::channel(16);

    let result = with_turn_token_budget(
        Some(1),
        run_agent_loop_streaming(
            &manifest,
            "Write notes.txt.",
            &mut session,
            &memory,
            driver,
            &tools,
            None,
            tx,
            None,
            None,
            None,
            None,
            None,
            Some(workspace.path()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ),
    )
    .await
    .unwrap();

    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        events.push(event);
    }

    assert!(result.response.contains("Budget de delegation atteint"));
    assert!(!workspace.path().join("notes.txt").exists());
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamEvent::ToolUseEnd { .. }))
            .count(),
        0
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamEvent::TextDelta { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamEvent::ContentComplete { .. }))
            .count(),
        1
    );
}
