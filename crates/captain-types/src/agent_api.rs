//! Shared agent-as-service API contract helpers.

use crate::agent::AgentId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const AGENT_API_PROTOCOL_VERSION: &str = "agent-as-service.v1";
pub const AGENT_API_AUTH_SCHEME: &str = "Authorization: Bearer $TOKEN";
pub const AGENT_API_CHANNEL_TYPE: &str = "agent_api";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentApiSpawnProvisionRequest {
    #[serde(default = "default_true")]
    pub provision_ingress_token: bool,
    #[serde(default)]
    pub egress_callback_url: Option<String>,
    #[serde(default)]
    pub egress_callback_secret: Option<String>,
    #[serde(default = "default_true")]
    pub generate_callback_secret: bool,
}

impl Default for AgentApiSpawnProvisionRequest {
    fn default() -> Self {
        Self {
            provision_ingress_token: true,
            egress_callback_url: None,
            egress_callback_secret: None,
            generate_callback_secret: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentApiSpawnProvisionReport {
    pub protocol: String,
    pub status: String,
    pub agent_id: String,
    pub manifest_url: String,
    pub audit_events_url: String,
    pub ingress: AgentApiIngressProvisionReport,
    pub egress: AgentApiEgressProvisionReport,
    pub operator_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentApiIngressProvisionReport {
    pub status: String,
    pub ingress_url: String,
    pub auth_scheme: String,
    pub token_env: String,
    pub token_rotate_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentApiEgressProvisionReport {
    pub status: String,
    pub callback_url_env: String,
    pub callback_secret_env: String,
    pub configure_url: String,
    pub test_url: String,
    pub queue_status_url: String,
    pub retry_url_template: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue: Option<String>,
}

impl AgentApiSpawnProvisionReport {
    pub fn new(
        agent_id: &AgentId,
        ingress: AgentApiIngressProvisionReport,
        egress: AgentApiEgressProvisionReport,
        mut operator_actions: Vec<String>,
    ) -> Self {
        if egress.status == "pending_callback_url" && operator_actions.is_empty() {
            operator_actions.push(pending_egress_operator_action(agent_id));
        }
        let status = if ingress.status == "ready" && egress.status == "ready" {
            "ready"
        } else if ingress.status == "ready" {
            "ingress_ready"
        } else {
            "not_ready"
        };
        Self {
            protocol: AGENT_API_PROTOCOL_VERSION.to_string(),
            status: status.to_string(),
            agent_id: agent_id.to_string(),
            manifest_url: agent_api_manifest_url(agent_id),
            audit_events_url: agent_api_audit_events_url(agent_id),
            ingress,
            egress,
            operator_actions,
        }
    }
}

pub fn pending_egress_operator_action(agent_id: &AgentId) -> String {
    format!(
        "Ingress is ready, but Captain cannot infer the external callback URL for outbound events. Configure signed callback egress with {} before treating the agent API as fully in/out ready.",
        agent_api_egress_configure_url(agent_id)
    )
}

pub fn ready_ingress_report(agent_id: &AgentId, token: String) -> AgentApiIngressProvisionReport {
    AgentApiIngressProvisionReport {
        status: "ready".to_string(),
        ingress_url: agent_api_ingress_url(agent_id),
        auth_scheme: AGENT_API_AUTH_SCHEME.to_string(),
        token_env: agent_api_token_env(agent_id),
        token_rotate_url: agent_api_token_rotate_url(agent_id),
        token: Some(token),
        warning: Some(
            "Token is returned once. Store it in the external service and use Authorization: Bearer <token>."
                .to_string(),
        ),
    }
}

pub fn skipped_ingress_report(agent_id: &AgentId) -> AgentApiIngressProvisionReport {
    AgentApiIngressProvisionReport {
        status: "skipped".to_string(),
        ingress_url: agent_api_ingress_url(agent_id),
        auth_scheme: AGENT_API_AUTH_SCHEME.to_string(),
        token_env: agent_api_token_env(agent_id),
        token_rotate_url: agent_api_token_rotate_url(agent_id),
        token: None,
        warning: Some("Ingress token provisioning was skipped by request.".to_string()),
    }
}

pub fn pending_egress_report(agent_id: &AgentId) -> AgentApiEgressProvisionReport {
    AgentApiEgressProvisionReport {
        status: "pending_callback_url".to_string(),
        callback_url_env: agent_api_callback_url_env(agent_id),
        callback_secret_env: agent_api_callback_secret_env(agent_id),
        configure_url: agent_api_egress_configure_url(agent_id),
        test_url: agent_api_egress_test_url(agent_id),
        queue_status_url: agent_api_egress_queue_url(agent_id),
        retry_url_template: agent_api_egress_retry_url_template(agent_id),
        callback_secret: None,
        warning: Some(
            "Captain cannot infer the external callback URL. Provide callback_url at creation or call the configure endpoint to enable signed outbound callbacks."
                .to_string(),
        ),
        issue: Some("egress callback URL is not configured".to_string()),
    }
}

pub fn ready_egress_report(
    agent_id: &AgentId,
    callback_secret: Option<String>,
) -> AgentApiEgressProvisionReport {
    AgentApiEgressProvisionReport {
        status: "ready".to_string(),
        callback_url_env: agent_api_callback_url_env(agent_id),
        callback_secret_env: agent_api_callback_secret_env(agent_id),
        configure_url: agent_api_egress_configure_url(agent_id),
        test_url: agent_api_egress_test_url(agent_id),
        queue_status_url: agent_api_egress_queue_url(agent_id),
        retry_url_template: agent_api_egress_retry_url_template(agent_id),
        callback_secret,
        warning: Some(
            "Generated callback secrets are returned once. Normal status responses only expose env names and readiness."
                .to_string(),
        ),
        issue: None,
    }
}

pub fn failed_egress_report(agent_id: &AgentId, issue: String) -> AgentApiEgressProvisionReport {
    AgentApiEgressProvisionReport {
        status: "failed".to_string(),
        callback_url_env: agent_api_callback_url_env(agent_id),
        callback_secret_env: agent_api_callback_secret_env(agent_id),
        configure_url: agent_api_egress_configure_url(agent_id),
        test_url: agent_api_egress_test_url(agent_id),
        queue_status_url: agent_api_egress_queue_url(agent_id),
        retry_url_template: agent_api_egress_retry_url_template(agent_id),
        callback_secret: None,
        warning: None,
        issue: Some(issue),
    }
}

pub fn agent_api_token_env(agent_id: &AgentId) -> String {
    format!("CAPTAIN_AGENT_API_TOKEN_{}", agent_api_env_suffix(agent_id))
}

pub fn agent_api_callback_url_env(agent_id: &AgentId) -> String {
    format!(
        "CAPTAIN_AGENT_API_CALLBACK_URL_{}",
        agent_api_env_suffix(agent_id)
    )
}

pub fn agent_api_callback_secret_env(agent_id: &AgentId) -> String {
    format!(
        "CAPTAIN_AGENT_API_CALLBACK_SECRET_{}",
        agent_api_env_suffix(agent_id)
    )
}

pub fn agent_api_ingress_url(agent_id: &AgentId) -> String {
    format!("/hooks/agents/{agent_id}/ingress")
}

pub fn agent_api_token_rotate_url(agent_id: &AgentId) -> String {
    format!("/api/agents/{agent_id}/api/token/rotate")
}

pub fn agent_api_manifest_url(agent_id: &AgentId) -> String {
    format!("/api/agents/{agent_id}/api/manifest")
}

pub fn agent_api_audit_events_url(agent_id: &AgentId) -> String {
    format!("/api/agents/{agent_id}/api/events")
}

pub fn agent_api_egress_configure_url(agent_id: &AgentId) -> String {
    format!("/api/agents/{agent_id}/api/egress/configure")
}

pub fn agent_api_egress_test_url(agent_id: &AgentId) -> String {
    format!("/api/agents/{agent_id}/api/egress/test")
}

pub fn agent_api_egress_queue_url(agent_id: &AgentId) -> String {
    format!("/api/agents/{agent_id}/api/egress")
}

pub fn agent_api_egress_retry_url_template(agent_id: &AgentId) -> String {
    format!("/api/agents/{agent_id}/api/egress/{{queue_id}}/retry")
}

pub fn generate_agent_api_token() -> String {
    format!(
        "cap_at_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

pub fn generate_agent_api_callback_secret() -> String {
    format!(
        "cap_cb_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

fn agent_api_env_suffix(agent_id: &AgentId) -> String {
    agent_id.to_string().replace('-', "_").to_ascii_uppercase()
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_agent_id() -> AgentId {
        "11111111-2222-3333-4444-555555555555".parse().unwrap()
    }

    #[test]
    fn env_names_match_agent_id_suffix_contract() {
        let agent_id = sample_agent_id();

        assert_eq!(
            agent_api_token_env(&agent_id),
            "CAPTAIN_AGENT_API_TOKEN_11111111_2222_3333_4444_555555555555"
        );
        assert_eq!(
            agent_api_callback_url_env(&agent_id),
            "CAPTAIN_AGENT_API_CALLBACK_URL_11111111_2222_3333_4444_555555555555"
        );
        assert_eq!(
            agent_api_callback_secret_env(&agent_id),
            "CAPTAIN_AGENT_API_CALLBACK_SECRET_11111111_2222_3333_4444_555555555555"
        );
    }

    #[test]
    fn default_spawn_provision_enables_ingress_and_generated_egress_secret() {
        let req = AgentApiSpawnProvisionRequest::default();

        assert!(req.provision_ingress_token);
        assert!(req.generate_callback_secret);
        assert!(req.egress_callback_url.is_none());
    }
}
