//! Skill system for Captain.
//!
//! Skills are pluggable tool bundles that extend agent capabilities.
//! They can be:
//! - TOML + Python scripts
//! - TOML + WASM modules
//! - TOML + Node.js modules (OpenClaw compatibility)
//! - Remote skills from Captain Marketplace registry

pub mod bundled;
pub mod clawhub;
pub mod families;
pub mod loader;
pub mod marketplace;
pub mod openclaw_compat;
pub mod registry;
pub mod verify;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Errors from the skill system.
#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("Skill not found: {0}")]
    NotFound(String),
    #[error("Invalid skill manifest: {0}")]
    InvalidManifest(String),
    #[error("Skill already installed: {0}")]
    AlreadyInstalled(String),
    #[error("Runtime not available: {0}")]
    RuntimeNotAvailable(String),
    #[error("Skill execution failed: {0}")]
    ExecutionFailed(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Network error: {0}")]
    Network(String),
    #[error("Rate limited by ClawHub — please wait a moment and try again: {0}")]
    RateLimited(String),
    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),
    #[error("YAML parse error: {0}")]
    YamlParse(String),
    #[error("Security blocked: {0}")]
    SecurityBlocked(String),
}

/// The runtime type for a skill.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillRuntime {
    /// Python script executed in subprocess.
    Python,
    /// WASM module executed in sandbox.
    Wasm,
    /// Node.js module (OpenClaw compatibility).
    Node,
    /// Shell/Bash script executed in subprocess.
    Shell,
    /// Built-in (compiled into the binary).
    Builtin,
    /// Prompt-only skill: injects context into the LLM system prompt.
    /// No executable code — the Markdown body teaches the LLM.
    #[default]
    PromptOnly,
}

/// Provenance tracking for skill origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum SkillSource {
    /// Built into Captain or manually installed.
    Native,
    /// Bundled at compile time (ships with Captain binary).
    Bundled,
    /// Converted from OpenClaw format.
    OpenClaw,
    /// Downloaded from ClawHub marketplace.
    ClawHub { slug: String, version: String },
}

/// A tool provided by a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillToolDef {
    /// Tool name (must be unique).
    pub name: String,
    /// Description shown to LLM.
    pub description: String,
    /// JSON Schema for the tool input.
    pub input_schema: serde_json::Value,
}

/// Requirements declared by a skill.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillRequirements {
    /// Built-in tools this skill needs access to.
    pub tools: Vec<String>,
    /// Capabilities this skill needs from the host.
    pub capabilities: Vec<String>,
    /// B.3 — Per-skill secret injection. Keys from `~/.captain/secrets.env`
    /// are mapped to environment variables exposed to the subprocess.
    /// A skill only sees the secrets it explicitly declares; other secrets
    /// stay invisible. Empty = no secret crosses the env boundary.
    ///
    /// Example (in `SKILL.toml`):
    /// ```toml
    /// [requirements.env_inject]
    /// SERVICE_USERNAME = "SERVICE_USER"
    /// SERVICE_PASSWORD = "SERVICE_PASS"
    /// ```
    pub env_inject: std::collections::BTreeMap<String, String>,
}

/// A skill manifest (parsed from skill.toml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    /// Skill metadata.
    pub skill: SkillMeta,
    /// Runtime configuration (defaults to PromptOnly if omitted).
    #[serde(default)]
    pub runtime: SkillRuntimeConfig,
    /// Tools provided by this skill.
    #[serde(default)]
    pub tools: SkillTools,
    /// Requirements from the host.
    #[serde(default)]
    pub requirements: SkillRequirements,
    /// Markdown body for prompt-only skills (injected into LLM system prompt).
    #[serde(default)]
    pub prompt_context: Option<String>,
    /// Provenance tracking — where this skill came from.
    #[serde(default)]
    pub source: Option<SkillSource>,
}

/// Skill metadata section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMeta {
    /// Unique skill name.
    pub name: String,
    /// Semantic version.
    #[serde(default = "default_version")]
    pub version: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Author.
    #[serde(default)]
    pub author: String,
    /// License.
    #[serde(default)]
    pub license: String,
    /// Tags for discovery.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Operator-facing governance summary derived from stable skill metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillGovernance {
    /// Skill was generated by Captain instead of authored directly by a human.
    pub generated: bool,
    /// Generated skill is in review/quarantine and should not be promoted silently.
    pub quarantined: bool,
    /// Skill should be treated as read-only until explicit promotion.
    pub locked: bool,
    /// Current promotion state when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub promotion_status: Option<String>,
    /// Minimal external proof required before promotion when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub promotion_required: Option<String>,
}

pub fn skill_governance_from_tags(tags: &[String]) -> SkillGovernance {
    let generated = has_skill_tag(tags, "generated");
    let quarantined =
        has_skill_tag(tags, "quarantine") || has_prefixed_skill_tag(tags, "promotion:quarantined");
    let locked = quarantined || has_skill_tag(tags, "locked");
    SkillGovernance {
        generated,
        quarantined,
        locked,
        promotion_status: quarantined.then(|| "quarantined".to_string()),
        promotion_required: quarantined.then(|| "schema_diff_tests_human".to_string()),
    }
}

fn has_skill_tag(tags: &[String], expected: &str) -> bool {
    tags.iter().any(|tag| tag.trim() == expected)
}

fn has_prefixed_skill_tag(tags: &[String], expected: &str) -> bool {
    tags.iter().any(|tag| tag.trim().starts_with(expected))
}

fn default_version() -> String {
    "0.1.0".to_string()
}

/// Runtime configuration section.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillRuntimeConfig {
    /// Runtime type.
    #[serde(rename = "type", default)]
    pub runtime_type: SkillRuntime,
    /// Entry point file (relative to skill directory).
    #[serde(default)]
    pub entry: String,
}

/// Tools section (wraps provided tools).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillTools {
    /// Tools provided by this skill.
    pub provided: Vec<SkillToolDef>,
}

/// An installed skill in the registry.
#[derive(Debug, Clone)]
pub struct InstalledSkill {
    /// Skill manifest.
    pub manifest: SkillManifest,
    /// Path to skill directory.
    pub path: PathBuf,
    /// Whether this skill is enabled.
    pub enabled: bool,
}

#[cfg(test)]
mod governance_tests {
    use super::skill_governance_from_tags;

    #[test]
    fn generated_quarantine_tags_mark_skill_locked() {
        let tags = vec![
            "generated".to_string(),
            "quarantine".to_string(),
            "family:general-automation".to_string(),
        ];
        let governance = skill_governance_from_tags(&tags);

        assert!(governance.generated);
        assert!(governance.quarantined);
        assert!(governance.locked);
        assert_eq!(governance.promotion_status.as_deref(), Some("quarantined"));
        assert_eq!(
            governance.promotion_required.as_deref(),
            Some("schema_diff_tests_human")
        );
    }

    #[test]
    fn plain_skill_tags_do_not_mark_quarantine() {
        let tags = vec!["custom".to_string(), "workspace".to_string()];
        let governance = skill_governance_from_tags(&tags);

        assert!(!governance.generated);
        assert!(!governance.quarantined);
        assert!(!governance.locked);
        assert!(governance.promotion_status.is_none());
        assert!(governance.promotion_required.is_none());
    }
}

/// Result of executing a skill tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillToolResult {
    /// Output content.
    pub output: serde_json::Value,
    /// Whether execution was an error.
    pub is_error: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_manifest_parse() {
        let toml_str = r#"
[skill]
name = "web-summarizer"
version = "0.1.0"
description = "Summarizes any web page into bullet points"
author = "captain-community"
license = "MIT"
tags = ["web", "summarizer", "research"]

[runtime]
type = "python"
entry = "src/main.py"

[[tools.provided]]
name = "summarize_url"
description = "Fetch a URL and return a concise bullet-point summary"
input_schema = { type = "object", properties = { url = { type = "string" } }, required = ["url"] }

[requirements]
tools = ["web_fetch"]
capabilities = ["NetConnect(*)"]
"#;

        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.skill.name, "web-summarizer");
        assert_eq!(manifest.runtime.runtime_type, SkillRuntime::Python);
        assert_eq!(manifest.tools.provided.len(), 1);
        assert_eq!(manifest.tools.provided[0].name, "summarize_url");
        assert_eq!(manifest.requirements.tools, vec!["web_fetch"]);
    }

    #[test]
    fn test_skill_runtime_serde() {
        let json = serde_json::to_string(&SkillRuntime::Python).unwrap();
        assert_eq!(json, "\"python\"");

        let rt: SkillRuntime = serde_json::from_str("\"wasm\"").unwrap();
        assert_eq!(rt, SkillRuntime::Wasm);

        let rt: SkillRuntime = serde_json::from_str("\"promptonly\"").unwrap();
        assert_eq!(rt, SkillRuntime::PromptOnly);
    }

    #[test]
    fn test_skill_source_serde() {
        let src = SkillSource::ClawHub {
            slug: "github-helper".to_string(),
            version: "1.0.0".to_string(),
        };
        let json = serde_json::to_string(&src).unwrap();
        let back: SkillSource = serde_json::from_str(&json).unwrap();
        assert_eq!(back, src);

        let native = SkillSource::Native;
        let json = serde_json::to_string(&native).unwrap();
        let back: SkillSource = serde_json::from_str(&json).unwrap();
        assert_eq!(back, SkillSource::Native);
    }
}
