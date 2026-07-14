use std::collections::HashMap;

use captain_extensions::{
    InstalledIntegration, IntegrationTemplate, McpTransportTemplate, RequiredEnvVar,
};
use captain_types::config::{McpServerConfigEntry, McpTransportEntry};
use serde_json::{Map, Value};

use super::CaptainKernel;

impl CaptainKernel {
    pub(super) fn handle_publish_integration_configured(&self, name: &str) {
        use captain_types::agent::AgentId;
        use captain_types::event::{Event, EventPayload, EventTarget, SystemEvent};
        let event = Event::new(
            AgentId::new(),
            EventTarget::System,
            EventPayload::System(SystemEvent::IntegrationConfigured {
                name: name.to_string(),
            }),
        );
        let bus = self.event_bus.clone();
        tokio::spawn(async move {
            bus.publish(event).await;
        });
    }

    pub(super) async fn handle_mcp_catalog_search(
        &self,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Value, String> {
        let registry = self
            .extension_registry
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let resolver = self
            .credential_resolver
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let limit = clamp_catalog_limit(limit);
        let templates = match query.map(str::trim).filter(|q| !q.is_empty()) {
            Some(q) => registry.search(q),
            None => registry.list_templates(),
        };

        let items: Vec<Value> = templates
            .into_iter()
            .take(limit)
            .map(|template| {
                let installed = registry.get_installed(&template.id);
                let required_keys: Vec<&str> = template
                    .required_env
                    .iter()
                    .map(|env| env.name.as_str())
                    .collect();
                let missing_credentials = resolver.missing_credentials(&required_keys);
                catalog_template_json(template, installed, &missing_credentials)
            })
            .collect();

        Ok(serde_json::json!({
            "query": query.unwrap_or(""),
            "count": items.len(),
            "results": items,
            "next_step": "If a template matches, call mcp_integration_install with its id. Store credentials only via the tool credentials object/vault path; do not hand-write raw secrets into files.",
        }))
    }

    pub(super) async fn handle_mcp_integration_install(
        &self,
        id: &str,
        credentials: Value,
        reload: bool,
    ) -> Result<Value, String> {
        let id = id.trim();
        if id.is_empty() {
            return Err("Missing MCP integration id".into());
        }
        let credentials = credentials
            .as_object()
            .ok_or_else(|| "credentials must be an object".to_string())?;

        let (result, missing_credentials) = {
            let mut registry = self
                .extension_registry
                .write()
                .unwrap_or_else(|e| e.into_inner());
            let template = registry
                .get_template(id)
                .ok_or_else(|| format!("Unknown MCP integration template: {id}"))?
                .clone();

            let provided_keys = provided_credentials_for_required_env(
                template.required_env.as_slice(),
                credentials,
            );

            for (key, value) in &provided_keys {
                self.handle_secret_write(key, value)
                    .map_err(|e| format!("Failed to persist MCP credential '{key}': {e}"))?;
            }

            let mut resolver = self
                .credential_resolver
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let result = captain_extensions::installer::install_integration(
                &mut registry,
                &mut resolver,
                id,
                &provided_keys,
            )
            .map_err(|e| e.to_string())?;
            let required_keys: Vec<&str> = template
                .required_env
                .iter()
                .map(|env| env.name.as_str())
                .collect();
            let missing = resolver.missing_credentials(&required_keys);
            (result, missing)
        };

        let reload_result = if reload {
            match self.reload_extension_mcps().await {
                Ok(connected) => serde_json::json!({
                    "attempted": true,
                    "connected_new_servers": connected,
                }),
                Err(e) => serde_json::json!({
                    "attempted": true,
                    "error": e,
                }),
            }
        } else {
            serde_json::json!({
                "attempted": false,
                "reason": "reload=false",
            })
        };

        Ok(serde_json::json!({
            "id": result.id,
            "status": result.status,
            "message": result.message,
            "missing_credentials": missing_credentials,
            "reload": reload_result,
            "tool_namespace": format!("mcp_{}_*", captain_runtime::mcp::normalize_name(id)),
            "next_step": "Call mcp_status or capability_search with sources:[\"mcp\"] to verify that the expected tools are visible before claiming success.",
        }))
    }

    pub(super) async fn handle_mcp_status(&self) -> Result<Value, String> {
        let configured = self
            .effective_mcp_servers
            .read()
            .map(|servers| {
                servers
                    .iter()
                    .map(configured_mcp_server_json)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let connections = self.mcp_connections.lock().await;
        let connected: Vec<Value> = connections
            .iter()
            .map(|conn| {
                serde_json::json!({
                    "name": conn.name(),
                    "tool_count": conn.tools().len(),
                    "tools": conn.tools().iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
                })
            })
            .collect();

        Ok(serde_json::json!({
            "configured_count": configured.len(),
            "connected_count": connected.len(),
            "configured": configured,
            "connected": connected,
        }))
    }
}

fn clamp_catalog_limit(limit: usize) -> usize {
    limit.clamp(1, 50)
}

fn catalog_template_json(
    template: &IntegrationTemplate,
    installed: Option<&InstalledIntegration>,
    missing_credentials: &[String],
) -> Value {
    serde_json::json!({
        "id": template.id,
        "name": template.name,
        "description": template.description,
        "category": template.category.to_string(),
        "status": template_status(installed, missing_credentials),
        "tags": template.tags,
        "transport": transport_template_json(&template.transport),
        "required_env": required_env_json(&template.required_env, missing_credentials),
        "tool_namespace": format!("mcp_{}_*", captain_runtime::mcp::normalize_name(&template.id)),
        "setup_instructions": template.setup_instructions,
    })
}

fn template_status(
    installed: Option<&InstalledIntegration>,
    missing_credentials: &[String],
) -> &'static str {
    match installed {
        Some(inst) if !inst.enabled => "disabled",
        Some(_) if missing_credentials.is_empty() => "ready",
        Some(_) => "setup",
        None => "available",
    }
}

fn transport_template_json(transport: &McpTransportTemplate) -> Value {
    match transport {
        McpTransportTemplate::Stdio { command, args } => serde_json::json!({
            "type": "stdio",
            "command": command,
            "args": args,
        }),
        McpTransportTemplate::Sse { url } => serde_json::json!({
            "type": "sse",
            "url": url,
        }),
    }
}

fn required_env_json(
    required_env: &[RequiredEnvVar],
    missing_credentials: &[String],
) -> Vec<Value> {
    required_env
        .iter()
        .map(|env| {
            serde_json::json!({
                "name": env.name,
                "label": env.label,
                "help": env.help,
                "is_secret": env.is_secret,
                "get_url": env.get_url,
                "present": !missing_credentials.contains(&env.name),
            })
        })
        .collect()
}

fn provided_credentials_for_required_env(
    required_env: &[RequiredEnvVar],
    credentials: &Map<String, Value>,
) -> HashMap<String, String> {
    let mut provided_keys = HashMap::new();
    for env in required_env {
        if let Some(value) = credentials.get(&env.name).and_then(Value::as_str) {
            provided_keys.insert(env.name.clone(), value.to_string());
        }
    }
    if provided_keys.is_empty() && required_env.len() == 1 {
        let env_name = &required_env[0].name;
        for alias in ["api_key", "token", "key", "value"] {
            if let Some(value) = credentials.get(alias).and_then(Value::as_str) {
                provided_keys.insert(env_name.clone(), value.to_string());
                break;
            }
        }
    }
    provided_keys
}

fn configured_mcp_server_json(server: &McpServerConfigEntry) -> Value {
    serde_json::json!({
        "name": server.name,
        "transport": mcp_transport_entry_type(&server.transport),
        "env": server.env,
        "auth_token_env": server.auth_token_env,
        "timeout_secs": server.timeout_secs,
    })
}

fn mcp_transport_entry_type(transport: &McpTransportEntry) -> &'static str {
    match transport {
        McpTransportEntry::Stdio { .. } => "stdio",
        McpTransportEntry::Sse { .. } => "sse",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_extensions::{HealthCheckConfig, IntegrationCategory};
    use chrono::Utc;

    fn required_env(name: &str) -> RequiredEnvVar {
        RequiredEnvVar {
            name: name.to_string(),
            label: format!("{name} label"),
            help: "create a token".to_string(),
            is_secret: true,
            get_url: Some("https://example.com/token".to_string()),
        }
    }

    fn installed(enabled: bool) -> InstalledIntegration {
        InstalledIntegration {
            id: "github".to_string(),
            installed_at: Utc::now(),
            enabled,
            oauth_provider: None,
            config: HashMap::new(),
        }
    }

    #[test]
    fn catalog_limit_is_clamped_to_public_window() {
        assert_eq!(clamp_catalog_limit(0), 1);
        assert_eq!(clamp_catalog_limit(24), 24);
        assert_eq!(clamp_catalog_limit(200), 50);
    }

    #[test]
    fn template_status_matches_installed_and_credential_state() {
        assert_eq!(template_status(None, &[]), "available");
        assert_eq!(template_status(Some(&installed(false)), &[]), "disabled");
        assert_eq!(template_status(Some(&installed(true)), &[]), "ready");
        assert_eq!(
            template_status(Some(&installed(true)), &["GITHUB_TOKEN".to_string()]),
            "setup"
        );
    }

    #[test]
    fn provided_credentials_accept_alias_for_single_required_env() {
        let mut credentials = Map::new();
        credentials.insert("api_key".to_string(), Value::String("secret".to_string()));

        let provided =
            provided_credentials_for_required_env(&[required_env("GITHUB_TOKEN")], &credentials);

        assert_eq!(
            provided.get("GITHUB_TOKEN").map(String::as_str),
            Some("secret")
        );
    }

    #[test]
    fn catalog_template_json_marks_missing_credentials() {
        let template = IntegrationTemplate {
            id: "github".to_string(),
            name: "GitHub".to_string(),
            description: "GitHub MCP".to_string(),
            category: IntegrationCategory::DevTools,
            icon: String::new(),
            transport: McpTransportTemplate::Stdio {
                command: "github-mcp".to_string(),
                args: vec!["stdio".to_string()],
            },
            required_env: vec![required_env("GITHUB_TOKEN")],
            oauth: None,
            tags: vec!["git".to_string()],
            setup_instructions: "Add token".to_string(),
            health_check: HealthCheckConfig::default(),
        };

        let payload = catalog_template_json(
            &template,
            Some(&installed(true)),
            &["GITHUB_TOKEN".to_string()],
        );

        assert_eq!(payload["status"], "setup");
        assert_eq!(payload["transport"]["type"], "stdio");
        assert_eq!(payload["required_env"][0]["present"], false);
        assert_eq!(payload["tool_namespace"], "mcp_github_*");
    }

    #[test]
    fn configured_server_status_json_preserves_public_fields() {
        let server = McpServerConfigEntry {
            name: "linear".to_string(),
            transport: McpTransportEntry::Sse {
                url: "https://linear.example/sse".to_string(),
            },
            timeout_secs: 12,
            env: vec!["LINEAR_API_KEY".to_string()],
            auth_token_env: Some("LINEAR_AUTH".to_string()),
        };

        let payload = configured_mcp_server_json(&server);

        assert_eq!(payload["name"], "linear");
        assert_eq!(payload["transport"], "sse");
        assert_eq!(payload["env"][0], "LINEAR_API_KEY");
        assert_eq!(payload["auth_token_env"], "LINEAR_AUTH");
        assert_eq!(payload["timeout_secs"], 12);
    }
}
