//! Core agent execution loop.

pub use crate::agent_loop_budget::{current_turn_token_budget, with_turn_token_budget};
pub use crate::agent_loop_control::AGENT_LOOP_MAX_ITERATIONS_KEY;
use crate::agent_loop_end_turn::{handle_end_turn_response, EndTurnInput};
use crate::agent_loop_iteration::{
    complete_iteration, stream_iteration, CompletionIterationInput, IterationCallOutcome,
    StreamingIterationInput,
};
use crate::agent_loop_limits::{
    fail_max_iterations, handle_incomplete_continuation, handle_max_tokens_continuation,
    IncompleteContinuationInput, MaxTokensContinuationInput,
};
pub use crate::agent_loop_phase::{LoopPhase, PhaseCallback};
use crate::agent_loop_quota::{check_mid_loop_quota, streaming_quota_should_break};
pub use crate::agent_loop_request::strip_provider_prefix;
pub use crate::agent_loop_result::AgentLoopResult;
use crate::agent_loop_tool_execution::{
    execute_tool_calls, execute_tool_calls_streaming, StreamingToolExecutionInput,
    ToolExecutionInput,
};
pub use crate::agent_loop_tool_runtime::is_exec_tool;
pub use crate::agent_loop_tool_trace::{format_tool_trace, tool_emoji, tool_input_preview};
use crate::agent_loop_turn::{prepare_agent_turn, PreparedAgentTurn};
use crate::context_budget::ContextBudget;
use crate::embedding::EmbeddingDriver;
use crate::kernel_handle::KernelHandle;
use crate::llm_driver::{CompletionResponse, LlmDriver, StreamEvent};
use crate::loop_guard::LoopGuard;
use crate::mcp::McpConnection;
use crate::web_search::WebToolsContext;
use crate::workflow_learning_runtime::{begin_episode_best_effort, run_in_workflow_episode};
use captain_memory::session::Session;
use captain_memory::MemorySubstrate;
use captain_skills::registry::SkillRegistry;
use captain_types::agent::AgentManifest;
use captain_types::error::CaptainResult;
use captain_types::message::{ContentBlock, StopReason, TokenUsage};
use captain_types::tool::ToolDefinition;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info};

pub use crate::agent_loop_tool_record::ToolCallRecord;

struct ActiveAgentTurn {
    hand_allowed_env: Vec<String>,
    agent_id_str: String,
    system_prompt: String,
    messages: Vec<captain_types::message::Message>,
    state: AgentLoopState,
}

struct AgentLoopState {
    total_usage: TokenUsage,
    tool_calls_recorded: Vec<ToolCallRecord>,
    max_iterations: u32,
    loop_guard: LoopGuard,
    consecutive_max_tokens: u32,
    consecutive_incomplete: u32,
    ctx_window: usize,
    context_budget: ContextBudget,
    any_tools_executed: bool,
    capability_denial_watchdog_used: bool,
    visible_tools: Vec<ToolDefinition>,
}

struct NonStreamingAgentLoopContext<'a> {
    manifest: &'a AgentManifest,
    user_message: &'a str,
    session: &'a mut Session,
    memory: &'a MemorySubstrate,
    driver: Arc<dyn LlmDriver>,
    available_tools: &'a [ToolDefinition],
    kernel: Option<Arc<dyn KernelHandle>>,
    skill_registry: Option<&'a SkillRegistry>,
    mcp_connections: Option<&'a tokio::sync::Mutex<Vec<McpConnection>>>,
    web_ctx: Option<&'a WebToolsContext>,
    browser_ctx: Option<&'a crate::browser::BrowserManager>,
    embedding_driver: Option<&'a (dyn EmbeddingDriver + Send + Sync)>,
    workspace_root: Option<&'a Path>,
    on_phase: Option<&'a PhaseCallback>,
    media_engine: Option<&'a crate::media_understanding::MediaEngine>,
    tts_engine: Option<&'a crate::tts::TtsEngine>,
    docker_config: Option<&'a captain_types::config::DockerSandboxConfig>,
    hooks: Option<&'a crate::hooks::HookRegistry>,
    process_manager: Option<&'a crate::process_manager::ProcessManager>,
    origin_channel: Option<String>,
}

struct StreamingAgentLoopContext<'a> {
    manifest: &'a AgentManifest,
    user_message: &'a str,
    session: &'a mut Session,
    memory: &'a MemorySubstrate,
    driver: Arc<dyn LlmDriver>,
    available_tools: &'a [ToolDefinition],
    kernel: Option<Arc<dyn KernelHandle>>,
    stream_tx: mpsc::Sender<StreamEvent>,
    skill_registry: Option<&'a SkillRegistry>,
    mcp_connections: Option<&'a tokio::sync::Mutex<Vec<McpConnection>>>,
    web_ctx: Option<&'a WebToolsContext>,
    browser_ctx: Option<&'a crate::browser::BrowserManager>,
    embedding_driver: Option<&'a (dyn EmbeddingDriver + Send + Sync)>,
    workspace_root: Option<&'a Path>,
    on_phase: Option<&'a PhaseCallback>,
    media_engine: Option<&'a crate::media_understanding::MediaEngine>,
    tts_engine: Option<&'a crate::tts::TtsEngine>,
    docker_config: Option<&'a captain_types::config::DockerSandboxConfig>,
    hooks: Option<&'a crate::hooks::HookRegistry>,
    process_manager: Option<&'a crate::process_manager::ProcessManager>,
    user_input_rx: Option<Arc<tokio::sync::Mutex<mpsc::Receiver<String>>>>,
    origin_channel: Option<String>,
}

impl From<PreparedAgentTurn> for ActiveAgentTurn {
    fn from(prepared: PreparedAgentTurn) -> Self {
        Self {
            hand_allowed_env: prepared.hand_allowed_env,
            agent_id_str: prepared.agent_id_str,
            system_prompt: prepared.system_prompt,
            messages: prepared.messages,
            state: AgentLoopState {
                total_usage: TokenUsage::default(),
                tool_calls_recorded: Vec::new(),
                max_iterations: prepared.max_iterations,
                loop_guard: prepared.loop_guard,
                consecutive_max_tokens: 0,
                consecutive_incomplete: 0,
                ctx_window: prepared.ctx_window,
                context_budget: prepared.context_budget,
                any_tools_executed: false,
                capability_denial_watchdog_used: false,
                visible_tools: prepared.visible_tools,
            },
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn prepare_active_turn(
    manifest: &AgentManifest,
    user_message: &str,
    session: &mut Session,
    memory: &MemorySubstrate,
    kernel: Option<&Arc<dyn KernelHandle>>,
    embedding_driver: Option<&(dyn EmbeddingDriver + Send + Sync)>,
    hooks: Option<&crate::hooks::HookRegistry>,
    user_content_blocks: Option<Vec<ContentBlock>>,
    available_tools: &[ToolDefinition],
    context_window_tokens: Option<usize>,
    streaming: bool,
) -> ActiveAgentTurn {
    ActiveAgentTurn::from(
        prepare_agent_turn(
            manifest,
            user_message,
            session,
            memory,
            kernel,
            embedding_driver,
            hooks,
            user_content_blocks,
            available_tools,
            context_window_tokens,
            streaming,
        )
        .await,
    )
}

async fn fail_active_turn_max_iterations(
    manifest: &AgentManifest,
    session: &mut Session,
    memory: &MemorySubstrate,
    hooks: Option<&crate::hooks::HookRegistry>,
    turn: &ActiveAgentTurn,
) -> CaptainResult<AgentLoopResult> {
    fail_max_iterations(
        manifest,
        session,
        memory,
        hooks,
        turn.agent_id_str.as_str(),
        turn.state.max_iterations,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn run_agent_loop(
    manifest: &AgentManifest,
    user_message: &str,
    session: &mut Session,
    memory: &MemorySubstrate,
    driver: Arc<dyn LlmDriver>,
    available_tools: &[ToolDefinition],
    kernel: Option<Arc<dyn KernelHandle>>,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<McpConnection>>>,
    web_ctx: Option<&WebToolsContext>,
    browser_ctx: Option<&crate::browser::BrowserManager>,
    embedding_driver: Option<&(dyn EmbeddingDriver + Send + Sync)>,
    workspace_root: Option<&Path>,
    on_phase: Option<&PhaseCallback>,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    docker_config: Option<&captain_types::config::DockerSandboxConfig>,
    hooks: Option<&crate::hooks::HookRegistry>,
    context_window_tokens: Option<usize>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    user_content_blocks: Option<Vec<ContentBlock>>,
    origin_channel: Option<String>,
) -> CaptainResult<AgentLoopResult> {
    let workflow_episode = begin_episode_best_effort(
        memory,
        &session.agent_id.to_string(),
        &session.id.to_string(),
        user_message,
        origin_channel.as_deref(),
        workspace_root,
    );
    run_in_workflow_episode(
        workflow_episode,
        Box::pin(async {
            info!(agent = %manifest.name, "Starting agent loop");

            let mut turn = prepare_active_turn(
                manifest,
                user_message,
                session,
                memory,
                kernel.as_ref(),
                embedding_driver,
                hooks,
                user_content_blocks,
                available_tools,
                context_window_tokens,
                false,
            )
            .await;

            let ctx = NonStreamingAgentLoopContext {
                manifest,
                user_message,
                session,
                memory,
                driver,
                available_tools,
                kernel,
                skill_registry,
                mcp_connections,
                web_ctx,
                browser_ctx,
                embedding_driver,
                workspace_root,
                on_phase,
                media_engine,
                tts_engine,
                docker_config,
                hooks,
                process_manager,
                origin_channel,
            };
            run_non_streaming_agent_loop_iterations(ctx, &mut turn).await
        }),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn run_agent_loop_streaming(
    manifest: &AgentManifest,
    user_message: &str,
    session: &mut Session,
    memory: &MemorySubstrate,
    driver: Arc<dyn LlmDriver>,
    available_tools: &[ToolDefinition],
    kernel: Option<Arc<dyn KernelHandle>>,
    stream_tx: mpsc::Sender<StreamEvent>,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<McpConnection>>>,
    web_ctx: Option<&WebToolsContext>,
    browser_ctx: Option<&crate::browser::BrowserManager>,
    embedding_driver: Option<&(dyn EmbeddingDriver + Send + Sync)>,
    workspace_root: Option<&Path>,
    on_phase: Option<&PhaseCallback>,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    docker_config: Option<&captain_types::config::DockerSandboxConfig>,
    hooks: Option<&crate::hooks::HookRegistry>,
    context_window_tokens: Option<usize>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    user_content_blocks: Option<Vec<ContentBlock>>,
    user_input_rx: Option<Arc<tokio::sync::Mutex<mpsc::Receiver<String>>>>,
    origin_channel: Option<String>,
) -> CaptainResult<AgentLoopResult> {
    let workflow_episode = begin_episode_best_effort(
        memory,
        &session.agent_id.to_string(),
        &session.id.to_string(),
        user_message,
        origin_channel.as_deref(),
        workspace_root,
    );
    run_in_workflow_episode(
        workflow_episode,
        Box::pin(async {
            info!(agent = %manifest.name, "Starting streaming agent loop");

            let mut turn = prepare_active_turn(
                manifest,
                user_message,
                session,
                memory,
                kernel.as_ref(),
                embedding_driver,
                hooks,
                user_content_blocks,
                available_tools,
                context_window_tokens,
                true,
            )
            .await;

            let ctx = StreamingAgentLoopContext {
                manifest,
                user_message,
                session,
                memory,
                driver,
                available_tools,
                kernel,
                stream_tx,
                skill_registry,
                mcp_connections,
                web_ctx,
                browser_ctx,
                embedding_driver,
                workspace_root,
                on_phase,
                media_engine,
                tts_engine,
                docker_config,
                hooks,
                process_manager,
                user_input_rx,
                origin_channel,
            };
            run_streaming_agent_loop_iterations(ctx, &mut turn).await
        }),
    )
    .await
}

async fn run_non_streaming_agent_loop_iterations(
    mut ctx: NonStreamingAgentLoopContext<'_>,
    turn: &mut ActiveAgentTurn,
) -> CaptainResult<AgentLoopResult> {
    for iteration in 0..turn.state.max_iterations {
        debug!(iteration, "Agent loop iteration");

        if let Some(result) = check_mid_loop_quota(
            ctx.manifest,
            ctx.kernel.as_ref(),
            iteration,
            &turn.state.total_usage,
            &turn.state.tool_calls_recorded,
        ) {
            return Ok(result);
        }

        let response = match complete_agent_loop_iteration(&mut ctx, turn, iteration).await? {
            IterationCallOutcome::Response(response) => response,
            IterationCallOutcome::Finished(result) => return Ok(result),
            IterationCallOutcome::Continue => continue,
        };

        if let Some(result) =
            handle_completion_response(&response, &mut ctx, turn, iteration).await?
        {
            return Ok(result);
        }
    }

    fail_active_turn_max_iterations(ctx.manifest, ctx.session, ctx.memory, ctx.hooks, turn).await
}

async fn complete_agent_loop_iteration(
    ctx: &mut NonStreamingAgentLoopContext<'_>,
    turn: &mut ActiveAgentTurn,
    iteration: u32,
) -> CaptainResult<IterationCallOutcome> {
    Box::pin(complete_iteration(CompletionIterationInput {
        manifest: ctx.manifest,
        session: &mut *ctx.session,
        memory: ctx.memory,
        driver: &*ctx.driver,
        messages: &mut turn.messages,
        system_prompt: &turn.system_prompt,
        visible_tools: &turn.state.visible_tools,
        context_budget: &turn.state.context_budget,
        ctx_window: turn.state.ctx_window,
        iteration,
        total_usage: &mut turn.state.total_usage,
        on_phase: ctx.on_phase,
    }))
    .await
}

async fn run_streaming_agent_loop_iterations(
    mut ctx: StreamingAgentLoopContext<'_>,
    turn: &mut ActiveAgentTurn,
) -> CaptainResult<AgentLoopResult> {
    let mut codex_missing_tool_watchdog_used = false;

    for iteration in 0..turn.state.max_iterations {
        debug!(iteration, "Streaming agent loop iteration");

        if streaming_quota_should_break(
            ctx.manifest,
            ctx.kernel.as_ref(),
            iteration,
            &ctx.stream_tx,
        )
        .await
        {
            break;
        }

        let response = match stream_agent_loop_iteration(
            &mut ctx,
            turn,
            iteration,
            &mut codex_missing_tool_watchdog_used,
        )
        .await?
        {
            IterationCallOutcome::Response(response) => response,
            IterationCallOutcome::Finished(result) => return Ok(result),
            IterationCallOutcome::Continue => continue,
        };

        if let Some(result) =
            handle_streaming_response(&response, &mut ctx, turn, iteration).await?
        {
            return Ok(result);
        }
    }

    fail_active_turn_max_iterations(ctx.manifest, ctx.session, ctx.memory, ctx.hooks, turn).await
}

async fn stream_agent_loop_iteration(
    ctx: &mut StreamingAgentLoopContext<'_>,
    turn: &mut ActiveAgentTurn,
    iteration: u32,
    codex_missing_tool_watchdog_used: &mut bool,
) -> CaptainResult<IterationCallOutcome> {
    Box::pin(stream_iteration(StreamingIterationInput {
        manifest: ctx.manifest,
        session: &mut *ctx.session,
        memory: ctx.memory,
        driver: &*ctx.driver,
        messages: &mut turn.messages,
        system_prompt: &turn.system_prompt,
        visible_tools: &turn.state.visible_tools,
        context_budget: &turn.state.context_budget,
        ctx_window: turn.state.ctx_window,
        iteration,
        total_usage: &mut turn.state.total_usage,
        on_phase: ctx.on_phase,
        stream_tx: &ctx.stream_tx,
        user_input_rx: &ctx.user_input_rx,
        codex_missing_tool_watchdog_used,
    }))
    .await
}

fn reset_after_tool_use(turn: &mut ActiveAgentTurn) {
    turn.state.consecutive_max_tokens = 0;
    turn.state.consecutive_incomplete = 0;
    turn.state.any_tools_executed = true;
}

#[allow(clippy::too_many_arguments)]
async fn handle_loop_end_turn(
    response: &CompletionResponse,
    manifest: &AgentManifest,
    user_message: &str,
    session: &mut Session,
    memory: &MemorySubstrate,
    embedding_driver: Option<&(dyn EmbeddingDriver + Send + Sync)>,
    on_phase: Option<&PhaseCallback>,
    hooks: Option<&crate::hooks::HookRegistry>,
    turn: &mut ActiveAgentTurn,
    iteration: u32,
    streaming: bool,
    phantom_action_watchdog: bool,
) -> CaptainResult<Option<AgentLoopResult>> {
    handle_end_turn_response(EndTurnInput {
        manifest,
        user_message,
        response,
        total_usage: &turn.state.total_usage,
        messages: &mut turn.messages,
        iteration,
        any_tools_executed: turn.state.any_tools_executed,
        capability_denial_watchdog_used: &mut turn.state.capability_denial_watchdog_used,
        visible_tools: &turn.state.visible_tools,
        streaming,
        phantom_action_watchdog,
        session,
        memory,
        embedding_driver,
        on_phase,
        hooks,
        agent_id_str: &turn.agent_id_str,
        tool_calls_recorded: &turn.state.tool_calls_recorded,
    })
    .await
}

#[allow(clippy::too_many_arguments)]
async fn handle_max_tokens_response(
    response: &CompletionResponse,
    manifest: &AgentManifest,
    session: &mut Session,
    memory: &MemorySubstrate,
    hooks: Option<&crate::hooks::HookRegistry>,
    turn: &mut ActiveAgentTurn,
    iteration: u32,
    streaming: bool,
) -> CaptainResult<Option<AgentLoopResult>> {
    handle_max_tokens_continuation(MaxTokensContinuationInput {
        response,
        session,
        memory,
        manifest,
        hooks,
        agent_id_str: turn.agent_id_str.as_str(),
        total_usage: &turn.state.total_usage,
        iteration,
        consecutive_max_tokens: &mut turn.state.consecutive_max_tokens,
        consecutive_incomplete: &mut turn.state.consecutive_incomplete,
        tool_calls_recorded: &turn.state.tool_calls_recorded,
        streaming,
        messages: &mut turn.messages,
    })
    .await
}

#[allow(clippy::too_many_arguments)]
async fn handle_incomplete_response(
    response: &CompletionResponse,
    manifest: &AgentManifest,
    session: &mut Session,
    memory: &MemorySubstrate,
    hooks: Option<&crate::hooks::HookRegistry>,
    turn: &mut ActiveAgentTurn,
    iteration: u32,
    streaming: bool,
) -> CaptainResult<Option<AgentLoopResult>> {
    handle_incomplete_continuation(IncompleteContinuationInput {
        response,
        provider_name: manifest.model.provider.as_str(),
        session,
        memory,
        manifest,
        hooks,
        agent_id_str: turn.agent_id_str.as_str(),
        total_usage: &turn.state.total_usage,
        iteration,
        consecutive_max_tokens: &mut turn.state.consecutive_max_tokens,
        consecutive_incomplete: &mut turn.state.consecutive_incomplete,
        tool_calls_recorded: &turn.state.tool_calls_recorded,
        streaming,
        messages: &mut turn.messages,
    })
    .await
}

async fn handle_completion_response(
    response: &CompletionResponse,
    ctx: &mut NonStreamingAgentLoopContext<'_>,
    turn: &mut ActiveAgentTurn,
    iteration: u32,
) -> CaptainResult<Option<AgentLoopResult>> {
    match response.stop_reason {
        StopReason::EndTurn | StopReason::StopSequence => {
            handle_loop_end_turn(
                response,
                ctx.manifest,
                ctx.user_message,
                &mut *ctx.session,
                ctx.memory,
                ctx.embedding_driver,
                ctx.on_phase,
                ctx.hooks,
                turn,
                iteration,
                false,
                true,
            )
            .await
        }
        StopReason::ToolUse => handle_completion_tool_use(response, ctx, turn).await,
        StopReason::MaxTokens => {
            handle_max_tokens_response(
                response,
                ctx.manifest,
                &mut *ctx.session,
                ctx.memory,
                ctx.hooks,
                turn,
                iteration,
                false,
            )
            .await
        }
        StopReason::Incomplete => {
            handle_incomplete_response(
                response,
                ctx.manifest,
                &mut *ctx.session,
                ctx.memory,
                ctx.hooks,
                turn,
                iteration,
                false,
            )
            .await
        }
    }
}

async fn handle_streaming_response(
    response: &CompletionResponse,
    ctx: &mut StreamingAgentLoopContext<'_>,
    turn: &mut ActiveAgentTurn,
    iteration: u32,
) -> CaptainResult<Option<AgentLoopResult>> {
    match response.stop_reason {
        StopReason::EndTurn | StopReason::StopSequence => {
            handle_loop_end_turn(
                response,
                ctx.manifest,
                ctx.user_message,
                &mut *ctx.session,
                ctx.memory,
                ctx.embedding_driver,
                ctx.on_phase,
                ctx.hooks,
                turn,
                iteration,
                true,
                false,
            )
            .await
        }
        StopReason::ToolUse => handle_streaming_tool_use(response, ctx, turn).await,
        StopReason::MaxTokens => {
            handle_max_tokens_response(
                response,
                ctx.manifest,
                &mut *ctx.session,
                ctx.memory,
                ctx.hooks,
                turn,
                iteration,
                true,
            )
            .await
        }
        StopReason::Incomplete => {
            handle_incomplete_response(
                response,
                ctx.manifest,
                &mut *ctx.session,
                ctx.memory,
                ctx.hooks,
                turn,
                iteration,
                true,
            )
            .await
        }
    }
}

async fn handle_completion_tool_use(
    response: &CompletionResponse,
    ctx: &mut NonStreamingAgentLoopContext<'_>,
    turn: &mut ActiveAgentTurn,
) -> CaptainResult<Option<AgentLoopResult>> {
    reset_after_tool_use(turn);

    Box::pin(execute_tool_calls(ToolExecutionInput {
        response,
        manifest: ctx.manifest,
        session: &mut *ctx.session,
        memory: ctx.memory,
        messages: &mut turn.messages,
        loop_guard: &mut turn.state.loop_guard,
        tool_calls_recorded: &mut turn.state.tool_calls_recorded,
        visible_tools: &mut turn.state.visible_tools,
        available_tools: ctx.available_tools,
        context_budget: &turn.state.context_budget,
        hand_allowed_env: &turn.hand_allowed_env,
        kernel: ctx.kernel.as_ref(),
        skill_registry: ctx.skill_registry,
        mcp_connections: ctx.mcp_connections,
        web_ctx: ctx.web_ctx,
        browser_ctx: ctx.browser_ctx,
        workspace_root: ctx.workspace_root,
        on_phase: ctx.on_phase,
        media_engine: ctx.media_engine,
        tts_engine: ctx.tts_engine,
        docker_config: ctx.docker_config,
        hooks: ctx.hooks,
        process_manager: ctx.process_manager,
        origin_channel: ctx.origin_channel.as_ref(),
        agent_id_str: turn.agent_id_str.as_str(),
    }))
    .await
}

async fn handle_streaming_tool_use(
    response: &CompletionResponse,
    ctx: &mut StreamingAgentLoopContext<'_>,
    turn: &mut ActiveAgentTurn,
) -> CaptainResult<Option<AgentLoopResult>> {
    reset_after_tool_use(turn);

    Box::pin(execute_tool_calls_streaming(StreamingToolExecutionInput {
        response,
        manifest: ctx.manifest,
        session: &mut *ctx.session,
        memory: ctx.memory,
        messages: &mut turn.messages,
        loop_guard: &mut turn.state.loop_guard,
        tool_calls_recorded: &mut turn.state.tool_calls_recorded,
        visible_tools: &mut turn.state.visible_tools,
        available_tools: ctx.available_tools,
        context_budget: &turn.state.context_budget,
        hand_allowed_env: &turn.hand_allowed_env,
        kernel: ctx.kernel.as_ref(),
        stream_tx: &ctx.stream_tx,
        user_input_rx: ctx.user_input_rx.as_ref(),
        skill_registry: ctx.skill_registry,
        mcp_connections: ctx.mcp_connections,
        web_ctx: ctx.web_ctx,
        browser_ctx: ctx.browser_ctx,
        workspace_root: ctx.workspace_root,
        on_phase: ctx.on_phase,
        media_engine: ctx.media_engine,
        tts_engine: ctx.tts_engine,
        docker_config: ctx.docker_config,
        hooks: ctx.hooks,
        process_manager: ctx.process_manager,
        origin_channel: ctx.origin_channel.as_ref(),
        agent_id_str: turn.agent_id_str.as_str(),
    }))
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: "test tool".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    #[test]
    fn active_turn_from_prepared_starts_with_clean_runtime_state() {
        let prepared = PreparedAgentTurn {
            hand_allowed_env: vec!["PATH".to_string()],
            agent_id_str: "agent-1".to_string(),
            system_prompt: "system".to_string(),
            messages: vec![captain_types::message::Message::user("hello")],
            max_iterations: 7,
            loop_guard: LoopGuard::new(crate::loop_guard::LoopGuardConfig::default()),
            ctx_window: 4096,
            context_budget: ContextBudget::new(4096),
            visible_tools: vec![test_tool("file_read")],
        };

        let turn = ActiveAgentTurn::from(prepared);

        assert_eq!(turn.hand_allowed_env, vec!["PATH"]);
        assert_eq!(turn.agent_id_str, "agent-1");
        assert_eq!(turn.system_prompt, "system");
        assert_eq!(turn.messages.len(), 1);
        assert_eq!(turn.state.max_iterations, 7);
        assert_eq!(turn.state.ctx_window, 4096);
        assert_eq!(turn.state.context_budget.context_window_tokens, 4096);
        assert_eq!(turn.state.visible_tools[0].name, "file_read");
        assert_eq!(turn.state.total_usage.input_tokens, 0);
        assert_eq!(turn.state.total_usage.output_tokens, 0);
        assert_eq!(turn.state.total_usage.cached_input_tokens, 0);
        assert_eq!(turn.state.total_usage.cache_creation_tokens, 0);
        assert!(turn.state.tool_calls_recorded.is_empty());
        assert!(!turn.state.any_tools_executed);
        assert!(!turn.state.capability_denial_watchdog_used);
    }
}
