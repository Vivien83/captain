use crate::agent_loop::AgentLoopResult;
use captain_memory::session::Session;
use captain_memory::MemorySubstrate;
use captain_types::agent::AgentManifest;
use captain_types::error::{CaptainError, CaptainResult};
use captain_types::message::ContentBlock;
use captain_types::tool::ToolCall;
use tracing::warn;

pub(crate) async fn fail_loop_guard_circuit_break(
    manifest: &AgentManifest,
    session: &mut Session,
    memory: &MemorySubstrate,
    hooks: Option<&crate::hooks::HookRegistry>,
    agent_id_str: &str,
    message: &str,
) -> CaptainResult<AgentLoopResult> {
    if let Err(e) = memory.save_session_async(session).await {
        warn!("Failed to save session on circuit break: {e}");
    }

    if let Some(hook_reg) = hooks {
        let ctx = crate::hooks::HookContext {
            agent_name: &manifest.name,
            agent_id: agent_id_str,
            event: captain_types::agent::HookEvent::AgentLoopEnd,
            data: serde_json::json!({
                "reason": "circuit_break",
                "error": message,
            }),
        };
        let _ = hook_reg.fire(&ctx);
    }

    Err(CaptainError::Internal(message.to_string()))
}

pub(crate) fn push_loop_guard_block_result(
    tool_result_blocks: &mut Vec<ContentBlock>,
    tool_call: &ToolCall,
    message: &str,
) {
    tool_result_blocks.push(ContentBlock::ToolResult {
        tool_use_id: tool_call.id.clone(),
        tool_name: tool_call.name.clone(),
        content: message.to_string(),
        is_error: true,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_memory::MemorySubstrate;
    use captain_types::agent::{AgentId, SessionId};
    use captain_types::message::Message;

    fn test_session(messages: Vec<Message>) -> Session {
        Session {
            id: SessionId::new(),
            agent_id: AgentId::new(),
            messages,
            context_window_tokens: 0,
            label: None,
        }
    }

    #[tokio::test]
    async fn circuit_break_saves_session_before_error() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let mut session = test_session(vec![Message::user("preserve me")]);
        let mut manifest = AgentManifest::default();
        manifest.name = "captain".to_string();

        let err = fail_loop_guard_circuit_break(
            &manifest,
            &mut session,
            &memory,
            None,
            "agent",
            "loop detected",
        )
        .await
        .unwrap_err();

        assert!(matches!(err, CaptainError::Internal(message) if message == "loop detected"));
        let saved = memory.get_session(session.id).unwrap().unwrap();
        assert_eq!(saved.messages.len(), 1);
        assert_eq!(saved.messages[0].content.text_content(), "preserve me");
    }

    #[test]
    fn block_result_records_tool_identity_and_error_message() {
        let tool_call = ToolCall {
            id: "call-1".to_string(),
            name: "shell_exec".to_string(),
            input: serde_json::json!({"cmd":"pwd"}),
        };
        let mut blocks = Vec::new();

        push_loop_guard_block_result(&mut blocks, &tool_call, "blocked");

        assert_eq!(blocks.len(), 1);
        assert!(matches!(
            &blocks[0],
            ContentBlock::ToolResult {
                tool_use_id,
                tool_name,
                content,
                is_error: true,
            } if tool_use_id == "call-1" && tool_name == "shell_exec" && content == "blocked"
        ));
    }
}
