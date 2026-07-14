use super::*;
use captain_types::message::{ContentBlock, TokenUsage};
use captain_types::tool::ToolCall;

fn tool_def(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: "test".into(),
        input_schema: serde_json::json!({}),
    }
}

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

fn captain_runtime_core_name_for_test(name: &str) -> bool {
    crate::core_tools::CORE_TOOLS.contains(&name)
}

#[test]
fn discovery_expands_deferred_builtin_tools() {
    let mut visible = crate::tool_runner::builtin_tool_definitions()
        .into_iter()
        .filter(|t| captain_runtime_core_name_for_test(&t.name))
        .collect::<Vec<_>>();
    assert!(!visible.iter().any(|t| t.name == "file_read"));

    let result = serde_json::json!({
        "query": "read a local file",
        "results": [
            {
                "name": "file_read",
                "source": "builtin",
                "status": "available,deferred",
                "metadata": { "core": false }
            }
        ]
    })
    .to_string();

    let catalog = crate::tool_runner::builtin_tool_definitions();
    let added =
        expand_visible_tools_from_discovery(&mut visible, &catalog, "capability_search", &result);

    assert_eq!(added, 1);
    assert!(visible.iter().any(|t| t.name == "file_read"));
}

#[test]
fn discovery_does_not_rehydrate_frozen_builtin_surfaces() {
    let catalog = crate::tool_runner::builtin_tool_definitions();
    let mut visible = catalog
        .iter()
        .filter(|t| t.name == "capability_search")
        .cloned()
        .collect::<Vec<_>>();
    let result = serde_json::json!({
        "results": [
            {
                "name": "hand_activate",
                "source": "builtin",
                "status": "available,deferred",
                "metadata": { "core": false }
            }
        ]
    })
    .to_string();

    let added =
        expand_visible_tools_from_discovery(&mut visible, &catalog, "capability_search", &result);

    assert_eq!(added, 0);
    assert!(!visible.iter().any(|t| t.name == "hand_activate"));
}

#[test]
fn discovery_expands_dynamic_mcp_tools_from_available_catalog() {
    let catalog = vec![
        tool_def("capability_search"),
        tool_def("mcp_mempalace_mempalace_search"),
    ];
    let mut visible = vec![catalog[0].clone()];
    let result = serde_json::json!({
        "results": [{
            "name": "mcp_mempalace_mempalace_search",
            "source": "mcp_tool",
            "status": "connected",
            "input_schema": {"type": "object"}
        }]
    })
    .to_string();

    let added =
        expand_visible_tools_from_discovery(&mut visible, &catalog, "capability_search", &result);

    assert_eq!(added, 1);
    assert!(visible
        .iter()
        .any(|t| t.name == "mcp_mempalace_mempalace_search"));
}

#[test]
fn discovery_result_wrapper_expands_and_reports_added_count() {
    let mut visible = crate::tool_runner::builtin_tool_definitions()
        .into_iter()
        .filter(|t| captain_runtime_core_name_for_test(&t.name))
        .collect::<Vec<_>>();
    let result = serde_json::json!({
        "results": [{
            "name": "file_read",
            "source": "builtin",
            "status": "available,deferred",
            "metadata": { "core": false }
        }]
    })
    .to_string();
    let catalog = crate::tool_runner::builtin_tool_definitions();

    let added = expand_visible_tools_after_discovery_result(
        &mut visible,
        &catalog,
        "capability_search",
        &result,
        true,
    );

    assert_eq!(added, 1);
    assert!(visible.iter().any(|t| t.name == "file_read"));
}

#[test]
fn capability_denial_retry_is_gated_by_discovery_tool() {
    let visible = crate::tool_runner::builtin_tool_definitions()
        .into_iter()
        .filter(|t| t.name == "capability_search")
        .collect::<Vec<_>>();

    assert!(capability_denial_should_retry(
        "Je n'ai pas accès au shell dans mes outils visibles.",
        &visible
    ));
    assert!(!capability_denial_should_retry(
        "Je n'ai pas accès au shell dans mes outils visibles.",
        &[]
    ));
}

#[test]
fn phantom_action_detects_channel_claim_without_tool() {
    assert!(phantom_action_detected(
        "I successfully sent the message on Telegram."
    ));
    assert!(!phantom_action_detected(
        "I can draft the Telegram message."
    ));
}

#[test]
fn tool_error_guidance_is_added_only_for_failed_tool_results() {
    let mut blocks = vec![ContentBlock::ToolResult {
        tool_use_id: "t1".to_string(),
        tool_name: "web_search".to_string(),
        content: "failed".to_string(),
        is_error: true,
    }];

    append_tool_error_guidance(&mut blocks);

    assert_eq!(blocks.len(), 2);
    assert!(matches!(blocks[1], ContentBlock::Text { .. }));
}

#[test]
fn codex_missing_tool_watchdog_detects_narrated_action() {
    let tools = vec![tool_def("web_search")];
    let response = text_response("Je vais utiliser l'outil web_search pour vérifier.");
    assert!(codex_missing_tool_call_should_retry(
        "codex", &response, &tools
    ));
}

#[test]
fn codex_missing_tool_watchdog_ignores_normal_text_and_other_providers() {
    let tools = vec![tool_def("web_search")];
    let response = text_response("Voici la réponse directe.");
    assert!(!codex_missing_tool_call_should_retry(
        "codex", &response, &tools
    ));
    let narrated = text_response("I will use the web_search tool.");
    assert!(!codex_missing_tool_call_should_retry(
        "anthropic",
        &narrated,
        &tools
    ));
}

#[test]
fn recovered_text_tool_call_suppresses_codex_watchdog() {
    let tools = vec![tool_def("web_search")];
    let mut response = text_response(
        r#"Je vais utiliser l'outil. <function=web_search>{"query":"captain"}</function>"#,
    );
    let recovered =
        crate::text_tool_call_recovery::recover_text_tool_calls(&response.text(), &tools);
    assert_eq!(recovered.len(), 1);
    response.tool_calls = recovered;
    response.stop_reason = StopReason::ToolUse;
    assert!(!codex_missing_tool_call_should_retry(
        "codex", &response, &tools
    ));
}

#[test]
fn codex_watchdog_ignores_existing_tool_calls() {
    let tools = vec![tool_def("web_search")];
    let mut response = text_response("I will use web_search.");
    response.tool_calls = vec![ToolCall {
        id: "call_1".to_string(),
        name: "web_search".to_string(),
        input: serde_json::json!({"query": "captain"}),
    }];

    assert!(!codex_missing_tool_call_should_retry(
        "codex", &response, &tools
    ));
}
