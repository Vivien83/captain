use super::kernel_agent_runtime::{apply_subagent_lineage_metadata, normalize_subagent_tool_scope};
use super::kernel_agent_workspace::{ensure_workspace, generate_identity_files};
use super::kernel_model_support::{apply_budget_defaults, manifest_to_capabilities};
use super::CaptainKernel;
use crate::error::{KernelError, KernelResult};
use captain_runtime::agent_loop::strip_provider_prefix;
use captain_types::agent::{
    AgentEntry, AgentId, AgentManifest, AgentMode, AgentState, ScheduleMode, SessionId,
};
use captain_types::config::DefaultModelConfig;
use captain_types::error::CaptainError;
use captain_types::event::{Event, EventPayload, EventTarget, LifecycleEvent};
use tracing::info;

impl CaptainKernel {
    /// Spawn a new agent from a manifest, optionally linking to a parent agent.
    pub fn spawn_agent(&self, manifest: AgentManifest) -> KernelResult<AgentId> {
        self.spawn_agent_with_parent(manifest, None, None)
    }

    /// Spawn a new agent with an optional parent for lineage tracking.
    /// If fixed_id is provided, use it instead of generating a new UUID.
    pub fn spawn_agent_with_parent(
        &self,
        manifest: AgentManifest,
        parent: Option<AgentId>,
        fixed_id: Option<AgentId>,
    ) -> KernelResult<AgentId> {
        let agent_id = fixed_id.unwrap_or_default();
        let name = manifest.name.clone();

        info!(agent = %name, id = %agent_id, parent = ?parent, "Spawning agent");

        let session_id = self.create_spawn_session(agent_id)?;
        let mut manifest = self.prepare_spawn_manifest(manifest, parent, &name, agent_id);
        self.prepare_spawn_workspace(&mut manifest, &name)?;
        let entry = build_spawn_entry(agent_id, manifest, parent, session_id);

        self.register_spawned_agent(&entry, parent)?;
        self.record_spawned_agent(&entry, parent);
        self.register_spawned_agent_triggers(&entry);
        self.publish_spawned_agent_event(agent_id, &name);

        Ok(agent_id)
    }

    fn create_spawn_session(&self, agent_id: AgentId) -> KernelResult<SessionId> {
        let session = self
            .memory
            .create_session(agent_id)
            .map_err(KernelError::Captain)?;
        Ok(session.id)
    }

    fn prepare_spawn_manifest(
        &self,
        mut manifest: AgentManifest,
        parent: Option<AgentId>,
        name: &str,
        agent_id: AgentId,
    ) -> AgentManifest {
        let parent_entry = parent.and_then(|parent_id| self.registry.get(parent_id));
        apply_subagent_lineage_metadata(&mut manifest, parent, parent_entry.as_ref());
        if parent.is_some() {
            normalize_subagent_tool_scope(&mut manifest);
        }
        if manifest.exec_policy.is_none() {
            manifest.exec_policy = Some(self.config.exec_policy.clone());
        }
        info!(agent = %name, id = %agent_id, exec_mode = ?manifest.exec_policy.as_ref().map(|p| &p.mode), "Agent exec_policy resolved");

        self.apply_spawn_default_model(&mut manifest);
        normalize_spawn_model_name(&mut manifest);
        apply_budget_defaults(&self.config.budget, &mut manifest.resources);
        manifest
    }

    fn apply_spawn_default_model(&self, manifest: &mut AgentManifest) {
        if !spawn_manifest_uses_default_model(manifest) {
            return;
        }
        let override_guard = self
            .default_model_override
            .read()
            .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
        let default_model = override_guard
            .as_ref()
            .unwrap_or(&self.config.default_model);
        apply_default_model_to_spawn_manifest(manifest, default_model);
    }

    fn prepare_spawn_workspace(
        &self,
        manifest: &mut AgentManifest,
        name: &str,
    ) -> KernelResult<()> {
        let workspace_dir = spawn_workspace_dir(self, manifest, name);
        let project_source_workspace = spawn_uses_project_source_workspace(manifest);
        if project_source_workspace {
            std::fs::create_dir_all(&workspace_dir).map_err(|e| {
                KernelError::Captain(CaptainError::Internal(format!(
                    "Failed to create project workspace dir {}: {e}",
                    workspace_dir.display()
                )))
            })?;
        } else {
            ensure_workspace(&workspace_dir)?;
        }
        if manifest.generate_identity_files && !project_source_workspace {
            generate_identity_files(&workspace_dir, manifest);
        }
        manifest.workspace = Some(workspace_dir);
        Ok(())
    }

    fn register_spawned_agent(
        &self,
        entry: &AgentEntry,
        parent: Option<AgentId>,
    ) -> KernelResult<()> {
        let caps = manifest_to_capabilities(&entry.manifest);
        self.capabilities.grant(entry.id, caps);
        self.scheduler
            .register(entry.id, entry.manifest.resources.clone());
        self.registry
            .register(entry.clone())
            .map_err(KernelError::Captain)?;
        if let Some(parent_id) = parent {
            self.registry.add_child(parent_id, entry.id);
        }
        self.memory.save_agent(entry).map_err(KernelError::Captain)
    }

    fn record_spawned_agent(&self, entry: &AgentEntry, parent: Option<AgentId>) {
        info!(agent = %entry.name, id = %entry.id, "Agent spawned");
        let model = entry.manifest.model.model.clone();
        let provider = entry.manifest.model.provider.clone();
        let _ = self.graph_memory.add_doc_entity(
            "agent",
            &entry.name,
            &format!(
                "Agent {} — modèle: {model}, provider: {provider}",
                entry.name
            ),
            &["agent", "spawned"],
        );
        self.audit_log.record(
            entry.id.to_string(),
            captain_runtime::audit::AuditAction::AgentSpawn,
            format!("name={}, parent={parent:?}", entry.name),
            "ok",
        );
    }

    fn register_spawned_agent_triggers(&self, entry: &AgentEntry) {
        if let ScheduleMode::Proactive { conditions } = &entry.manifest.schedule {
            self.register_proactive_triggers(entry.id, &entry.name, conditions);
        }
    }

    fn publish_spawned_agent_event(&self, agent_id: AgentId, name: &str) {
        let event = Event::new(
            agent_id,
            EventTarget::Broadcast,
            EventPayload::Lifecycle(LifecycleEvent::Spawned {
                agent_id,
                name: name.to_string(),
            }),
        );
        let _triggered = self.triggers.evaluate(&event);
        // Also route it through the event bus (unlike triggers.evaluate,
        // subscribe_all() consumers — TUI SSE badge, agent-API egress
        // webhooks — actually receive this). `try_current` because this is
        // reachable from plain (non-async) unit tests that spawn agents
        // outside of a tokio runtime — best-effort, safe to skip there.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let bus = self.event_bus.clone();
            runtime.spawn(async move {
                bus.publish(event).await;
            });
        }
    }
}

fn build_spawn_entry(
    agent_id: AgentId,
    manifest: AgentManifest,
    parent: Option<AgentId>,
    session_id: SessionId,
) -> AgentEntry {
    let tags = manifest.tags.clone();
    AgentEntry {
        id: agent_id,
        name: manifest.name.clone(),
        manifest,
        state: AgentState::Running,
        mode: AgentMode::default(),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        parent,
        children: vec![],
        session_id,
        tags,
        identity: Default::default(),
        onboarding_completed: false,
        onboarding_completed_at: None,
        mission: None,
        mission_set_at: None,
        autoscale: None,
        last_scale_event: None,
    }
}

fn spawn_manifest_uses_default_model(manifest: &AgentManifest) -> bool {
    let is_default_provider =
        manifest.model.provider.is_empty() || manifest.model.provider == "default";
    let is_default_model = manifest.model.model.is_empty() || manifest.model.model == "default";
    is_default_provider && is_default_model
}

fn apply_default_model_to_spawn_manifest(
    manifest: &mut AgentManifest,
    default_model: &DefaultModelConfig,
) {
    if !default_model.provider.is_empty() {
        manifest.model.provider = default_model.provider.clone();
    }
    if !default_model.model.is_empty() {
        manifest.model.model = default_model.model.clone();
    }
    if !default_model.api_key_env.is_empty() && manifest.model.api_key_env.is_none() {
        manifest.model.api_key_env = Some(default_model.api_key_env.clone());
    }
    if default_model.base_url.is_some() && manifest.model.base_url.is_none() {
        manifest.model.base_url.clone_from(&default_model.base_url);
    }
}

fn normalize_spawn_model_name(manifest: &mut AgentManifest) {
    let normalized = strip_provider_prefix(&manifest.model.model, &manifest.model.provider);
    if normalized != manifest.model.model {
        manifest.model.model = normalized;
    }
}

fn spawn_workspace_dir(
    kernel: &CaptainKernel,
    manifest: &AgentManifest,
    name: &str,
) -> std::path::PathBuf {
    manifest
        .workspace
        .clone()
        .unwrap_or_else(|| kernel.config.effective_workspaces_dir().join(name))
}

fn spawn_uses_project_source_workspace(manifest: &AgentManifest) -> bool {
    manifest
        .metadata
        .get("workspace_kind")
        .and_then(|v| v.as_str())
        == Some("project_source")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_default_model_applies_provider_model_and_missing_auth_fields() {
        let mut manifest = AgentManifest::default();
        manifest.model.provider = "default".to_string();
        manifest.model.model.clear();

        apply_default_model_to_spawn_manifest(
            &mut manifest,
            &DefaultModelConfig {
                provider: "openai".to_string(),
                model: "gpt-4.1".to_string(),
                api_key_env: "OPENAI_API_KEY".to_string(),
                base_url: Some("https://api.openai.test/v1".to_string()),
            },
        );

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
    fn spawn_default_model_preserves_existing_auth_hints() {
        let mut manifest = AgentManifest::default();
        manifest.model.api_key_env = Some("CUSTOM_KEY".to_string());
        manifest.model.base_url = Some("https://custom.test/v1".to_string());

        apply_default_model_to_spawn_manifest(
            &mut manifest,
            &DefaultModelConfig {
                provider: "codex".to_string(),
                model: "gpt-5.5".to_string(),
                api_key_env: "OPENAI_API_KEY".to_string(),
                base_url: Some("https://api.openai.test/v1".to_string()),
            },
        );

        assert_eq!(manifest.model.api_key_env.as_deref(), Some("CUSTOM_KEY"));
        assert_eq!(
            manifest.model.base_url.as_deref(),
            Some("https://custom.test/v1")
        );
    }

    #[test]
    fn spawn_project_source_workspace_detection_uses_metadata() {
        let mut manifest = AgentManifest::default();
        assert!(!spawn_uses_project_source_workspace(&manifest));
        manifest.metadata.insert(
            "workspace_kind".to_string(),
            serde_json::json!("project_source"),
        );
        assert!(spawn_uses_project_source_workspace(&manifest));
    }

    #[test]
    fn spawn_default_model_detection_requires_default_provider_and_model() {
        let mut manifest = AgentManifest::default();
        manifest.model.provider = "default".to_string();
        manifest.model.model = "default".to_string();
        assert!(spawn_manifest_uses_default_model(&manifest));

        manifest.model.provider = "openai".to_string();
        assert!(!spawn_manifest_uses_default_model(&manifest));

        manifest.model.provider = "default".to_string();
        manifest.model.model = "gpt-4.1".to_string();
        assert!(!spawn_manifest_uses_default_model(&manifest));
    }

    #[test]
    fn spawn_entry_carries_registry_state_parent_session_and_tags() {
        let agent_id = AgentId::new();
        let parent_id = AgentId::new();
        let session_id = SessionId::new();
        let mut manifest = AgentManifest::default();
        manifest.name = "worker".to_string();
        manifest.tags = vec!["ops".to_string(), "subagent".to_string()];

        let entry = build_spawn_entry(agent_id, manifest, Some(parent_id), session_id);

        assert_eq!(entry.id, agent_id);
        assert_eq!(entry.name, "worker");
        assert_eq!(entry.parent, Some(parent_id));
        assert_eq!(entry.session_id, session_id);
        assert_eq!(entry.tags, vec!["ops", "subagent"]);
        assert!(matches!(entry.state, AgentState::Running));
        assert!(entry.children.is_empty());
    }
}
