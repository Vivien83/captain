use super::*;
use crate::tui::screens::chat::{ToolInfo, ToolStatus};

#[test]
fn export_agent_filename_falls_back_and_replaces_spaces() {
    let mut state = ChatState::new();
    assert_eq!(export_agent_filename(&state), "captain");

    state.agent_name = "Agent One".to_string();
    assert_eq!(export_agent_filename(&state), "Agent_One");
}

#[test]
fn markdown_export_includes_metadata_messages_and_tools() {
    let mut state = ChatState::new();
    state.agent_name = "Agent One".to_string();
    state.model_label = "codex".to_string();
    state.mode_label = "chat".to_string();
    state.session_input_tokens = 10;
    state.session_output_tokens = 4;
    state.session_cached_input_tokens = 3;
    state.session_cache_creation_tokens = 2;
    state.session_cost_usd = 0.01234;
    state.push_message(Role::User, "hello".to_string());
    state.push_message(Role::Agent, "hi".to_string());
    state.messages.push(ChatMessage {
        role: Role::Tool,
        text: String::new(),
        tool: Some(ToolInfo {
            id: "tool-1".to_string(),
            name: "shell_exec".to_string(),
            input: "ls".to_string(),
            result: "ok".to_string(),
            stdout: String::new(),
            stderr: String::new(),
            is_error: false,
            status: ToolStatus::Success,
            started_at: None,
            completed_at: None,
            duration_ms: None,
            expanded: false,
        }),
    });

    let markdown = build_markdown_export(&state);

    assert!(markdown.contains("# Captain — Agent One"));
    assert!(markdown.contains("- model: `codex`"));
    assert!(markdown.contains("- tokens session: 14 (in 10 / out 4)"));
    assert!(markdown.contains("- cache session: 3 input cached / 2 creation"));
    assert!(markdown.contains("- coût session: $0.0123"));
    assert!(markdown.contains("### Toi\n\nhello"));
    assert!(markdown.contains("### Agent\n\nhi"));
    assert!(markdown.contains("**tool** `shell_exec` ✔"));
}
