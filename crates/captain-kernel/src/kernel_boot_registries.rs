use captain_extensions::health::{HealthMonitor, HealthMonitorConfig};
use captain_extensions::registry::IntegrationRegistry;
use captain_hands::registry::HandRegistry;
use captain_skills::registry::SkillRegistry;
use captain_types::config::{KernelConfig, KernelMode, McpServerConfigEntry};
use tracing::{info, warn};

pub(super) struct BootRegistries {
    pub(super) skill_registry: SkillRegistry,
    pub(super) hand_registry: HandRegistry,
    pub(super) extension_registry: IntegrationRegistry,
    pub(super) extension_health: HealthMonitor,
    pub(super) all_mcp_servers: Vec<McpServerConfigEntry>,
}

pub(super) fn build_boot_registries(config: &KernelConfig) -> BootRegistries {
    let skill_registry = build_skill_registry(config);
    let hand_registry = build_hand_registry(config);
    let extension_registry = build_extension_registry(config);
    let all_mcp_servers = merge_extension_mcp_configs(
        config.mcp_servers.clone(),
        extension_registry.to_mcp_configs(),
    );
    let extension_health = build_extension_health(config);

    for inst in extension_registry.to_mcp_configs() {
        extension_health.register(&inst.name);
    }

    BootRegistries {
        skill_registry,
        hand_registry,
        extension_registry,
        extension_health,
        all_mcp_servers,
    }
}

fn build_skill_registry(config: &KernelConfig) -> SkillRegistry {
    let skills_dir = config.home_dir.join("skills");
    let mut skill_registry = SkillRegistry::new(skills_dir);

    let bundled_count = skill_registry.load_bundled();
    if bundled_count > 0 {
        info!("Loaded {bundled_count} bundled skill(s)");
    }

    match skill_registry.load_all() {
        Ok(count) => {
            if count > 0 {
                info!("Loaded {count} user skill(s) from skill registry");
            }
        }
        Err(e) => {
            warn!("Failed to load skill registry: {e}");
        }
    }

    if config.mode == KernelMode::Stable {
        skill_registry.freeze();
    }

    skill_registry
}

fn build_hand_registry(config: &KernelConfig) -> HandRegistry {
    let hand_registry = HandRegistry::new();
    let hand_count = hand_registry.load_bundled();
    if hand_count > 0 {
        info!("Loaded {hand_count} bundled hand(s)");
    }

    let hands_dir = config.home_dir.join("hands");
    let custom_count = hand_registry.scan_directory(&hands_dir);
    if custom_count > 0 {
        info!(
            "Loaded {custom_count} custom hand(s) from {}",
            hands_dir.display()
        );
    }

    hand_registry
}

fn build_extension_registry(config: &KernelConfig) -> IntegrationRegistry {
    let mut extension_registry = IntegrationRegistry::new(&config.home_dir);
    let ext_bundled = extension_registry.load_bundled();
    match extension_registry.load_installed() {
        Ok(count) => {
            if count > 0 {
                info!("Loaded {count} installed integration(s)");
            }
        }
        Err(e) => {
            warn!("Failed to load installed integrations: {e}");
        }
    }
    info!(
        "Extension registry: {ext_bundled} templates available, {} installed",
        extension_registry.installed_count()
    );

    extension_registry
}

fn merge_extension_mcp_configs(
    mut manual: Vec<McpServerConfigEntry>,
    extension: Vec<McpServerConfigEntry>,
) -> Vec<McpServerConfigEntry> {
    for ext_cfg in extension {
        if !manual.iter().any(|server| server.name == ext_cfg.name) {
            manual.push(ext_cfg);
        }
    }
    manual
}

fn build_extension_health(config: &KernelConfig) -> HealthMonitor {
    let health_config = HealthMonitorConfig {
        auto_reconnect: config.extensions.auto_reconnect,
        max_reconnect_attempts: config.extensions.reconnect_max_attempts,
        max_backoff_secs: config.extensions.reconnect_max_backoff_secs,
        check_interval_secs: config.extensions.health_check_interval_secs,
    };

    HealthMonitor::new(health_config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::config::McpTransportEntry;

    #[test]
    fn merge_extension_mcp_configs_keeps_manual_server_when_names_collide() {
        let manual = vec![stdio_server("github", "manual-github")];
        let extension = vec![
            stdio_server("github", "extension-github"),
            stdio_server("linear", "extension-linear"),
        ];

        let merged = merge_extension_mcp_configs(manual, extension);

        assert_eq!(server_names(&merged), vec!["github", "linear"]);
        assert_eq!(stdio_command(&merged[0]), "manual-github");
        assert_eq!(stdio_command(&merged[1]), "extension-linear");
    }

    fn stdio_server(name: &str, command: &str) -> McpServerConfigEntry {
        McpServerConfigEntry {
            name: name.to_string(),
            transport: McpTransportEntry::Stdio {
                command: command.to_string(),
                args: Vec::new(),
            },
            timeout_secs: 30,
            env: Vec::new(),
            auth_token_env: None,
        }
    }

    fn server_names(servers: &[McpServerConfigEntry]) -> Vec<&str> {
        servers.iter().map(|server| server.name.as_str()).collect()
    }

    fn stdio_command(server: &McpServerConfigEntry) -> &str {
        match &server.transport {
            McpTransportEntry::Stdio { command, .. } => command,
            McpTransportEntry::Sse { .. } => panic!("expected stdio transport"),
        }
    }
}
