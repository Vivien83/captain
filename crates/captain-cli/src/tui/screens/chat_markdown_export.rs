//! Markdown export helpers for chat sessions.

use super::chat::{ChatMessage, ChatState, Role};
use std::path::PathBuf;

#[cfg(test)]
mod tests;

pub(super) fn export_markdown(state: &ChatState) -> Result<PathBuf, String> {
    let Some(home) = dirs::home_dir() else {
        return Err("HOME inconnu".into());
    };
    let dir = home.join(".captain").join("exports");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create_dir: {e}"))?;

    let path = dir.join(format!(
        "{}_{}.md",
        export_agent_filename(state),
        unix_timestamp_secs()
    ));
    std::fs::write(&path, build_markdown_export(state)).map_err(|e| format!("write: {e}"))?;
    Ok(path)
}

fn unix_timestamp_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn export_agent_filename(state: &ChatState) -> String {
    if state.agent_name.is_empty() {
        "captain".to_string()
    } else {
        state.agent_name.replace(' ', "_")
    }
}

fn build_markdown_export(state: &ChatState) -> String {
    let mut buf = String::new();
    buf.push_str(&format!("# Captain — {}\n\n", state.agent_name));
    push_session_metadata(&mut buf, state);
    buf.push('\n');
    for message in &state.messages {
        push_message(&mut buf, message);
    }
    buf
}

fn push_session_metadata(buf: &mut String, state: &ChatState) {
    if !state.model_label.is_empty() {
        buf.push_str(&format!("- model: `{}`\n", state.model_label));
    }
    if !state.mode_label.is_empty() {
        buf.push_str(&format!("- mode: `{}`\n", state.mode_label));
    }

    let total = state.session_input_tokens + state.session_output_tokens;
    if total > 0 {
        buf.push_str(&format!(
            "- tokens session: {} (in {} / out {})\n",
            total, state.session_input_tokens, state.session_output_tokens
        ));
    }
    if state.session_cached_input_tokens > 0 {
        buf.push_str(&format!(
            "- cache session: {} input cached / {} creation\n",
            state.session_cached_input_tokens, state.session_cache_creation_tokens
        ));
    }
    if state.session_cost_usd > 0.0 {
        buf.push_str(&format!("- coût session: ${:.4}\n", state.session_cost_usd));
    }
}

fn push_message(buf: &mut String, message: &ChatMessage) {
    match message.role {
        Role::User => buf.push_str(&format!("### Toi\n\n{}\n\n", message.text)),
        Role::Agent => buf.push_str(&format!("### Agent\n\n{}\n\n", message.text)),
        Role::System => buf.push_str(&format!("> _{}_\n\n", message.text)),
        Role::Tool => {
            if let Some(tool) = &message.tool {
                buf.push_str(&format!(
                    "**tool** `{}`{}\n\n```\n{}\n```\n\n```\n{}\n```\n\n",
                    tool.name,
                    if tool.is_error { " ✘" } else { " ✔" },
                    tool.input,
                    tool.result
                ));
            }
        }
    }
}
