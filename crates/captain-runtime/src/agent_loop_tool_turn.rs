use crate::llm_driver::CompletionResponse;
use captain_memory::session::Session;
use captain_types::message::{ContentBlock, Message, MessageContent, Role};

pub(crate) fn append_tool_use_assistant_turn(
    response: &CompletionResponse,
    session: &mut Session,
    messages: &mut Vec<Message>,
) {
    let assistant_blocks = tool_use_assistant_blocks(response);
    session.messages.push(Message {
        role: Role::Assistant,
        content: MessageContent::Blocks(assistant_blocks.clone()),
    });
    messages.push(Message {
        role: Role::Assistant,
        content: MessageContent::Blocks(assistant_blocks),
    });
}

fn tool_use_assistant_blocks(response: &CompletionResponse) -> Vec<ContentBlock> {
    response
        .content
        .iter()
        .filter(|block| !matches!(block, ContentBlock::Text { .. }))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::agent::{AgentId, SessionId};
    use captain_types::message::{StopReason, TokenUsage};

    fn test_session(messages: Vec<Message>) -> Session {
        Session {
            id: SessionId::new(),
            agent_id: AgentId::new(),
            messages,
            context_window_tokens: 0,
            label: None,
        }
    }

    fn completion_response(content: Vec<ContentBlock>) -> CompletionResponse {
        CompletionResponse {
            content,
            stop_reason: StopReason::ToolUse,
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
        }
    }

    fn blocks(message: &Message) -> &[ContentBlock] {
        match &message.content {
            MessageContent::Blocks(blocks) => blocks,
            MessageContent::Text(_) => panic!("expected block message"),
        }
    }

    #[test]
    fn strips_narrative_text_and_preserves_non_text_blocks() {
        let metadata = serde_json::json!({"thoughtSignature": "sig"});
        let response = completion_response(vec![
            ContentBlock::Text {
                text: "Let me check that.".to_string(),
                provider_metadata: None,
            },
            ContentBlock::Thinking {
                thinking: "state".to_string(),
                provider_metadata: Some(metadata.clone()),
            },
            ContentBlock::ToolUse {
                id: "tool-1".to_string(),
                name: "shell_exec".to_string(),
                input: serde_json::json!({"cmd":"pwd"}),
                provider_metadata: None,
            },
            ContentBlock::Unknown,
        ]);
        let mut session = test_session(Vec::new());
        let mut request_messages = Vec::new();

        append_tool_use_assistant_turn(&response, &mut session, &mut request_messages);

        let session_blocks = blocks(session.messages.last().unwrap());
        assert_eq!(session_blocks.len(), 3);
        assert!(session_blocks
            .iter()
            .all(|block| !matches!(block, ContentBlock::Text { .. })));
        assert!(matches!(
            &session_blocks[0],
            ContentBlock::Thinking {
                provider_metadata: Some(meta),
                ..
            } if meta == &metadata
        ));
        assert!(matches!(
            &session_blocks[1],
            ContentBlock::ToolUse { name, .. } if name == "shell_exec"
        ));
        assert!(matches!(session_blocks[2], ContentBlock::Unknown));
    }

    #[test]
    fn appends_same_assistant_turn_to_session_and_request_messages() {
        let response = completion_response(vec![ContentBlock::ToolUse {
            id: "tool-1".to_string(),
            name: "read_file".to_string(),
            input: serde_json::json!({"path":"Cargo.toml"}),
            provider_metadata: None,
        }]);
        let mut session = test_session(vec![Message::user("previous")]);
        let mut request_messages = vec![Message::user("current")];

        append_tool_use_assistant_turn(&response, &mut session, &mut request_messages);

        assert_eq!(session.messages.len(), 2);
        assert_eq!(request_messages.len(), 2);
        assert_eq!(session.messages.last().unwrap().role, Role::Assistant);
        assert_eq!(request_messages.last().unwrap().role, Role::Assistant);
        assert_eq!(blocks(session.messages.last().unwrap()).len(), 1);
        assert_eq!(blocks(request_messages.last().unwrap()).len(), 1);
    }

    #[test]
    fn text_only_tool_turn_still_appends_empty_block_messages() {
        let response = completion_response(vec![ContentBlock::Text {
            text: "No tool block arrived.".to_string(),
            provider_metadata: None,
        }]);
        let mut session = test_session(Vec::new());
        let mut request_messages = Vec::new();

        append_tool_use_assistant_turn(&response, &mut session, &mut request_messages);

        assert!(blocks(session.messages.last().unwrap()).is_empty());
        assert!(blocks(request_messages.last().unwrap()).is_empty());
    }
}
