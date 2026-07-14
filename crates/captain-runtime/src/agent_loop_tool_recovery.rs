use crate::llm_driver::CompletionResponse;
use captain_types::message::{ContentBlock, StopReason};
use captain_types::tool::{ToolCall, ToolDefinition};

pub(crate) fn promote_recovered_text_tool_calls(
    response: &mut CompletionResponse,
    visible_tools: &[ToolDefinition],
) -> Option<usize> {
    if !matches!(
        response.stop_reason,
        StopReason::EndTurn | StopReason::StopSequence
    ) || !response.tool_calls.is_empty()
    {
        return None;
    }

    let recovered =
        crate::text_tool_call_recovery::recover_text_tool_calls(&response.text(), visible_tools);
    if recovered.is_empty() {
        return None;
    }

    response.tool_calls = recovered;
    response.stop_reason = StopReason::ToolUse;
    response.content = tool_use_blocks(&response.tool_calls);
    Some(response.tool_calls.len())
}

fn tool_use_blocks(tool_calls: &[ToolCall]) -> Vec<ContentBlock> {
    tool_calls
        .iter()
        .map(|tc| ContentBlock::ToolUse {
            id: tc.id.clone(),
            name: tc.name.clone(),
            input: tc.input.clone(),
            provider_metadata: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::message::TokenUsage;

    fn tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: "Test tool".to_string(),
            input_schema: serde_json::json!({}),
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
            usage: TokenUsage::default(),
        }
    }

    #[test]
    fn promotes_text_tool_call_to_tool_use_content() {
        let tools = vec![tool("web_search")];
        let mut response = text_response(
            r#"I'll search. <function=web_search>{"query":"captain"}</function>"#,
            StopReason::EndTurn,
        );

        let promoted = promote_recovered_text_tool_calls(&mut response, &tools);

        assert_eq!(promoted, Some(1));
        assert_eq!(response.stop_reason, StopReason::ToolUse);
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "web_search");
        assert_eq!(
            response.tool_calls[0].input,
            serde_json::json!({"query": "captain"})
        );
        assert!(matches!(
            &response.content[..],
            [ContentBlock::ToolUse { name, input, .. }]
                if name == "web_search" && input == &serde_json::json!({"query": "captain"})
        ));
    }

    #[test]
    fn preserves_native_tool_calls() {
        let tools = vec![tool("web_search")];
        let mut response = text_response(
            r#"<function=web_search>{"query":"captain"}</function>"#,
            StopReason::EndTurn,
        );
        response.tool_calls.push(ToolCall {
            id: "native_1".to_string(),
            name: "web_search".to_string(),
            input: serde_json::json!({"query": "native"}),
        });

        let promoted = promote_recovered_text_tool_calls(&mut response, &tools);

        assert_eq!(promoted, None);
        assert_eq!(response.stop_reason, StopReason::EndTurn);
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].id, "native_1");
        assert!(matches!(response.content[0], ContentBlock::Text { .. }));
    }

    #[test]
    fn ignores_non_terminal_or_unknown_text_calls() {
        let tools = vec![tool("web_search")];
        let mut max_tokens = text_response(
            r#"<function=web_search>{"query":"captain"}</function>"#,
            StopReason::MaxTokens,
        );
        let mut unknown = text_response(
            r#"<function=unknown_tool>{"query":"captain"}</function>"#,
            StopReason::StopSequence,
        );

        assert_eq!(
            promote_recovered_text_tool_calls(&mut max_tokens, &tools),
            None
        );
        assert_eq!(
            promote_recovered_text_tool_calls(&mut unknown, &tools),
            None
        );
        assert_eq!(max_tokens.stop_reason, StopReason::MaxTokens);
        assert_eq!(unknown.stop_reason, StopReason::StopSequence);
        assert!(max_tokens.tool_calls.is_empty());
        assert!(unknown.tool_calls.is_empty());
    }
}
