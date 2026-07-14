use serde::{Deserialize, Serialize};

/// Configuration entry for an MCP server.
///
/// This is the config.toml representation. The runtime `McpServerConfig`
/// struct is constructed from this during kernel boot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfigEntry {
    /// Display name for this server.
    pub name: String,
    /// Transport configuration.
    pub transport: McpTransportEntry,
    /// Request timeout in seconds.
    #[serde(default = "default_mcp_timeout")]
    pub timeout_secs: u64,
    /// Environment variables to pass through, e.g. ["GITHUB_PERSONAL_ACCESS_TOKEN"].
    #[serde(default)]
    pub env: Vec<String>,
    /// Optional env var whose value is sent as `Authorization: Bearer ...` for SSE transports.
    #[serde(default)]
    pub auth_token_env: Option<String>,
}

fn default_mcp_timeout() -> u64 {
    30
}

/// Transport configuration for an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransportEntry {
    /// Subprocess with JSON-RPC over stdin/stdout.
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// HTTP Server-Sent Events.
    Sse { url: String },
}

/// A2A (Agent-to-Agent) protocol configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct A2aConfig {
    /// Whether A2A is enabled.
    pub enabled: bool,
    /// Path to serve A2A endpoints (default: "/a2a").
    #[serde(default = "default_a2a_path")]
    pub listen_path: String,
    /// External A2A agents to connect to.
    #[serde(default)]
    pub external_agents: Vec<ExternalAgent>,
}

fn default_a2a_path() -> String {
    "/a2a".to_string()
}

/// An external A2A agent to discover and interact with.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalAgent {
    /// Display name.
    pub name: String,
    /// Agent endpoint URL.
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::{A2aConfig, McpServerConfigEntry, McpTransportEntry};
    use crate::config::{ExternalAgent, KernelConfig};

    #[test]
    fn mcp_server_config_defaults_timeout_and_env_fields() {
        let config: McpServerConfigEntry = toml::from_str(
            r#"
            name = "github"

            [transport]
            type = "stdio"
            command = "mcp-github"
            "#,
        )
        .unwrap();

        assert_eq!(config.name, "github");
        assert_eq!(config.timeout_secs, 30);
        assert!(config.env.is_empty());
        assert!(config.auth_token_env.is_none());
        match config.transport {
            McpTransportEntry::Stdio { command, args } => {
                assert_eq!(command, "mcp-github");
                assert!(args.is_empty());
            }
            McpTransportEntry::Sse { .. } => panic!("expected stdio transport"),
        }
    }

    #[test]
    fn mcp_transport_serde_accepts_sse() {
        let config: McpServerConfigEntry = toml::from_str(
            r#"
            name = "remote"
            timeout_secs = 45
            auth_token_env = "REMOTE_MCP_TOKEN"

            [transport]
            type = "sse"
            url = "https://example.com/sse"
            "#,
        )
        .unwrap();

        assert_eq!(config.timeout_secs, 45);
        assert_eq!(config.auth_token_env.as_deref(), Some("REMOTE_MCP_TOKEN"));
        match config.transport {
            McpTransportEntry::Sse { url } => assert_eq!(url, "https://example.com/sse"),
            McpTransportEntry::Stdio { .. } => panic!("expected sse transport"),
        }
    }

    #[test]
    fn a2a_config_defaults_keep_endpoint_disabled() {
        let config = A2aConfig::default();

        assert!(!config.enabled);
        assert!(config.listen_path.is_empty());
        assert!(config.external_agents.is_empty());
    }

    #[test]
    fn a2a_config_deserializes_default_path_and_agents() {
        let config: A2aConfig = toml::from_str(
            r#"
            enabled = true

            [[external_agents]]
            name = "peer"
            url = "https://peer.example/a2a"
            "#,
        )
        .unwrap();

        assert!(config.enabled);
        assert_eq!(config.listen_path, "/a2a");
        assert_eq!(config.external_agents.len(), 1);
        assert_eq!(config.external_agents[0].name, "peer");
        assert_eq!(config.external_agents[0].url, "https://peer.example/a2a");
    }

    #[test]
    fn protocol_sections_deserialize_from_kernel_toml() {
        let config: KernelConfig = toml::from_str(
            r#"
            [[mcp_servers]]
            name = "filesystem"
            timeout_secs = 12
            env = ["FILESYSTEM_ROOT"]

            [mcp_servers.transport]
            type = "stdio"
            command = "mcp-filesystem"
            args = ["."]

            [a2a]
            enabled = true
            listen_path = "/agents"

            [[a2a.external_agents]]
            name = "planner"
            url = "https://planner.example/a2a"
            "#,
        )
        .unwrap();

        assert_eq!(config.mcp_servers.len(), 1);
        assert_eq!(config.mcp_servers[0].timeout_secs, 12);
        assert_eq!(config.mcp_servers[0].env, vec!["FILESYSTEM_ROOT"]);
        assert!(config.a2a.as_ref().is_some_and(|a2a| a2a.enabled));
        assert_eq!(
            config
                .a2a
                .as_ref()
                .and_then(|a2a| a2a.external_agents.first())
                .map(|agent| agent.name.as_str()),
            Some("planner")
        );
    }

    #[test]
    fn external_agent_roundtrips_json() {
        let agent = ExternalAgent {
            name: "planner".to_string(),
            url: "https://planner.example/a2a".to_string(),
        };

        let json = serde_json::to_string(&agent).unwrap();
        let back: ExternalAgent = serde_json::from_str(&json).unwrap();

        assert_eq!(back.name, agent.name);
        assert_eq!(back.url, agent.url);
    }
}
