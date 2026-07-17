use crate::error::{KernelError, KernelResult};
use captain_memory::session::Session;
use captain_runtime::agent_loop::AgentLoopResult;
use captain_runtime::kernel_handle::KernelHandle;
use captain_runtime::llm_driver::{LlmDriver, StreamEvent};
use captain_types::agent::{AgentEntry, AgentId, AgentManifest, AgentMode, SessionId};
use captain_types::message::ContentBlock;
use captain_types::tool::ToolDefinition;
use std::sync::Arc;
use tracing::{info, warn};

use super::kernel_agent_runtime::{
    apply_agent_loop_config, context_window_for_model, is_lean_direct_turn,
    DEFAULT_CONTEXT_WINDOW_TOKENS,
};
use super::kernel_agent_workspace::ensure_workspace;
use super::kernel_llm_launch::{NonStreamingLlmLoopRequest, StreamingLlmLoopRequest};
use super::kernel_llm_prompt::LlmPromptRequest;
use super::kernel_llm_runtime::LlmPreLoopCompactionStage;
use super::CaptainKernel;

pub(super) type StreamingLlmTurnResult = (
    tokio::sync::mpsc::Receiver<StreamEvent>,
    tokio::task::JoinHandle<KernelResult<AgentLoopResult>>,
    tokio::sync::mpsc::Sender<String>,
);

pub(super) struct StreamingLlmTurnRequest<'a> {
    pub(super) agent_id: AgentId,
    pub(super) entry: &'a AgentEntry,
    pub(super) message: &'a str,
    pub(super) kernel_handle: Option<Arc<dyn KernelHandle>>,
    pub(super) sender_id: Option<String>,
    pub(super) sender_name: Option<String>,
    pub(super) content_blocks: Option<Vec<ContentBlock>>,
    pub(super) channel_type: Option<String>,
}

struct NonStreamingLlmTurnRequest<'a> {
    entry: &'a AgentEntry,
    agent_id: AgentId,
    message: &'a str,
    kernel_handle: Option<Arc<dyn KernelHandle>>,
    content_blocks: Option<Vec<ContentBlock>>,
    sender_id: Option<String>,
    sender_name: Option<String>,
    channel_type: Option<String>,
}

pub(super) struct LlmTurnBasics {
    pub session: Session,
    pub context_window: usize,
    pub lean_direct: bool,
    pub tools: Vec<ToolDefinition>,
}

struct PreparedNonStreamingLlmTurn {
    session: Session,
    manifest: AgentManifest,
    tools: Vec<ToolDefinition>,
    driver: Arc<dyn LlmDriver>,
    lean_direct: bool,
    messages_before: usize,
    ctx_window: Option<usize>,
}

impl CaptainKernel {
    pub(super) fn start_streaming_llm_turn(
        self: &Arc<Self>,
        request: StreamingLlmTurnRequest<'_>,
    ) -> KernelResult<StreamingLlmTurnResult> {
        let StreamingLlmTurnRequest {
            agent_id,
            entry,
            message,
            kernel_handle,
            sender_id,
            sender_name,
            content_blocks,
            channel_type,
        } = request;

        let prepared =
            self.prepare_llm_turn_basics(agent_id, entry, message, content_blocks.is_some())?;
        let session = prepared.session;
        let effective_ctx_window = prepared.context_window;
        let lean_direct = prepared.lean_direct;
        let tools = prepared.tools;
        let pre_loop_compaction = self.plan_llm_session_compaction_before_loop(
            agent_id,
            &session,
            &entry.manifest,
            &tools,
            effective_ctx_window,
        );

        let driver = self.resolve_driver(&entry.manifest)?;

        let ctx_window = Some(effective_ctx_window);

        let mut manifest = entry.manifest.clone();
        self.prepare_llm_manifest_for_prompt(agent_id, &mut manifest, true);

        self.prepare_llm_prompt(LlmPromptRequest {
            agent_id,
            message,
            manifest: &mut manifest,
            session: &session,
            tools: &tools,
            lean_direct,
            sender_id,
            sender_name,
            channel_type: channel_type.clone(),
            include_graph_recall: true,
        });

        self.stream_llm_agent_loop(StreamingLlmLoopRequest {
            agent_id,
            message: message.to_string(),
            session,
            manifest,
            tools,
            driver,
            kernel_handle,
            lean_direct,
            effective_ctx_window,
            ctx_window,
            pre_loop_compaction,
            content_blocks,
            channel_type,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_llm_agent(
        &self,
        entry: &AgentEntry,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
        content_blocks: Option<Vec<ContentBlock>>,
        sender_id: Option<String>,
        sender_name: Option<String>,
        channel_type: Option<String>,
    ) -> KernelResult<AgentLoopResult> {
        let request = NonStreamingLlmTurnRequest {
            entry,
            agent_id,
            message,
            kernel_handle,
            content_blocks,
            sender_id,
            sender_name,
            channel_type,
        };

        self.check_non_streaming_llm_quota(&request)?;
        let mut prepared = self.prepare_non_streaming_llm_turn(&request).await?;
        let result = self
            .run_prepared_non_streaming_llm_turn(request, &mut prepared)
            .await?;

        Ok(self.finish_non_streaming_llm_success(
            agent_id,
            &prepared.session,
            prepared.messages_before,
            &prepared.manifest,
            result,
        ))
    }

    fn check_non_streaming_llm_quota(
        &self,
        request: &NonStreamingLlmTurnRequest<'_>,
    ) -> KernelResult<()> {
        self.metering
            .check_quota(request.agent_id, &request.entry.manifest.resources)
            .map_err(KernelError::Captain)
    }

    async fn prepare_non_streaming_llm_turn(
        &self,
        request: &NonStreamingLlmTurnRequest<'_>,
    ) -> KernelResult<PreparedNonStreamingLlmTurn> {
        let prepared = self.prepare_llm_turn_basics(
            request.agent_id,
            request.entry,
            request.message,
            request.content_blocks.is_some(),
        )?;
        let mut session = prepared.session;
        let initial_ctx_window = prepared.context_window;
        let lean_direct = prepared.lean_direct;
        let tools = prepared.tools;
        self.compact_llm_session_before_loop(
            request.agent_id,
            &mut session,
            &request.entry.manifest,
            &tools,
            initial_ctx_window,
            LlmPreLoopCompactionStage::NonStreamingInitial,
        )
        .await;

        let messages_before = session.messages.len();
        log_llm_tools_selected(request.entry, request.agent_id, &tools, lean_direct);

        let manifest =
            self.prepare_non_streaming_manifest_for_loop(request, &session, &tools, lean_direct);
        let driver = self.resolve_driver(&manifest)?;
        let effective_ctx_window =
            self.prepare_non_streaming_effective_context(&mut session, &manifest);

        self.compact_llm_session_before_loop(
            request.agent_id,
            &mut session,
            &manifest,
            &tools,
            effective_ctx_window,
            LlmPreLoopCompactionStage::NonStreamingFinal,
        )
        .await;

        Ok(PreparedNonStreamingLlmTurn {
            session,
            manifest,
            tools,
            driver,
            lean_direct,
            messages_before,
            ctx_window: Some(effective_ctx_window),
        })
    }

    fn prepare_non_streaming_manifest_for_loop(
        &self,
        request: &NonStreamingLlmTurnRequest<'_>,
        session: &Session,
        tools: &[ToolDefinition],
        lean_direct: bool,
    ) -> AgentManifest {
        let mut manifest = request.entry.manifest.clone();
        self.prepare_llm_manifest_for_prompt(request.agent_id, &mut manifest, false);

        self.prepare_llm_prompt(LlmPromptRequest {
            agent_id: request.agent_id,
            message: request.message,
            manifest: &mut manifest,
            session,
            tools,
            lean_direct,
            sender_id: request.sender_id.clone(),
            sender_name: request.sender_name.clone(),
            channel_type: request.channel_type.clone(),
            include_graph_recall: true,
        });
        manifest
    }

    fn prepare_non_streaming_effective_context(
        &self,
        session: &mut Session,
        manifest: &AgentManifest,
    ) -> usize {
        let effective_ctx_window = self.context_window_for_llm_manifest(manifest);
        session.context_window_tokens = effective_ctx_window as u64;
        effective_ctx_window
    }

    async fn run_prepared_non_streaming_llm_turn(
        &self,
        request: NonStreamingLlmTurnRequest<'_>,
        prepared: &mut PreparedNonStreamingLlmTurn,
    ) -> KernelResult<AgentLoopResult> {
        self.run_non_streaming_llm_loop(NonStreamingLlmLoopRequest {
            agent_id: request.agent_id,
            message: request.message,
            session: &mut prepared.session,
            manifest: &mut prepared.manifest,
            tools: &prepared.tools,
            driver: Arc::clone(&prepared.driver),
            kernel_handle: request.kernel_handle,
            lean_direct: prepared.lean_direct,
            ctx_window: prepared.ctx_window,
            content_blocks: request.content_blocks,
            channel_type: request.channel_type,
        })
        .await
    }

    pub(super) fn prepare_llm_turn_basics(
        &self,
        agent_id: AgentId,
        entry: &AgentEntry,
        message: &str,
        has_content_blocks: bool,
    ) -> KernelResult<LlmTurnBasics> {
        let mut session = self
            .memory
            .get_session(entry.session_id)
            .map_err(KernelError::Captain)?
            .unwrap_or_else(|| default_llm_session(entry.session_id, agent_id));
        super::kernel_task_checkpoint_recovery::maybe_reinject_task_checkpoint(
            self,
            agent_id,
            &mut session,
        );
        let context_window = self.context_window_for_llm_manifest(&entry.manifest);
        session.context_window_tokens = context_window as u64;

        let lean_direct = !has_content_blocks && is_lean_direct_turn(message);
        let tools =
            select_llm_tools_for_turn(entry.mode, self.available_tools(agent_id), lean_direct);

        Ok(LlmTurnBasics {
            session,
            context_window,
            lean_direct,
            tools,
        })
    }

    pub(super) fn context_window_for_llm_manifest(&self, manifest: &AgentManifest) -> usize {
        self.model_catalog
            .read()
            .ok()
            .and_then(|cat| {
                context_window_for_model(&cat, &manifest.model.provider, &manifest.model.model)
            })
            .unwrap_or(DEFAULT_CONTEXT_WINDOW_TOKENS)
    }

    /// Effective context window used by the next turn for this agent.
    ///
    /// This reads the live catalog on every call, so provider catalog refreshes
    /// and explicit model switches are visible without restarting Captain.
    pub fn effective_context_window_for_agent(&self, agent_id: AgentId) -> Option<usize> {
        self.registry
            .get(agent_id)
            .map(|entry| self.context_window_for_llm_manifest(&entry.manifest))
    }

    pub(super) fn prepare_llm_manifest_for_prompt(
        &self,
        agent_id: AgentId,
        manifest: &mut AgentManifest,
        streaming: bool,
    ) {
        if manifest.workspace.is_none() {
            let workspace_dir = self.config.effective_workspaces_dir().join(&manifest.name);
            if let Err(e) = ensure_workspace(&workspace_dir) {
                if streaming {
                    warn!(agent_id = %agent_id, "Failed to backfill workspace (streaming): {e}");
                } else {
                    warn!(agent_id = %agent_id, "Failed to backfill workspace: {e}");
                }
            } else {
                manifest.workspace = Some(workspace_dir);
                let _ = self
                    .registry
                    .update_workspace(agent_id, manifest.workspace.clone());
            }
        }
        apply_agent_loop_config(manifest, &self.config);
    }
}

fn default_llm_session(session_id: SessionId, agent_id: AgentId) -> Session {
    Session {
        id: session_id,
        agent_id,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
    }
}

fn select_llm_tools_for_turn(
    mode: AgentMode,
    tools: Vec<ToolDefinition>,
    lean_direct: bool,
) -> Vec<ToolDefinition> {
    if lean_direct {
        Vec::new()
    } else {
        mode.filter_tools(tools)
    }
}

fn log_llm_tools_selected(
    entry: &AgentEntry,
    agent_id: AgentId,
    tools: &[ToolDefinition],
    lean_direct: bool,
) {
    // TS.2/CR.1 - Tool RAG removed; CORE filter inside available_tools()
    // now pins the visible builtin surface. The LLM discovers
    // missing capabilities via `capability_search`, then exact
    // non-CORE builtin schemas via `tool_search`.
    info!(
        agent = %entry.name,
        agent_id = %agent_id,
        tool_count = tools.len(),
        tool_names = ?tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        lean_direct,
        "Tools selected for LLM request (CORE filter)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: String::new(),
            input_schema: serde_json::json!({}),
        }
    }

    #[test]
    fn default_llm_session_starts_empty_for_agent_session_pair() {
        let agent_id = AgentId::new();
        let session_id = SessionId::new();
        let session = default_llm_session(session_id, agent_id);

        assert_eq!(session.id, session_id);
        assert_eq!(session.agent_id, agent_id);
        assert!(session.messages.is_empty());
        assert_eq!(session.context_window_tokens, 0);
        assert!(session.label.is_none());
    }

    #[test]
    fn select_llm_tools_honors_mode_before_full_prompt() {
        let tools = vec![tool("file_read"), tool("shell"), tool("web_search")];
        let selected = select_llm_tools_for_turn(AgentMode::Assist, tools, false);

        let names: Vec<_> = selected.into_iter().map(|tool| tool.name).collect();
        assert_eq!(names, vec!["file_read", "web_search"]);
    }

    #[test]
    fn select_llm_tools_clears_all_tools_for_lean_direct_turn() {
        let tools = vec![tool("file_read"), tool("web_search")];

        assert!(select_llm_tools_for_turn(AgentMode::Full, tools, true).is_empty());
    }

    #[test]
    fn select_llm_tools_keeps_full_surface_for_non_lean_full_mode() {
        let tools = vec![tool("file_read"), tool("shell"), tool("web_search")];

        let names: Vec<_> = select_llm_tools_for_turn(AgentMode::Full, tools, false)
            .into_iter()
            .map(|tool| tool.name)
            .collect();

        assert_eq!(names, vec!["file_read", "shell", "web_search"]);
    }

    #[test]
    fn streaming_and_non_streaming_both_enable_graph_recall() {
        let source = include_str!("kernel_llm_turn.rs");
        let enabled_count = source
            .matches(concat!("include_graph_recall", ": true"))
            .count();
        let disabled_count = source
            .matches(concat!("include_graph_recall", ": false"))
            .count();

        assert!(
            enabled_count >= 2,
            "both streaming and non-streaming LLM turns must inject graph recall"
        );
        assert_eq!(
            disabled_count, 0,
            "no LLM turn path should silently drop graph recall"
        );
    }
}
