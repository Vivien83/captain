//! Operator-facing configuration status for per-agent APIs.

use crate::{
    agent_api_egress::AgentApiCallbackConfigStatus,
    agent_api_egress_queue::agent_api_egress_queue_summary, agent_api_routes::AgentApiDescriptor,
};
use captain_types::agent::AgentId;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct AgentApiConfigStatus {
    pub state: &'static str,
    pub can_receive: bool,
    pub can_send_callbacks: bool,
    pub operator_action_required: bool,
    pub operator_actions: Vec<String>,
    pub ingress: AgentApiIngressConfigStatus,
    pub egress: AgentApiEgressConfigStatus,
    pub queue: AgentApiQueueConfigStatus,
}

#[derive(Debug, Serialize)]
pub struct AgentApiIngressConfigStatus {
    pub state: &'static str,
    pub ingress_url: String,
    pub token_env: String,
    pub token_configured: bool,
    pub token_rotate_url: String,
    pub auth_scheme: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AgentApiEgressConfigStatus {
    pub state: &'static str,
    pub ready: bool,
    pub callback_url_env: String,
    pub callback_url_configured: bool,
    pub callback_url_valid: bool,
    pub callback_secret_env: String,
    pub callback_secret_configured: bool,
    pub configure_url: String,
    pub test_url: String,
    pub queue_status_url: String,
    pub retry_url_template: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AgentApiQueueConfigStatus {
    pub state: &'static str,
    pub readable: bool,
    pub pending: usize,
    pub due: usize,
    pub dead_letters: usize,
    pub status_url: String,
    pub retry_url_template: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue: Option<String>,
}

pub async fn agent_api_config_status(
    home_dir: &Path,
    agent_id: &AgentId,
    descriptor: &AgentApiDescriptor,
) -> AgentApiConfigStatus {
    let ingress = ingress_status(descriptor);
    let egress = egress_status(&descriptor.egress.config_status, descriptor);
    let queue = queue_status(home_dir, agent_id, descriptor).await;
    let mut actions = Vec::new();

    if !ingress.token_configured {
        actions.push(format!(
            "Set {} to a bearer token with at least 32 characters.",
            ingress.token_env
        ));
    }
    if egress.state == "misconfigured" {
        actions.push(
            egress
                .issue
                .clone()
                .unwrap_or_else(|| "Fix agent API callback configuration.".to_string()),
        );
    } else if egress.state == "disabled" {
        actions.push(format!(
            "Configure signed callback egress with {} before treating this agent API as fully in/out ready.",
            egress.configure_url
        ));
    }
    if !queue.readable {
        actions.push(
            queue
                .issue
                .clone()
                .unwrap_or_else(|| "Repair the agent API egress queue store.".to_string()),
        );
    } else if queue.dead_letters > 0 {
        actions.push(format!(
            "Review {} dead-lettered agent API callback(s) and retry or fix the target.",
            queue.dead_letters
        ));
    }

    let state = if !ingress.token_configured {
        "not_ready"
    } else if egress.state == "disabled" {
        "ingress_ready"
    } else if egress.state == "misconfigured" || !queue.readable || queue.dead_letters > 0 {
        "degraded"
    } else {
        "ready"
    };

    AgentApiConfigStatus {
        state,
        can_receive: ingress.token_configured,
        can_send_callbacks: egress.ready,
        operator_action_required: !actions.is_empty(),
        operator_actions: actions,
        ingress,
        egress,
        queue,
    }
}

fn ingress_status(descriptor: &AgentApiDescriptor) -> AgentApiIngressConfigStatus {
    let issue = (!descriptor.token_configured)
        .then(|| format!("{} is missing or too short.", descriptor.token_env));
    AgentApiIngressConfigStatus {
        state: if descriptor.token_configured {
            "ready"
        } else {
            "missing_token"
        },
        ingress_url: descriptor.ingress_url.clone(),
        token_env: descriptor.token_env.clone(),
        token_configured: descriptor.token_configured,
        token_rotate_url: descriptor.token_rotate_url.clone(),
        auth_scheme: descriptor.auth_scheme,
        issue,
    }
}

fn egress_status(
    callback: &AgentApiCallbackConfigStatus,
    descriptor: &AgentApiDescriptor,
) -> AgentApiEgressConfigStatus {
    AgentApiEgressConfigStatus {
        state: callback.state,
        ready: callback.ready,
        callback_url_env: callback.callback_url_env.clone(),
        callback_url_configured: callback.callback_url_configured,
        callback_url_valid: callback.callback_url_valid,
        callback_secret_env: callback.callback_secret_env.clone(),
        callback_secret_configured: callback.callback_secret_configured,
        configure_url: descriptor.egress.configure_url.clone(),
        test_url: descriptor.egress.test_url.clone(),
        queue_status_url: descriptor.egress.queue_status_url.clone(),
        retry_url_template: descriptor.egress.retry_url_template.clone(),
        issue: callback.issue.clone(),
    }
}

async fn queue_status(
    home_dir: &Path,
    agent_id: &AgentId,
    descriptor: &AgentApiDescriptor,
) -> AgentApiQueueConfigStatus {
    match agent_api_egress_queue_summary(home_dir, agent_id).await {
        Ok(summary) => AgentApiQueueConfigStatus {
            state: if summary.dead_letters > 0 {
                "attention"
            } else if summary.pending > 0 {
                "retrying"
            } else {
                "idle"
            },
            readable: true,
            pending: summary.pending,
            due: summary.due,
            dead_letters: summary.dead_letters,
            status_url: descriptor.egress.queue_status_url.clone(),
            retry_url_template: descriptor.egress.retry_url_template.clone(),
            issue: None,
        },
        Err(err) => AgentApiQueueConfigStatus {
            state: "unavailable",
            readable: false,
            pending: 0,
            due: 0,
            dead_letters: 0,
            status_url: descriptor.egress.queue_status_url.clone(),
            retry_url_template: descriptor.egress.retry_url_template.clone(),
            issue: Some(err),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent_api_egress::{agent_api_callback_secret_env, agent_api_callback_url_env},
        agent_api_egress_queue::enqueue_agent_api_callback,
        agent_api_routes::{agent_api_descriptor, agent_api_token_env},
    };

    fn agent_id(value: &str) -> AgentId {
        value.parse().unwrap()
    }

    #[tokio::test]
    async fn missing_ingress_token_is_not_ready() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_id = agent_id("11111111-1111-1111-1111-111111111111");
        std::env::remove_var(agent_api_token_env(&agent_id));

        let descriptor = agent_api_descriptor(&agent_id);
        let status = agent_api_config_status(tmp.path(), &agent_id, &descriptor).await;

        assert_eq!(status.state, "not_ready");
        assert!(!status.can_receive);
        assert!(status.operator_action_required);
        assert!(status.operator_actions[0].contains("CAPTAIN_AGENT_API_TOKEN"));
    }

    #[tokio::test]
    async fn ready_status_does_not_expose_secret_values() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_id = agent_id("22222222-2222-2222-2222-222222222222");
        let token = "token-value-token-value-token-value-32";
        let secret = "secret-value-secret-value-32";
        let callback_url = "https://example.com/hook?private=hidden";
        std::env::set_var(agent_api_token_env(&agent_id), token);
        std::env::set_var(agent_api_callback_url_env(&agent_id), callback_url);
        std::env::set_var(agent_api_callback_secret_env(&agent_id), secret);

        let descriptor = agent_api_descriptor(&agent_id);
        let status = agent_api_config_status(tmp.path(), &agent_id, &descriptor).await;
        let encoded = serde_json::to_string(&status).unwrap();

        assert_eq!(status.state, "ready");
        assert!(status.can_receive);
        assert!(status.can_send_callbacks);
        assert!(!encoded.contains(token));
        assert!(!encoded.contains(secret));
        assert!(!encoded.contains(callback_url));

        std::env::remove_var(agent_api_token_env(&agent_id));
        std::env::remove_var(agent_api_callback_url_env(&agent_id));
        std::env::remove_var(agent_api_callback_secret_env(&agent_id));
    }

    #[tokio::test]
    async fn pending_queue_is_visible_without_degrading_ingress() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_id = agent_id("33333333-3333-3333-3333-333333333333");
        std::env::set_var(
            agent_api_token_env(&agent_id),
            "token-value-token-value-token-value-33",
        );
        enqueue_agent_api_callback(
            tmp.path(),
            &agent_id,
            &serde_json::json!({"event": "agent_api.completed"}),
            Some("HTTP 503"),
        )
        .await
        .unwrap();

        let descriptor = agent_api_descriptor(&agent_id);
        let status = agent_api_config_status(tmp.path(), &agent_id, &descriptor).await;

        assert_eq!(status.state, "ingress_ready");
        assert_eq!(status.queue.state, "retrying");
        assert_eq!(status.queue.pending, 1);
        assert_eq!(status.queue.dead_letters, 0);

        std::env::remove_var(agent_api_token_env(&agent_id));
    }

    #[tokio::test]
    async fn ingress_only_status_requires_egress_operator_action() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_id = agent_id("44444444-4444-4444-4444-444444444445");
        std::env::set_var(
            agent_api_token_env(&agent_id),
            "token-value-token-value-token-value-44",
        );
        std::env::remove_var(agent_api_callback_url_env(&agent_id));
        std::env::remove_var(agent_api_callback_secret_env(&agent_id));

        let descriptor = agent_api_descriptor(&agent_id);
        let status = agent_api_config_status(tmp.path(), &agent_id, &descriptor).await;

        assert_eq!(status.state, "ingress_ready");
        assert!(status.can_receive);
        assert!(!status.can_send_callbacks);
        assert!(status.operator_action_required);
        assert!(status.operator_actions[0].contains("/api/agents/"));
        assert!(status.operator_actions[0].contains("/api/egress/configure"));

        std::env::remove_var(agent_api_token_env(&agent_id));
    }
}
