use crate::capability_routing::ensure_active_model_supports;
use crate::error::{KernelError, KernelResult};
use captain_runtime::kernel_handle::KernelHandle;
use captain_types::agent::{AgentEntry, AgentId, AgentManifest, SessionId};
use captain_types::message::ContentBlock;
use std::sync::Arc;

use super::kernel_llm_turn::{StreamingLlmTurnRequest, StreamingLlmTurnResult};
use super::CaptainKernel;

struct StreamingMessageRequest<'a> {
    message: &'a str,
    kernel_handle: Option<Arc<dyn KernelHandle>>,
    sender_id: Option<String>,
    sender_name: Option<String>,
    content_blocks: Option<Vec<ContentBlock>>,
    channel_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamingModuleKind {
    Wasm,
    Python,
}

impl CaptainKernel {
    /// Send a message to an agent with streaming responses.
    ///
    /// Returns a receiver for incremental `StreamEvent`s and a `JoinHandle`
    /// that resolves to the final `AgentLoopResult`. The caller reads stream
    /// events while the agent loop runs, then awaits the handle for final stats.
    ///
    /// WASM and Python agents don't support true streaming — they execute
    /// synchronously and emit a single `TextDelta` + `ContentComplete` pair.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    pub fn send_message_streaming(
        self: &Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
        sender_id: Option<String>,
        sender_name: Option<String>,
        content_blocks: Option<Vec<ContentBlock>>,
        channel_type: Option<String>,
    ) -> KernelResult<StreamingLlmTurnResult> {
        self.send_message_streaming_in_session(
            agent_id,
            message,
            kernel_handle,
            sender_id,
            sender_name,
            content_blocks,
            channel_type,
            None,
        )
    }

    /// Stream a message against a persisted session without switching the
    /// agent's registry entry. Independent clients can therefore reopen and
    /// continue different conversations safely.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    pub fn send_message_streaming_in_session(
        self: &Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
        sender_id: Option<String>,
        sender_name: Option<String>,
        content_blocks: Option<Vec<ContentBlock>>,
        channel_type: Option<String>,
        session_id: Option<SessionId>,
    ) -> KernelResult<StreamingLlmTurnResult> {
        let request = StreamingMessageRequest {
            message,
            kernel_handle: Some(self.resolve_streaming_kernel_handle(kernel_handle)),
            sender_id,
            sender_name,
            content_blocks,
            channel_type,
        };
        let entry = self.resolve_agent_session_entry(agent_id, session_id)?;

        if let Some(result) = self.maybe_handle_first_use_onboarding(
            &entry,
            message,
            request.channel_type.as_deref(),
        )? {
            return self.static_stream_result(agent_id, result);
        }

        // Enforce quota before spawning the streaming task.
        self.scheduler
            .check_quota(agent_id)
            .map_err(KernelError::Captain)?;

        if let Some(response) =
            self.consume_codex_model_update_keep_request(agent_id, request.message)?
        {
            return self.static_stream_result(agent_id, Self::empty_agent_loop_result(response));
        }

        if let Some(result) = self.consume_pending_model_switch_choice(agent_id, request.message)? {
            return self.static_stream_result(agent_id, result);
        }

        if let Some(result) = self.handle_direct_model_switch_request(agent_id, request.message)? {
            return self.static_stream_result(agent_id, result);
        }

        self.validate_streaming_capabilities(&entry, request.content_blocks.as_deref())?;

        if let Some(module_kind) = streaming_module_kind(&entry.manifest) {
            return self.stream_module_agent(
                agent_id,
                entry.clone(),
                request.message.to_string(),
                matches!(module_kind, StreamingModuleKind::Wasm),
                request.kernel_handle,
            );
        }

        self.start_streaming_llm_turn(StreamingLlmTurnRequest {
            agent_id,
            entry: &entry,
            message: request.message,
            kernel_handle: request.kernel_handle,
            sender_id: request.sender_id,
            sender_name: request.sender_name,
            content_blocks: request.content_blocks,
            channel_type: request.channel_type,
        })
    }

    fn resolve_streaming_kernel_handle(
        self: &Arc<Self>,
        explicit: Option<Arc<dyn KernelHandle>>,
    ) -> Arc<dyn KernelHandle> {
        explicit.unwrap_or_else(|| self.clone() as Arc<dyn KernelHandle>)
    }

    fn validate_streaming_capabilities(
        &self,
        entry: &AgentEntry,
        content_blocks: Option<&[ContentBlock]>,
    ) -> KernelResult<()> {
        let catalog = self.model_catalog.read().unwrap_or_else(|e| e.into_inner());
        ensure_active_model_supports(
            &catalog,
            &entry.manifest.model.provider,
            &entry.manifest.model.model,
            content_blocks,
        )
        .map_err(KernelError::Captain)
    }
}

fn streaming_module_kind(manifest: &AgentManifest) -> Option<StreamingModuleKind> {
    if manifest.module.starts_with("wasm:") {
        Some(StreamingModuleKind::Wasm)
    } else if manifest.module.starts_with("python:") {
        Some(StreamingModuleKind::Python)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::config::KernelConfig;

    #[tokio::test]
    async fn streaming_kernel_handle_defaults_to_self_and_preserves_explicit_override() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home_dir = tmp.path().join("streaming-kernel-handle");
        let kernel = Arc::new(
            CaptainKernel::boot_with_config(KernelConfig {
                home_dir: home_dir.clone(),
                data_dir: home_dir.join("data"),
                ..KernelConfig::default()
            })
            .expect("kernel boot"),
        );
        kernel.set_self_handle();

        let fallback = kernel.resolve_streaming_kernel_handle(None);
        assert!(
            fallback
                .list_agents()
                .iter()
                .any(|agent| agent.name == "captain"),
            "an in-process streaming turn must receive the live kernel handle"
        );

        let explicit: Arc<dyn KernelHandle> = kernel.clone();
        let resolved = kernel.resolve_streaming_kernel_handle(Some(explicit.clone()));
        assert!(
            Arc::ptr_eq(&explicit, &resolved),
            "an explicit bridge handle must remain authoritative"
        );

        kernel.shutdown();
    }

    #[test]
    fn streaming_module_kind_detects_non_llm_modules_only() {
        let mut manifest = AgentManifest {
            module: "wasm:agent.wasm".to_string(),
            ..Default::default()
        };
        assert_eq!(
            streaming_module_kind(&manifest),
            Some(StreamingModuleKind::Wasm)
        );

        manifest.module = "python:agent.py".to_string();
        assert_eq!(
            streaming_module_kind(&manifest),
            Some(StreamingModuleKind::Python)
        );

        manifest.module = "llm".to_string();
        assert_eq!(streaming_module_kind(&manifest), None);
    }
}
