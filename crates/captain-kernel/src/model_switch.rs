use serde::{Deserialize, Serialize};

/// How Captain should handle existing context when switching model/provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSwitchSessionStrategy {
    /// Start with an empty active session and drop active canonical context.
    NewSession,
    /// Convert the current context into a provider-neutral summary first.
    CompactSession,
}

impl ModelSwitchSessionStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NewSession => "new_session",
            Self::CompactSession => "compact_session",
        }
    }
}

impl std::str::FromStr for ModelSwitchSessionStrategy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "new_session" | "new" | "reset" => Ok(Self::NewSession),
            "compact_session" | "compact" | "summary" => Ok(Self::CompactSession),
            other => Err(format!(
                "Invalid session_strategy '{other}'. Use 'new_session' or 'compact_session'."
            )),
        }
    }
}

/// Coarse risk signal shown to users before applying a provider/model switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSwitchRisk {
    Low,
    Medium,
    High,
}

/// Read-only preflight result for a requested model/provider switch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSwitchPlan {
    pub agent_id: String,
    pub agent_name: String,
    pub current_provider: String,
    pub current_model: String,
    pub target_provider: String,
    pub target_model: String,
    pub provider_changed: bool,
    pub model_changed: bool,
    pub active_session_id: String,
    pub active_message_count: usize,
    pub canonical_summary_present: bool,
    pub canonical_recent_count: usize,
    pub session_strategy_required: bool,
    pub recommended_session_strategy: ModelSwitchSessionStrategy,
    pub target_model_known: bool,
    pub target_provider_known: bool,
    pub target_auth_configured: bool,
    pub target_supports_tools: Option<bool>,
    pub target_supports_vision: Option<bool>,
    pub target_supports_streaming: Option<bool>,
    pub driver_ready: bool,
    pub driver_error: Option<String>,
    pub risk: ModelSwitchRisk,
    pub can_apply: bool,
    pub blocking_issues: Vec<String>,
    pub warnings: Vec<String>,
}

/// Result returned after applying a safe switch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSwitchApplyResult {
    pub status: String,
    pub plan: ModelSwitchPlan,
    pub session_strategy: ModelSwitchSessionStrategy,
    pub previous_session_id: String,
    pub new_session_id: String,
    pub compacted_summary_chars: usize,
    /// True when switching the principal Captain agent also updated the
    /// system-wide `[default_model]` config.
    pub global_default_updated: bool,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::ModelSwitchSessionStrategy;

    #[test]
    fn strategy_accepts_short_aliases() {
        assert_eq!(
            "new".parse::<ModelSwitchSessionStrategy>().unwrap(),
            ModelSwitchSessionStrategy::NewSession
        );
        assert_eq!(
            "compact".parse::<ModelSwitchSessionStrategy>().unwrap(),
            ModelSwitchSessionStrategy::CompactSession
        );
    }
}
