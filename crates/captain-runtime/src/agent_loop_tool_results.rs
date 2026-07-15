use crate::agent_loop_tool_flow::append_tool_error_guidance;
use crate::context_budget::{
    compact_tool_result_for_context, truncate_tool_result_dynamic, ContextBudget,
};
use captain_memory::session::Session;
use captain_memory::MemorySubstrate;
use captain_types::message::{ContentBlock, Message, MessageContent, Role};
use captain_types::tool::ToolResult;
use tracing::warn;

pub(crate) fn prepare_tool_result_content(
    tool_name: &str,
    result: &ToolResult,
    context_budget: &ContextBudget,
    loop_guard_warning: Option<&str>,
) -> String {
    let compacted = compact_tool_result_for_context(
        tool_name,
        &result.content,
        result.is_error,
        context_budget,
    );
    let content = truncate_tool_result_dynamic(&compacted, context_budget);

    if let Some(warn_msg) = loop_guard_warning {
        format!("{content}\n\n[LOOP GUARD] {warn_msg}")
    } else {
        content
    }
}

pub(crate) fn push_tool_result_block(
    tool_result_blocks: &mut Vec<ContentBlock>,
    tool_name: &str,
    result: ToolResult,
    content: String,
) {
    tool_result_blocks.push(ContentBlock::ToolResult {
        tool_use_id: result.tool_use_id,
        tool_name: tool_name.to_string(),
        content,
        is_error: result.is_error,
    });
}

pub(crate) fn append_tool_result_turn(
    tool_result_blocks: &mut Vec<ContentBlock>,
    session: &mut Session,
    messages: &mut Vec<Message>,
) {
    append_tool_error_guidance(tool_result_blocks);
    append_approval_denial_guidance(tool_result_blocks);

    let request_tool_results_msg = Message {
        role: Role::User,
        content: MessageContent::Blocks(tool_result_blocks.clone()),
    };
    let persisted_blocks = tool_result_blocks
        .iter()
        .filter(|block| !matches!(block, ContentBlock::Image { .. }))
        .cloned()
        .collect();
    session.messages.push(Message {
        role: Role::User,
        content: MessageContent::Blocks(persisted_blocks),
    });
    messages.push(request_tool_results_msg);
}

pub(crate) async fn interim_save_tool_turn(
    session: &Session,
    memory: &MemorySubstrate,
    boundary: &'static str,
) {
    if let Err(e) = memory.save_session_async(session).await {
        warn!(boundary, "Failed to persist tool-turn boundary: {e}");
    }
}

fn append_approval_denial_guidance(tool_result_blocks: &mut Vec<ContentBlock>) {
    let denial_count = tool_result_blocks
        .iter()
        .filter(|block| {
            matches!(block, ContentBlock::ToolResult { content, is_error: true, .. }
                if content.contains("requires human approval and was denied"))
        })
        .count();
    if denial_count == 0 {
        return;
    }

    tool_result_blocks.push(ContentBlock::Text {
        text: format!(
            "[System: {} tool call(s) were denied by approval policy. \
             Do NOT retry denied tools. Explain to the user what you \
             wanted to do and that it requires their approval. \
             Hint: set auto_approve = true in [approval] section of \
             config.toml, or start with --yolo flag, to auto-approve \
             all tool calls.]",
            denial_count
        ),
        provider_metadata: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_memory::MemorySubstrate;
    use captain_types::agent::{AgentId, SessionId};

    fn test_session(messages: Vec<Message>) -> Session {
        Session {
            id: SessionId::new(),
            agent_id: AgentId::new(),
            messages,
            context_window_tokens: 0,
            label: None,
        }
    }

    fn tool_result(content: &str, is_error: bool) -> ContentBlock {
        ContentBlock::ToolResult {
            tool_use_id: "tool-1".to_string(),
            tool_name: "shell_exec".to_string(),
            content: content.to_string(),
            is_error,
        }
    }

    fn exec_result(content: String, is_error: bool) -> ToolResult {
        ToolResult {
            tool_use_id: "tool-1".to_string(),
            content,
            is_error,
            transient_content: Vec::new(),
        }
    }

    fn blocks(message: &Message) -> &[ContentBlock] {
        match &message.content {
            MessageContent::Blocks(blocks) => blocks,
            MessageContent::Text(_) => panic!("expected block message"),
        }
    }

    #[test]
    fn prepare_tool_result_content_keeps_short_result() {
        let budget = ContextBudget::new(200_000);
        let result = exec_result("ok".to_string(), false);

        let content = prepare_tool_result_content("shell_exec", &result, &budget, None);

        assert_eq!(content, "ok");
    }

    #[test]
    fn prepare_tool_result_content_appends_loop_guard_warning() {
        let budget = ContextBudget::new(200_000);
        let result = exec_result("ok".to_string(), false);

        let content =
            prepare_tool_result_content("shell_exec", &result, &budget, Some("repeat detected"));

        assert_eq!(content, "ok\n\n[LOOP GUARD] repeat detected");
    }

    #[test]
    fn prepare_tool_result_content_applies_dynamic_truncation() {
        let budget = ContextBudget::new(1_000);
        let content = (0..300)
            .map(|idx| format!("line {idx}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = exec_result(content, false);

        let content = prepare_tool_result_content("file_read", &result, &budget, None);

        assert!(content.contains("[TRUNCATED:"));
    }

    #[test]
    fn push_tool_result_block_uses_execution_result_identity() {
        let result = exec_result("raw".to_string(), true);
        let mut blocks = Vec::new();

        push_tool_result_block(&mut blocks, "file_read", result, "final".to_string());

        assert_eq!(blocks.len(), 1);
        assert!(matches!(
            &blocks[0],
            ContentBlock::ToolResult {
                tool_use_id,
                tool_name,
                content,
                is_error: true,
            } if tool_use_id == "tool-1"
                && tool_name == "file_read"
                && content == "final"
        ));
    }

    #[test]
    fn appends_same_user_tool_result_turn_to_session_and_request_messages() {
        let mut tool_result_blocks = vec![tool_result("ok", false)];
        let mut session = test_session(vec![Message::user("previous")]);
        let mut request_messages = vec![Message::user("current")];

        append_tool_result_turn(&mut tool_result_blocks, &mut session, &mut request_messages);

        assert_eq!(session.messages.len(), 2);
        assert_eq!(request_messages.len(), 2);
        assert_eq!(session.messages.last().unwrap().role, Role::User);
        assert_eq!(request_messages.last().unwrap().role, Role::User);
        assert_eq!(blocks(session.messages.last().unwrap()).len(), 1);
        assert_eq!(blocks(request_messages.last().unwrap()).len(), 1);
    }

    #[test]
    fn transient_tool_images_reach_the_request_but_not_the_durable_session() {
        let mut tool_result_blocks = vec![
            tool_result("screenshot metadata", false),
            ContentBlock::Image {
                media_type: "image/png".to_string(),
                data: "cG5n".to_string(),
            },
        ];
        let mut session = test_session(Vec::new());
        let mut request_messages = Vec::new();

        append_tool_result_turn(&mut tool_result_blocks, &mut session, &mut request_messages);

        assert_eq!(blocks(session.messages.last().unwrap()).len(), 1);
        assert!(!blocks(session.messages.last().unwrap())
            .iter()
            .any(|block| matches!(block, ContentBlock::Image { .. })));
        assert_eq!(blocks(request_messages.last().unwrap()).len(), 2);
        assert!(blocks(request_messages.last().unwrap())
            .iter()
            .any(|block| matches!(block, ContentBlock::Image { .. })));
    }

    #[test]
    fn failed_tool_results_get_error_guidance_before_message_append() {
        let mut tool_result_blocks = vec![tool_result("boom", true)];
        let mut session = test_session(Vec::new());
        let mut request_messages = Vec::new();

        append_tool_result_turn(&mut tool_result_blocks, &mut session, &mut request_messages);

        let appended = blocks(session.messages.last().unwrap());
        assert_eq!(appended.len(), 2);
        assert!(matches!(appended[0], ContentBlock::ToolResult { .. }));
        assert!(matches!(
            &appended[1],
            ContentBlock::Text { text, .. } if text.contains("Tool call(s) failed")
        ));
    }

    #[test]
    fn approval_denials_get_non_retry_guidance() {
        let mut tool_result_blocks = vec![
            tool_result("requires human approval and was denied", true),
            tool_result("requires human approval and was denied", true),
        ];
        let mut session = test_session(Vec::new());
        let mut request_messages = Vec::new();

        append_tool_result_turn(&mut tool_result_blocks, &mut session, &mut request_messages);

        let appended = blocks(session.messages.last().unwrap());
        assert_eq!(appended.len(), 4);
        assert!(matches!(
            &appended[3],
            ContentBlock::Text { text, .. }
                if text.contains("2 tool call(s) were denied")
                    && text.contains("Do NOT retry denied tools")
        ));
    }

    #[tokio::test]
    async fn interim_save_tool_turn_persists_session() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let session = test_session(vec![Message::user("tool results preserved")]);

        interim_save_tool_turn(&session, &memory, "tool_results").await;

        let saved = memory.get_session(session.id).unwrap().unwrap();
        assert_eq!(saved.messages.len(), 1);
        assert_eq!(
            saved.messages[0].content.text_content(),
            "tool results preserved"
        );
    }
}
