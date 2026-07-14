use super::CaptainKernel;
use crate::error::{KernelError, KernelResult};
use crate::metering::MeteringEngine;
use captain_memory::usage::UsageRecord;
use captain_types::agent::AgentId;
use captain_types::error::CaptainError;
use captain_types::message::{Role, TokenUsage};

impl CaptainKernel {
    pub(super) fn estimate_usage_cost(
        &self,
        provider: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) -> f64 {
        let pricing_model = MeteringEngine::catalog_pricing_model_id(provider, model);
        MeteringEngine::estimate_cost_with_catalog(
            &self.model_catalog.read().unwrap_or_else(|e| e.into_inner()),
            &pricing_model,
            input_tokens,
            output_tokens,
        )
    }

    pub(super) fn record_usage_metering(
        &self,
        agent_id: AgentId,
        provider: &str,
        model: &str,
        usage: &TokenUsage,
        iterations: u32,
    ) -> f64 {
        let cost =
            self.estimate_usage_cost(provider, model, usage.input_tokens, usage.output_tokens);
        let _ = self.metering.record(&UsageRecord {
            agent_id,
            model: model.to_string(),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cached_input_tokens: usage.cached_input_tokens,
            cache_creation_tokens: usage.cache_creation_tokens,
            cost_usd: cost,
            tool_calls: iterations.saturating_sub(1),
        });
        cost
    }

    /// Get session token usage and estimated cost for an agent.
    pub fn session_usage_cost(&self, agent_id: AgentId) -> KernelResult<(u64, u64, f64)> {
        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Captain(CaptainError::AgentNotFound(agent_id.to_string()))
        })?;

        let session = self
            .memory
            .get_session(entry.session_id)
            .map_err(KernelError::Captain)?;

        let (input_tokens, output_tokens) = session
            .map(|s| {
                let mut input = 0u64;
                let mut output = 0u64;
                // Keep this rough estimator aligned with Hermes: one token per four chars.
                for msg in &s.messages {
                    let tokens = msg.content.text_content().len() as u64 / 4;
                    match msg.role {
                        Role::User | Role::System => input += tokens,
                        Role::Assistant => output += tokens,
                    }
                }
                (input, output)
            })
            .unwrap_or((0, 0));

        let cost = self.estimate_usage_cost(
            &entry.manifest.model.provider,
            &entry.manifest.model.model,
            input_tokens,
            output_tokens,
        );

        Ok((input_tokens, output_tokens, cost))
    }
}

#[cfg(test)]
mod tests {
    use crate::kernel::CaptainKernel;
    use captain_memory::session::Session;
    use captain_types::config::KernelConfig;
    use captain_types::message::Message;
    use std::collections::HashMap;

    #[test]
    fn session_usage_cost_counts_persisted_session_tokens() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("captain-kernel-usage-runtime-test");
        std::fs::create_dir_all(&home_dir).unwrap();

        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };

        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");
        let instance = kernel
            .activate_hand("browser", HashMap::new())
            .expect("browser hand activates");
        let agent_id = instance.agent_id.expect("agent id present");
        let entry = kernel.registry.get(agent_id).expect("agent entry");

        kernel
            .memory
            .save_session(&Session {
                id: entry.session_id,
                agent_id,
                messages: vec![
                    Message::system("ssssssss"),
                    Message::user("uuuuuuuuuuuu"),
                    Message::assistant("aaaaaaaaaaaaaaaa"),
                ],
                context_window_tokens: 0,
                label: None,
            })
            .expect("session saved");

        let (input_tokens, output_tokens, cost) = kernel
            .session_usage_cost(agent_id)
            .expect("session usage cost");

        assert_eq!(input_tokens, 5);
        assert_eq!(output_tokens, 4);
        assert!(cost >= 0.0);

        kernel.shutdown();
    }
}
