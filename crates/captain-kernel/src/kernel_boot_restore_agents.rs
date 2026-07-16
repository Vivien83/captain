use super::kernel_model_support::{
    apply_budget_defaults, build_configured_fallbacks, manifest_to_capabilities,
};
use super::kernel_workspace_security::is_runtime_agent_manifest_toml;
use super::CaptainKernel;
use captain_types::agent::{AgentEntry, AgentManifest, AgentState};
use captain_types::config::DefaultModelConfig;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

pub(super) fn restore_persisted_agents(kernel: &CaptainKernel) {
    match kernel.memory.load_all_agents() {
        Ok(agents) => restore_agent_entries(kernel, agents),
        Err(e) => {
            tracing::warn!("Failed to load persisted agents: {e}");
        }
    }
}

fn restore_agent_entries(kernel: &CaptainKernel, agents: Vec<AgentEntry>) {
    let count = agents.len();
    for entry in agents {
        restore_agent_entry(kernel, entry);
    }
    if count > 0 {
        info!("Restored {count} agent(s) from persistent storage");
    }

    restore_manager_mission_reminders(kernel);
}

fn restore_agent_entry(kernel: &CaptainKernel, mut entry: AgentEntry) {
    let agent_id = entry.id;
    let name = entry.name.clone();

    refresh_agent_manifest_from_disk(kernel, &mut entry, &name);
    restore_agent_runtime_memberships(kernel, &entry);

    let restored = prepare_restored_agent(kernel, entry, &name);
    persist_restored_agent_repair(kernel, &restored.entry, &name, restored.manifest_changed);
    register_restored_agent(kernel, restored.entry, &name, agent_id);
}

fn runtime_agent_toml_path(kernel: &CaptainKernel, name: &str) -> PathBuf {
    kernel
        .config
        .home_dir
        .join("agents")
        .join(name)
        .join("agent.toml")
}

fn refresh_agent_manifest_from_disk(kernel: &CaptainKernel, entry: &mut AgentEntry, name: &str) {
    let toml_path = runtime_agent_toml_path(kernel, name);
    let Some(disk_manifest) = read_runtime_agent_manifest(&toml_path, name) else {
        return;
    };
    if !runtime_manifest_differs(&disk_manifest, &entry.manifest) {
        return;
    }

    info!(
        agent = %name,
        "Agent TOML on disk differs from DB, updating"
    );
    entry.manifest = disk_manifest;
    if let Err(e) = kernel.memory.save_agent(entry) {
        warn!(
            agent = %name,
            "Failed to persist TOML update: {e}"
        );
    }
}

fn read_runtime_agent_manifest(toml_path: &Path, name: &str) -> Option<AgentManifest> {
    if !toml_path.exists() {
        return None;
    }
    let toml_str = match std::fs::read_to_string(toml_path) {
        Ok(toml_str) => toml_str,
        Err(e) => {
            warn!(
                agent = %name,
                "Failed to read agent TOML: {e}"
            );
            return None;
        }
    };
    let disk_manifest = match toml::from_str::<AgentManifest>(&toml_str) {
        Ok(manifest) => manifest,
        Err(e) => {
            warn!(
                agent = %name,
                path = %toml_path.display(),
                "Invalid agent TOML on disk, using DB version: {e}"
            );
            return None;
        }
    };
    if !is_runtime_agent_manifest_toml(&toml_str, &disk_manifest) {
        debug!(
            agent = %name,
            path = %toml_path.display(),
            "Skipping bundled/template agent TOML during runtime restore"
        );
        return None;
    }
    Some(disk_manifest)
}

fn restore_agent_runtime_memberships(kernel: &CaptainKernel, entry: &AgentEntry) {
    let caps = manifest_to_capabilities(&entry.manifest);
    kernel.capabilities.grant(entry.id, caps);
    kernel
        .scheduler
        .register(entry.id, entry.manifest.resources.clone());
}

struct RestoredAgent {
    entry: AgentEntry,
    manifest_changed: bool,
}

fn prepare_restored_agent(kernel: &CaptainKernel, entry: AgentEntry, name: &str) -> RestoredAgent {
    let mut restored_entry = entry;
    restored_entry.state = AgentState::Running;
    ensure_restored_exec_policy(kernel, &mut restored_entry);
    apply_budget_defaults(
        &kernel.config.budget,
        &mut restored_entry.manifest.resources,
    );

    let model_repair =
        reconcile_restored_agent_model(&mut restored_entry, &kernel.config.default_model, name);
    let fallback_changed = repair_restored_agent_fallbacks(kernel, &mut restored_entry, name);

    RestoredAgent {
        entry: restored_entry,
        manifest_changed: model_repair.manifest_changed || fallback_changed,
    }
}

fn ensure_restored_exec_policy(kernel: &CaptainKernel, restored_entry: &mut AgentEntry) {
    if restored_entry.manifest.exec_policy.is_none() {
        restored_entry.manifest.exec_policy = Some(kernel.config.exec_policy.clone());
    }
}

struct RestoredModelRepair {
    manifest_changed: bool,
}

fn reconcile_restored_agent_model(
    restored_entry: &mut AgentEntry,
    default_model: &DefaultModelConfig,
    name: &str,
) -> RestoredModelRepair {
    let principal_reconciled =
        CaptainKernel::reconcile_principal_agent_with_default_model(restored_entry, default_model);
    if principal_reconciled {
        info!(
            agent = %name,
            provider = %restored_entry.manifest.model.provider,
            model = %restored_entry.manifest.model.model,
            "Reconciled principal Captain agent with global default_model"
        );
    }
    let default_model_applied = !principal_reconciled
        && apply_default_model_to_placeholder_manifest(&mut restored_entry.manifest, default_model);
    RestoredModelRepair {
        manifest_changed: principal_reconciled || default_model_applied,
    }
}

fn apply_default_model_to_placeholder_manifest(
    manifest: &mut AgentManifest,
    default_model: &DefaultModelConfig,
) -> bool {
    if !restored_agent_uses_default_model(manifest) {
        return false;
    }
    let mut changed = false;
    if !default_model.provider.is_empty() {
        manifest.model.provider = default_model.provider.clone();
        changed = true;
    }
    if !default_model.model.is_empty() {
        manifest.model.model = default_model.model.clone();
        changed = true;
    }
    let desired_api_key_env = default_api_key_env(default_model);
    if manifest.model.api_key_env != desired_api_key_env {
        manifest.model.api_key_env = desired_api_key_env;
        changed = true;
    }
    if manifest.model.base_url != default_model.base_url {
        manifest.model.base_url.clone_from(&default_model.base_url);
        changed = true;
    }
    changed
}

fn restored_agent_uses_default_model(manifest: &AgentManifest) -> bool {
    let is_default_provider =
        manifest.model.provider.is_empty() || manifest.model.provider == "default";
    let is_default_model = manifest.model.model.is_empty() || manifest.model.model == "default";
    is_default_provider && is_default_model
}

fn default_api_key_env(default_model: &DefaultModelConfig) -> Option<String> {
    if default_model.api_key_env.is_empty() {
        None
    } else {
        Some(default_model.api_key_env.clone())
    }
}

fn repair_restored_agent_fallbacks(
    kernel: &CaptainKernel,
    restored_entry: &mut AgentEntry,
    name: &str,
) -> bool {
    if kernel.config.fallback_providers.is_empty() {
        return false;
    }
    let fallbacks = build_configured_fallbacks(&kernel.config.fallback_providers);
    if restored_entry.manifest.fallback_models == fallbacks {
        return false;
    }
    tracing::info!(
        agent = %name,
        count = fallbacks.len(),
        source = "config",
        "Applying explicitly configured fallback chain"
    );
    restored_entry.manifest.fallback_models = fallbacks;
    true
}

fn persist_restored_agent_repair(
    kernel: &CaptainKernel,
    restored_entry: &AgentEntry,
    name: &str,
    manifest_changed: bool,
) {
    if !manifest_changed {
        return;
    }
    if let Err(e) = kernel.memory.save_agent(restored_entry) {
        warn!(
            agent = %name,
            "Failed to persist restored agent repair: {e}"
        );
    }
}

fn register_restored_agent(
    kernel: &CaptainKernel,
    restored_entry: AgentEntry,
    name: &str,
    agent_id: captain_types::agent::AgentId,
) {
    if let Err(e) = kernel.registry.register(restored_entry) {
        tracing::warn!(agent = %name, "Failed to restore agent: {e}");
    } else {
        tracing::debug!(agent = %name, id = %agent_id, "Restored agent");
    }
}

fn runtime_manifest_differs(
    disk_manifest: &AgentManifest,
    stored_manifest: &AgentManifest,
) -> bool {
    disk_manifest.name != stored_manifest.name
        || disk_manifest.description != stored_manifest.description
        || disk_manifest.model.system_prompt != stored_manifest.model.system_prompt
        || disk_manifest.model.provider != stored_manifest.model.provider
        || disk_manifest.model.model != stored_manifest.model.model
        || disk_manifest.capabilities.tools != stored_manifest.capabilities.tools
        || disk_manifest.tool_allowlist != stored_manifest.tool_allowlist
        || disk_manifest.tool_blocklist != stored_manifest.tool_blocklist
}

fn restore_manager_mission_reminders(kernel: &CaptainKernel) {
    let cutoff = chrono::Utc::now() - chrono::Duration::hours(24);
    for entry in kernel.registry.list() {
        if !entry.tags.iter().any(|tag| tag == "manager") {
            continue;
        }
        let Some(mission) = entry.mission.as_deref() else {
            continue;
        };
        if let Some(set_at) = entry.mission_set_at {
            if set_at < cutoff {
                tracing::info!(agent = %entry.name, "Skipping stale mission (>24h)");
                continue;
            }
        }
        tracing::info!(
            agent = %entry.name,
            mission = %mission,
            "Mission restored at boot — will be replayed on next interaction"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_manifest_differs_tracks_only_restore_compared_fields() {
        let mut stored = AgentManifest::default();
        stored.name = "agent".to_string();
        stored.description = "stored".to_string();
        stored.model.provider = "codex".to_string();
        stored.model.model = "gpt-5.5".to_string();

        let mut same = stored.clone();
        same.tags.push("ignored-by-restore-diff".to_string());
        assert!(!runtime_manifest_differs(&same, &stored));

        let mut changed = stored.clone();
        changed.model.model = "gpt-5.5-mini".to_string();
        assert!(runtime_manifest_differs(&changed, &stored));

        let mut changed_allowlist = stored.clone();
        changed_allowlist.tool_allowlist.push("shell".to_string());
        assert!(runtime_manifest_differs(&changed_allowlist, &stored));
    }

    #[test]
    fn placeholder_agent_manifest_inherits_default_model_fields() {
        let mut manifest = AgentManifest::default();
        manifest.model.provider = "default".to_string();
        manifest.model.model.clear();
        manifest.model.api_key_env = Some("OLD_KEY".to_string());
        manifest.model.base_url = None;

        let default_model = DefaultModelConfig {
            provider: "openai".to_string(),
            model: "gpt-4.1".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            base_url: Some("https://api.openai.test/v1".to_string()),
        };

        assert!(apply_default_model_to_placeholder_manifest(
            &mut manifest,
            &default_model
        ));
        assert_eq!(manifest.model.provider, "openai");
        assert_eq!(manifest.model.model, "gpt-4.1");
        assert_eq!(
            manifest.model.api_key_env.as_deref(),
            Some("OPENAI_API_KEY")
        );
        assert_eq!(
            manifest.model.base_url.as_deref(),
            Some("https://api.openai.test/v1")
        );
    }

    #[test]
    fn explicit_agent_manifest_keeps_existing_model_fields() {
        let mut manifest = AgentManifest::default();
        manifest.model.provider = "anthropic".to_string();
        manifest.model.model = "claude-sonnet-4-6".to_string();
        manifest.model.api_key_env = Some("ANTHROPIC_API_KEY".to_string());
        manifest.model.base_url = Some("https://anthropic.test/v1".to_string());

        let original = manifest.model.clone();
        let default_model = DefaultModelConfig {
            provider: "openai".to_string(),
            model: "gpt-4.1".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            base_url: Some("https://api.openai.test/v1".to_string()),
        };

        assert!(!apply_default_model_to_placeholder_manifest(
            &mut manifest,
            &default_model
        ));
        assert_eq!(manifest.model.provider, original.provider);
        assert_eq!(manifest.model.model, original.model);
        assert_eq!(manifest.model.api_key_env, original.api_key_env);
        assert_eq!(manifest.model.base_url, original.base_url);
    }
}
