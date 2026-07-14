use serde::{Deserialize, Serialize};

/// Session checkpoint summarizer configuration.
///
/// Checkpoints are compact `checkpoint.md` files generated for inactive
/// sessions so cross-session recall can stay cheap and structured.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CheckpointConfig {
    /// Master switch for the background checkpoint summarizer.
    pub enabled: bool,
    /// Optional provider override. None means use `default_model.provider`.
    pub provider: Option<String>,
    /// Background model used for checkpoint summaries.
    pub model: String,
    /// Optional API key env var override for non-OAuth providers.
    pub api_key_env: Option<String>,
    /// Seconds of session inactivity before a checkpoint is generated.
    pub inactivity_secs: u64,
    /// Background scan interval in seconds.
    pub scan_interval_secs: u64,
    /// Delay between two summaries in the same scan pass.
    pub per_summary_delay_secs: u64,
    /// Maximum transcript characters sent to the summarizer.
    pub transcript_cap_chars: usize,
    /// Emit the OBSERVE -> THINK -> PLAN -> BUILD -> EXECUTE -> VERIFY -> LEARN
    /// learning review after writing a checkpoint.
    pub emit_learning_review: bool,
}

impl Default for CheckpointConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: None,
            model: "gpt-5.5".to_string(),
            api_key_env: None,
            inactivity_secs: 600,
            scan_interval_secs: 300,
            per_summary_delay_secs: 10,
            transcript_cap_chars: 8_000,
            emit_learning_review: true,
        }
    }
}

/// SkillSynthesizer configuration (v3.13).
///
/// Controls the pattern -> LLM judge -> review queue -> file writer pipeline.
/// Default mode is `approval` because approved proposals eventually land on
/// disk as executable skills; auto is opt-in.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillsConfig {
    pub enabled: bool,
    pub mode: LearningMode,
    /// Occurrences of a tool sequence before the LLM judge is called.
    pub pattern_threshold: u32,
    pub pattern_window_days: u32,
    pub proposer_model: String,
    pub fallback_models: Vec<String>,
    pub reflection_timeout_secs: u64,
    /// Max accepted proposals per UTC day.
    pub rate_limit_per_day: u32,
    pub min_confidence: f32,
    /// Directory (relative to home_dir when not absolute) for
    /// generated skill files.
    pub generated_dir: String,
    /// Reflection provider override; default: use default_model.
    pub reflection_provider: Option<String>,
    pub reflection_api_key_env: Option<String>,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: LearningMode::Approval,
            pattern_threshold: 5,
            pattern_window_days: 7,
            proposer_model: "gpt-5.5".to_string(),
            fallback_models: Vec::new(),
            reflection_timeout_secs: 30,
            rate_limit_per_day: 3,
            min_confidence: 0.7,
            generated_dir: "skills/generated".to_string(),
            reflection_provider: None,
            reflection_api_key_env: None,
        }
    }
}

/// Learning engine configuration (v3.12).
///
/// Controls the auto-learning pipeline that reflects on classified outcomes
/// and writes memory candidates to MemPalace rooms. MemPalace remains the
/// source of truth; this config only tunes the reflector.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LearningConfig {
    /// Master switch. When false the whole pipeline is inert.
    pub enabled: bool,
    /// Review mode: "off", "approval", "auto". v3.12g wires the approval
    /// flow. "auto" writes directly, "off" skips commit, "approval"
    /// funnels through the v3.8 approval manager.
    pub mode: LearningMode,
    /// Autonomy/aggressiveness coefficient for learning and self-improvement.
    ///
    /// `1.0` is neutral and preserves the existing behaviour. Lower values
    /// make the learning loop more conservative; higher values allow more
    /// candidates/proposals through the deterministic gates. Critical writes
    /// still obey `mode` and approval queues.
    #[serde(
        default = "default_learning_autonomy_aggressiveness",
        alias = "aggressiveness"
    )]
    pub autonomy_aggressiveness: f32,
    /// Reflection model ID (primary). Defaults to the Codex background path;
    /// when provider is `codex`, runtime normalizes it against the live/cache
    /// Codex model catalogue before static fallback.
    pub reflection_model: String,
    /// Fallback models tried in order if the primary call fails.
    pub fallback_models: Vec<String>,
    /// Reflection provider override when set.
    pub reflection_provider: Option<String>,
    /// Env var name for the reflection provider's API key. When `None`,
    /// the provider's conventional env var is used via
    /// `KernelConfig::resolve_api_key_env`.
    pub reflection_api_key_env: Option<String>,
    /// Reflection call timeout.
    pub reflection_timeout_secs: u64,
    /// Per-project daily rate limit on committed learnings.
    pub rate_limit_per_project_per_day: u32,
    /// Global daily rate limit on committed learnings.
    pub rate_limit_global_per_day: u32,
    /// Retention in days for `synced` rows. 0 = permanent.
    pub retention_synced_days: u32,
    /// Retention in days for `error` rows before GC.
    pub retention_error_days: u32,
    /// Minimum confidence threshold on memory candidates; lower values
    /// are rejected before commit.
    pub min_confidence: f32,
}

pub const DEFAULT_LEARNING_AUTONOMY_AGGRESSIVENESS: f32 = 1.0;
pub const MIN_LEARNING_AUTONOMY_AGGRESSIVENESS: f32 = 0.25;
pub const MAX_LEARNING_AUTONOMY_AGGRESSIVENESS: f32 = 3.0;

pub fn default_learning_autonomy_aggressiveness() -> f32 {
    DEFAULT_LEARNING_AUTONOMY_AGGRESSIVENESS
}

impl LearningConfig {
    pub fn effective_autonomy_aggressiveness(&self) -> f32 {
        if self.autonomy_aggressiveness.is_finite() {
            self.autonomy_aggressiveness.clamp(
                MIN_LEARNING_AUTONOMY_AGGRESSIVENESS,
                MAX_LEARNING_AUTONOMY_AGGRESSIVENESS,
            )
        } else {
            DEFAULT_LEARNING_AUTONOMY_AGGRESSIVENESS
        }
    }
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: LearningMode::Auto,
            autonomy_aggressiveness: default_learning_autonomy_aggressiveness(),
            reflection_model: "gpt-5.5".to_string(),
            fallback_models: Vec::new(),
            reflection_provider: None,
            reflection_api_key_env: None,
            reflection_timeout_secs: 30,
            rate_limit_per_project_per_day: 20,
            rate_limit_global_per_day: 50,
            retention_synced_days: 0,
            retention_error_days: 30,
            // Phase N.3: filtre dur retire. Le LLM decide lui-meme si un
            // candidat merite d'etre memorise via son prompt systeme. Le
            // champ confidence reste utilise pour ranking, pas pour gating.
            min_confidence: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LearningMode {
    /// Pipeline disabled end-to-end; emit() returns without running.
    Off,
    /// Every candidate goes through v3.8 approval manager before commit.
    Approval,
    /// Default: candidates pass policy filters and commit directly.
    #[default]
    Auto,
}

#[cfg(test)]
mod tests {
    use super::{
        CheckpointConfig, LearningConfig, LearningMode, SkillsConfig,
        MAX_LEARNING_AUTONOMY_AGGRESSIVENESS, MIN_LEARNING_AUTONOMY_AGGRESSIVENESS,
    };
    use crate::config::KernelConfig;

    #[test]
    fn checkpoint_config_defaults_keep_background_summary_enabled() {
        let cfg = CheckpointConfig::default();

        assert!(cfg.enabled);
        assert_eq!(cfg.model, "gpt-5.5");
        assert_eq!(cfg.inactivity_secs, 600);
        assert_eq!(cfg.transcript_cap_chars, 8_000);
        assert!(cfg.emit_learning_review);
    }

    #[test]
    fn skills_config_defaults_require_approval_before_writing() {
        let cfg = SkillsConfig::default();

        assert!(cfg.enabled);
        assert_eq!(cfg.mode, LearningMode::Approval);
        assert_eq!(cfg.pattern_threshold, 5);
        assert_eq!(cfg.rate_limit_per_day, 3);
        assert_eq!(cfg.generated_dir, "skills/generated");
    }

    #[test]
    fn learning_autonomy_aggressiveness_default_is_neutral() {
        let cfg = LearningConfig::default();

        assert!((cfg.autonomy_aggressiveness - 1.0).abs() < f32::EPSILON);
        assert!((cfg.effective_autonomy_aggressiveness() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn learning_autonomy_aggressiveness_toml_and_alias() {
        let config: KernelConfig = toml::from_str(
            r#"
            [learning]
            autonomy_aggressiveness = 1.75
            "#,
        )
        .unwrap();
        assert!((config.learning.autonomy_aggressiveness - 1.75).abs() < f32::EPSILON);

        let alias: KernelConfig = toml::from_str(
            r#"
            [learning]
            aggressiveness = 0.5
            "#,
        )
        .unwrap();
        assert!((alias.learning.autonomy_aggressiveness - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn learning_autonomy_aggressiveness_effective_bounds() {
        let mut cfg = LearningConfig {
            autonomy_aggressiveness: 0.01,
            ..Default::default()
        };
        assert_eq!(
            cfg.effective_autonomy_aggressiveness(),
            MIN_LEARNING_AUTONOMY_AGGRESSIVENESS
        );
        cfg.autonomy_aggressiveness = 99.0;
        assert_eq!(
            cfg.effective_autonomy_aggressiveness(),
            MAX_LEARNING_AUTONOMY_AGGRESSIVENESS
        );
    }

    #[test]
    fn learning_sections_deserialize_from_kernel_toml() {
        let config: KernelConfig = toml::from_str(
            r#"
            [learning]
            enabled = false
            mode = "approval"
            autonomy_aggressiveness = 0.75

            [skills]
            enabled = true
            mode = "approval"
            generated_dir = "skills/generated"

            [checkpoints]
            enabled = true
            provider = "codex"
            model = "gpt-5.3-codex-spark"
            "#,
        )
        .unwrap();

        assert!(!config.learning.enabled);
        assert_eq!(config.learning.mode, LearningMode::Approval);
        assert!((config.learning.autonomy_aggressiveness - 0.75).abs() < f32::EPSILON);
        assert!(config.skills.enabled);
        assert_eq!(config.skills.mode, LearningMode::Approval);
        assert_eq!(config.skills.generated_dir, "skills/generated");
        assert!(config.checkpoints.enabled);
        assert_eq!(config.checkpoints.provider.as_deref(), Some("codex"));
    }
}
