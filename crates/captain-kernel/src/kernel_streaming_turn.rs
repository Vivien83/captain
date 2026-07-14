use crate::capability_routing::{decide_routing, RoutingDecision};
use crate::error::{KernelError, KernelResult};
use captain_runtime::kernel_handle::KernelHandle;
use captain_types::agent::{AgentEntry, AgentId, AgentManifest, SessionId};
use captain_types::error::CaptainError;
use captain_types::message::ContentBlock;
use std::sync::Arc;
use tracing::info;

use super::kernel_llm_turn::{StreamingLlmTurnRequest, StreamingLlmTurnResult};
use super::CaptainKernel;

#[derive(Clone)]
struct StreamingMessageRequest<'a> {
    agent_id: AgentId,
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
            agent_id,
            message,
            kernel_handle,
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

        if let Some(result) = self.route_streaming_to_specialist(&entry, request.clone())? {
            return Ok(result);
        }

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

    fn route_streaming_to_specialist(
        self: &Arc<Self>,
        entry: &AgentEntry,
        request: StreamingMessageRequest<'_>,
    ) -> KernelResult<Option<StreamingLlmTurnResult>> {
        let catalog = self.model_catalog.read().unwrap_or_else(|e| e.into_inner());
        let decision = decide_routing(
            &self.registry,
            &catalog,
            request.agent_id,
            &entry.manifest.model.model,
            request.content_blocks.as_deref(),
        );
        drop(catalog);

        match decision {
            RoutingDecision::Proceed | RoutingDecision::NoCandidateAvailable(_) => Ok(None),
            RoutingDecision::DelegateTo(target_id) => {
                info!(agent_id = %request.agent_id, target = %target_id, "Streaming: redirect to specialist");
                self.send_message_streaming(
                    target_id,
                    request.message,
                    request.kernel_handle,
                    request.sender_id,
                    request.sender_name,
                    request.content_blocks,
                    request.channel_type,
                )
                .map(Some)
            }
            RoutingDecision::SpawnAndDelegate { manifest_toml, .. } => {
                let manifest: AgentManifest = toml::from_str(&manifest_toml).map_err(|e| {
                    KernelError::Captain(CaptainError::ManifestParse(format!("Vision agent: {e}")))
                })?;
                let new_id = self.spawn_agent(manifest)?;
                self.send_message_streaming(
                    new_id,
                    request.message,
                    request.kernel_handle,
                    request.sender_id,
                    request.sender_name,
                    request.content_blocks,
                    request.channel_type,
                )
                .map(Some)
            }
        }
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
