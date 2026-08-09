//! Shared contracts for opt-in, exact-action approval suggestions.

use crate::approval::RiskLevel;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MIN_SUGGESTION_APPROVALS: u16 = 2;
pub const MAX_SUGGESTION_APPROVALS: u16 = 10;
pub const MIN_SUGGESTION_WINDOW_HOURS: u64 = 1;
pub const MAX_SUGGESTION_WINDOW_HOURS: u64 = 24 * 90;

/// User-owned consent and circuit-breaker policy for approval suggestions.
/// Disabled means Captain records no approval-learning observations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ApprovalSuggestionPolicy {
    pub enabled: bool,
    pub minimum_approvals: u16,
    pub observation_window_hours: u64,
    pub dismissal_cooldown_hours: u64,
}

impl Default for ApprovalSuggestionPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            minimum_approvals: 3,
            observation_window_hours: 24 * 30,
            dismissal_cooldown_hours: 24 * 7,
        }
    }
}

impl ApprovalSuggestionPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if !(MIN_SUGGESTION_APPROVALS..=MAX_SUGGESTION_APPROVALS).contains(&self.minimum_approvals)
        {
            return Err(format!(
                "approval suggestion minimum_approvals must be {MIN_SUGGESTION_APPROVALS}..={MAX_SUGGESTION_APPROVALS}"
            ));
        }
        for (name, value) in [
            ("observation_window_hours", self.observation_window_hours),
            ("dismissal_cooldown_hours", self.dismissal_cooldown_hours),
        ] {
            if !(MIN_SUGGESTION_WINDOW_HOURS..=MAX_SUGGESTION_WINDOW_HOURS).contains(&value) {
                return Err(format!(
                    "approval suggestion {name} must be {MIN_SUGGESTION_WINDOW_HOURS}..={MAX_SUGGESTION_WINDOW_HOURS}"
                ));
            }
        }
        Ok(())
    }
}

/// Public-safe pending suggestion. It carries no raw action or display preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalSuggestion {
    pub id: Uuid,
    pub agent_id: String,
    pub tool_name: String,
    pub action_digest: String,
    pub risk_level: RiskLevel,
    pub observation_count: u16,
    pub first_observed_at: DateTime<Utc>,
    pub last_observed_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalSuggestionStatus {
    pub enabled: bool,
    pub healthy: bool,
    pub pending_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

pub fn risk_is_suggestion_eligible(risk_level: RiskLevel) -> bool {
    matches!(risk_level, RiskLevel::Low | RiskLevel::Medium)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_are_opt_in_and_bounded_by_default() {
        let policy = ApprovalSuggestionPolicy::default();
        assert!(!policy.enabled);
        assert_eq!(policy.minimum_approvals, 3);
        assert!(policy.validate().is_ok());
    }

    #[test]
    fn policy_rejects_unbounded_thresholds_and_windows() {
        let mut policy = ApprovalSuggestionPolicy {
            enabled: true,
            ..ApprovalSuggestionPolicy::default()
        };
        policy.minimum_approvals = 1;
        assert!(policy.validate().unwrap_err().contains("minimum_approvals"));

        policy.minimum_approvals = 3;
        policy.observation_window_hours = MAX_SUGGESTION_WINDOW_HOURS + 1;
        assert!(policy
            .validate()
            .unwrap_err()
            .contains("observation_window_hours"));
    }

    #[test]
    fn only_low_and_medium_risk_are_eligible() {
        assert!(risk_is_suggestion_eligible(RiskLevel::Low));
        assert!(risk_is_suggestion_eligible(RiskLevel::Medium));
        assert!(!risk_is_suggestion_eligible(RiskLevel::High));
        assert!(!risk_is_suggestion_eligible(RiskLevel::Critical));
    }
}
