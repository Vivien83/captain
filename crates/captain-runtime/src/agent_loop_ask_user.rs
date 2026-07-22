use crate::agent_loop::ToolCallRecord;
use crate::llm_driver::StreamEvent;
use captain_types::message::ContentBlock;
use captain_types::tool::ToolCall;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{info, warn};

pub(crate) async fn try_handle_ask_user_tool_call(
    agent_name: &str,
    tool_call: &ToolCall,
    stream_tx: &mpsc::Sender<StreamEvent>,
    user_input_rx: Option<&Arc<tokio::sync::Mutex<mpsc::Receiver<String>>>>,
    tool_calls_recorded: &mut Vec<ToolCallRecord>,
    tool_result_blocks: &mut Vec<ContentBlock>,
) -> bool {
    if tool_call.name != "ask_user" {
        return false;
    }

    crate::workflow_learning_runtime::record_tool_started(
        &tool_call.id,
        &tool_call.name,
        &tool_call.input,
    );

    let question = tool_call
        .input
        .get("question")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let options = ask_user_options(tool_call);

    info!(agent = %agent_name, question = %question, options = ?options, "ask_user: waiting for user response");
    let _ = stream_tx
        .send(StreamEvent::AskUser {
            question: question.clone(),
            options: options.clone(),
        })
        .await;

    let answer = wait_for_ask_user_answer(agent_name, user_input_rx).await;

    // Emit a UserResponse stream event for real answers only — timeout/
    // unsupported-context placeholders aren't something a human typed, and
    // shouldn't be persisted to the timeline as if they were (same prefix
    // check as ask_user_result_preview below, kept separate since one
    // returns a bool and the other a display string).
    if !answer.starts_with("[No response") && !answer.starts_with("[ask_user not") {
        let _ = stream_tx
            .send(StreamEvent::UserResponse {
                content: answer.clone(),
            })
            .await;
    }

    send_ask_user_tool_result_event(agent_name, tool_call, stream_tx, &answer);

    tool_calls_recorded.push(ToolCallRecord {
        tool_name: "ask_user".to_string(),
        reason: "Ask the user for missing context before continuing.".to_string(),
        is_error: false,
        duration_ms: 0,
        input_summary: question,
        output_summary: answer.clone(),
    });
    let response_unavailable =
        answer.starts_with("[No response") || answer.starts_with("[ask_user not");
    tool_result_blocks.push(ContentBlock::ToolResult {
        tool_use_id: tool_call.id.clone(),
        tool_name: "ask_user".to_string(),
        content: answer,
        is_error: false,
    });
    crate::workflow_learning_runtime::record_tool_finished(
        &tool_call.id,
        &tool_call.name,
        response_unavailable,
        0,
        if response_unavailable {
            "user_response_unavailable"
        } else {
            "user_response_received"
        },
    );

    true
}

fn send_ask_user_tool_result_event(
    agent_name: &str,
    tool_call: &ToolCall,
    stream_tx: &mpsc::Sender<StreamEvent>,
    answer: &str,
) {
    let result_preview = ask_user_result_preview(answer);
    if stream_tx
        .try_send(StreamEvent::ToolExecutionResult {
            tool_use_id: tool_call.id.clone(),
            name: "ask_user".to_string(),
            result_preview,
            is_error: false,
        })
        .is_err()
    {
        warn!(
            agent = %agent_name,
            tool_use_id = %tool_call.id,
            "ask_user: stream consumer did not accept tool completion event"
        );
    }
}

fn ask_user_result_preview(answer: &str) -> String {
    if answer.starts_with("[No response") || answer.starts_with("[ask_user not") {
        answer.to_string()
    } else {
        "User response received.".to_string()
    }
}

fn ask_user_options(tool_call: &ToolCall) -> Option<Vec<String>> {
    tool_call.input.get("options").and_then(|v| {
        v.as_array().map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
    })
}

async fn wait_for_ask_user_answer(
    agent_name: &str,
    user_input_rx: Option<&Arc<tokio::sync::Mutex<mpsc::Receiver<String>>>>,
) -> String {
    if let Some(rx) = user_input_rx {
        return match tokio::time::timeout(Duration::from_secs(300), rx.lock().await.recv()).await {
            Ok(Some(resp)) => {
                info!(agent = %agent_name, answer = %resp, "ask_user: user responded");
                resp
            }
            Ok(None) => {
                warn!(agent = %agent_name, "ask_user: channel closed before response");
                "[No response — channel closed]".to_string()
            }
            Err(_) => {
                warn!(agent = %agent_name, "ask_user: timed out after 5 minutes");
                "[No response — timed out after 5 minutes]".to_string()
            }
        };
    }

    warn!(agent = %agent_name, "ask_user: not supported in non-streaming context");
    "[ask_user not supported in this context]".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask_user_call(input: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "ask-1".to_string(),
            name: "ask_user".to_string(),
            input,
        }
    }

    fn other_call() -> ToolCall {
        ToolCall {
            id: "tool-1".to_string(),
            name: "shell_exec".to_string(),
            input: serde_json::json!({"cmd": "pwd"}),
        }
    }

    #[tokio::test]
    async fn ignores_non_ask_user_tool_call() {
        let (stream_tx, mut stream_rx) = mpsc::channel(1);
        let mut recorded = Vec::new();
        let mut blocks = Vec::new();

        let handled = try_handle_ask_user_tool_call(
            "captain",
            &other_call(),
            &stream_tx,
            None,
            &mut recorded,
            &mut blocks,
        )
        .await;

        assert!(!handled);
        assert!(recorded.is_empty());
        assert!(blocks.is_empty());
        assert!(stream_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn sends_question_and_records_user_answer() {
        let (stream_tx, mut stream_rx) = mpsc::channel(3);
        let (answer_tx, answer_rx) = mpsc::channel(1);
        answer_tx.send("bleu".to_string()).await.unwrap();
        let answer_rx = Some(Arc::new(tokio::sync::Mutex::new(answer_rx)));
        let call = ask_user_call(serde_json::json!({
            "question": "Couleur ?",
            "options": ["bleu", "rouge", 7]
        }));
        let mut recorded = Vec::new();
        let mut blocks = Vec::new();

        let handled = try_handle_ask_user_tool_call(
            "captain",
            &call,
            &stream_tx,
            answer_rx.as_ref(),
            &mut recorded,
            &mut blocks,
        )
        .await;

        assert!(handled);
        let event = stream_rx.recv().await.expect("ask event");
        assert!(matches!(
            event,
            StreamEvent::AskUser {
                question,
                options: Some(options),
            } if question == "Couleur ?" && options == vec!["bleu".to_string(), "rouge".to_string()]
        ));
        let event = stream_rx.recv().await.expect("user response event");
        assert!(matches!(
            event,
            StreamEvent::UserResponse { content } if content == "bleu"
        ));
        let event = stream_rx.recv().await.expect("tool result event");
        assert!(matches!(
            event,
            StreamEvent::ToolExecutionResult {
                tool_use_id,
                name,
                result_preview,
                is_error: false,
            } if tool_use_id == "ask-1"
                && name == "ask_user"
                && result_preview == "User response received."
        ));
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].tool_name, "ask_user");
        assert_eq!(recorded[0].input_summary, "Couleur ?");
        assert_eq!(recorded[0].output_summary, "bleu");
        assert!(matches!(
            &blocks[0],
            ContentBlock::ToolResult {
                tool_use_id,
                tool_name,
                content,
                is_error: false,
            } if tool_use_id == "ask-1" && tool_name == "ask_user" && content == "bleu"
        ));
    }

    #[tokio::test]
    async fn channel_closed_records_closed_answer() {
        let (stream_tx, _stream_rx) = mpsc::channel(1);
        let (answer_tx, answer_rx) = mpsc::channel(1);
        drop(answer_tx);
        let answer_rx = Some(Arc::new(tokio::sync::Mutex::new(answer_rx)));
        let mut recorded = Vec::new();
        let mut blocks = Vec::new();

        let handled = try_handle_ask_user_tool_call(
            "captain",
            &ask_user_call(serde_json::json!({"question": "Go ?"})),
            &stream_tx,
            answer_rx.as_ref(),
            &mut recorded,
            &mut blocks,
        )
        .await;

        assert!(handled);
        assert_eq!(recorded[0].output_summary, "[No response — channel closed]");
        assert!(matches!(
            &blocks[0],
            ContentBlock::ToolResult { content, .. } if content == "[No response — channel closed]"
        ));
    }

    #[tokio::test]
    async fn missing_receiver_records_unsupported_answer() {
        let (stream_tx, _stream_rx) = mpsc::channel(1);
        let mut recorded = Vec::new();
        let mut blocks = Vec::new();

        let handled = try_handle_ask_user_tool_call(
            "captain",
            &ask_user_call(serde_json::json!({"question": "Go ?"})),
            &stream_tx,
            None,
            &mut recorded,
            &mut blocks,
        )
        .await;

        assert!(handled);
        assert_eq!(
            recorded[0].output_summary,
            "[ask_user not supported in this context]"
        );
        assert!(matches!(
            &blocks[0],
            ContentBlock::ToolResult { content, .. }
                if content == "[ask_user not supported in this context]"
        ));
    }
}
