use crate::llm_driver::CompletionResponse;
use captain_memory::session::Session;
use captain_types::message::{ContentBlock, Message, MessageContent, Role};

pub(crate) fn assistant_message_for_response(
    response: &CompletionResponse,
    visible_text: impl Into<String>,
) -> Message {
    let visible_text = visible_text.into();
    let has_provider_metadata = response.content.iter().any(|block| match block {
        ContentBlock::Text {
            provider_metadata, ..
        }
        | ContentBlock::ToolUse {
            provider_metadata, ..
        }
        | ContentBlock::Thinking {
            provider_metadata, ..
        } => provider_metadata.is_some(),
        _ => false,
    });
    if !has_provider_metadata {
        return Message::assistant(visible_text);
    }

    let mut blocks = Vec::new();
    let mut inserted_text = false;
    for block in &response.content {
        match block {
            ContentBlock::Thinking { .. } => blocks.push(block.clone()),
            ContentBlock::Text {
                provider_metadata, ..
            } if provider_metadata.is_some() && !inserted_text => {
                blocks.push(ContentBlock::Text {
                    text: visible_text.clone(),
                    provider_metadata: provider_metadata.clone(),
                });
                inserted_text = true;
            }
            _ => {}
        }
    }
    if !visible_text.is_empty() && !inserted_text {
        blocks.push(ContentBlock::Text {
            text: visible_text,
            provider_metadata: None,
        });
    }
    if blocks.is_empty() {
        Message::assistant(String::new())
    } else {
        Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(blocks),
        }
    }
}

pub(crate) fn prepare_turn_messages(
    session: &mut Session,
    user_message: &str,
    user_content_blocks: Option<Vec<ContentBlock>>,
    lean_direct_turn: bool,
    canonical_context_msg: Option<&str>,
    retractions: &[crate::memory_retractions::MemoryRetraction],
) -> Vec<Message> {
    push_user_turn(session, user_message, user_content_blocks);
    repair_session_in_place(session);
    let llm_messages = llm_messages_before_session_image_prune(session, lean_direct_turn);
    strip_images_from_session(session);

    let mut messages = crate::session_repair::validate_and_repair(&llm_messages);
    if !lean_direct_turn {
        if let Some(cc_msg) = canonical_context_msg {
            if let Some(cc_msg) =
                crate::memory_retractions::filter_retracted_lines(cc_msg, retractions)
            {
                messages.insert(0, Message::user(cc_msg));
            }
        }
    }
    messages
}

/// Repair the stored session itself, not just the per-turn working copy.
/// A persisted anomaly (e.g. an orphaned ToolResult saved by an interrupted
/// turn) used to be re-repaired on the copy every single turn — a WARN per
/// request, forever — while the source stayed broken. Fixing the session
/// here means the next save persists the repair and the noise stops.
fn repair_session_in_place(session: &mut Session) {
    let (repaired, stats) =
        crate::session_repair::repair_stored_session_with_stats(&session.messages);
    let repairs = stats.orphaned_results_removed
        + stats.empty_messages_removed
        + stats.messages_merged
        + stats.results_reordered
        + stats.synthetic_results_inserted
        + stats.duplicates_removed;
    if repairs > 0 {
        tracing::info!(
            repairs,
            "Persisted session history repaired in place (source of recurring per-turn repairs)"
        );
        session.messages = repaired;
    }
}

fn push_user_turn(
    session: &mut Session,
    user_message: &str,
    user_content_blocks: Option<Vec<ContentBlock>>,
) {
    if let Some(blocks) = user_content_blocks {
        session.messages.push(Message::user_with_blocks(blocks));
    } else {
        session.messages.push(Message::user(user_message));
    }
}

fn llm_messages_before_session_image_prune(
    session: &Session,
    lean_direct_turn: bool,
) -> Vec<Message> {
    if lean_direct_turn {
        return session
            .messages
            .last()
            .filter(|m| m.role != Role::System)
            .cloned()
            .into_iter()
            .collect();
    }

    session
        .messages
        .iter()
        .filter(|m| m.role != Role::System)
        .cloned()
        .collect()
}

fn strip_images_from_session(session: &mut Session) {
    for msg in session.messages.iter_mut() {
        if let MessageContent::Blocks(blocks) = &mut msg.content {
            let had_images = blocks
                .iter()
                .any(|b| matches!(b, ContentBlock::Image { .. }));
            if had_images {
                blocks.retain(|b| !matches!(b, ContentBlock::Image { .. }));
                if blocks.is_empty() {
                    blocks.push(ContentBlock::Text {
                        text: "[Image processed]".to_string(),
                        provider_metadata: None,
                    });
                }
            }
        }
    }
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
            stop_reason: StopReason::EndTurn,
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
        }
    }

    #[test]
    fn assistant_message_without_provider_metadata_is_plain_text() {
        let response = completion_response(vec![ContentBlock::Text {
            text: "raw".to_string(),
            provider_metadata: None,
        }]);

        let message = assistant_message_for_response(&response, "visible");

        assert_eq!(message.role, Role::Assistant);
        assert_eq!(message.content.text_content(), "visible");
    }

    #[test]
    fn assistant_message_preserves_thinking_and_text_metadata() {
        let metadata = serde_json::json!({"thoughtSignature": "abc"});
        let response = completion_response(vec![
            ContentBlock::Thinking {
                thinking: "hidden".to_string(),
                provider_metadata: Some(metadata.clone()),
            },
            ContentBlock::Text {
                text: "raw text".to_string(),
                provider_metadata: Some(metadata.clone()),
            },
        ]);

        let message = assistant_message_for_response(&response, "visible text");

        let MessageContent::Blocks(blocks) = message.content else {
            panic!("expected block assistant message");
        };
        assert!(matches!(blocks[0], ContentBlock::Thinking { .. }));
        assert!(matches!(
            &blocks[1],
            ContentBlock::Text {
                text,
                provider_metadata: Some(meta),
            } if text == "visible text" && meta == &metadata
        ));
    }

    #[test]
    fn lean_direct_turn_keeps_only_current_user_message() {
        let mut session = test_session(vec![Message::user("old context")]);

        let messages =
            prepare_turn_messages(&mut session, "new task", None, true, Some("canon"), &[]);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content.text_content(), "new task");
    }

    #[test]
    fn canonical_context_is_inserted_for_full_turns() {
        let mut session = test_session(vec![Message::system("system")]);

        let messages =
            prepare_turn_messages(&mut session, "hello", None, false, Some("canon ctx"), &[]);

        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[0].content.text_content(), "canon ctx");
        assert!(messages.iter().all(|m| m.role != Role::System));
    }

    /// A persisted orphaned ToolResult used to be re-repaired on the working
    /// copy every turn (one WARN per request, forever) while the stored
    /// session stayed broken. The turn preparation now repairs the session
    /// itself so the next save persists the fix.
    #[test]
    fn persisted_orphaned_tool_result_is_repaired_in_the_session() {
        let orphan = Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "call-ghost".to_string(),
                tool_name: "shell_exec".to_string(),
                content: "orphaned".to_string(),
                is_error: false,
            }]),
        };
        let mut session = test_session(vec![Message::user("earlier"), orphan]);

        let messages = prepare_turn_messages(&mut session, "hello", None, false, None, &[]);

        let has_orphan = |msgs: &[Message]| {
            msgs.iter().any(|m| {
                matches!(&m.content, MessageContent::Blocks(blocks)
                    if blocks.iter().any(|b| matches!(b, ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == "call-ghost")))
            })
        };
        assert!(!has_orphan(&messages), "working copy is repaired");
        assert!(
            !has_orphan(&session.messages),
            "the stored session itself is repaired, not just the copy"
        );
    }

    #[test]
    fn llm_messages_keep_image_while_session_is_pruned() {
        let image = ContentBlock::Image {
            media_type: "image/png".to_string(),
            data: "ZmFrZQ==".to_string(),
        };
        let mut session = test_session(Vec::new());

        let messages = prepare_turn_messages(&mut session, "", Some(vec![image]), false, None, &[]);

        assert!(matches!(
            &messages.last().unwrap().content,
            MessageContent::Blocks(blocks) if blocks.iter().any(|b| matches!(b, ContentBlock::Image { .. }))
        ));
        assert_eq!(
            session.messages.last().unwrap().content.text_content(),
            "[Image processed]"
        );
    }
}
