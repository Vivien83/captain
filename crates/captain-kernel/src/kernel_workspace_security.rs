use captain_types::agent::{AgentId, AgentManifest};

/// Stable name of the principal agent: only this name unlocks Captain's
/// extended workspace. Subagents and hands keep the legacy single-root sandbox.
pub(crate) const PRINCIPAL_AGENT_NAME: &str = "captain";

/// A well-known agent ID used for shared memory operations across agents.
/// This fixed UUID keeps all agents on the same explicit memory namespace.
pub fn shared_memory_agent_id() -> AgentId {
    AgentId(uuid::Uuid::from_bytes([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ]))
}

/// Default blocklist of credential / sensitive paths. The kernel runtime and
/// out-of-process callers reuse this same source of truth to avoid drift.
pub fn default_blocked_workspace_paths(captain_home: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut blocked = Vec::new();
    if let Some(home) = dirs::home_dir() {
        blocked.push(home.join(".ssh"));
        blocked.push(home.join(".gnupg"));
    }
    blocked.push(captain_home.join(".env"));
    blocked.push(captain_home.join(".env.tmp"));
    blocked.push(captain_home.join("secrets.env"));
    blocked.push(captain_home.join("secrets.env.tmp"));
    blocked.push(captain_home.join("secrets-backups"));
    blocked.push(captain_home.join("vault.enc"));
    blocked.push(captain_home.join("vault.enc.bak"));
    blocked
}

pub(crate) fn is_runtime_agent_manifest_toml(toml_str: &str, manifest: &AgentManifest) -> bool {
    let Ok(value) = toml_str.parse::<toml::Value>() else {
        return true;
    };
    let Some(table) = value.as_table() else {
        return true;
    };

    let has_runtime_keys = [
        "name",
        "description",
        "version",
        "author",
        "module",
        "schedule",
        "model",
        "fallback_models",
        "resources",
        "priority",
        "profile",
        "tools",
        "skills",
        "mcp_servers",
        "metadata",
        "tags",
        "autonomous",
        "workspace",
        "exec_policy",
        "tool_allowlist",
        "tool_blocklist",
        "orchestration_mode",
    ]
    .iter()
    .any(|key| table.contains_key(*key));
    let has_template_keys = ["agent", "personality", "limits", "permissions"]
        .iter()
        .any(|key| table.contains_key(*key));

    if has_template_keys && !has_runtime_keys {
        return false;
    }

    if !has_runtime_keys {
        let default_manifest = AgentManifest::default();
        if manifest.name == default_manifest.name
            && manifest.description.is_empty()
            && manifest.model.provider == default_manifest.model.provider
            && manifest.model.model == default_manifest.model.model
        {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_agent_template_toml_is_not_runtime_manifest() {
        let template_toml = r#"[agent]
name = "captain"
description = "Captain - principal agent"
version = "2.0.0"

[personality]
system_prompt_file = "captain.md"

[capabilities]
tags = ["coordination"]

[permissions]
can_create_agents = true
"#;
        let parsed: AgentManifest = toml::from_str(template_toml).unwrap();

        assert_eq!(parsed.name, "unnamed");
        assert_eq!(parsed.model.provider, "anthropic");
        assert!(!is_runtime_agent_manifest_toml(template_toml, &parsed));

        let runtime_toml = r#"name = "captain"
description = "Captain runtime manifest"

[model]
provider = "codex"
model = "gpt-5.5"
"#;
        let runtime_manifest: AgentManifest = toml::from_str(runtime_toml).unwrap();
        assert!(is_runtime_agent_manifest_toml(
            runtime_toml,
            &runtime_manifest
        ));
    }
}
