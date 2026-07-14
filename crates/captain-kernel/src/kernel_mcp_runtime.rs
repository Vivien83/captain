use super::CaptainKernel;
use captain_runtime::mcp::{
    McpConnection, McpServerConfig as RuntimeMcpServerConfig, McpTransport,
};
use captain_types::config::{McpServerConfigEntry, McpTransportEntry};
use std::sync::Arc;
use tracing::{debug, info, warn};

impl CaptainKernel {
    /// Connect to all configured MCP servers and cache their tool definitions.
    pub(super) async fn connect_mcp_servers(self: &Arc<Self>) {
        let servers = self
            .effective_mcp_servers
            .read()
            .map(|s| s.clone())
            .unwrap_or_default();

        for server_config in &servers {
            self.ensure_mcp_env_vars(server_config);

            match McpConnection::connect(runtime_mcp_config(server_config)).await {
                Ok(conn) => {
                    let tool_count = conn.tools().len();
                    // Cache tool definitions
                    if let Ok(mut tools) = self.mcp_tools.lock() {
                        tools.extend(conn.tools().iter().cloned());
                    }
                    info!(
                        server = %server_config.name,
                        tools = tool_count,
                        "MCP server connected"
                    );
                    // Update extension health if this is an extension-provided server
                    self.extension_health
                        .report_ok(&server_config.name, tool_count);
                    self.mcp_connections.lock().await.push(conn);
                }
                Err(e) => {
                    warn!(
                        server = %server_config.name,
                        error = %e,
                        "Failed to connect to MCP server"
                    );
                    self.extension_health
                        .report_error(&server_config.name, e.to_string());
                }
            }
        }

        let tool_count = self.mcp_tools.lock().map(|t| t.len()).unwrap_or(0);
        if tool_count > 0 {
            info!(
                "MCP: {tool_count} tools available from {} server(s)",
                self.mcp_connections.lock().await.len()
            );
        }
    }

    /// Reload extension configs and connect any new MCP servers.
    ///
    /// Called by the API reload endpoint after CLI installs/removes integrations.
    pub async fn reload_extension_mcps(&self) -> Result<usize, String> {
        // 1. Reload installed integrations from disk
        let installed_count = {
            let mut registry = self
                .extension_registry
                .write()
                .unwrap_or_else(|e| e.into_inner());
            registry.load_installed().map_err(|e| e.to_string())?
        };

        // 2. Rebuild effective MCP server list
        let new_configs = {
            let registry = self
                .extension_registry
                .read()
                .unwrap_or_else(|e| e.into_inner());
            let ext_mcp_configs = registry.to_mcp_configs();
            let mut all = self.config.mcp_servers.clone();
            for ext_cfg in ext_mcp_configs {
                if !all.iter().any(|s| s.name == ext_cfg.name) {
                    all.push(ext_cfg);
                }
            }
            all
        };

        // 3. Find servers that aren't already connected
        let already_connected: Vec<String> = self
            .mcp_connections
            .lock()
            .await
            .iter()
            .map(|c| c.name().to_string())
            .collect();

        let new_servers = new_mcp_server_configs(&new_configs, &already_connected);

        // 4. Update effective list
        if let Ok(mut effective) = self.effective_mcp_servers.write() {
            *effective = new_configs;
        }

        // 5. Connect new servers
        let mut connected_count = 0;
        for server_config in &new_servers {
            self.ensure_mcp_env_vars(server_config);
            self.extension_health.register(&server_config.name);

            match McpConnection::connect(runtime_mcp_config(server_config)).await {
                Ok(conn) => {
                    let tool_count = conn.tools().len();
                    if let Ok(mut tools) = self.mcp_tools.lock() {
                        tools.extend(conn.tools().iter().cloned());
                    }
                    self.extension_health
                        .report_ok(&server_config.name, tool_count);
                    info!(
                        server = %server_config.name,
                        tools = tool_count,
                        "Extension MCP server connected (hot-reload)"
                    );
                    self.mcp_connections.lock().await.push(conn);
                    connected_count += 1;
                }
                Err(e) => {
                    self.extension_health
                        .report_error(&server_config.name, e.to_string());
                    warn!(
                        server = %server_config.name,
                        error = %e,
                        "Failed to connect extension MCP server"
                    );
                }
            }
        }

        // 6. Remove connections for uninstalled integrations
        let removed: Vec<String> = already_connected
            .iter()
            .filter(|name| {
                let effective = self
                    .effective_mcp_servers
                    .read()
                    .unwrap_or_else(|e| e.into_inner());
                !effective.iter().any(|s| &s.name == *name)
            })
            .cloned()
            .collect();

        if !removed.is_empty() {
            let mut conns = self.mcp_connections.lock().await;
            conns.retain(|c| !removed.contains(&c.name().to_string()));
            self.rebuild_mcp_tool_cache(&conns);
            for name in &removed {
                self.extension_health.unregister(name);
                info!(server = %name, "Extension MCP server disconnected (removed)");
            }
        }

        info!(
            "Extension reload: {} installed, {} new connections, {} removed",
            installed_count,
            connected_count,
            removed.len()
        );
        Ok(connected_count)
    }

    /// Reconnect a single extension MCP server by ID.
    pub async fn reconnect_extension_mcp(self: &Arc<Self>, id: &str) -> Result<usize, String> {
        // Find the config for this server
        let server_config = {
            let effective = self
                .effective_mcp_servers
                .read()
                .unwrap_or_else(|e| e.into_inner());
            effective.iter().find(|s| s.name == id).cloned()
        };

        let server_config =
            server_config.ok_or_else(|| format!("No MCP config found for integration '{id}'"))?;

        // Disconnect existing connection if any
        {
            let mut conns = self.mcp_connections.lock().await;
            let old_len = conns.len();
            conns.retain(|c| c.name() != id);
            if conns.len() < old_len {
                self.rebuild_mcp_tool_cache(&conns);
            }
        }

        self.extension_health.mark_reconnecting(id);
        self.ensure_mcp_env_vars(&server_config);

        match McpConnection::connect(runtime_mcp_config(&server_config)).await {
            Ok(conn) => {
                let tool_count = conn.tools().len();
                if let Ok(mut tools) = self.mcp_tools.lock() {
                    tools.extend(conn.tools().iter().cloned());
                }
                self.extension_health.report_ok(id, tool_count);
                info!(
                    server = %id,
                    tools = tool_count,
                    "Extension MCP server reconnected"
                );
                self.mcp_connections.lock().await.push(conn);
                Ok(tool_count)
            }
            Err(e) => {
                self.extension_health.report_error(id, e.to_string());
                Err(format!("Reconnect failed for '{id}': {e}"))
            }
        }
    }

    /// Background loop that checks extension MCP health and auto-reconnects.
    pub(super) async fn run_extension_health_loop(self: &Arc<Self>) {
        let interval_secs = self.extension_health.config().check_interval_secs;
        if interval_secs == 0 {
            return;
        }

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        interval.tick().await; // skip first immediate tick

        loop {
            interval.tick().await;

            // Check each registered integration
            let health_entries = self.extension_health.all_health();
            for entry in health_entries {
                // Try reconnect for errored integrations
                if self.extension_health.should_reconnect(&entry.id) {
                    let backoff = self
                        .extension_health
                        .backoff_duration(entry.reconnect_attempts);
                    debug!(
                        server = %entry.id,
                        attempt = entry.reconnect_attempts + 1,
                        backoff_secs = backoff.as_secs(),
                        "Auto-reconnecting extension MCP server"
                    );
                    tokio::time::sleep(backoff).await;

                    if let Err(e) = self.reconnect_extension_mcp(&entry.id).await {
                        debug!(server = %entry.id, error = %e, "Auto-reconnect failed");
                    }
                }
            }
        }
    }

    fn ensure_mcp_env_vars(&self, server_config: &McpServerConfigEntry) {
        // Resolve env vars from vault/dotenv before passing to MCP subprocess.
        // The MCP spawn calls env_clear() then re-adds only whitelisted vars
        // from std::env, so we must ensure they're in std::env first.
        for var_name in server_config
            .env
            .iter()
            .chain(server_config.auth_token_env.iter())
        {
            if std::env::var(var_name).is_err() {
                if let Some(val) = self.resolve_credential(var_name) {
                    std::env::set_var(var_name, &val);
                }
            }
        }
    }

    fn rebuild_mcp_tool_cache(&self, conns: &[McpConnection]) {
        if let Ok(mut tools) = self.mcp_tools.lock() {
            tools.clear();
            for conn in conns.iter() {
                tools.extend(conn.tools().iter().cloned());
            }
        }
    }
}

fn runtime_mcp_config(server_config: &McpServerConfigEntry) -> RuntimeMcpServerConfig {
    RuntimeMcpServerConfig {
        name: server_config.name.clone(),
        transport: runtime_mcp_transport(&server_config.transport),
        timeout_secs: server_config.timeout_secs,
        env: server_config.env.clone(),
        auth_token_env: server_config.auth_token_env.clone(),
    }
}

fn runtime_mcp_transport(transport: &McpTransportEntry) -> McpTransport {
    match transport {
        McpTransportEntry::Stdio { command, args } => McpTransport::Stdio {
            command: command.clone(),
            args: args.clone(),
        },
        McpTransportEntry::Sse { url } => McpTransport::Sse { url: url.clone() },
    }
}

fn new_mcp_server_configs(
    configs: &[McpServerConfigEntry],
    already_connected: &[String],
) -> Vec<McpServerConfigEntry> {
    configs
        .iter()
        .filter(|server| !already_connected.contains(&server.name))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio_entry(name: &str) -> McpServerConfigEntry {
        McpServerConfigEntry {
            name: name.to_string(),
            transport: McpTransportEntry::Stdio {
                command: "node".to_string(),
                args: vec!["server.js".to_string()],
            },
            timeout_secs: 42,
            env: vec!["TOKEN".to_string()],
            auth_token_env: Some("AUTH_TOKEN".to_string()),
        }
    }

    #[test]
    fn runtime_config_preserves_stdio_transport_and_security_env() {
        let config = runtime_mcp_config(&stdio_entry("github"));

        assert_eq!(config.name, "github");
        assert_eq!(config.timeout_secs, 42);
        assert_eq!(config.env, vec!["TOKEN"]);
        assert_eq!(config.auth_token_env.as_deref(), Some("AUTH_TOKEN"));
        match config.transport {
            McpTransport::Stdio { command, args } => {
                assert_eq!(command, "node");
                assert_eq!(args, vec!["server.js"]);
            }
            McpTransport::Sse { .. } => panic!("expected stdio transport"),
        }
    }

    #[test]
    fn runtime_config_preserves_sse_transport() {
        let entry = McpServerConfigEntry {
            name: "docs".to_string(),
            transport: McpTransportEntry::Sse {
                url: "https://example.test/sse".to_string(),
            },
            timeout_secs: 30,
            env: vec![],
            auth_token_env: None,
        };

        let config = runtime_mcp_config(&entry);
        match config.transport {
            McpTransport::Sse { url } => assert_eq!(url, "https://example.test/sse"),
            McpTransport::Stdio { .. } => panic!("expected sse transport"),
        }
    }

    #[test]
    fn new_mcp_server_configs_excludes_already_connected_servers() {
        let configs = vec![stdio_entry("github"), stdio_entry("filesystem")];
        let connected = vec!["github".to_string()];

        let new_servers = new_mcp_server_configs(&configs, &connected);

        assert_eq!(new_servers.len(), 1);
        assert_eq!(new_servers[0].name, "filesystem");
    }
}
