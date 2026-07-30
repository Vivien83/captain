//! Provider-reported model reasoning capabilities and durable user selection.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

const MAX_REASONING_EFFORT_LEN: usize = 32;

/// Open-string reasoning effort.
///
/// Codex can add new effort names through its model catalog. Captain validates
/// the shape here and validates membership against the selected model at the
/// kernel boundary instead of freezing a closed enum in the persisted schema.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ReasoningEffort(String);

impl ReasoningEffort {
    pub const LOW: &'static str = "low";
    pub const MEDIUM: &'static str = "medium";
    pub const HIGH: &'static str = "high";
    pub const XHIGH: &'static str = "xhigh";
    pub const MAX: &'static str = "max";
    pub const ULTRA: &'static str = "ultra";

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ReasoningEffort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ReasoningEffort {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let normalized = match value.trim().to_ascii_lowercase().as_str() {
            "extra-high" | "extra_high" => Self::XHIGH.to_string(),
            value => value.to_string(),
        };
        if normalized.is_empty() {
            return Err("reasoning effort cannot be empty".to_string());
        }
        if normalized.len() > MAX_REASONING_EFFORT_LEN {
            return Err(format!(
                "reasoning effort exceeds {MAX_REASONING_EFFORT_LEN} characters"
            ));
        }
        if !normalized
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(
                "reasoning effort must contain only lowercase letters, digits, or underscores"
                    .to_string(),
            );
        }
        if normalized == "auto" {
            return Err("`auto` resets the override and is not a persisted effort".to_string());
        }
        Ok(Self(normalized))
    }
}

impl TryFrom<String> for ReasoningEffort {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<ReasoningEffort> for String {
    fn from(value: ReasoningEffort) -> Self {
        value.0
    }
}

/// One effort option advertised by the selected model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasoningEffortOption {
    pub effort: ReasoningEffort,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Provider-owned reasoning capabilities for one model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelReasoningCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<ReasoningEffort>,
    #[serde(default)]
    pub supported_efforts: Vec<ReasoningEffortOption>,
    /// `true` only when the values came from the provider model catalog.
    pub reported_by_provider: bool,
}

impl ModelReasoningCapabilities {
    pub fn supports(&self, effort: &ReasoningEffort) -> bool {
        self.supported_efforts
            .iter()
            .any(|option| option.effort == *effort)
    }
}

/// Why Captain selected the effective reasoning effort shown to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningSelectionSource {
    AgentOverride,
    ModelDefault,
    ProviderDefault,
    Unsupported,
}

/// Stable cross-surface status for one agent's model reasoning selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentReasoningStatus {
    pub provider: String,
    pub model: String,
    pub supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configured_effort: Option<ReasoningEffort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_effort: Option<ReasoningEffort>,
    pub source: ReasoningSelectionSource,
    pub override_valid: bool,
    #[serde(default)]
    pub options: Vec<ReasoningEffortOption>,
    pub reported_by_provider: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effort_is_forward_compatible_but_shape_checked() {
        assert_eq!(
            "ultra".parse::<ReasoningEffort>().unwrap().as_str(),
            "ultra"
        );
        assert_eq!(
            "extra-high".parse::<ReasoningEffort>().unwrap().as_str(),
            "xhigh"
        );
        assert_eq!(
            "future_2".parse::<ReasoningEffort>().unwrap().as_str(),
            "future_2"
        );
        assert!("auto".parse::<ReasoningEffort>().is_err());
        assert!("not valid".parse::<ReasoningEffort>().is_err());
    }

    #[test]
    fn effort_serde_roundtrip_preserves_provider_value() {
        let effort = "max".parse::<ReasoningEffort>().unwrap();
        let encoded = serde_json::to_string(&effort).unwrap();
        assert_eq!(encoded, "\"max\"");
        assert_eq!(
            serde_json::from_str::<ReasoningEffort>(&encoded).unwrap(),
            effort
        );
    }
}
