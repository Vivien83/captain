//! Replay helpers for persisted chat sessions.

use super::{
    chat::{ChatMessage, ChatState, Role, ToolInfo, ToolStatus},
    chat_model_label,
};
use crate::tui::session_store::{self as store, PersistedMessage, PersistedSession, PersistedTool};
use std::path::Path;

#[cfg(test)]
mod tests;

pub(super) fn replay_session_from(state: &mut ChatState, key: &str, path: &Path) {
    let Some(loaded) = store::load_session_at(path) else {
        return;
    };
    apply_loaded_session(state, key, path, loaded);
}

fn apply_loaded_session(state: &mut ChatState, key: &str, path: &Path, loaded: PersistedSession) {
    state.messages = loaded
        .messages
        .into_iter()
        .map(chat_message_from_persisted)
        .collect();
    state.streaming_text.clear();
    state.is_streaming = false;
    state.thinking = false;
    state.session_key = key.to_string();
    state.session_path = Some(path.to_path_buf());
    state.authoritative_session_id = loaded.session_id;
    state.authoritative_agent_id = loaded.agent_id;
    state.session_created_at = loaded.created_at;
    state.current_context_tokens = loaded.current_context_tokens;
    state.context_window_tokens = loaded.context_window_tokens;
    state.context_stream_checkpoint_chars = None;
    state.session_input_tokens = loaded.session_input_tokens;
    state.session_output_tokens = loaded.session_output_tokens;
    state.session_cached_input_tokens = loaded.session_cached_input_tokens;
    state.session_cache_creation_tokens = loaded.session_cache_creation_tokens;
    state.session_cost_usd = loaded.session_cost_usd;
    state.agent_name = loaded.agent_name;
    state.model_label = chat_model_label::sanitize_model_label(&loaded.model_label);
    state.mode_label = loaded.mode_label;
}

fn chat_message_from_persisted(message: PersistedMessage) -> ChatMessage {
    ChatMessage {
        role: role_from_persisted(&message.role),
        text: message.text,
        tool: message.tool.map(tool_from_persisted),
    }
}

fn role_from_persisted(role: &str) -> Role {
    match role {
        "user" => Role::User,
        "agent" => Role::Agent,
        "system" => Role::System,
        "tool" => Role::Tool,
        _ => Role::System,
    }
}

fn tool_from_persisted(tool: PersistedTool) -> ToolInfo {
    ToolInfo {
        id: String::new(),
        name: tool.name,
        input: tool.input,
        result: tool.result,
        stdout: String::new(),
        stderr: String::new(),
        is_error: tool.is_error,
        status: if tool.is_error {
            ToolStatus::Error
        } else {
            ToolStatus::Success
        },
        started_at: None,
        completed_at: None,
        duration_ms: None,
        expanded: tool.is_error,
    }
}
