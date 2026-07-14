use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentApiSpawnSheet {
    pub protocol: String,
    pub status: String,
    pub manifest_url: String,
    pub audit_events_url: String,
    pub ingress_status: String,
    pub ingress_url: String,
    pub auth_scheme: String,
    pub token_env: String,
    pub token: Option<String>,
    pub egress_status: String,
    pub egress_configure_url: String,
    pub egress_test_url: String,
    pub egress_queue_url: String,
    pub callback_secret: Option<String>,
    pub operator_actions: Vec<String>,
}

impl AgentApiSpawnSheet {
    pub(crate) fn from_spawn_body(body: &Value) -> Option<Self> {
        let provisioning = body.get("agent_api_provisioning")?;
        if !provisioning.is_object() {
            return None;
        }
        let ingress = &provisioning["ingress"];
        let egress = &provisioning["egress"];
        Some(Self {
            protocol: string_field(provisioning, "protocol", "agent-as-service.v1"),
            status: string_field(provisioning, "status", "?"),
            manifest_url: string_field(provisioning, "manifest_url", "?"),
            audit_events_url: string_field(provisioning, "audit_events_url", "?"),
            ingress_status: string_field(ingress, "status", "?"),
            ingress_url: string_field(ingress, "ingress_url", "?"),
            auth_scheme: string_field(ingress, "auth_scheme", "Authorization: Bearer $TOKEN"),
            token_env: string_field(ingress, "token_env", "?"),
            token: optional_string_field(ingress, "token"),
            egress_status: string_field(egress, "status", "?"),
            egress_configure_url: string_field(egress, "configure_url", "?"),
            egress_test_url: string_field(egress, "test_url", "?"),
            egress_queue_url: string_field(egress, "queue_status_url", "?"),
            callback_secret: optional_string_field(egress, "callback_secret"),
            operator_actions: string_array(provisioning, "operator_actions"),
        })
    }

    pub(crate) fn cli_lines(&self) -> Vec<String> {
        self.lines(true)
    }

    pub(crate) fn tui_notice_lines(&self) -> Vec<String> {
        let mut lines = vec!["Agent API provisioned".to_string()];
        lines.extend(self.lines(true));
        if self.token.is_some() || self.callback_secret.is_some() {
            lines.push("Secrets shown once here are not saved in chat history.".to_string());
        }
        lines
    }

    fn lines(&self, include_secrets: bool) -> Vec<String> {
        let mut lines = vec![
            format!("Protocol: {}", self.protocol),
            format!("Status: {}", self.status),
            format!("Manifest: {}", self.manifest_url),
            format!("Events: {}", self.audit_events_url),
            String::new(),
            format!("Ingress: {}", self.ingress_status),
            format!("Ingress URL: {}", self.ingress_url),
            format!("Auth: {}", self.auth_scheme),
            format!("Token env: {}", self.token_env),
        ];
        if include_secrets {
            if let Some(token) = self.token.as_deref() {
                lines.push(format!("Token: {token}"));
            }
        }

        lines.extend([
            String::new(),
            format!("Egress: {}", self.egress_status),
            format!("Configure: {}", self.egress_configure_url),
            format!("Test: {}", self.egress_test_url),
            format!("Queue: {}", self.egress_queue_url),
        ]);
        if include_secrets {
            if let Some(secret) = self.callback_secret.as_deref() {
                lines.push(format!("Secret: {secret}"));
            }
        }

        if !self.operator_actions.is_empty() {
            lines.push(String::new());
            lines.push("Operator actions:".to_string());
            lines.extend(
                self.operator_actions
                    .iter()
                    .map(|action| format!("- {action}")),
            );
        }

        lines
    }
}

fn string_field(value: &Value, key: &str, fallback: &str) -> String {
    value[key].as_str().unwrap_or(fallback).to_string()
}

fn optional_string_field(value: &Value, key: &str) -> Option<String> {
    value[key]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn string_array(value: &Value, key: &str) -> Vec<String> {
    value[key]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn spawn_sheet_maps_agent_api_provisioning() {
        let body = json!({
            "agent_api_provisioning": {
                "protocol": "agent-as-service.v1",
                "status": "ingress_ready",
                "manifest_url": "/api/agents/a1/api/manifest",
                "audit_events_url": "/api/agents/a1/api/events",
                "ingress": {
                    "status": "ready",
                    "ingress_url": "/hooks/agents/a1/ingress",
                    "auth_scheme": "Authorization: Bearer $TOKEN",
                    "token_env": "CAPTAIN_AGENT_A1_TOKEN",
                    "token": "secret-token"
                },
                "egress": {
                    "status": "pending_callback_url",
                    "configure_url": "/api/agents/a1/api/egress/configure",
                    "test_url": "/api/agents/a1/api/egress/test",
                    "queue_status_url": "/api/agents/a1/api/egress/queue",
                    "callback_secret": "secret-hmac"
                },
                "operator_actions": [
                    "Ingress is ready, but Captain cannot infer the external callback URL."
                ]
            }
        });

        let sheet = AgentApiSpawnSheet::from_spawn_body(&body).expect("sheet");

        assert_eq!(sheet.status, "ingress_ready");
        assert_eq!(sheet.token.as_deref(), Some("secret-token"));
        assert_eq!(sheet.callback_secret.as_deref(), Some("secret-hmac"));
        assert!(sheet
            .operator_actions
            .iter()
            .any(|action| action.contains("cannot infer the external callback URL")));
        assert!(sheet.tui_notice_lines().join("\n").contains("not saved"));
    }

    #[test]
    fn spawn_sheet_is_absent_without_provisioning() {
        assert_eq!(AgentApiSpawnSheet::from_spawn_body(&json!({})), None);
    }
}
