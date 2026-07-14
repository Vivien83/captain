//! Command catalog route handlers.

use crate::state::AppState;
use axum::{extract::State, response::IntoResponse, Json};
use std::sync::Arc;

/// GET /api/commands - List available chat commands for dynamic slash menus.
pub async fn list_commands(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut commands = base_commands();

    if let Ok(registry) = state.kernel.skill_registry.read() {
        for skill in registry.list() {
            let desc: String = skill.manifest.skill.description.chars().take(80).collect();
            commands.push(serde_json::json!({
                "cmd": format!("/{}", skill.manifest.skill.name),
                "desc": if desc.is_empty() {
                    format!("Skill: {}", skill.manifest.skill.name)
                } else {
                    desc
                },
                "source": "skill",
            }));
        }
    }

    Json(serde_json::json!({"commands": commands}))
}

fn base_commands() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({"cmd": "/help", "desc": "Show available commands"}),
        serde_json::json!({"cmd": "/new", "desc": "Start a new session (previous session remains in history)"}),
        serde_json::json!({"cmd": "/compact", "desc": "Trigger LLM session compaction"}),
        serde_json::json!({"cmd": "/model", "desc": "Show or switch model (/model [name])"}),
        serde_json::json!({"cmd": "/stop", "desc": "Cancel current agent run"}),
        serde_json::json!({"cmd": "/usage", "desc": "Show session token usage & cost"}),
        serde_json::json!({"cmd": "/think", "desc": "Toggle extended thinking (/think [on|off|stream])"}),
        serde_json::json!({"cmd": "/context", "desc": "Show context window usage & pressure"}),
        serde_json::json!({"cmd": "/verbose", "desc": "Cycle tool detail level (/verbose [off|on|full])"}),
        serde_json::json!({"cmd": "/queue", "desc": "Check if agent is processing"}),
        serde_json::json!({"cmd": "/status", "desc": "Show system status"}),
        serde_json::json!({"cmd": "/health", "desc": "Show daemon health"}),
        serde_json::json!({"cmd": "/version", "desc": "Show daemon version and paths"}),
        serde_json::json!({"cmd": "/config", "desc": "Show exact config.toml (owner only)"}),
        serde_json::json!({"cmd": "/reload", "desc": "Hot-reload config.toml (owner only)"}),
        serde_json::json!({"cmd": "/restart", "desc": "Restart daemon (owner only)"}),
        serde_json::json!({"cmd": "/shutdown confirm", "desc": "Stop daemon (owner only)"}),
        serde_json::json!({"cmd": "/clear", "desc": "Clear chat display"}),
        serde_json::json!({"cmd": "/exit", "desc": "Disconnect from agent"}),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_commands_include_stop_and_status() {
        let commands = base_commands();
        let names: Vec<&str> = commands
            .iter()
            .filter_map(|command| command["cmd"].as_str())
            .collect();

        assert!(names.contains(&"/stop"));
        assert!(names.contains(&"/status"));
    }
}
