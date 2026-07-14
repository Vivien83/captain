use super::*;

#[test]
fn assistant_style_context_includes_display_name_and_workspace_style() {
    let assistant = AssistantConfig {
        display_name: "Nova".to_string(),
        style: "developer".to_string(),
        onboarding_completed: true,
    };

    let ctx = assistant_style_context(&assistant, Some("Use crisp bullets.".to_string()))
        .expect("style context");

    assert!(ctx.contains("User-facing name: Nova"));
    assert!(ctx.contains("Configured style: developer"));
    assert!(ctx.contains("implementation-oriented"));
    assert!(ctx.contains("Use crisp bullets."));
    assert!(ctx.contains("Internal routing slug: captain"));
}

#[test]
fn generated_workspace_markdown_is_not_prompt_content() {
    assert!(!workspace_prompt_file_has_product_content(
        "MEMORY.md",
        "# Long-Term Memory\n- stale note"
    ));
    assert!(!workspace_prompt_file_has_product_content(
        "AGENTS.md",
        "# Agent Behavioral Guidelines\n\n## Memory Journal\nUpdate MEMORY.md after significant actions."
    ));
    assert!(workspace_prompt_file_has_product_content(
        "AGENTS.md",
        "# Project Rules\nUse the local staging API."
    ));
}

#[test]
fn error_diagnosis_request_detection_is_narrow() {
    assert!(asks_for_error_diagnosis("Pourquoi tu as eu des erreurs ?"));
    assert!(asks_for_error_diagnosis("why did the tool fail?"));
    assert!(!asks_for_error_diagnosis(
        "corrige cette erreur dans le code"
    ));
    assert!(!asks_for_error_diagnosis("pourquoi tu choisis Telegram ?"));
}

#[test]
fn diagnostic_context_reports_no_visible_failures_without_polluting_user_message() {
    let agent_id = AgentId::new();
    let session = captain_memory::session::Session {
        id: SessionId::new(),
        agent_id,
        messages: vec![captain_types::message::Message::assistant(
            "Dernier tour termine.",
        )],
        context_window_tokens: 0,
        label: None,
    };

    let context = append_turn_diagnostic_context(
        Some("contexte canonique".to_string()),
        &session,
        "Pourquoi tu as eu des erreurs ?",
    )
    .expect("diagnostic context");

    assert!(context.contains("contexte canonique"));
    assert!(context.contains("Echecs outil recents visibles: aucun."));
    assert!(context.contains("ce n'est pas une nouvelle demande utilisateur"));
}

#[test]
fn diagnostic_context_classifies_failures_without_raw_json() {
    use captain_types::message::{ContentBlock, Message, MessageContent, Role};

    let agent_id = AgentId::new();
    let session = captain_memory::session::Session {
        id: SessionId::new(),
        agent_id,
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "toolu_1".to_string(),
                tool_name: "web_fetch".to_string(),
                content: "{\"error\":\"secret backend payload\"}".to_string(),
                is_error: true,
            }]),
        }],
        context_window_tokens: 0,
        label: None,
    };

    let context = append_turn_diagnostic_context(None, &session, "Pourquoi tu as eu des erreurs ?")
        .expect("diagnostic context");

    assert!(context.contains("web_fetch: other:raw_error_hidden_json_payload"));
    assert!(!context.contains("secret backend payload"));
}
