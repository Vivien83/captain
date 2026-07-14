use crate::error::{KernelError, KernelResult};
use crate::model_switch::{
    ModelSwitchApplyResult, ModelSwitchPlan, ModelSwitchRisk, ModelSwitchSessionStrategy,
};
use captain_runtime::llm_driver::DriverConfig;
use captain_types::agent::{AgentEntry, AgentId};
use captain_types::error::CaptainError;
use captain_types::message::Message;

use super::{drivers, infer_provider_from_model, strip_provider_prefix, CaptainKernel};

impl CaptainKernel {
    fn resolve_model_switch_target(
        &self,
        agent_id: AgentId,
        model: &str,
        explicit_provider: Option<&str>,
    ) -> KernelResult<(String, String)> {
        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Captain(CaptainError::AgentNotFound(agent_id.to_string()))
        })?;

        let provider = if let Some(ep) = explicit_provider {
            ep.to_string()
        } else {
            let has_custom_url = entry.manifest.model.base_url.is_some();
            if has_custom_url {
                entry.manifest.model.provider.clone()
            } else {
                self.model_catalog
                    .read()
                    .ok()
                    .and_then(|catalog| catalog.find_model(model).map(|m| m.provider.clone()))
                    .or_else(|| infer_provider_from_model(model))
                    .unwrap_or_else(|| entry.manifest.model.provider.clone())
            }
        };

        let mut normalized_model = strip_provider_prefix(model, &provider);
        if let Ok(catalog) = self.model_catalog.read() {
            if let Some(catalog_model) = catalog.find_model(model) {
                if catalog_model.provider == provider {
                    normalized_model = strip_provider_prefix(&catalog_model.id, &provider);
                }
            }
        }

        Ok((provider, normalized_model))
    }

    fn model_switch_driver_status(&self, provider: &str) -> (bool, Option<String>) {
        let env_var = self.config.resolve_api_key_env(provider);
        let api_key = self.resolve_credential(&env_var);
        let driver_config = DriverConfig {
            provider: provider.to_string(),
            api_key,
            base_url: self.lookup_provider_url(provider),
            skip_permissions: true,
        };
        match drivers::create_driver(&driver_config) {
            Ok(_) => (true, None),
            Err(e) => (false, Some(e.to_string())),
        }
    }

    /// Build a read-only preflight for switching an agent to another model/provider.
    ///
    /// This is intentionally separate from `set_agent_model`: provider switches are
    /// session migrations, not simple string updates.
    pub fn plan_model_switch(
        &self,
        agent_id: AgentId,
        model: &str,
        explicit_provider: Option<&str>,
    ) -> KernelResult<ModelSwitchPlan> {
        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Captain(CaptainError::AgentNotFound(agent_id.to_string()))
        })?;
        let (target_provider, target_model) =
            self.resolve_model_switch_target(agent_id, model, explicit_provider)?;
        let context = self.model_switch_context(agent_id, &entry)?;
        let provider_changed = entry.manifest.model.provider != target_provider;
        let model_changed = entry.manifest.model.model != target_model;
        let target =
            self.model_switch_target_info(model, target_provider.as_str(), target_model.as_str());
        let (driver_ready, driver_error) = self.model_switch_driver_status(&target_provider);
        let warnings = model_switch_warnings(
            provider_changed,
            context.has_active_context(),
            &target_provider,
            &target_model,
            &target,
        );
        let blocking_issues = model_switch_blocking_issues(
            &target_provider,
            &target_model,
            &target,
            driver_ready,
            driver_error.as_deref(),
        );
        let can_apply = blocking_issues.is_empty();
        let risk = model_switch_risk(
            can_apply,
            provider_changed,
            context.has_active_context(),
            &target,
        );

        Ok(ModelSwitchPlan {
            agent_id: agent_id.to_string(),
            agent_name: entry.name,
            current_provider: entry.manifest.model.provider,
            current_model: entry.manifest.model.model,
            target_provider,
            target_model,
            provider_changed,
            model_changed,
            active_session_id: entry.session_id.to_string(),
            active_message_count: context.active_message_count,
            canonical_summary_present: context.canonical_summary_present,
            canonical_recent_count: context.canonical_recent_count,
            session_strategy_required: provider_changed && context.has_active_context(),
            recommended_session_strategy: context.recommended_strategy(),
            target_model_known: target.model_known,
            target_provider_known: target.provider_known,
            target_auth_configured: target.auth_configured,
            target_supports_tools: target.supports_tools,
            target_supports_vision: target.supports_vision,
            target_supports_streaming: target.supports_streaming,
            driver_ready,
            driver_error,
            risk,
            can_apply,
            blocking_issues,
            warnings,
        })
    }

    fn model_switch_context(
        &self,
        agent_id: AgentId,
        entry: &AgentEntry,
    ) -> KernelResult<ModelSwitchContext> {
        let active_message_count = self
            .memory
            .get_session(entry.session_id)
            .map_err(KernelError::Captain)?
            .map(|s| s.messages.len())
            .unwrap_or(0);
        let (canonical_summary_present, canonical_recent_count) = self
            .memory
            .canonical_context(agent_id, Some(8))
            .map(|(summary, recent)| {
                (
                    summary
                        .as_ref()
                        .map(|s| !s.trim().is_empty())
                        .unwrap_or(false),
                    recent.len(),
                )
            })
            .unwrap_or((false, 0));

        Ok(ModelSwitchContext {
            active_message_count,
            canonical_summary_present,
            canonical_recent_count,
        })
    }

    fn model_switch_target_info(
        &self,
        requested_model: &str,
        target_provider: &str,
        target_model: &str,
    ) -> ModelSwitchTargetInfo {
        let catalog = self.model_catalog.read().unwrap_or_else(|e| e.into_inner());
        let provider_info = catalog.get_provider(target_provider);
        let target_model_prefixed = format!("{target_provider}/{target_model}");
        let model_info = catalog
            .find_model(requested_model)
            .or_else(|| catalog.find_model(target_model))
            .or_else(|| catalog.find_model(&target_model_prefixed));

        ModelSwitchTargetInfo {
            model_known: model_info.is_some(),
            provider_known: provider_info.is_some(),
            auth_configured: provider_info
                .map(|p| {
                    matches!(
                        p.auth_status,
                        captain_types::model_catalog::AuthStatus::Configured
                            | captain_types::model_catalog::AuthStatus::NotRequired
                    )
                })
                .unwrap_or(false),
            supports_tools: model_info.map(|m| m.supports_tools),
            supports_vision: model_info.map(|m| m.supports_vision),
            supports_streaming: model_info.map(|m| m.supports_streaming),
        }
    }

    fn portable_switch_summary(
        &self,
        agent_id: AgentId,
        entry: &AgentEntry,
    ) -> KernelResult<Option<String>> {
        let session = self
            .memory
            .get_session(entry.session_id)
            .map_err(KernelError::Captain)?;
        let (canonical_summary, canonical_recent) = self
            .memory
            .canonical_context(agent_id, Some(12))
            .unwrap_or((None, Vec::new()));
        let session_messages = session
            .as_ref()
            .map(|s| s.messages.as_slice())
            .unwrap_or(&[]);
        if !portable_context_exists(
            session_messages,
            canonical_summary.as_deref(),
            &canonical_recent,
        ) {
            return Ok(None);
        }

        let mut lines = portable_summary_header(entry);
        append_canonical_summary(&mut lines, canonical_summary.as_deref());
        append_recent_switch_messages(
            &mut lines,
            portable_recent_messages(session_messages, &canonical_recent),
        );

        let joined = lines.join("\n");
        Ok(Some(
            captain_types::truncate_str(&joined, 12_000).to_string(),
        ))
    }

    /// Apply a model/provider switch through the safe provider-portability rail.
    ///
    /// The old provider-specific active history is never carried verbatim into
    /// the new provider. It is either dropped (`new_session`) or converted into a
    /// provider-neutral canonical summary (`compact_session`).
    pub fn apply_model_switch(
        &self,
        agent_id: AgentId,
        model: &str,
        explicit_provider: Option<&str>,
        session_strategy: ModelSwitchSessionStrategy,
    ) -> KernelResult<ModelSwitchApplyResult> {
        let plan = self.plan_model_switch(agent_id, model, explicit_provider)?;
        ensure_model_switch_can_apply(&plan)?;

        let entry_before = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Captain(CaptainError::AgentNotFound(agent_id.to_string()))
        })?;
        let previous_session_id = entry_before.session_id.to_string();
        let portable_summary =
            self.portable_summary_for_strategy(agent_id, &entry_before, session_strategy)?;
        let global_default_updated = Self::is_principal_agent(&entry_before);

        if global_default_updated {
            self.persist_principal_default_model_switch(&plan.target_provider, &plan.target_model)?;
        }

        self.set_agent_model(
            agent_id,
            &plan.target_model,
            Some(plan.target_provider.as_str()),
        )?;

        let compacted_summary_chars = portable_summary.as_ref().map(|s| s.len()).unwrap_or(0);
        self.apply_model_switch_context_strategy(
            agent_id,
            session_strategy,
            portable_summary.as_deref(),
        )?;
        let new_session_id = self.create_model_switch_session(agent_id, &plan.target_provider)?;
        let message = model_switch_apply_message(&plan, session_strategy);
        self.resolve_codex_model_update_after_switch(
            agent_id,
            &plan.target_provider,
            &plan.target_model,
        );

        Ok(ModelSwitchApplyResult {
            status: "ok".to_string(),
            plan,
            session_strategy,
            previous_session_id,
            new_session_id,
            compacted_summary_chars,
            global_default_updated,
            message,
        })
    }

    fn portable_summary_for_strategy(
        &self,
        agent_id: AgentId,
        entry: &AgentEntry,
        session_strategy: ModelSwitchSessionStrategy,
    ) -> KernelResult<Option<String>> {
        if session_strategy == ModelSwitchSessionStrategy::CompactSession {
            self.portable_switch_summary(agent_id, entry)
        } else {
            Ok(None)
        }
    }

    fn apply_model_switch_context_strategy(
        &self,
        agent_id: AgentId,
        session_strategy: ModelSwitchSessionStrategy,
        portable_summary: Option<&str>,
    ) -> KernelResult<()> {
        match session_strategy {
            ModelSwitchSessionStrategy::NewSession => {
                let _ = self.memory.delete_canonical_session(agent_id);
            }
            ModelSwitchSessionStrategy::CompactSession => {
                if let Some(summary) = portable_summary {
                    self.memory
                        .store_llm_summary(agent_id, summary, Vec::new())
                        .map_err(KernelError::Captain)?;
                } else {
                    let _ = self.memory.delete_canonical_session(agent_id);
                }
            }
        }
        Ok(())
    }

    fn create_model_switch_session(
        &self,
        agent_id: AgentId,
        target_provider: &str,
    ) -> KernelResult<String> {
        let label = format!(
            "model-switch-{}-{}",
            target_provider,
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        );
        let session = self.create_agent_session(agent_id, Some(&label))?;
        self.scheduler.reset_usage(agent_id);
        Ok(session
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string())
    }
}

#[derive(Debug, Clone, Copy)]
struct ModelSwitchContext {
    active_message_count: usize,
    canonical_summary_present: bool,
    canonical_recent_count: usize,
}

impl ModelSwitchContext {
    fn has_active_context(self) -> bool {
        self.active_message_count > 0
            || self.canonical_summary_present
            || self.canonical_recent_count > 0
    }

    fn recommended_strategy(self) -> ModelSwitchSessionStrategy {
        if self.has_active_context() {
            ModelSwitchSessionStrategy::CompactSession
        } else {
            ModelSwitchSessionStrategy::NewSession
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ModelSwitchTargetInfo {
    model_known: bool,
    provider_known: bool,
    auth_configured: bool,
    supports_tools: Option<bool>,
    supports_vision: Option<bool>,
    supports_streaming: Option<bool>,
}

fn model_switch_warnings(
    provider_changed: bool,
    has_active_context: bool,
    target_provider: &str,
    target_model: &str,
    target: &ModelSwitchTargetInfo,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if provider_changed && has_active_context {
        warnings.push(
            "Provider switch with active context: choose new_session or compact_session to avoid provider-specific tool-call history corruption."
                .to_string(),
        );
    }
    if !target.model_known {
        warnings.push(format!(
            "Target model '{target_model}' is not in the model catalog; custom models are allowed, but capabilities cannot be verified."
        ));
    }
    if !target.provider_known {
        warnings.push(format!(
            "Target provider '{target_provider}' is not in the model catalog; it must have a configured base_url or driver support."
        ));
    }
    if !target.auth_configured {
        warnings.push(format!(
            "Target provider '{target_provider}' is not authenticated according to the model catalog."
        ));
    }
    warnings
}

fn model_switch_blocking_issues(
    target_provider: &str,
    target_model: &str,
    target: &ModelSwitchTargetInfo,
    driver_ready: bool,
    driver_error: Option<&str>,
) -> Vec<String> {
    let mut blocking_issues = Vec::new();
    if target.supports_tools == Some(false) {
        blocking_issues.push(format!(
            "Target model '{target_model}' does not advertise tool/function calling; Captain workflows would break."
        ));
    }
    if !driver_ready {
        blocking_issues.push(format!(
            "Target provider '{target_provider}' driver cannot initialize: {}",
            driver_error.unwrap_or("unknown driver initialization error")
        ));
    }
    blocking_issues
}

fn model_switch_risk(
    can_apply: bool,
    provider_changed: bool,
    has_active_context: bool,
    target: &ModelSwitchTargetInfo,
) -> ModelSwitchRisk {
    if !can_apply || (provider_changed && has_active_context) {
        ModelSwitchRisk::High
    } else if provider_changed || !target.model_known || !target.provider_known {
        ModelSwitchRisk::Medium
    } else {
        ModelSwitchRisk::Low
    }
}

fn portable_context_exists(
    session_messages: &[Message],
    canonical_summary: Option<&str>,
    canonical_recent: &[Message],
) -> bool {
    !session_messages.is_empty()
        || canonical_summary
            .map(|summary| !summary.trim().is_empty())
            .unwrap_or(false)
        || !canonical_recent.is_empty()
}

fn portable_summary_header(entry: &AgentEntry) -> Vec<String> {
    vec![
        "Provider-switch portable context summary.".to_string(),
        format!("Generated at: {}", chrono::Utc::now().to_rfc3339()),
        format!(
            "Previous model: {}/{}",
            entry.manifest.model.provider, entry.manifest.model.model
        ),
    ]
}

fn append_canonical_summary(lines: &mut Vec<String>, canonical_summary: Option<&str>) {
    let Some(summary) = canonical_summary
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
    else {
        return;
    };
    lines.push("Existing canonical summary:".to_string());
    lines.push(captain_types::truncate_str(summary, 2_000).to_string());
}

fn portable_recent_messages<'a>(
    session_messages: &'a [Message],
    canonical_recent: &'a [Message],
) -> &'a [Message] {
    if session_messages.is_empty() {
        canonical_recent
    } else {
        session_messages
    }
}

fn append_recent_switch_messages(lines: &mut Vec<String>, recent: &[Message]) {
    if recent.is_empty() {
        return;
    }
    lines.push("Recent conversation before switch:".to_string());
    for msg in recent
        .iter()
        .rev()
        .take(40)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let text = msg.content.text_content();
        let text = text.trim();
        if !text.is_empty() {
            lines.push(format!(
                "- {}: {}",
                portable_message_role(msg),
                captain_types::truncate_str(text, 500)
            ));
        }
    }
}

fn portable_message_role(msg: &Message) -> &'static str {
    match msg.role {
        captain_types::message::Role::User => "User",
        captain_types::message::Role::Assistant => "Assistant",
        captain_types::message::Role::System => "System",
    }
}

fn ensure_model_switch_can_apply(plan: &ModelSwitchPlan) -> KernelResult<()> {
    if plan.can_apply {
        return Ok(());
    }
    Err(KernelError::Captain(CaptainError::Internal(format!(
        "Model switch preflight failed: {}",
        plan.blocking_issues.join("; ")
    ))))
}

fn model_switch_apply_message(
    plan: &ModelSwitchPlan,
    session_strategy: ModelSwitchSessionStrategy,
) -> String {
    match session_strategy {
        ModelSwitchSessionStrategy::NewSession => format!(
            "Switched to {}/{} with a fresh session.",
            plan.target_provider, plan.target_model
        ),
        ModelSwitchSessionStrategy::CompactSession => format!(
            "Switched to {}/{} with a provider-neutral context summary.",
            plan.target_provider, plan.target_model
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::message::{MessageContent, Role};

    fn target_info(
        model_known: bool,
        provider_known: bool,
        auth_configured: bool,
        supports_tools: Option<bool>,
    ) -> ModelSwitchTargetInfo {
        ModelSwitchTargetInfo {
            model_known,
            provider_known,
            auth_configured,
            supports_tools,
            supports_vision: None,
            supports_streaming: None,
        }
    }

    fn message(role: Role, text: &str) -> Message {
        Message {
            role,
            content: MessageContent::Text(text.to_string()),
        }
    }

    #[test]
    fn model_switch_context_detects_active_context_and_strategy() {
        let empty = ModelSwitchContext {
            active_message_count: 0,
            canonical_summary_present: false,
            canonical_recent_count: 0,
        };
        let active = ModelSwitchContext {
            active_message_count: 0,
            canonical_summary_present: true,
            canonical_recent_count: 0,
        };

        assert!(!empty.has_active_context());
        assert_eq!(
            empty.recommended_strategy(),
            ModelSwitchSessionStrategy::NewSession
        );
        assert!(active.has_active_context());
        assert_eq!(
            active.recommended_strategy(),
            ModelSwitchSessionStrategy::CompactSession
        );
    }

    #[test]
    fn model_switch_safety_messages_cover_unknowns_and_blockers() {
        let target = target_info(false, false, false, Some(false));

        let warnings = model_switch_warnings(true, true, "custom", "model-x", &target);
        let blockers =
            model_switch_blocking_issues("custom", "model-x", &target, false, Some("missing key"));

        assert!(warnings
            .iter()
            .any(|warning| warning.contains("Provider switch with active context")));
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("not in the model catalog")));
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("not authenticated")));
        assert!(blockers
            .iter()
            .any(|issue| issue.contains("does not advertise tool/function calling")));
        assert!(blockers.iter().any(|issue| issue.contains("missing key")));
    }

    #[test]
    fn model_switch_risk_escalates_for_blockers_and_provider_context() {
        let known = target_info(true, true, true, Some(true));
        let unknown_model = target_info(false, true, true, Some(true));

        assert_eq!(
            model_switch_risk(true, false, false, &known),
            ModelSwitchRisk::Low
        );
        assert_eq!(
            model_switch_risk(true, false, false, &unknown_model),
            ModelSwitchRisk::Medium
        );
        assert_eq!(
            model_switch_risk(true, true, true, &known),
            ModelSwitchRisk::High
        );
        assert_eq!(
            model_switch_risk(false, false, false, &known),
            ModelSwitchRisk::High
        );
    }

    #[test]
    fn portable_summary_helpers_prefer_session_messages() {
        let session_messages = vec![message(Role::User, "session text")];
        let canonical_recent = vec![message(Role::Assistant, "canonical text")];
        let selected = portable_recent_messages(&session_messages, &canonical_recent);
        let mut lines = Vec::new();

        assert!(portable_context_exists(&[], Some(" summary "), &[]));
        assert!(!portable_context_exists(&[], Some("  "), &[]));
        assert_eq!(selected[0].content.text_content(), "session text");

        append_recent_switch_messages(&mut lines, selected);
        assert!(lines.iter().any(|line| line == "- User: session text"));
    }
}
