use super::*;
use std::path::Path;

#[test]
fn persisted_roles_fall_back_to_system() {
    assert!(matches!(role_from_persisted("user"), Role::User));
    assert!(matches!(role_from_persisted("agent"), Role::Agent));
    assert!(matches!(role_from_persisted("tool"), Role::Tool));
    assert!(matches!(role_from_persisted("unknown"), Role::System));
}

#[test]
fn persisted_tool_restores_status_and_expansion() {
    let tool = tool_from_persisted(PersistedTool {
        name: "shell_exec".to_string(),
        input: "ls".to_string(),
        result: "failed".to_string(),
        is_error: true,
    });

    assert_eq!(tool.status, ToolStatus::Error);
    assert!(tool.expanded);
    assert_eq!(tool.name, "shell_exec");
}

#[test]
fn loaded_session_replaces_runtime_identity_and_messages() {
    let mut state = ChatState::new();
    state.streaming_text = "partial".to_string();
    state.is_streaming = true;
    state.thinking = true;
    state.push_message(Role::User, "old".to_string());

    apply_loaded_session(
        &mut state,
        "agent-key",
        Path::new("/tmp/session.json"),
        PersistedSession {
            session_id: Some("00000000-0000-0000-0000-000000000042".to_string()),
            agent_id: None,
            agent_name: "Agent".to_string(),
            model_label: "?/?".to_string(),
            mode_label: "chat".to_string(),
            messages: vec![PersistedMessage {
                role: "agent".to_string(),
                text: "restored".to_string(),
                tool: None,
            }],
            session_input_tokens: 3,
            session_output_tokens: 5,
            session_cached_input_tokens: 2,
            session_cache_creation_tokens: 1,
            session_cost_usd: 0.1,
            created_at: 42,
            updated_at: 0,
        },
    );

    assert_eq!(state.session_key, "agent-key");
    assert_eq!(state.messages.len(), 1);
    assert!(matches!(state.messages[0].role, Role::Agent));
    assert_eq!(state.messages[0].text, "restored");
    assert!(state.streaming_text.is_empty());
    assert!(!state.is_streaming);
    assert!(!state.thinking);
    assert!(state.model_label.is_empty());
    assert_eq!(state.session_created_at, 42);
}
