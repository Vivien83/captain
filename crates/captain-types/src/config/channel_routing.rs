use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Agent binding - routes specific channel/account/peer patterns to agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentBinding {
    /// Target agent name or ID.
    pub agent: String,
    /// Match criteria (all specified fields must match).
    pub match_rule: BindingMatchRule,
}

/// Match rule for agent bindings. All specified (non-None) fields must match.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BindingMatchRule {
    /// Channel type (e.g., "discord", "telegram", "slack").
    pub channel: Option<String>,
    /// Specific account/bot ID within the channel.
    pub account_id: Option<String>,
    /// Peer/user ID for DM routing.
    pub peer_id: Option<String>,
    /// Guild/server ID (Discord/Slack).
    pub guild_id: Option<String>,
    /// Role-based routing (user must have at least one).
    #[serde(default)]
    pub roles: Vec<String>,
}

impl BindingMatchRule {
    /// Calculate specificity score for binding priority ordering.
    /// Higher = more specific = checked first.
    pub fn specificity(&self) -> u32 {
        let mut score = 0u32;
        if self.peer_id.is_some() {
            score += 8;
        }
        if self.guild_id.is_some() {
            score += 4;
        }
        if !self.roles.is_empty() {
            score += 2;
        }
        if self.account_id.is_some() {
            score += 2;
        }
        if self.channel.is_some() {
            score += 1;
        }
        score
    }
}

/// Broadcast config - send same message to multiple agents.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BroadcastConfig {
    /// Broadcast strategy.
    pub strategy: BroadcastStrategy,
    /// Map of peer_id -> list of agent names to receive the message.
    pub routes: HashMap<String, Vec<String>>,
}

/// Broadcast delivery strategy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BroadcastStrategy {
    /// Send to all agents simultaneously.
    #[default]
    Parallel,
    /// Send to agents one at a time in order.
    Sequential,
}

/// Auto-reply engine configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoReplyConfig {
    /// Enable auto-reply engine. Default: false.
    pub enabled: bool,
    /// Max concurrent auto-reply tasks. Default: 3.
    pub max_concurrent: usize,
    /// Default timeout per reply in seconds. Default: 120.
    pub timeout_secs: u64,
    /// Patterns that suppress auto-reply (e.g., "/stop", "/pause").
    pub suppress_patterns: Vec<String>,
}

impl Default for AutoReplyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_concurrent: 3,
            timeout_secs: 120,
            suppress_patterns: vec!["/stop".to_string(), "/pause".to_string()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AutoReplyConfig, BindingMatchRule, BroadcastConfig, BroadcastStrategy};
    use crate::config::KernelConfig;

    #[test]
    fn binding_specificity_scores_more_specific_rules_higher() {
        let empty = BindingMatchRule::default();
        let channel_only = BindingMatchRule {
            channel: Some("telegram".to_string()),
            ..Default::default()
        };
        let full = BindingMatchRule {
            channel: Some("discord".to_string()),
            account_id: Some("bot".to_string()),
            peer_id: Some("user".to_string()),
            guild_id: Some("guild".to_string()),
            roles: vec!["admin".to_string()],
        };

        assert_eq!(empty.specificity(), 0);
        assert_eq!(channel_only.specificity(), 1);
        assert_eq!(full.specificity(), 17);
    }

    #[test]
    fn broadcast_and_auto_reply_defaults_match_runtime_contract() {
        let broadcast = BroadcastConfig::default();
        let auto_reply = AutoReplyConfig::default();

        assert_eq!(broadcast.strategy, BroadcastStrategy::Parallel);
        assert!(broadcast.routes.is_empty());
        assert!(!auto_reply.enabled);
        assert_eq!(auto_reply.max_concurrent, 3);
        assert_eq!(auto_reply.timeout_secs, 120);
        assert_eq!(auto_reply.suppress_patterns, vec!["/stop", "/pause"]);
    }

    #[test]
    fn routing_sections_deserialize_from_kernel_toml() {
        let config: KernelConfig = toml::from_str(
            r#"
            [[bindings]]
            agent = "ops"

            [bindings.match_rule]
            channel = "discord"
            peer_id = "user-1"
            roles = ["admin"]

            [broadcast]
            strategy = "sequential"

            [broadcast.routes]
            "peer-1" = ["ops", "scribe"]

            [auto_reply]
            enabled = true
            max_concurrent = 2
            timeout_secs = 45
            suppress_patterns = ["/stop"]
            "#,
        )
        .unwrap();

        assert_eq!(config.bindings.len(), 1);
        assert_eq!(config.bindings[0].agent, "ops");
        assert_eq!(config.bindings[0].match_rule.specificity(), 11);
        assert_eq!(config.broadcast.strategy, BroadcastStrategy::Sequential);
        assert_eq!(
            config.broadcast.routes.get("peer-1"),
            Some(&vec!["ops".to_string(), "scribe".to_string()])
        );
        assert!(config.auto_reply.enabled);
        assert_eq!(config.auto_reply.max_concurrent, 2);
        assert_eq!(config.auto_reply.timeout_secs, 45);
        assert_eq!(config.auto_reply.suppress_patterns, vec!["/stop"]);
    }
}
