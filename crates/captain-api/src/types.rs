//! Request/response types for the Captain API.

use serde::{Deserialize, Serialize};

/// Request to spawn an agent from a TOML manifest string or a template name.
#[derive(Debug, Deserialize)]
pub struct SpawnRequest {
    /// Agent manifest as TOML string (optional if `template` is provided).
    #[serde(default)]
    pub manifest_toml: String,
    /// Template name from `~/.captain/agents/{template}/agent.toml`.
    /// When provided and `manifest_toml` is empty, the template is loaded automatically.
    #[serde(default)]
    pub template: Option<String>,
    /// Optional Ed25519 signed manifest envelope (JSON).
    /// When present, the signature is verified before spawning.
    #[serde(default)]
    pub signed_manifest: Option<String>,
    /// Agent-as-service provisioning protocol. By default Captain rotates an
    /// ingress bearer token during creation and returns it once. Supplying an
    /// egress callback URL also configures signed outbound callbacks.
    #[serde(default)]
    pub agent_api: captain_types::agent_api::AgentApiSpawnProvisionRequest,
}

/// Response after spawning an agent.
#[derive(Debug, Serialize)]
pub struct SpawnResponse {
    pub agent_id: String,
    pub name: String,
}

/// A file attachment reference (from a prior upload).
#[derive(Debug, Clone, Deserialize)]
pub struct AttachmentRef {
    pub file_id: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub content_type: String,
}

/// Request to send a message to an agent.
#[derive(Debug, Deserialize)]
pub struct MessageRequest {
    pub message: String,
    /// Optional persisted session UUID. When present, the turn is scoped to
    /// that session without changing the agent's globally active session.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Optional file attachments (uploaded via /upload endpoint).
    #[serde(default)]
    pub attachments: Vec<AttachmentRef>,
    /// Sender identity (e.g. WhatsApp phone number, Telegram user ID).
    #[serde(default)]
    pub sender_id: Option<String>,
    /// Sender display name.
    #[serde(default)]
    pub sender_name: Option<String>,
    /// Origin channel for learning/self-improvement feedback routing
    /// (`cli`, `telegram`, `web`, ...). Optional for older clients.
    #[serde(default)]
    pub channel_type: Option<String>,
}

/// Compact summary of a single tool call made during the agent loop.
///
/// Surfaced in `MessageResponse` so external callers (debug scripts, CI
/// integration tests, third-party tooling) can verify which tools the LLM
/// actually invoked without having to parse daemon logs or open an SSE
/// stream. The full input/output snapshots stay internal — only the name,
/// success flag and duration travel through the synchronous JSON.
#[derive(Debug, Serialize)]
pub struct ToolCallSummary {
    pub name: String,
    pub is_error: bool,
    pub duration_ms: u64,
}

/// Response from sending a message.
#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub response: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub iterations: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    pub tool_calls: Vec<ToolCallSummary>,
}

/// Request to install a skill from the marketplace.
#[derive(Debug, Deserialize)]
pub struct SkillInstallRequest {
    pub name: String,
}

/// Request to uninstall a skill.
#[derive(Debug, Deserialize)]
pub struct SkillUninstallRequest {
    pub name: String,
}

/// Request to update an agent's manifest.
#[derive(Debug, Deserialize)]
pub struct AgentUpdateRequest {
    pub manifest_toml: String,
}

/// Request to change an agent's operational mode.
#[derive(Debug, Deserialize)]
pub struct SetModeRequest {
    pub mode: captain_types::agent::AgentMode,
}

/// Request to install a skill from ClawHub.
#[derive(Debug, Deserialize)]
pub struct ClawHubInstallRequest {
    /// ClawHub skill slug (e.g., "github-helper").
    pub slug: String,
}

/// Request body for POST /api/agents/{id}/session/restore. Carries the
/// historical messages the kernel should rehydrate into a fresh session
/// so the next LLM call sees them as context.
#[derive(Debug, Deserialize)]
pub struct SessionRestoreRequest {
    pub messages: Vec<captain_types::message::Message>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_response_serializes_tool_calls_with_items() {
        let resp = MessageResponse {
            response: "ok".into(),
            input_tokens: 10,
            output_tokens: 5,
            iterations: 2,
            cost_usd: Some(0.01),
            tool_calls: vec![ToolCallSummary {
                name: "memory_save".into(),
                is_error: false,
                duration_ms: 42,
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"tool_calls\""));
        assert!(json.contains("\"memory_save\""));
        assert!(json.contains("\"is_error\":false"));
        assert!(json.contains("\"duration_ms\":42"));
    }

    #[test]
    fn message_response_keeps_tool_calls_field_when_empty() {
        let resp = MessageResponse {
            response: "ok".into(),
            input_tokens: 10,
            output_tokens: 5,
            iterations: 1,
            cost_usd: None,
            tool_calls: vec![],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(
            json.contains("\"tool_calls\":[]"),
            "field must always be present so callers can rely on it"
        );
        assert!(!json.contains("\"cost_usd\""));
    }
}
