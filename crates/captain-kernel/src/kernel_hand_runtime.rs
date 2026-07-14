use super::CaptainKernel;
use crate::error::{KernelError, KernelResult};
use crate::triggers::{FileChangeTrigger, Trigger};
use captain_types::agent::{
    AgentId, AgentManifest, AutonomousConfig, ManifestCapabilities, ModelConfig, ScheduleMode,
    ToolProfile,
};
use captain_types::error::CaptainError;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use tracing::{info, warn};

struct SavedHandAgentState {
    old_agent_id: Option<AgentId>,
    triggers: Vec<Trigger>,
    file_triggers: Vec<FileChangeTrigger>,
}

fn map_hand_activation_error(error: captain_hands::HandError) -> KernelError {
    match error {
        captain_hands::HandError::AlreadyActive(id) => {
            KernelError::Captain(CaptainError::Internal(format!("Hand already active: {id}")))
        }
        other => KernelError::Captain(CaptainError::Internal(other.to_string())),
    }
}

fn hand_schedule(def: &captain_hands::HandDefinition) -> ScheduleMode {
    if def.agent.max_iterations.is_some() {
        ScheduleMode::Continuous {
            check_interval_secs: 60,
        }
    } else {
        ScheduleMode::default()
    }
}

fn hand_exec_policy(
    def: &captain_hands::HandDefinition,
) -> Option<captain_types::config::ExecPolicy> {
    if def.tools.iter().any(|tool| tool == "shell_exec") {
        Some(captain_types::config::ExecPolicy {
            mode: captain_types::config::ExecSecurityMode::Full,
            timeout_secs: 300,
            no_output_timeout_secs: 120,
            ..Default::default()
        })
    } else {
        None
    }
}

fn hand_tool_profile(def: &captain_hands::HandDefinition) -> Option<ToolProfile> {
    if def.tools.is_empty() {
        None
    } else {
        Some(ToolProfile::Custom)
    }
}

fn apply_hand_settings(
    def: &captain_hands::HandDefinition,
    instance: &captain_hands::HandInstance,
    manifest: &mut AgentManifest,
) {
    let resolved = captain_hands::resolve_settings(&def.settings, &instance.config);
    if !resolved.prompt_block.is_empty() {
        manifest.model.system_prompt = format!(
            "{}\n\n---\n\n{}",
            manifest.model.system_prompt, resolved.prompt_block
        );
    }

    let mut allowed_env = resolved.env_vars;
    for req in &def.requires {
        if matches!(
            req.requirement_type,
            captain_hands::RequirementType::ApiKey | captain_hands::RequirementType::EnvVar
        ) && !req.check_value.is_empty()
            && !allowed_env.contains(&req.check_value)
        {
            allowed_env.push(req.check_value.clone());
        }
    }
    if !allowed_env.is_empty() {
        manifest.metadata.insert(
            "hand_allowed_env".to_string(),
            serde_json::to_value(&allowed_env).unwrap_or_default(),
        );
    }
}

fn apply_hand_skill_content(def: &captain_hands::HandDefinition, manifest: &mut AgentManifest) {
    if let Some(ref skill_content) = def.skill_content {
        manifest.model.system_prompt = format!(
            "{}\n\n---\n\n## Reference Knowledge\n\n{}",
            manifest.model.system_prompt, skill_content
        );
    }
}

impl CaptainKernel {
    /// Activate a hand: check requirements, create instance, spawn agent.
    pub fn activate_hand(
        &self,
        hand_id: &str,
        config: HashMap<String, serde_json::Value>,
    ) -> KernelResult<captain_hands::HandInstance> {
        let (def, instance) = self.activate_hand_instance(hand_id, config)?;
        let manifest = self.hand_manifest(&def, &instance);
        let saved_state = self.remove_existing_hand_agent(&def.agent.name);

        let fixed_agent_id = AgentId::from_string(hand_id);
        let agent_id = self.spawn_agent_with_parent(manifest, None, Some(fixed_agent_id))?;
        self.restore_reactivated_hand_runtime(agent_id, &saved_state);

        self.set_hand_instance_agent(instance.instance_id, agent_id)?;

        info!(
            hand = %hand_id,
            instance = %instance.instance_id,
            agent = %agent_id,
            "Hand activated with agent"
        );

        self.persist_hand_state();

        Ok(self
            .hand_registry
            .get_instance(instance.instance_id)
            .unwrap_or(instance))
    }

    fn activate_hand_instance(
        &self,
        hand_id: &str,
        config: HashMap<String, serde_json::Value>,
    ) -> KernelResult<(captain_hands::HandDefinition, captain_hands::HandInstance)> {
        let def = self
            .hand_registry
            .get_definition(hand_id)
            .ok_or_else(|| {
                KernelError::Captain(CaptainError::AgentNotFound(format!(
                    "Hand not found: {hand_id}"
                )))
            })?
            .clone();
        let instance = self
            .hand_registry
            .activate(hand_id, config)
            .map_err(map_hand_activation_error)?;
        Ok((def, instance))
    }

    fn hand_manifest(
        &self,
        def: &captain_hands::HandDefinition,
        instance: &captain_hands::HandInstance,
    ) -> AgentManifest {
        let (hand_provider, hand_model) = self.resolved_hand_model(def);
        let mut manifest = AgentManifest {
            name: def.agent.name.clone(),
            description: def.agent.description.clone(),
            module: def.agent.module.clone(),
            model: ModelConfig {
                provider: hand_provider,
                model: hand_model,
                max_tokens: def.agent.max_tokens,
                temperature: def.agent.temperature,
                system_prompt: def.agent.system_prompt.clone(),
                api_key_env: def.agent.api_key_env.clone(),
                base_url: def.agent.base_url.clone(),
            },
            capabilities: ManifestCapabilities {
                tools: def.tools.clone(),
                ..Default::default()
            },
            tags: vec![
                format!("hand:{}", def.id),
                format!("hand_instance:{}", instance.instance_id),
            ],
            autonomous: def.agent.max_iterations.map(|max_iter| AutonomousConfig {
                max_iterations: max_iter,
                ..Default::default()
            }),
            schedule: hand_schedule(def),
            skills: def.skills.clone(),
            mcp_servers: def.mcp_servers.clone(),
            exec_policy: hand_exec_policy(def),
            tool_blocklist: Vec::new(),
            profile: hand_tool_profile(def),
            ..Default::default()
        };
        apply_hand_settings(def, instance, &mut manifest);
        apply_hand_skill_content(def, &mut manifest);
        manifest
    }

    fn resolved_hand_model(&self, def: &captain_hands::HandDefinition) -> (String, String) {
        let provider = if def.agent.provider == "default" {
            self.config.default_model.provider.clone()
        } else {
            def.agent.provider.clone()
        };
        let model = if def.agent.model == "default" {
            self.config.default_model.model.clone()
        } else {
            def.agent.model.clone()
        };
        (provider, model)
    }

    fn remove_existing_hand_agent(&self, agent_name: &str) -> SavedHandAgentState {
        let existing = self
            .registry
            .list()
            .into_iter()
            .find(|entry| entry.name == agent_name);
        let old_agent_id = existing.as_ref().map(|entry| entry.id);
        let triggers = old_agent_id
            .map(|id| self.triggers.take_agent_triggers(id))
            .unwrap_or_default();
        let file_triggers = old_agent_id
            .map(|id| self.triggers.take_agent_file_triggers(id))
            .unwrap_or_default();
        if let Some(old) = existing {
            info!(agent = %old.name, id = %old.id, "Removing existing hand agent for reactivation");
            self.reactivating_hand.store(true, Ordering::Relaxed);
            let _ = self.kill_agent(old.id);
            self.reactivating_hand.store(false, Ordering::Relaxed);
        }
        SavedHandAgentState {
            old_agent_id,
            triggers,
            file_triggers,
        }
    }

    fn restore_reactivated_hand_runtime(
        &self,
        agent_id: AgentId,
        saved_state: &SavedHandAgentState,
    ) {
        self.restore_reactivated_hand_triggers(agent_id, saved_state);
        self.restore_reactivated_hand_file_triggers(agent_id, saved_state);
        self.migrate_reactivated_hand_cron_jobs(agent_id, saved_state.old_agent_id);
    }

    fn restore_reactivated_hand_triggers(
        &self,
        agent_id: AgentId,
        saved_state: &SavedHandAgentState,
    ) {
        if saved_state.triggers.is_empty() {
            return;
        }
        let restored = self
            .triggers
            .restore_triggers(agent_id, saved_state.triggers.clone());
        if restored > 0 {
            if let Some(old_agent_id) = saved_state.old_agent_id {
                info!(
                    old_agent = %old_agent_id,
                    new_agent = %agent_id,
                    restored,
                    "Reassigned triggers after hand reactivation"
                );
            }
        }
    }

    fn restore_reactivated_hand_file_triggers(
        &self,
        agent_id: AgentId,
        saved_state: &SavedHandAgentState,
    ) {
        if saved_state.file_triggers.is_empty() {
            return;
        }
        match self
            .triggers
            .restore_file_triggers(agent_id, saved_state.file_triggers.clone())
        {
            Ok(restored) => self.arm_restored_hand_file_triggers(agent_id, restored, saved_state),
            Err(e) => warn!(
                old_agent = ?saved_state.old_agent_id,
                new_agent = %agent_id,
                error = %e,
                "Failed to restore file-change triggers after hand reactivation"
            ),
        }
    }

    fn arm_restored_hand_file_triggers(
        &self,
        agent_id: AgentId,
        restored: Vec<FileChangeTrigger>,
        saved_state: &SavedHandAgentState,
    ) {
        for trigger in &restored {
            if let Err(e) = self.arm_file_change_trigger(trigger.clone()) {
                warn!(
                    trigger_id = %trigger.id,
                    error = %e,
                    "Failed to arm restored file-change trigger"
                );
            }
        }
        if let Some(old_agent_id) = saved_state.old_agent_id {
            info!(
                old_agent = %old_agent_id,
                new_agent = %agent_id,
                restored = restored.len(),
                "Reassigned file-change triggers after hand reactivation"
            );
        }
    }

    fn migrate_reactivated_hand_cron_jobs(&self, agent_id: AgentId, old_agent_id: Option<AgentId>) {
        if let Some(old_id) = old_agent_id {
            let migrated = self.cron_scheduler.reassign_agent_jobs(old_id, agent_id);
            if migrated > 0 {
                if let Err(e) = self.cron_scheduler.persist() {
                    warn!("Failed to persist cron jobs after agent migration: {e}");
                }
            }
        }
    }

    fn set_hand_instance_agent(
        &self,
        instance_id: uuid::Uuid,
        agent_id: AgentId,
    ) -> KernelResult<()> {
        self.hand_registry
            .set_agent(instance_id, agent_id)
            .map_err(|e| KernelError::Captain(CaptainError::Internal(e.to_string())))
    }

    /// Deactivate a hand: kill agent and remove instance.
    pub fn deactivate_hand(&self, instance_id: uuid::Uuid) -> KernelResult<()> {
        let instance = self
            .hand_registry
            .deactivate(instance_id)
            .map_err(|e| KernelError::Captain(CaptainError::Internal(e.to_string())))?;

        if let Some(agent_id) = instance.agent_id {
            if let Err(e) = self.kill_agent(agent_id) {
                warn!(agent = %agent_id, error = %e, "Failed to kill hand agent (may already be dead)");
            }
        } else {
            let hand_tag = format!("hand:{}", instance.hand_id);
            for entry in self.registry.list() {
                if entry.tags.contains(&hand_tag) {
                    if let Err(e) = self.kill_agent(entry.id) {
                        warn!(agent = %entry.id, error = %e, "Failed to kill orphaned hand agent");
                    } else {
                        info!(agent_id = %entry.id, hand_id = %instance.hand_id, "Cleaned up orphaned hand agent");
                    }
                }
            }
        }
        self.persist_hand_state();
        Ok(())
    }

    fn persist_hand_state(&self) {
        let state_path = self.config.home_dir.join("hand_state.json");
        if let Err(e) = self.hand_registry.persist_state(&state_path) {
            warn!(error = %e, "Failed to persist hand state");
        }
    }

    /// Pause a hand.
    pub fn pause_hand(&self, instance_id: uuid::Uuid) -> KernelResult<()> {
        self.hand_registry
            .pause(instance_id)
            .map_err(|e| KernelError::Captain(CaptainError::Internal(e.to_string())))
    }

    /// Resume a paused hand.
    pub fn resume_hand(&self, instance_id: uuid::Uuid) -> KernelResult<()> {
        self.hand_registry
            .resume(instance_id)
            .map_err(|e| KernelError::Captain(CaptainError::Internal(e.to_string())))
    }

    pub(super) fn restore_persisted_hands(&self) {
        let state_path = self.config.home_dir.join("hand_state.json");
        let saved_hands = captain_hands::registry::HandRegistry::load_state(&state_path);
        if saved_hands.is_empty() {
            return;
        }

        info!("Restoring {} persisted hand(s)", saved_hands.len());
        for (hand_id, config, old_agent_id) in saved_hands {
            match self.activate_hand(&hand_id, config) {
                Ok(inst) => {
                    info!(hand = %hand_id, instance = %inst.instance_id, "Hand restored");
                    if let (Some(old_id), Some(new_id)) = (old_agent_id, inst.agent_id) {
                        if old_id != new_id {
                            let migrated = self.cron_scheduler.reassign_agent_jobs(old_id, new_id);
                            if migrated > 0 {
                                info!(
                                    hand = %hand_id,
                                    old_agent = %old_id,
                                    new_agent = %new_id,
                                    migrated,
                                    "Reassigned cron jobs after restart"
                                );
                                if let Err(e) = self.cron_scheduler.persist() {
                                    warn!("Failed to persist cron jobs after hand restore: {e}");
                                }
                            }

                            let t_migrated = self.triggers.reassign_agent_triggers(old_id, new_id);
                            if t_migrated > 0 {
                                info!(
                                    hand = %hand_id,
                                    old_agent = %old_id,
                                    new_agent = %new_id,
                                    migrated = t_migrated,
                                    "Reassigned triggers after restart"
                                );
                            }
                        }
                    }
                }
                Err(e) => warn!(hand = %hand_id, error = %e, "Failed to restore hand"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::CaptainKernel;
    use captain_hands::{HandAgentConfig, HandCategory, HandDashboard, HandDefinition};
    use captain_types::agent::AgentId;
    use captain_types::config::KernelConfig;
    use std::collections::HashMap;

    fn test_hand_definition() -> HandDefinition {
        HandDefinition {
            id: "demo".to_string(),
            name: "Demo".to_string(),
            description: "Demo hand".to_string(),
            category: HandCategory::Productivity,
            icon: String::new(),
            tools: Vec::new(),
            skills: Vec::new(),
            mcp_servers: Vec::new(),
            requires: Vec::new(),
            settings: Vec::new(),
            agent: HandAgentConfig::default(),
            dashboard: HandDashboard::default(),
            skill_content: None,
        }
    }

    #[test]
    fn hand_manifest_helpers_keep_schedule_exec_and_profile_contracts() {
        let mut def = test_hand_definition();

        assert!(matches!(hand_schedule(&def), ScheduleMode::Reactive));
        assert!(hand_exec_policy(&def).is_none());
        assert!(hand_tool_profile(&def).is_none());

        def.agent.max_iterations = Some(5);
        assert!(matches!(
            hand_schedule(&def),
            ScheduleMode::Continuous {
                check_interval_secs: 60
            }
        ));

        def.tools.push("file_read".to_string());
        assert_eq!(hand_tool_profile(&def), Some(ToolProfile::Custom));
        assert!(hand_exec_policy(&def).is_none());

        def.tools.push("shell_exec".to_string());
        let exec = hand_exec_policy(&def).expect("shell_exec enables curated hand exec policy");
        assert_eq!(exec.timeout_secs, 300);
        assert_eq!(exec.no_output_timeout_secs, 120);
    }

    #[test]
    fn hand_activation_error_mapping_preserves_already_active_context() {
        let error = map_hand_activation_error(captain_hands::HandError::AlreadyActive(
            "browser".to_string(),
        ));

        assert!(format!("{error}").contains("Hand already active: browser"));
    }

    #[test]
    fn hand_activation_does_not_seed_runtime_tool_filters() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("captain-kernel-hand-test");
        std::fs::create_dir_all(&home_dir).unwrap();

        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };

        let kernel = CaptainKernel::boot_with_config(config).expect("Kernel should boot");
        let instance = kernel
            .activate_hand("browser", HashMap::new())
            .expect("browser hand should activate");
        let agent_id = instance.agent_id.expect("browser hand agent id");
        let entry = kernel
            .registry
            .get(agent_id)
            .expect("browser hand agent entry");

        assert!(
            entry.manifest.tool_allowlist.is_empty(),
            "hand activation should leave the runtime tool allowlist empty so skill/MCP tools remain visible"
        );
        assert!(
            entry.manifest.tool_blocklist.is_empty(),
            "hand activation should not set a runtime blocklist by default"
        );

        kernel.shutdown();
    }

    #[test]
    fn restore_persisted_hands_activates_saved_hand_with_stable_agent_id() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("captain-kernel-hand-restore-test");
        std::fs::create_dir_all(&home_dir).unwrap();

        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };
        let old_agent_id = AgentId::new();
        std::fs::write(
            home_dir.join("hand_state.json"),
            serde_json::to_string_pretty(&serde_json::json!([
                {
                    "hand_id": "browser",
                    "config": {},
                    "agent_id": old_agent_id
                }
            ]))
            .unwrap(),
        )
        .unwrap();

        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");
        kernel.restore_persisted_hands();

        let instances = kernel.hand_registry.list_instances();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].hand_id, "browser");
        assert_eq!(instances[0].agent_id, Some(AgentId::from_string("browser")));
        assert!(kernel
            .registry
            .get(AgentId::from_string("browser"))
            .is_some());

        kernel.shutdown();
    }
}
