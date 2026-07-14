use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Gap 7: Thinking level support
// ---------------------------------------------------------------------------

/// Extended thinking configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ThinkingConfig {
    /// Maximum tokens for thinking (budget).
    pub budget_tokens: u32,
    /// Whether to stream thinking tokens to the client.
    pub stream_thinking: bool,
}

impl Default for ThinkingConfig {
    fn default() -> Self {
        Self {
            budget_tokens: 10_000,
            stream_thinking: false,
        }
    }
}

/// Extra workspace roots authorized for the principal `captain` agent.
/// Subagents are unaffected — they keep their own workspace sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspaceConfig {
    /// Absolute paths the user explicitly opened up via `workspace_add`.
    #[serde(default)]
    pub extra_paths: Vec<PathBuf>,
}

/// User-facing assistant identity and communication style.
///
/// The internal principal agent slug remains `captain`; `display_name` is
/// only the name presented to the user and injected into the prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AssistantConfig {
    /// Public name used in conversations. Defaults to "Captain".
    pub display_name: String,
    /// Communication style identifier (balanced, concise, professional,
    /// developer, friendly, classic, or a future custom style).
    pub style: String,
    /// True once the first-run product onboarding has collected the basics.
    pub onboarding_completed: bool,
}

impl Default for AssistantConfig {
    fn default() -> Self {
        Self {
            display_name: "Captain".to_string(),
            style: "balanced".to_string(),
            onboarding_completed: false,
        }
    }
}

pub const AGENT_LOOP_MAX_ITERATIONS_DEFAULT: u32 = 90;
pub const AGENT_LOOP_MAX_ITERATIONS_HARD_CAP: u32 = 1000;

/// Global runtime guardrail for one agent turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentLoopConfig {
    /// Maximum LLM/tool iterations before Captain stops the current turn.
    ///
    /// This protects against infinite tool loops. Per-agent
    /// `AutonomousConfig.max_iterations` still takes precedence.
    pub max_iterations: u32,
}

impl AgentLoopConfig {
    pub fn effective_max_iterations(&self) -> u32 {
        self.max_iterations
            .clamp(1, AGENT_LOOP_MAX_ITERATIONS_HARD_CAP)
    }
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: AGENT_LOOP_MAX_ITERATIONS_DEFAULT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AgentLoopConfig, AssistantConfig, ThinkingConfig, WorkspaceConfig,
        AGENT_LOOP_MAX_ITERATIONS_DEFAULT, AGENT_LOOP_MAX_ITERATIONS_HARD_CAP,
    };
    use crate::config::KernelConfig;
    use std::path::PathBuf;

    #[test]
    fn assistant_defaults_keep_captain_identity() {
        let config = AssistantConfig::default();

        assert_eq!(config.display_name, "Captain");
        assert_eq!(config.style, "balanced");
        assert!(!config.onboarding_completed);
    }

    #[test]
    fn agent_loop_defaults_and_clamps_iterations() {
        let default = AgentLoopConfig::default();
        let zero = AgentLoopConfig { max_iterations: 0 };
        let excessive = AgentLoopConfig {
            max_iterations: AGENT_LOOP_MAX_ITERATIONS_HARD_CAP + 1,
        };

        assert_eq!(default.max_iterations, AGENT_LOOP_MAX_ITERATIONS_DEFAULT);
        assert_eq!(
            default.effective_max_iterations(),
            AGENT_LOOP_MAX_ITERATIONS_DEFAULT
        );
        assert_eq!(zero.effective_max_iterations(), 1);
        assert_eq!(
            excessive.effective_max_iterations(),
            AGENT_LOOP_MAX_ITERATIONS_HARD_CAP
        );
    }

    #[test]
    fn thinking_defaults_keep_streaming_disabled() {
        let config = ThinkingConfig::default();

        assert_eq!(config.budget_tokens, 10_000);
        assert!(!config.stream_thinking);
    }

    #[test]
    fn workspace_defaults_to_no_extra_roots() {
        let config = WorkspaceConfig::default();

        assert!(config.extra_paths.is_empty());
    }

    #[test]
    fn agent_runtime_sections_deserialize_from_kernel_toml() {
        let config: KernelConfig = toml::from_str(
            r#"
            [agent_loop]
            max_iterations = 42

            [assistant]
            display_name = "Captain Ops"
            style = "developer"
            onboarding_completed = true

            [workspace]
            extra_paths = ["/tmp/captain-extra"]

            [thinking]
            budget_tokens = 2048
            stream_thinking = true
            "#,
        )
        .unwrap();

        assert_eq!(config.agent_loop.max_iterations, 42);
        assert_eq!(config.agent_loop.effective_max_iterations(), 42);
        assert_eq!(config.assistant.display_name, "Captain Ops");
        assert_eq!(config.assistant.style, "developer");
        assert!(config.assistant.onboarding_completed);
        assert_eq!(
            config.workspace.extra_paths,
            vec![PathBuf::from("/tmp/captain-extra")]
        );

        let thinking = config.thinking.unwrap();
        assert_eq!(thinking.budget_tokens, 2048);
        assert!(thinking.stream_thinking);
    }
}
