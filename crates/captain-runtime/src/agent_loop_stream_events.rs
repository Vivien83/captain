use crate::context_overflow::RecoveryStage;
use crate::llm_driver::StreamEvent;
use captain_types::tool::{ToolCall, ToolResult};
use tokio::sync::mpsc;
use tracing::warn;

pub(crate) async fn send_phase_change_event(
    stream_tx: &mpsc::Sender<StreamEvent>,
    phase: &str,
    detail: Option<String>,
    disconnect_warning: Option<&str>,
) {
    if stream_tx
        .send(StreamEvent::PhaseChange {
            phase: phase.to_string(),
            detail,
        })
        .await
        .is_err()
    {
        if let Some(warning) = disconnect_warning {
            warn!("{warning}");
        }
    }
}

pub(crate) async fn send_context_recovery_phase_event(
    stream_tx: &mpsc::Sender<StreamEvent>,
    recovery: &RecoveryStage,
) {
    match recovery {
        RecoveryStage::None => {}
        RecoveryStage::FinalError => {
            send_phase_change_event(
                stream_tx,
                "context_warning",
                Some("Context overflow unrecoverable. Use /reset or /compact.".to_string()),
                Some("Stream consumer disconnected while sending context overflow warning"),
            )
            .await;
        }
        _ => {
            send_phase_change_event(
                stream_tx,
                "context_warning",
                Some(
                    "Older messages trimmed to stay within context limits. Use /compact for smarter summarization."
                        .to_string(),
                ),
                Some("Stream consumer disconnected while sending context trim warning"),
            )
            .await;
        }
    }
}

pub(crate) async fn send_tool_execution_result_event(
    stream_tx: &mpsc::Sender<StreamEvent>,
    agent_name: &str,
    tool_call: &ToolCall,
    result: &ToolResult,
    final_content: &str,
) {
    let preview: String = final_content.chars().take(300).collect();
    if stream_tx
        .send(StreamEvent::ToolExecutionResult {
            tool_use_id: result.tool_use_id.clone(),
            name: tool_call.name.clone(),
            result_preview: preview,
            is_error: result.is_error,
        })
        .await
        .is_err()
    {
        warn!(agent = %agent_name, "Stream consumer disconnected — continuing tool loop but will not stream further");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_call() -> ToolCall {
        ToolCall {
            id: "call-1".to_string(),
            name: "shell_exec".to_string(),
            input: serde_json::json!({"cmd": "pwd"}),
        }
    }

    fn tool_result(content: String, is_error: bool) -> ToolResult {
        ToolResult {
            tool_use_id: "call-1".to_string(),
            content,
            is_error,
        }
    }

    #[tokio::test]
    async fn sends_phase_change_event() {
        let (tx, mut rx) = mpsc::channel(1);

        send_phase_change_event(
            &tx,
            "context_warning",
            Some("trimmed".to_string()),
            Some("disconnect"),
        )
        .await;

        let event = rx.recv().await.expect("event");
        assert!(matches!(
            event,
            StreamEvent::PhaseChange {
                phase,
                detail: Some(detail),
            } if phase == "context_warning" && detail == "trimmed"
        ));
    }

    #[tokio::test]
    async fn dropped_stream_consumer_does_not_fail_phase_event() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);

        send_phase_change_event(&tx, "quota_exceeded", None, Some("disconnect")).await;
    }

    #[tokio::test]
    async fn context_recovery_none_sends_no_event() {
        let (tx, mut rx) = mpsc::channel(1);

        send_context_recovery_phase_event(&tx, &RecoveryStage::None).await;

        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn context_recovery_final_error_sends_unrecoverable_warning() {
        let (tx, mut rx) = mpsc::channel(1);

        send_context_recovery_phase_event(&tx, &RecoveryStage::FinalError).await;

        let event = rx.recv().await.expect("event");
        assert!(matches!(
            event,
            StreamEvent::PhaseChange {
                phase,
                detail: Some(detail),
            } if phase == "context_warning"
                && detail.contains("unrecoverable")
                && detail.contains("/reset")
        ));
    }

    #[tokio::test]
    async fn context_recovery_trim_sends_compaction_hint() {
        let (tx, mut rx) = mpsc::channel(1);

        send_context_recovery_phase_event(
            &tx,
            &RecoveryStage::ToolResultTruncation { truncated: 2 },
        )
        .await;

        let event = rx.recv().await.expect("event");
        assert!(matches!(
            event,
            StreamEvent::PhaseChange {
                phase,
                detail: Some(detail),
            } if phase == "context_warning"
                && detail.contains("Older messages trimmed")
                && detail.contains("/compact")
        ));
    }

    #[tokio::test]
    async fn dropped_consumer_does_not_fail_context_recovery_event() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);

        send_context_recovery_phase_event(&tx, &RecoveryStage::AutoCompaction { removed: 1 }).await;
    }

    #[tokio::test]
    async fn sends_tool_execution_result_event_with_capped_preview() {
        let (tx, mut rx) = mpsc::channel(1);
        let call = tool_call();
        let result = tool_result("raw".to_string(), true);
        let final_content = "x".repeat(400);

        send_tool_execution_result_event(&tx, "captain", &call, &result, &final_content).await;

        let event = rx.recv().await.expect("event");
        assert!(matches!(
            event,
            StreamEvent::ToolExecutionResult {
                tool_use_id,
                name,
                result_preview,
                is_error: true,
            } if tool_use_id == "call-1"
                && name == "shell_exec"
                && result_preview.chars().count() == 300
        ));
    }

    #[tokio::test]
    async fn dropped_stream_consumer_does_not_fail_loop() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let call = tool_call();
        let result = tool_result("raw".to_string(), false);

        send_tool_execution_result_event(&tx, "captain", &call, &result, "ok").await;
    }
}
