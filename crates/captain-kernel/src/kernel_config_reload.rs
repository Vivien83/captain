use super::CaptainKernel;
use crate::config::load_config;
use crate::config_reload::{
    build_reload_plan, should_apply_hot, validate_config_for_reload, HotAction, ReloadPlan,
};
use captain_types::config::{AgentBinding, KernelConfig, ReloadMode};
use tracing::info;

impl CaptainKernel {
    /// List all agent bindings.
    pub fn list_bindings(&self) -> Vec<AgentBinding> {
        self.bindings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Add a binding at runtime.
    pub fn add_binding(&self, binding: AgentBinding) {
        let mut bindings = self.bindings.lock().unwrap_or_else(|e| e.into_inner());
        bindings.push(binding);
        bindings.sort_by_key(|b| std::cmp::Reverse(b.match_rule.specificity()));
    }

    /// Remove a binding by index, returns the removed binding if valid.
    pub fn remove_binding(&self, index: usize) -> Option<AgentBinding> {
        let mut bindings = self.bindings.lock().unwrap_or_else(|e| e.into_inner());
        if index < bindings.len() {
            Some(bindings.remove(index))
        } else {
            None
        }
    }

    /// Reload configuration: read the config file, diff against current, and
    /// apply hot-reloadable actions. Returns the reload plan for API response.
    pub fn reload_config(&self) -> Result<ReloadPlan, String> {
        let config_path = self.config.home_dir.join("config.toml");
        let new_config = if config_path.exists() {
            load_config(Some(&config_path))
        } else {
            return Err("Config file not found".to_string());
        };

        if let Err(errors) = validate_config_for_reload(&new_config) {
            return Err(format!("Validation failed: {}", errors.join("; ")));
        }

        let mut plan = build_reload_plan(&self.config, &new_config);
        plan.log_summary();
        let requested_hot_actions = plan.hot_actions.clone();

        if should_apply_hot(self.config.reload.mode, &plan) {
            let applied_hot_actions = self.apply_hot_actions(&requested_hot_actions, &new_config);
            finalize_hot_action_status(
                &mut plan,
                &requested_hot_actions,
                applied_hot_actions,
                "kernel reload path does not auto-apply this subsystem",
            );
        } else if !requested_hot_actions.is_empty() {
            let reason = hot_reload_skipped_reason(self.config.reload.mode);
            finalize_hot_action_status(&mut plan, &requested_hot_actions, Vec::new(), &reason);
        }

        Ok(plan)
    }

    /// Apply hot-reload actions to the running kernel.
    fn apply_hot_actions(
        &self,
        actions: &[HotAction],
        new_config: &KernelConfig,
    ) -> Vec<HotAction> {
        let mut applied = Vec::new();
        for action in actions {
            match action {
                HotAction::UpdateApprovalPolicy => {
                    info!("Hot-reload: updating approval policy");
                    self.approval_manager
                        .update_policy(new_config.approval.clone());
                    applied.push(action.clone());
                }
                HotAction::UpdateCronConfig => {
                    info!(
                        "Hot-reload: updating cron config (max_jobs={})",
                        new_config.max_cron_jobs
                    );
                    self.cron_scheduler
                        .set_max_total_jobs(new_config.max_cron_jobs);
                    applied.push(action.clone());
                }
                HotAction::ReloadProviderUrls => {
                    info!("Hot-reload: applying provider URL overrides");
                    let mut catalog = self
                        .model_catalog
                        .write()
                        .unwrap_or_else(|e| e.into_inner());
                    catalog.apply_url_overrides(&new_config.provider_urls);
                    applied.push(action.clone());
                }
                HotAction::UpdateDefaultModel => {
                    info!(
                        "Hot-reload: updating default model to {}/{}",
                        new_config.default_model.provider, new_config.default_model.model
                    );
                    let mut guard = self
                        .default_model_override
                        .write()
                        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
                    *guard = Some(new_config.default_model.clone());
                    applied.push(action.clone());
                }
                HotAction::UpdateTtsConfig => {
                    info!(
                        provider = ?new_config.tts.provider,
                        elevenlabs_voice_id = %new_config.tts.elevenlabs.voice_id,
                        openai_voice = %new_config.tts.openai.voice,
                        "Hot-reload: updating TTS runtime config"
                    );
                    self.tts_engine.update_config(new_config.tts.clone());
                    applied.push(action.clone());
                }
                _ => {
                    info!(
                        "Hot-reload: action {:?} noted but not yet auto-applied",
                        action
                    );
                }
            }
        }
        applied
    }
}

fn hot_reload_skipped_reason(mode: ReloadMode) -> String {
    format!("reload mode {mode:?} does not apply hot actions")
}

fn finalize_hot_action_status(
    plan: &mut ReloadPlan,
    requested_hot_actions: &[HotAction],
    applied_hot_actions: Vec<HotAction>,
    deferred_reason: &str,
) {
    for action in requested_hot_actions {
        if applied_hot_actions.contains(action) {
            continue;
        }
        plan.restart_required = true;
        plan.restart_reasons.push(format!(
            "{action:?} detected but not applied ({deferred_reason})"
        ));
    }
    plan.hot_actions = applied_hot_actions;
}

#[cfg(test)]
mod tests {
    use super::{finalize_hot_action_status, hot_reload_skipped_reason};
    use crate::config_reload::{HotAction, ReloadPlan};
    use crate::kernel::CaptainKernel;
    use captain_types::config::{AgentBinding, BindingMatchRule, KernelConfig, ReloadMode};

    #[test]
    fn runtime_bindings_are_sorted_by_specificity() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("captain-kernel-binding-test");
        std::fs::create_dir_all(&home_dir).unwrap();

        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };

        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");
        kernel.add_binding(AgentBinding {
            agent: "fallback".to_string(),
            match_rule: BindingMatchRule {
                channel: Some("discord".to_string()),
                ..Default::default()
            },
        });
        kernel.add_binding(AgentBinding {
            agent: "direct".to_string(),
            match_rule: BindingMatchRule {
                channel: Some("discord".to_string()),
                account_id: Some("bot-1".to_string()),
                peer_id: Some("user-1".to_string()),
                guild_id: Some("guild-1".to_string()),
                roles: vec!["admin".to_string()],
            },
        });

        let bindings = kernel.list_bindings();
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].agent, "direct");
        assert_eq!(bindings[1].agent, "fallback");
        assert!(kernel.remove_binding(99).is_none());
        assert_eq!(kernel.remove_binding(1).unwrap().agent, "fallback");

        kernel.shutdown();
    }

    #[test]
    fn finalize_hot_action_status_keeps_only_applied_actions() {
        let mut plan = ReloadPlan {
            restart_required: false,
            restart_reasons: Vec::new(),
            hot_actions: vec![HotAction::UpdateCronConfig, HotAction::ReloadChannels],
            noop_changes: Vec::new(),
        };
        let requested = plan.hot_actions.clone();

        finalize_hot_action_status(
            &mut plan,
            &requested,
            vec![HotAction::UpdateCronConfig],
            "not wired",
        );

        assert_eq!(plan.hot_actions, vec![HotAction::UpdateCronConfig]);
        assert!(plan.restart_required);
        assert!(plan
            .restart_reasons
            .iter()
            .any(|change| change.contains("ReloadChannels detected but not applied")));
    }

    #[test]
    fn hot_reload_skipped_reason_names_reload_mode() {
        let reason = hot_reload_skipped_reason(ReloadMode::Restart);
        assert!(reason.contains("Restart"));
        assert!(reason.contains("does not apply hot actions"));
    }
}
