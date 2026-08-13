//! Core agent execution loop.

pub use crate::agent_loop_budget::{current_turn_token_budget, with_turn_token_budget};
pub use crate::agent_loop_control::AGENT_LOOP_MAX_ITERATIONS_KEY;
use crate::agent_loop_end_turn::{
    begin_delivery_verification, delivery_verification_report, handle_end_turn_response,
    incomplete_delivery_text, record_delivery_verification, EndTurnInput,
};
use crate::agent_loop_iteration::{
    complete_iteration, stream_iteration, CompletionIterationInput, IterationCallOutcome,
    StreamingIterationInput,
};
use crate::agent_loop_limits::{
    continuation_limit_text, fail_max_iterations, handle_incomplete_continuation,
    handle_max_tokens_continuation, ContinuationLimitKind, IncompleteContinuationInput,
    MaxTokensContinuationInput, MAX_CONTINUATIONS,
};
pub use crate::agent_loop_phase::{LoopPhase, PhaseCallback};
use crate::agent_loop_quota::{check_mid_loop_quota, streaming_quota_should_break};
pub use crate::agent_loop_request::strip_provider_prefix;
pub use crate::agent_loop_result::AgentLoopResult;
use crate::agent_loop_stream_delivery::StreamDeliveryBuffer;
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
use crate::work_verification::{
    evaluate_tool_receipts, VerificationDisposition, WorkVerificationReport,
    MAX_VERIFICATION_CORRECTION_ROUNDS,
};
use crate::workflow_learning_runtime::{begin_episode_best_effort, run_in_workflow_episode};
use captain_memory::session::Session;
use captain_memory::work_verification_progress::WorkVerificationState;
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
    verification_correction_rounds: u8,
    verification_operation:
        Option<captain_memory::work_verification_progress::WorkVerificationLease>,
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
                verification_correction_rounds: 0,
                verification_operation: None,
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
    let workflow_episode =
        (!crate::client_authority::is_paired_client_origin(origin_channel.as_deref()))
            .then(|| {
                begin_episode_best_effort(
                    memory,
                    &session.agent_id.to_string(),
                    &session.id.to_string(),
                    user_message,
                    origin_channel.as_deref(),
                    workspace_root,
                )
            })
            .flatten();
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
    let workflow_episode =
        (!crate::client_authority::is_paired_client_origin(origin_channel.as_deref()))
            .then(|| {
                begin_episode_best_effort(
                    memory,
                    &session.agent_id.to_string(),
                    &session.id.to_string(),
                    user_message,
                    origin_channel.as_deref(),
                    workspace_root,
                )
            })
            .flatten();
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
            record_forced_incomplete_stop(turn, ctx.session, ctx.memory, ctx.on_phase)?;
            return Ok(result);
        }

        let response = match complete_agent_loop_iteration(&mut ctx, turn, iteration).await? {
            IterationCallOutcome::Response(response) => response,
            IterationCallOutcome::Finished(mut result) => {
                record_forced_incomplete_stop(turn, ctx.session, ctx.memory, ctx.on_phase)?;
                result.tool_calls = turn.state.tool_calls_recorded.clone();
                return Ok(result);
            }
            IterationCallOutcome::Continue => continue,
        };

        if let Some(result) =
            handle_completion_response(&response, &mut ctx, turn, iteration).await?
        {
            return Ok(result);
        }
    }

    record_forced_incomplete_stop(turn, ctx.session, ctx.memory, ctx.on_phase)?;
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
    let mut held_delivery = StreamDeliveryBuffer::default();

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

        let hold_stream_delivery =
            !held_delivery.is_empty() || stream_delivery_requires_hold(turn, iteration);
        let delivery_checkpoint = held_delivery.checkpoint();
        let response = match stream_agent_loop_iteration(
            &mut ctx,
            turn,
            iteration,
            &mut codex_missing_tool_watchdog_used,
            hold_stream_delivery,
            &mut held_delivery,
        )
        .await?
        {
            IterationCallOutcome::Response(response) => response,
            IterationCallOutcome::Finished(mut result) => {
                record_forced_incomplete_stop(turn, ctx.session, ctx.memory, ctx.on_phase)?;
                result.tool_calls = turn.state.tool_calls_recorded.clone();
                if hold_stream_delivery {
                    held_delivery
                        .replace_with_final(
                            &ctx.stream_tx,
                            &result.response,
                            StopReason::EndTurn,
                            result.total_usage,
                        )
                        .await;
                }
                return Ok(result);
            }
            IterationCallOutcome::Continue => {
                if hold_stream_delivery {
                    held_delivery.rollback(delivery_checkpoint);
                }
                continue;
            }
        };

        if hold_stream_delivery {
            held_delivery.validate_segment(
                delivery_checkpoint,
                &response.text(),
                response.stop_reason,
            )?;
        }

        if response.stop_reason == StopReason::ToolUse && hold_stream_delivery {
            held_delivery.release(&ctx.stream_tx).await?;
        }

        let result = handle_streaming_response(&response, &mut ctx, turn, iteration).await?;
        if let Some(result) = result {
            if hold_stream_delivery && response.stop_reason != StopReason::ToolUse {
                deliver_held_final_response(&mut held_delivery, &ctx.stream_tx, &response, &result)
                    .await?;
            }
            return Ok(result);
        }

        match response.stop_reason {
            StopReason::EndTurn | StopReason::StopSequence => held_delivery.discard(),
            StopReason::ToolUse => debug_assert!(held_delivery.is_empty()),
            StopReason::MaxTokens | StopReason::Incomplete => {}
        }
    }

    record_forced_incomplete_stop(turn, ctx.session, ctx.memory, ctx.on_phase)?;
    held_delivery.discard();
    fail_active_turn_max_iterations(ctx.manifest, ctx.session, ctx.memory, ctx.hooks, turn).await
}

async fn stream_agent_loop_iteration(
    ctx: &mut StreamingAgentLoopContext<'_>,
    turn: &mut ActiveAgentTurn,
    iteration: u32,
    codex_missing_tool_watchdog_used: &mut bool,
    hold_stream_delivery: bool,
    delivery_buffer: &mut StreamDeliveryBuffer,
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
        hold_stream_delivery,
        delivery_buffer,
    }))
    .await
}

fn stream_delivery_requires_hold(turn: &ActiveAgentTurn, iteration: u32) -> bool {
    if current_turn_token_budget().is_some_and(|budget| budget > 0) {
        return true;
    }

    matches!(
        delivery_verification_report(
            &turn.state.tool_calls_recorded,
            turn.state.any_tools_executed,
            turn.state.verification_correction_rounds,
            iteration,
            turn.state.max_iterations,
        )
        .disposition,
        VerificationDisposition::NeedsCorrection | VerificationDisposition::Incomplete
    )
}

async fn deliver_held_final_response(
    delivery: &mut StreamDeliveryBuffer,
    stream_tx: &mpsc::Sender<StreamEvent>,
    provider_response: &CompletionResponse,
    result: &AgentLoopResult,
) -> CaptainResult<()> {
    let raw_text = provider_response.text();
    if !result.silent && result.response == raw_text && delivery.all_events_validated() {
        delivery.release(stream_tx).await?;
    } else {
        delivery
            .replace_with_final(
                stream_tx,
                if result.silent { "" } else { &result.response },
                StopReason::EndTurn,
                result.total_usage,
            )
            .await;
    }
    Ok(())
}

fn record_forced_incomplete_stop(
    turn: &mut ActiveAgentTurn,
    session: &Session,
    memory: &MemorySubstrate,
    on_phase: Option<&PhaseCallback>,
) -> CaptainResult<()> {
    let report = evaluate_tool_receipts(
        &turn.state.tool_calls_recorded,
        turn.state.any_tools_executed,
        MAX_VERIFICATION_CORRECTION_ROUNDS,
    );
    if report.disposition != VerificationDisposition::Incomplete {
        return Ok(());
    }
    begin_delivery_verification(
        memory,
        session,
        &mut turn.state.verification_operation,
        MAX_VERIFICATION_CORRECTION_ROUNDS,
        &report,
        on_phase,
    )?;
    record_delivery_verification(
        memory,
        session,
        &mut turn.state.verification_operation,
        WorkVerificationState::Incomplete,
        MAX_VERIFICATION_CORRECTION_ROUNDS,
        &report,
        on_phase,
    )
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
        verification_correction_rounds: &mut turn.state.verification_correction_rounds,
        verification_operation: &mut turn.state.verification_operation,
        max_iterations: turn.state.max_iterations,
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
    on_phase: Option<&PhaseCallback>,
    turn: &mut ActiveAgentTurn,
    iteration: u32,
    streaming: bool,
) -> CaptainResult<Option<AgentLoopResult>> {
    let next_continuation_count = turn.state.consecutive_max_tokens.saturating_add(1);
    let forced_report = begin_forced_continuation_verification(
        turn,
        session,
        memory,
        on_phase,
        next_continuation_count,
    )?;
    let final_text_override = forced_report.as_ref().map(|report| {
        incomplete_delivery_text(
            report,
            &continuation_limit_text(ContinuationLimitKind::MaxTokens, response),
        )
    });
    let result = handle_max_tokens_continuation(MaxTokensContinuationInput {
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
        final_text_override,
    })
    .await?;
    finish_forced_continuation_verification(
        turn,
        session,
        memory,
        on_phase,
        forced_report.as_ref(),
        result.is_some(),
    )?;
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
async fn handle_incomplete_response(
    response: &CompletionResponse,
    manifest: &AgentManifest,
    session: &mut Session,
    memory: &MemorySubstrate,
    hooks: Option<&crate::hooks::HookRegistry>,
    on_phase: Option<&PhaseCallback>,
    turn: &mut ActiveAgentTurn,
    iteration: u32,
    streaming: bool,
) -> CaptainResult<Option<AgentLoopResult>> {
    let next_continuation_count = turn.state.consecutive_incomplete.saturating_add(1);
    let forced_report = begin_forced_continuation_verification(
        turn,
        session,
        memory,
        on_phase,
        next_continuation_count,
    )?;
    let final_text_override = forced_report.as_ref().map(|report| {
        incomplete_delivery_text(
            report,
            &continuation_limit_text(ContinuationLimitKind::Incomplete, response),
        )
    });
    let result = handle_incomplete_continuation(IncompleteContinuationInput {
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
        final_text_override,
    })
    .await?;
    finish_forced_continuation_verification(
        turn,
        session,
        memory,
        on_phase,
        forced_report.as_ref(),
        result.is_some(),
    )?;
    Ok(result)
}

fn begin_forced_continuation_verification(
    turn: &mut ActiveAgentTurn,
    session: &Session,
    memory: &MemorySubstrate,
    on_phase: Option<&PhaseCallback>,
    next_continuation_count: u32,
) -> CaptainResult<Option<WorkVerificationReport>> {
    if next_continuation_count < MAX_CONTINUATIONS {
        return Ok(None);
    }
    let report = evaluate_tool_receipts(
        &turn.state.tool_calls_recorded,
        turn.state.any_tools_executed,
        MAX_VERIFICATION_CORRECTION_ROUNDS,
    );
    if report.disposition != VerificationDisposition::Incomplete {
        return Ok(None);
    }
    begin_delivery_verification(
        memory,
        session,
        &mut turn.state.verification_operation,
        MAX_VERIFICATION_CORRECTION_ROUNDS,
        &report,
        on_phase,
    )?;
    Ok(Some(report))
}

fn finish_forced_continuation_verification(
    turn: &mut ActiveAgentTurn,
    session: &Session,
    memory: &MemorySubstrate,
    on_phase: Option<&PhaseCallback>,
    report: Option<&WorkVerificationReport>,
    limit_reached: bool,
) -> CaptainResult<()> {
    let Some(report) = report.filter(|_| limit_reached) else {
        return Ok(());
    };
    record_delivery_verification(
        memory,
        session,
        &mut turn.state.verification_operation,
        WorkVerificationState::Incomplete,
        MAX_VERIFICATION_CORRECTION_ROUNDS,
        report,
        on_phase,
    )
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
                ctx.on_phase,
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
                ctx.on_phase,
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
                ctx.on_phase,
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
                ctx.on_phase,
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
    use captain_types::tool::{ToolCall, ToolResult};

    fn test_tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: "test tool".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    fn tool_record(
        name: &str,
        input: serde_json::Value,
        output: &str,
        sequence: u32,
    ) -> ToolCallRecord {
        let call = ToolCall {
            id: format!("call-{sequence}"),
            name: name.to_string(),
            input,
        };
        let result = ToolResult {
            tool_use_id: call.id.clone(),
            content: output.to_string(),
            is_error: false,
            transient_content: Vec::new(),
        };
        ToolCallRecord {
            tool_name: name.to_string(),
            reason: "test receipt".to_string(),
            is_error: false,
            duration_ms: 1,
            input_summary: String::new(),
            output_summary: String::new(),
            verification: Some(
                crate::work_verification::ToolVerificationReceipt::from_tool_call(
                    &call, &result, sequence,
                ),
            ),
        }
    }

    fn active_test_turn() -> ActiveAgentTurn {
        PreparedAgentTurn {
            hand_allowed_env: Vec::new(),
            agent_id_str: "agent-1".to_string(),
            system_prompt: "system".to_string(),
            messages: vec![captain_types::message::Message::user("hello")],
            max_iterations: 7,
            loop_guard: LoopGuard::new(crate::loop_guard::LoopGuardConfig::default()),
            ctx_window: 4096,
            context_budget: ContextBudget::new(4096),
            visible_tools: vec![test_tool("file_write"), test_tool("file_read")],
        }
        .into()
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
        assert_eq!(turn.state.verification_correction_rounds, 0);
        assert!(turn.state.verification_operation.is_none());
    }

    #[test]
    fn stream_delivery_is_held_only_until_mutation_has_postcondition_evidence() {
        let mut turn = active_test_turn();
        turn.state.any_tools_executed = true;
        turn.state.tool_calls_recorded.push(tool_record(
            "file_write",
            serde_json::json!({"path": "notes.txt", "content": "done"}),
            "ok",
            0,
        ));

        assert!(stream_delivery_requires_hold(&turn, 1));

        turn.state.tool_calls_recorded.push(tool_record(
            "file_read",
            serde_json::json!({"path": "notes.txt"}),
            "done",
            1,
        ));

        assert!(!stream_delivery_requires_hold(&turn, 2));
    }

    #[tokio::test]
    async fn delegation_budget_forces_transactional_stream_delivery() {
        let turn = active_test_turn();

        with_turn_token_budget(Some(100), async {
            assert!(stream_delivery_requires_hold(&turn, 0));
        })
        .await;
    }

    #[tokio::test]
    async fn matching_verified_final_is_released_once_without_replacement() {
        let usage = TokenUsage {
            output_tokens: 3,
            ..Default::default()
        };
        let response = CompletionResponse {
            content: vec![ContentBlock::Text {
                text: "verified".to_string(),
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: Vec::new(),
            usage,
        };
        let result = AgentLoopResult {
            response: "verified".to_string(),
            total_usage: usage,
            iterations: 2,
            cost_usd: None,
            silent: false,
            directives: Default::default(),
            tool_calls: Vec::new(),
        };
        let mut delivery = StreamDeliveryBuffer::default();
        let checkpoint = delivery.checkpoint();
        delivery
            .push(StreamEvent::TextDelta {
                text: "verified".to_string(),
            })
            .unwrap();
        delivery
            .push(StreamEvent::ContentComplete {
                stop_reason: StopReason::EndTurn,
                usage,
            })
            .unwrap();
        delivery
            .validate_segment(checkpoint, "verified", StopReason::EndTurn)
            .unwrap();
        let (tx, mut rx) = mpsc::channel(4);

        deliver_held_final_response(&mut delivery, &tx, &response, &result)
            .await
            .unwrap();
        drop(tx);

        assert!(matches!(
            rx.recv().await,
            Some(StreamEvent::TextDelta { text }) if text == "verified"
        ));
        assert!(matches!(
            rx.recv().await,
            Some(StreamEvent::ContentComplete {
                stop_reason: StopReason::EndTurn,
                ..
            })
        ));
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn continuation_limit_records_an_honest_incomplete_delivery() {
        let mut turn = active_test_turn();
        turn.state.any_tools_executed = true;
        turn.state.consecutive_incomplete = MAX_CONTINUATIONS - 1;
        turn.state.tool_calls_recorded.push(tool_record(
            "file_write",
            serde_json::json!({"path": "notes.txt", "content": "done"}),
            "ok",
            0,
        ));
        let manifest = AgentManifest::default();
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let mut session = Session {
            id: captain_types::agent::SessionId::new(),
            agent_id: captain_types::agent::AgentId::new(),
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let response = CompletionResponse {
            content: vec![ContentBlock::Text {
                text: "everything is complete".to_string(),
                provider_metadata: None,
            }],
            stop_reason: StopReason::Incomplete,
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
        };

        let result = handle_incomplete_response(
            &response,
            &manifest,
            &mut session,
            &memory,
            None,
            None,
            &mut turn,
            4,
            true,
        )
        .await
        .unwrap()
        .expect("continuation limit should finish honestly");

        assert!(result.response.starts_with("Verification incomplete:"));
        assert!(result.response.contains("Unverified draft from the agent:"));
        assert!(turn.state.verification_operation.is_none());
        let saved = memory.get_session(session.id).unwrap().unwrap();
        assert_eq!(
            saved.messages.last().unwrap().content.text_content(),
            result.response
        );
    }
}
