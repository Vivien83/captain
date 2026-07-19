use crate::error::KernelResult;
use captain_memory::MemorySubstrate;
use captain_runtime::agent_loop::{AgentLoopResult, LoopPhase, PhaseCallback};
use captain_runtime::kernel_handle::KernelHandle;
use captain_runtime::llm_driver::StreamEvent;
use captain_types::agent::{AgentEntry, AgentId, AgentManifest, AgentState};
use captain_types::error::CaptainError;
use captain_types::message::StopReason;
use captain_types::tool::ToolDefinition;
use std::sync::Arc;
use tracing::{info, warn};

use super::kernel_agent_runtime::STREAMING_USER_INPUT_BUFFER;
use super::kernel_memory_bridge::append_daily_memory_log;
use super::kernel_running_tasks::RunningTaskCleanup;
use super::CaptainKernel;

type AgentStreamResult = (
    tokio::sync::mpsc::Receiver<StreamEvent>,
    tokio::task::JoinHandle<KernelResult<AgentLoopResult>>,
    tokio::sync::mpsc::Sender<String>,
);

impl CaptainKernel {
    pub(super) fn static_stream_result(
        self: &Arc<Self>,
        agent_id: AgentId,
        result: AgentLoopResult,
    ) -> KernelResult<AgentStreamResult> {
        let (tx, rx, user_input_tx) = streaming_channels(8);
        let kernel_clone = Arc::clone(self);
        let run_id = uuid::Uuid::new_v4();
        let response = result.response.clone();
        let usage = result.total_usage;
        let handle = self.spawn_supervised_agent_task(agent_id, async move {
            let _running_task_cleanup =
                RunningTaskCleanup::new(Arc::clone(&kernel_clone), agent_id, run_id);
            send_complete_response(tx, response, usage).await;
            kernel_clone.scheduler.record_usage(agent_id, &usage);
            if usage.total() > 0 {
                if let Some(entry) = kernel_clone.registry.get(agent_id) {
                    kernel_clone.record_usage_metering(
                        agent_id,
                        &entry.manifest.model.provider,
                        &entry.manifest.model.model,
                        &usage,
                        result.iterations,
                    );
                }
            }
            let _ = kernel_clone
                .registry
                .set_state(agent_id, AgentState::Running);
            Ok(result)
        });
        self.track_running_task(agent_id, run_id, handle.abort_handle());
        if handle.is_finished() {
            self.clear_running_task(agent_id, run_id);
        }
        Ok((rx, handle, user_input_tx))
    }

    pub(super) fn stream_module_agent(
        self: &Arc<Self>,
        agent_id: AgentId,
        entry: AgentEntry,
        message: String,
        is_wasm: bool,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
    ) -> KernelResult<AgentStreamResult> {
        let (tx, rx, user_input_tx) = streaming_channels(64);
        let kernel_clone = Arc::clone(self);
        let run_id = uuid::Uuid::new_v4();

        let handle = self.spawn_supervised_agent_task(agent_id, async move {
            let _running_task_cleanup =
                RunningTaskCleanup::new(Arc::clone(&kernel_clone), agent_id, run_id);
            let result = if is_wasm {
                kernel_clone
                    .execute_wasm_agent(&entry, &message, kernel_handle)
                    .await
            } else {
                kernel_clone
                    .execute_python_agent(&entry, agent_id, &message)
                    .await
            };

            match result {
                Ok(result) => {
                    let usage = result.total_usage;
                    send_complete_response(tx, result.response.clone(), usage).await;
                    kernel_clone.scheduler.record_usage(agent_id, &usage);
                    let _ = kernel_clone
                        .registry
                        .set_state(agent_id, AgentState::Running);
                    Ok(result)
                }
                Err(e) => {
                    kernel_clone.supervisor.record_failure();
                    warn!(agent_id = %agent_id, error = %e, "Non-LLM agent failed with a recoverable error");
                    Err(e)
                }
            }
        });

        self.track_running_task(agent_id, run_id, handle.abort_handle());
        if handle.is_finished() {
            self.clear_running_task(agent_id, run_id);
        }

        Ok((rx, handle, user_input_tx))
    }

    pub(super) fn streaming_phase_callback(
        &self,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> PhaseCallback {
        Arc::new(move |phase| {
            let event = stream_phase_event(&phase);
            let _ = tx.try_send(event);
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn finish_streaming_llm_success(
        self: &Arc<Self>,
        agent_id: AgentId,
        memory: &Arc<MemorySubstrate>,
        session: &captain_memory::session::Session,
        messages_before: usize,
        manifest: &AgentManifest,
        tools: &[ToolDefinition],
        ctx_window: Option<usize>,
        result: &AgentLoopResult,
    ) {
        if session.messages.len() > messages_before {
            let new_messages = session.messages[messages_before..].to_vec();
            if let Err(e) = memory.append_canonical(agent_id, &new_messages, None) {
                warn!(agent_id = %agent_id, "Failed to update canonical session (streaming): {e}");
            }
        }

        if let Some(ref workspace) = manifest.workspace {
            if let Err(e) = memory.write_jsonl_mirror(session, &workspace.join("sessions")) {
                warn!("Failed to write JSONL session mirror (streaming): {e}");
            }
            append_daily_memory_log(workspace, &result.response);
        }

        self.scheduler.record_usage(agent_id, &result.total_usage);
        self.record_usage_metering(
            agent_id,
            &manifest.model.provider,
            &manifest.model.model,
            &result.total_usage,
            result.iterations,
        );

        let _ = self.registry.set_state(agent_id, AgentState::Running);

        if streaming_post_loop_compaction_needed(session, manifest, tools, ctx_window) {
            let estimated = captain_runtime::compactor::estimate_token_count(
                &session.messages,
                Some(&manifest.model.system_prompt),
                Some(tools),
            );
            let kernel = Arc::clone(self);
            tokio::spawn(async move {
                info!(agent_id = %agent_id, estimated_tokens = estimated, "Post-loop compaction triggered");
                if let Err(e) = kernel.compact_agent_session(agent_id).await {
                    warn!(agent_id = %agent_id, "Post-loop compaction failed: {e}");
                }
            });
        }
    }

    pub(super) fn record_streaming_llm_failure(&self, agent_id: AgentId, error: &CaptainError) {
        self.supervisor.record_failure();
        warn!(agent_id = %agent_id, error = %error, "Streaming agent loop failed with a recoverable error");
    }
}

fn stream_phase_event(phase: &LoopPhase) -> StreamEvent {
    let (phase, detail) = match phase {
        LoopPhase::Thinking => ("thinking".to_string(), None),
        LoopPhase::ToolUse { tool_name } => ("tool_use".to_string(), Some(tool_name.clone())),
        LoopPhase::Streaming => ("streaming".to_string(), None),
        LoopPhase::Done => ("done".to_string(), None),
        LoopPhase::Error => ("error".to_string(), None),
    };
    StreamEvent::PhaseChange { phase, detail }
}

fn streaming_post_loop_compaction_needed(
    session: &captain_memory::session::Session,
    manifest: &AgentManifest,
    tools: &[ToolDefinition],
    ctx_window: Option<usize>,
) -> bool {
    let config = super::kernel_agent_runtime::compaction_config_for_manifest(manifest, ctx_window);
    let estimated = captain_runtime::compactor::estimate_token_count(
        &session.messages,
        Some(&manifest.model.system_prompt),
        Some(tools),
    );
    captain_runtime::compactor::needs_compaction_by_tokens(estimated, &config)
}

fn streaming_channels(
    event_capacity: usize,
) -> (
    tokio::sync::mpsc::Sender<StreamEvent>,
    tokio::sync::mpsc::Receiver<StreamEvent>,
    tokio::sync::mpsc::Sender<String>,
) {
    let (tx, rx) = tokio::sync::mpsc::channel::<StreamEvent>(event_capacity);
    let (user_input_tx, _user_input_rx) =
        tokio::sync::mpsc::channel::<String>(STREAMING_USER_INPUT_BUFFER);
    (tx, rx, user_input_tx)
}

async fn send_complete_response(
    tx: tokio::sync::mpsc::Sender<StreamEvent>,
    response: String,
    usage: captain_types::message::TokenUsage,
) {
    let _ = tx.send(StreamEvent::TextDelta { text: response }).await;
    let _ = tx
        .send(StreamEvent::ContentComplete {
            stop_reason: StopReason::EndTurn,
            usage,
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::CaptainKernel;
    use captain_memory::session::Session;
    use captain_runtime::agent_loop::AgentLoopResult;
    use captain_types::config::KernelConfig;
    use captain_types::message::{Message, ReplyDirectives, TokenUsage};

    fn principal_agent_id(kernel: &CaptainKernel) -> AgentId {
        kernel
            .registry
            .list()
            .into_iter()
            .find(|entry| entry.name == "captain")
            .or_else(|| kernel.registry.list().into_iter().next())
            .expect("kernel should boot with at least one agent")
            .id
    }

    fn test_result() -> AgentLoopResult {
        AgentLoopResult {
            response: "stream ok".to_string(),
            total_usage: TokenUsage {
                input_tokens: 4,
                output_tokens: 7,
                ..Default::default()
            },
            iterations: 1,
            cost_usd: Some(0.0),
            silent: false,
            directives: ReplyDirectives::default(),
            tool_calls: Vec::new(),
        }
    }

    #[tokio::test]
    async fn static_stream_result_emits_response_completion_and_records_usage() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("static-stream");
        let kernel = Arc::new(
            CaptainKernel::boot_with_config(KernelConfig {
                home_dir: home_dir.clone(),
                data_dir: home_dir.join("data"),
                ..KernelConfig::default()
            })
            .expect("kernel boot"),
        );
        let agent_id = principal_agent_id(&kernel);

        let (mut rx, handle, _user_input_tx) = kernel
            .static_stream_result(agent_id, test_result())
            .expect("static stream result");

        match rx.recv().await.expect("text delta") {
            StreamEvent::TextDelta { text } => assert_eq!(text, "stream ok"),
            other => panic!("unexpected first event: {other:?}"),
        }
        match rx.recv().await.expect("completion") {
            StreamEvent::ContentComplete { stop_reason, usage } => {
                assert_eq!(stop_reason, StopReason::EndTurn);
                assert_eq!(usage.input_tokens, 4);
                assert_eq!(usage.output_tokens, 7);
            }
            other => panic!("unexpected second event: {other:?}"),
        }

        let result = handle
            .await
            .expect("join static stream")
            .expect("static stream ok");
        assert_eq!(result.response, "stream ok");
        let (tokens, _) = kernel
            .scheduler
            .get_usage(agent_id)
            .expect("scheduler usage");
        assert_eq!(tokens, 11);
        assert!(matches!(
            kernel.registry.get(agent_id).map(|entry| entry.state),
            Some(AgentState::Running)
        ));

        kernel.shutdown();
    }

    #[test]
    fn stream_phase_event_maps_loop_phases_to_wire_events() {
        match stream_phase_event(&LoopPhase::Thinking) {
            StreamEvent::PhaseChange { phase, detail } => {
                assert_eq!(phase, "thinking");
                assert!(detail.is_none());
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match stream_phase_event(&LoopPhase::ToolUse {
            tool_name: "shell_exec".to_string(),
        }) {
            StreamEvent::PhaseChange { phase, detail } => {
                assert_eq!(phase, "tool_use");
                assert_eq!(detail.as_deref(), Some("shell_exec"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn streaming_post_loop_compaction_respects_context_window() {
        let mut manifest = captain_types::agent::AgentManifest::default();
        manifest.model.provider = "openai".to_string();
        manifest.model.system_prompt = "system".to_string();
        let session = Session {
            id: captain_types::agent::SessionId::new(),
            agent_id: AgentId::new(),
            messages: vec![Message::user("large ".repeat(12_000))],
            context_window_tokens: 1_000,
            label: None,
        };

        assert!(streaming_post_loop_compaction_needed(
            &session,
            &manifest,
            &[],
            Some(1_000),
        ));
        assert!(!streaming_post_loop_compaction_needed(
            &session,
            &manifest,
            &[],
            Some(200_000),
        ));
    }
}
