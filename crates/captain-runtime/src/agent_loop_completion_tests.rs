use super::*;
use captain_types::message::{ContentBlock, StopReason};

fn text_response(text: &str) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::Text {
            text: text.to_string(),
            provider_metadata: None,
        }],
        stop_reason: StopReason::EndTurn,
        tool_calls: Vec::new(),
        usage: TokenUsage::default(),
    }
}

fn empty_response_with_usage(input_tokens: u64) -> CompletionResponse {
    CompletionResponse {
        content: Vec::new(),
        stop_reason: StopReason::EndTurn,
        tool_calls: Vec::new(),
        usage: TokenUsage {
            input_tokens,
            output_tokens: 0,
            ..Default::default()
        },
    }
}

fn tool(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: "tool".to_string(),
        input_schema: serde_json::json!({"type": "object"}),
    }
}

fn decide(response: &CompletionResponse) -> EndTurnDecision {
    decide_end_turn_response(EndTurnDecisionInput {
        agent_name: "captain",
        response,
        total_usage: &TokenUsage::default(),
        messages_len: 1,
        iteration: 0,
        any_tools_executed: false,
        capability_denial_watchdog_used: false,
        visible_tools: &[],
        streaming: false,
        phantom_action_watchdog: false,
    })
}

#[test]
fn silent_directive_returns_silent_reply_directives() {
    let response = text_response("[[reply:abc]] [[silent]] hidden");

    let EndTurnDecision::Silent { directives } = decide(&response) else {
        panic!("expected silent decision");
    };

    assert_eq!(directives.reply_to.as_deref(), Some("abc"));
    assert!(directives.silent);
}

#[test]
fn empty_initial_response_requests_retry() {
    let response = empty_response_with_usage(0);

    let EndTurnDecision::RetryEmpty { silent_failure } = decide(&response) else {
        panic!("expected empty retry");
    };

    assert!(silent_failure);
}

#[test]
fn empty_after_tool_returns_guarded_completion() {
    let response = empty_response_with_usage(7);

    let decision = decide_end_turn_response(EndTurnDecisionInput {
        agent_name: "captain",
        response: &response,
        total_usage: &TokenUsage {
            input_tokens: 7,
            output_tokens: 0,
            ..Default::default()
        },
        messages_len: 3,
        iteration: 2,
        any_tools_executed: true,
        capability_denial_watchdog_used: false,
        visible_tools: &[],
        streaming: true,
        phantom_action_watchdog: false,
    });

    let EndTurnDecision::Complete { text } = decision else {
        panic!("expected guarded completion");
    };
    assert!(text.contains("executed tools"));
}

#[test]
fn phantom_action_retry_is_explicitly_gated() {
    let response = text_response("The Telegram message has been sent successfully.");

    let decision = decide_end_turn_response(EndTurnDecisionInput {
        agent_name: "captain",
        response: &response,
        total_usage: &TokenUsage::default(),
        messages_len: 1,
        iteration: 0,
        any_tools_executed: false,
        capability_denial_watchdog_used: false,
        visible_tools: &[],
        streaming: false,
        phantom_action_watchdog: true,
    });
    assert!(matches!(decision, EndTurnDecision::RetryPhantom { .. }));

    let decision = decide_end_turn_response(EndTurnDecisionInput {
        agent_name: "captain",
        response: &response,
        total_usage: &TokenUsage::default(),
        messages_len: 1,
        iteration: 0,
        any_tools_executed: false,
        capability_denial_watchdog_used: false,
        visible_tools: &[],
        streaming: true,
        phantom_action_watchdog: false,
    });
    assert!(matches!(decision, EndTurnDecision::Complete { .. }));
}

#[test]
fn capability_denial_retries_when_discovery_tool_is_visible() {
    let response = text_response("I don't have access to that tool.");
    let visible_tools = vec![tool("capability_search")];

    let decision = decide_end_turn_response(EndTurnDecisionInput {
        agent_name: "captain",
        response: &response,
        total_usage: &TokenUsage::default(),
        messages_len: 1,
        iteration: 1,
        any_tools_executed: false,
        capability_denial_watchdog_used: false,
        visible_tools: &visible_tools,
        streaming: false,
        phantom_action_watchdog: false,
    });

    assert!(matches!(decision, EndTurnDecision::RetryCapability { .. }));
}
