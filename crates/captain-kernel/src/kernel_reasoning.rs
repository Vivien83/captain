//! Durable reasoning-effort selection for agent models.

use crate::error::{KernelError, KernelResult};
use captain_types::agent::AgentId;
use captain_types::error::CaptainError;
use captain_types::reasoning::{
    AgentReasoningStatus, ModelReasoningCapabilities, ReasoningEffort, ReasoningSelectionSource,
};

use super::CaptainKernel;

impl CaptainKernel {
    /// Return the exact configured and effective reasoning state for an agent.
    pub fn agent_reasoning_status(&self, agent_id: AgentId) -> KernelResult<AgentReasoningStatus> {
        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Captain(CaptainError::AgentNotFound(agent_id.to_string()))
        })?;
        Ok(reasoning_status_for_model(
            &entry.manifest.model.provider,
            &entry.manifest.model.model,
            entry.manifest.model.reasoning_effort.clone(),
        ))
    }

    /// Set or clear a durable agent reasoning override.
    ///
    /// `None` and `auto` both restore the selected model's provider-owned
    /// default. Explicit values are accepted only when the live model catalog
    /// advertises them (or the conservative offline Codex fallback does).
    pub fn set_agent_reasoning_effort(
        &self,
        agent_id: AgentId,
        requested: Option<&str>,
    ) -> KernelResult<AgentReasoningStatus> {
        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Captain(CaptainError::AgentNotFound(agent_id.to_string()))
        })?;
        let requested = requested
            .map(str::trim)
            .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("auto"))
            .map(str::parse::<ReasoningEffort>)
            .transpose()
            .map_err(|error| KernelError::Captain(CaptainError::Config(error)))?;

        let capabilities =
            reasoning_capabilities(&entry.manifest.model.provider, &entry.manifest.model.model);
        if let Some(effort) = requested.as_ref() {
            let Some(capabilities) = capabilities.as_ref() else {
                return Err(KernelError::Captain(CaptainError::Config(format!(
                    "Model {}/{} does not advertise configurable reasoning",
                    entry.manifest.model.provider, entry.manifest.model.model
                ))));
            };
            if !capabilities.supports(effort) {
                let supported = capabilities
                    .supported_efforts
                    .iter()
                    .map(|option| option.effort.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(KernelError::Captain(CaptainError::Config(format!(
                    "Reasoning effort '{effort}' is not supported by {}/{}; available: {}",
                    entry.manifest.model.provider,
                    entry.manifest.model.model,
                    if supported.is_empty() {
                        "provider default only"
                    } else {
                        supported.as_str()
                    }
                ))));
            }
        }

        self.registry
            .update_reasoning_effort(agent_id, requested)
            .map_err(KernelError::Captain)?;
        if let Some(updated) = self.registry.get(agent_id) {
            self.memory
                .save_agent(&updated)
                .map_err(KernelError::Captain)?;
        }
        self.agent_reasoning_status(agent_id)
    }
}

fn reasoning_capabilities(provider: &str, model: &str) -> Option<ModelReasoningCapabilities> {
    if !matches!(
        provider.to_ascii_lowercase().as_str(),
        "codex" | "openai-codex"
    ) {
        return None;
    }
    captain_runtime::model_catalog::codex_reasoning_capabilities(model)
}

fn reasoning_status_for_model(
    provider: &str,
    model: &str,
    configured: Option<ReasoningEffort>,
) -> AgentReasoningStatus {
    let capabilities = reasoning_capabilities(provider, model);
    let Some(capabilities) = capabilities else {
        return AgentReasoningStatus {
            provider: provider.to_string(),
            model: model.to_string(),
            supported: false,
            configured_effort: configured.clone(),
            effective_effort: None,
            source: ReasoningSelectionSource::Unsupported,
            override_valid: configured.is_none(),
            options: Vec::new(),
            reported_by_provider: false,
        };
    };

    let configured_valid = configured
        .as_ref()
        .map(|effort| capabilities.supports(effort))
        .unwrap_or(true);
    let effective = configured
        .as_ref()
        .filter(|_| configured_valid)
        .cloned()
        .or_else(|| capabilities.default_effort.clone());
    let source = if configured.is_some() && configured_valid {
        ReasoningSelectionSource::AgentOverride
    } else if capabilities.default_effort.is_some() {
        ReasoningSelectionSource::ModelDefault
    } else {
        ReasoningSelectionSource::ProviderDefault
    };

    AgentReasoningStatus {
        provider: provider.to_string(),
        model: model.to_string(),
        supported: !capabilities.supported_efforts.is_empty(),
        configured_effort: configured,
        effective_effort: effective,
        source,
        override_valid: configured_valid,
        options: capabilities.supported_efforts,
        reported_by_provider: capabilities.reported_by_provider,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsupported_provider_never_claims_a_reasoning_level() {
        let status = reasoning_status_for_model("ollama", "llama3", None);

        assert!(!status.supported);
        assert!(status.effective_effort.is_none());
        assert_eq!(status.source, ReasoningSelectionSource::Unsupported);
    }

    #[test]
    fn offline_codex_fallback_is_honest_about_provider_provenance() {
        let status = reasoning_status_for_model("codex", "gpt-5-test", None);

        assert!(status.supported);
        assert!(!status.reported_by_provider);
        assert_eq!(status.source, ReasoningSelectionSource::ProviderDefault);
        assert!(status.effective_effort.is_none());
        assert!(status
            .options
            .iter()
            .any(|option| option.effort.as_str() == "high"));
    }
}
