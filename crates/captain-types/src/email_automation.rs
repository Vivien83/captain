//! Shared, secret-free contracts for Gmail automation operator surfaces.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::AgentId;
use crate::email::GmailAccountAlias;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GmailAutomationConditionSpec {
    pub from_contains: Option<String>,
    pub recipient_contains: Option<String>,
    pub subject_contains: Option<String>,
    #[serde(default)]
    pub all_label_ids: Vec<String>,
    #[serde(default)]
    pub any_label_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailAutomationRuleSaveRequest {
    pub id: Option<String>,
    pub expected_version: Option<u64>,
    pub account_alias: Option<GmailAccountAlias>,
    pub name: String,
    pub condition: GmailAutomationConditionSpec,
    pub target_agent: String,
    pub instruction: String,
    pub include_body: bool,
    pub max_body_bytes: usize,
    pub max_delivery_attempts: u8,
    pub enabled: bool,
    pub max_fires_per_hour: u16,
    pub confirm_automation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailAutomationRuleQuery {
    pub rule_id: Option<String>,
    pub account_alias: Option<GmailAccountAlias>,
    pub limit: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailAutomationRuleStateRequest {
    pub rule_id: String,
    pub expected_version: u64,
    pub enabled: bool,
    pub confirm_change: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailAutomationRuleRemoveRequest {
    pub rule_id: String,
    pub expected_version: u64,
    pub confirm_delete_unused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailAutomationRuleActionView {
    pub target_agent_id: AgentId,
    pub target_agent_name: Option<String>,
    pub instruction: String,
    pub include_body: bool,
    pub max_body_bytes: usize,
    pub max_delivery_attempts: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailAutomationRuleView {
    pub id: String,
    pub account_alias: GmailAccountAlias,
    pub name: String,
    pub condition: GmailAutomationConditionSpec,
    pub action: GmailAutomationRuleActionView,
    pub enabled: bool,
    pub max_fires_per_hour: u16,
    pub state_version: u64,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GmailAutomationDeliveryState {
    Pending,
    Delivering,
    RetryWait,
    Delivered,
    Dead,
    Uncertain,
}

impl GmailAutomationDeliveryState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivering => "delivering",
            Self::RetryWait => "retry_wait",
            Self::Delivered => "delivered",
            Self::Dead => "dead",
            Self::Uncertain => "uncertain",
        }
    }
}

impl fmt::Display for GmailAutomationDeliveryState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for GmailAutomationDeliveryState {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "delivering" => Ok(Self::Delivering),
            "retry_wait" => Ok(Self::RetryWait),
            "delivered" => Ok(Self::Delivered),
            "dead" => Ok(Self::Dead),
            "uncertain" => Ok(Self::Uncertain),
            _ => Err(format!(
                "unsupported Gmail automation delivery state '{value}'"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailAutomationDeliveryQuery {
    pub delivery_id: Option<String>,
    pub status: Option<GmailAutomationDeliveryState>,
    pub limit: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailAutomationDeliveryRequeueRequest {
    pub delivery_id: String,
    pub expected_status: GmailAutomationDeliveryState,
    pub confirm_duplicate_risk: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailAutomationDeliveryView {
    pub id: String,
    pub event_id: String,
    pub target_agent_id: AgentId,
    pub target_agent_name: Option<String>,
    pub status: GmailAutomationDeliveryState,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub run_after_unix_ms: i64,
    pub lease_expires_at_unix_ms: Option<i64>,
    pub last_error: Option<String>,
    pub delivered_at_unix_ms: Option<i64>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
    pub delivery_session_id: String,
    pub rule_id: Option<String>,
    pub rule_version: Option<u64>,
    pub rule_name: Option<String>,
    pub account_alias: Option<GmailAccountAlias>,
    pub message_id: Option<String>,
    pub history_id: Option<String>,
    pub instruction: Option<String>,
    pub include_body: Option<bool>,
    pub payload_error: Option<String>,
    #[serde(default, skip_serializing)]
    pub untrusted_message_metadata: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_states_have_stable_wire_names() {
        for (state, wire) in [
            (GmailAutomationDeliveryState::Pending, "pending"),
            (GmailAutomationDeliveryState::Delivering, "delivering"),
            (GmailAutomationDeliveryState::RetryWait, "retry_wait"),
            (GmailAutomationDeliveryState::Delivered, "delivered"),
            (GmailAutomationDeliveryState::Dead, "dead"),
            (GmailAutomationDeliveryState::Uncertain, "uncertain"),
        ] {
            assert_eq!(state.as_str(), wire);
            assert_eq!(GmailAutomationDeliveryState::from_str(wire).unwrap(), state);
            assert_eq!(
                serde_json::to_string(&state).unwrap(),
                format!("\"{wire}\"")
            );
        }
    }

    #[test]
    fn delivery_serialization_never_includes_untrusted_metadata_implicitly() {
        let view = GmailAutomationDeliveryView {
            id: "delivery".to_string(),
            event_id: "event".to_string(),
            target_agent_id: AgentId::from_string("captain"),
            target_agent_name: Some("captain".to_string()),
            status: GmailAutomationDeliveryState::Uncertain,
            attempt_count: 1,
            max_attempts: 3,
            run_after_unix_ms: 1,
            lease_expires_at_unix_ms: None,
            last_error: None,
            delivered_at_unix_ms: None,
            created_at_unix_ms: 1,
            updated_at_unix_ms: 2,
            delivery_session_id: "session".to_string(),
            rule_id: None,
            rule_version: None,
            rule_name: None,
            account_alias: None,
            message_id: None,
            history_id: None,
            instruction: None,
            include_body: None,
            payload_error: None,
            untrusted_message_metadata: Some(serde_json::json!({"subject": "secret"})),
        };

        let encoded = serde_json::to_string(&view).unwrap();
        assert!(!encoded.contains("secret"));
        assert!(!encoded.contains("untrusted_message_metadata"));
    }
}
