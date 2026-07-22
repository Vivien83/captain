use crate::agent_loop::{AgentLoopResult, PhaseCallback, ToolCallRecord};
use crate::agent_loop_ask_user::try_handle_ask_user_tool_call;
use crate::agent_loop_guard::{fail_loop_guard_circuit_break, push_loop_guard_block_result};
use crate::agent_loop_hooks::before_tool_call_allows_execution;
use crate::agent_loop_phase::notify_tool_use_phase;
use crate::agent_loop_policy::{effective_tool_policy, manifest_subagent_depth};
use crate::agent_loop_tool_finish::{finish_tool_call, FinishToolCallInput};
use crate::agent_loop_tool_results::{append_tool_result_turn, interim_save_tool_turn};
use crate::agent_loop_tool_runtime::{
    run_tool_with_timeout_guard, spawn_tool_progress_forwarder, tool_timeout_guard_secs,
};
use crate::agent_loop_tool_turn::append_tool_use_assistant_turn;
use crate::context_budget::ContextBudget;
use crate::kernel_handle::KernelHandle;
use crate::llm_driver::{CompletionResponse, StreamEvent};
use crate::loop_guard::{LoopGuard, LoopGuardVerdict};
use crate::mcp::McpConnection;
use crate::tool_parallelism::{
    log_parallel_opportunity, partition_parallel_groups, ExecutionGroup,
};
use crate::tool_runner;
use crate::web_search::WebToolsContext;
use crate::workflow_learning_runtime::{
    advance_dependency_frontier, current_dependency_frontier, register_parallel_tool_dependencies,
};
use captain_memory::session::Session;
use captain_memory::MemorySubstrate;
use captain_skills::registry::SkillRegistry;
use captain_types::agent::AgentManifest;
use captain_types::error::CaptainResult;
use captain_types::message::{ContentBlock, Message};
use captain_types::tool::{ToolCall, ToolDefinition, ToolResult};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, warn};

pub(crate) struct ToolExecutionInput<'a> {
    pub(crate) response: &'a CompletionResponse,
    pub(crate) manifest: &'a AgentManifest,
    pub(crate) session: &'a mut Session,
    pub(crate) memory: &'a MemorySubstrate,
    pub(crate) messages: &'a mut Vec<Message>,
    pub(crate) loop_guard: &'a mut LoopGuard,
    pub(crate) tool_calls_recorded: &'a mut Vec<ToolCallRecord>,
    pub(crate) visible_tools: &'a mut Vec<ToolDefinition>,
    pub(crate) available_tools: &'a [ToolDefinition],
    pub(crate) context_budget: &'a ContextBudget,
    pub(crate) hand_allowed_env: &'a [String],
    pub(crate) kernel: Option<&'a Arc<dyn KernelHandle>>,
    pub(crate) skill_registry: Option<&'a SkillRegistry>,
    pub(crate) mcp_connections: Option<&'a tokio::sync::Mutex<Vec<McpConnection>>>,
    pub(crate) web_ctx: Option<&'a WebToolsContext>,
    pub(crate) browser_ctx: Option<&'a crate::browser::BrowserManager>,
    pub(crate) workspace_root: Option<&'a Path>,
    pub(crate) on_phase: Option<&'a PhaseCallback>,
    pub(crate) media_engine: Option<&'a crate::media_understanding::MediaEngine>,
    pub(crate) tts_engine: Option<&'a crate::tts::TtsEngine>,
    pub(crate) docker_config: Option<&'a captain_types::config::DockerSandboxConfig>,
    pub(crate) hooks: Option<&'a crate::hooks::HookRegistry>,
    pub(crate) process_manager: Option<&'a crate::process_manager::ProcessManager>,
    pub(crate) origin_channel: Option<&'a String>,
    pub(crate) agent_id_str: &'a str,
}

pub(crate) struct StreamingToolExecutionInput<'a> {
    pub(crate) response: &'a CompletionResponse,
    pub(crate) manifest: &'a AgentManifest,
    pub(crate) session: &'a mut Session,
    pub(crate) memory: &'a MemorySubstrate,
    pub(crate) messages: &'a mut Vec<Message>,
    pub(crate) loop_guard: &'a mut LoopGuard,
    pub(crate) tool_calls_recorded: &'a mut Vec<ToolCallRecord>,
    pub(crate) visible_tools: &'a mut Vec<ToolDefinition>,
    pub(crate) available_tools: &'a [ToolDefinition],
    pub(crate) context_budget: &'a ContextBudget,
    pub(crate) hand_allowed_env: &'a [String],
    pub(crate) kernel: Option<&'a Arc<dyn KernelHandle>>,
    pub(crate) stream_tx: &'a mpsc::Sender<StreamEvent>,
    pub(crate) user_input_rx: Option<&'a Arc<tokio::sync::Mutex<mpsc::Receiver<String>>>>,
    pub(crate) skill_registry: Option<&'a SkillRegistry>,
    pub(crate) mcp_connections: Option<&'a tokio::sync::Mutex<Vec<McpConnection>>>,
    pub(crate) web_ctx: Option<&'a WebToolsContext>,
    pub(crate) browser_ctx: Option<&'a crate::browser::BrowserManager>,
    pub(crate) workspace_root: Option<&'a Path>,
    pub(crate) on_phase: Option<&'a PhaseCallback>,
    pub(crate) media_engine: Option<&'a crate::media_understanding::MediaEngine>,
    pub(crate) tts_engine: Option<&'a crate::tts::TtsEngine>,
    pub(crate) docker_config: Option<&'a captain_types::config::DockerSandboxConfig>,
    pub(crate) hooks: Option<&'a crate::hooks::HookRegistry>,
    pub(crate) process_manager: Option<&'a crate::process_manager::ProcessManager>,
    pub(crate) origin_channel: Option<&'a String>,
    pub(crate) agent_id_str: &'a str,
}

enum ToolGuardAction {
    Execute,
    Continue,
    Finish(AgentLoopResult),
}

type ToolCallExecutionFuture<'a> =
    Pin<Box<dyn Future<Output = CaptainResult<Option<AgentLoopResult>>> + Send + 'a>>;

#[allow(clippy::too_many_arguments)]
async fn apply_loop_guard_verdict(
    manifest: &AgentManifest,
    session: &mut Session,
    memory: &MemorySubstrate,
    hooks: Option<&crate::hooks::HookRegistry>,
    agent_id_str: &str,
    tool_call: &ToolCall,
    verdict: &LoopGuardVerdict,
    tool_result_blocks: &mut Vec<ContentBlock>,
    streaming: bool,
) -> CaptainResult<ToolGuardAction> {
    match verdict {
        LoopGuardVerdict::CircuitBreak(msg) => {
            crate::workflow_learning_runtime::record_terminal_tool_attempt(
                tool_call,
                true,
                "loop_guard_circuit_break",
            );
            let suffix = if streaming { " (streaming)" } else { "" };
            warn!(tool = %tool_call.name, "Circuit breaker triggered{}", suffix);
            fail_loop_guard_circuit_break(manifest, session, memory, hooks, agent_id_str, msg)
                .await
                .map(ToolGuardAction::Finish)
        }
        LoopGuardVerdict::Block(msg) => {
            crate::workflow_learning_runtime::record_terminal_tool_attempt(
                tool_call,
                true,
                "loop_guard_blocked",
            );
            let suffix = if streaming { " (streaming)" } else { "" };
            warn!(tool = %tool_call.name, "Tool call blocked by loop guard{}", suffix);
            push_loop_guard_block_result(tool_result_blocks, tool_call, msg);
            Ok(ToolGuardAction::Continue)
        }
        _ => Ok(ToolGuardAction::Execute),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_non_streaming_tool_call(
    tool_call: &ToolCall,
    kernel: Option<&Arc<dyn KernelHandle>>,
    effective_allowlist: Option<&[String]>,
    caller_id_str: &str,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<McpConnection>>>,
    web_ctx: Option<&WebToolsContext>,
    browser_ctx: Option<&crate::browser::BrowserManager>,
    hand_allowed_env: &[String],
    workspace_root: Option<&Path>,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    effective_exec_policy: Option<&captain_types::config::ExecPolicy>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    docker_config: Option<&captain_types::config::DockerSandboxConfig>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    origin_channel: Option<&String>,
    subagent_depth: u32,
) -> (ToolResult, u64) {
    let tool_start = std::time::Instant::now();
    let run_id = crate::tool_runs::global_registry().start(
        tool_call.name.clone(),
        Some(caller_id_str.to_string()),
        Some(tool_call.id.clone()),
        false,
    );
    let exec_fut = tool_runner::with_origin_channel(
        origin_channel.cloned(),
        tool_runner::execute_tool(
            &tool_call.id,
            &tool_call.name,
            &tool_call.input,
            kernel,
            effective_allowlist,
            Some(caller_id_str),
            skill_registry,
            mcp_connections,
            web_ctx,
            browser_ctx,
            allowed_env_vars(hand_allowed_env),
            workspace_root,
            media_engine,
            effective_exec_policy,
            tts_engine,
            docker_config,
            process_manager,
        ),
    );
    let exec_fut = tool_runner::with_agent_lineage_depth(subagent_depth, exec_fut);
    let result = run_tool_with_timeout_guard(
        tool_call,
        tool_timeout_guard_secs(&tool_call.name, &tool_call.input, effective_exec_policy),
        false,
        exec_fut,
    )
    .await;
    crate::tool_runs::global_registry().finish(&run_id, &result);
    (result, tool_start.elapsed().as_millis() as u64)
}

#[allow(clippy::too_many_arguments)]
async fn run_streaming_tool_call(
    tool_call: &ToolCall,
    kernel: Option<&Arc<dyn KernelHandle>>,
    effective_allowlist: Option<&[String]>,
    caller_id_str: &str,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<McpConnection>>>,
    web_ctx: Option<&WebToolsContext>,
    browser_ctx: Option<&crate::browser::BrowserManager>,
    hand_allowed_env: &[String],
    workspace_root: Option<&Path>,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    effective_exec_policy: Option<&captain_types::config::ExecPolicy>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    docker_config: Option<&captain_types::config::DockerSandboxConfig>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    origin_channel: Option<&String>,
    stream_tx: &mpsc::Sender<StreamEvent>,
    subagent_depth: u32,
) -> (ToolResult, u64) {
    let tool_start = std::time::Instant::now();
    let run_id = crate::tool_runs::global_registry().start(
        tool_call.name.clone(),
        Some(caller_id_str.to_string()),
        Some(tool_call.id.clone()),
        false,
    );
    let stream_ctx = Some(crate::tool_runner::ToolStreamCtx {
        tool_use_id: tool_call.id.clone(),
        tx: stream_tx.clone(),
    });
    let (progress_tx, progress_rx) = mpsc::channel(16);
    let _progress_forwarder = spawn_tool_progress_forwarder(progress_rx, stream_tx.clone());
    let exec_fut = tool_runner::with_progress_sink(
        progress_tx,
        crate::tool_runner::TOOL_STREAM.scope(
            stream_ctx,
            tool_runner::with_origin_channel(
                origin_channel.cloned(),
                tool_runner::execute_tool(
                    &tool_call.id,
                    &tool_call.name,
                    &tool_call.input,
                    kernel,
                    effective_allowlist,
                    Some(caller_id_str),
                    skill_registry,
                    mcp_connections,
                    web_ctx,
                    browser_ctx,
                    allowed_env_vars(hand_allowed_env),
                    workspace_root,
                    media_engine,
                    effective_exec_policy,
                    tts_engine,
                    docker_config,
                    process_manager,
                ),
            ),
        ),
    );
    let exec_fut = tool_runner::with_agent_lineage_depth(subagent_depth, exec_fut);
    let result = run_tool_with_timeout_guard(
        tool_call,
        tool_timeout_guard_secs(&tool_call.name, &tool_call.input, effective_exec_policy),
        true,
        exec_fut,
    )
    .await;
    crate::tool_runs::global_registry().finish(&run_id, &result);
    (result, tool_start.elapsed().as_millis() as u64)
}

struct NonStreamingOneToolCallInput<'a> {
    tool_call: &'a ToolCall,
    manifest: &'a AgentManifest,
    session: &'a mut Session,
    memory: &'a MemorySubstrate,
    loop_guard: &'a mut LoopGuard,
    tool_calls_recorded: &'a mut Vec<ToolCallRecord>,
    visible_tools: &'a mut Vec<ToolDefinition>,
    available_tools: &'a [ToolDefinition],
    context_budget: &'a ContextBudget,
    hand_allowed_env: &'a [String],
    kernel: Option<&'a Arc<dyn KernelHandle>>,
    skill_registry: Option<&'a SkillRegistry>,
    mcp_connections: Option<&'a tokio::sync::Mutex<Vec<McpConnection>>>,
    web_ctx: Option<&'a WebToolsContext>,
    browser_ctx: Option<&'a crate::browser::BrowserManager>,
    workspace_root: Option<&'a Path>,
    on_phase: Option<&'a PhaseCallback>,
    media_engine: Option<&'a crate::media_understanding::MediaEngine>,
    tts_engine: Option<&'a crate::tts::TtsEngine>,
    docker_config: Option<&'a captain_types::config::DockerSandboxConfig>,
    hooks: Option<&'a crate::hooks::HookRegistry>,
    process_manager: Option<&'a crate::process_manager::ProcessManager>,
    origin_channel: Option<&'a String>,
    agent_id_str: &'a str,
    caller_id_str: &'a str,
    effective_allowlist: Option<&'a [String]>,
    subagent_depth: u32,
    tool_result_blocks: &'a mut Vec<ContentBlock>,
}

fn non_streaming_finish_input<'a>(
    input: &'a mut NonStreamingOneToolCallInput<'_>,
    result: ToolResult,
    verdict: &'a LoopGuardVerdict,
    tool_elapsed_ms: u64,
) -> FinishToolCallInput<'a> {
    FinishToolCallInput {
        manifest: input.manifest,
        tool_call: input.tool_call,
        result,
        verdict,
        context_budget: input.context_budget,
        available_tools: input.available_tools,
        visible_tools: &mut *input.visible_tools,
        tool_calls_recorded: &mut *input.tool_calls_recorded,
        tool_result_blocks: &mut *input.tool_result_blocks,
        kernel: input.kernel,
        hooks: input.hooks,
        caller_id_str: input.caller_id_str,
        tool_elapsed_ms,
        streaming: false,
        stream_tx: None,
    }
}

fn execute_one_tool_call(input: NonStreamingOneToolCallInput<'_>) -> ToolCallExecutionFuture<'_> {
    Box::pin(async move {
        let mut input = input;
        let verdict = input
            .loop_guard
            .check(&input.tool_call.name, &input.tool_call.input);
        match apply_loop_guard_verdict(
            input.manifest,
            &mut *input.session,
            input.memory,
            input.hooks,
            input.agent_id_str,
            input.tool_call,
            &verdict,
            &mut *input.tool_result_blocks,
            false,
        )
        .await?
        {
            ToolGuardAction::Execute => {}
            ToolGuardAction::Continue => return Ok(None),
            ToolGuardAction::Finish(result) => return Ok(Some(result)),
        }

        debug!(tool = %input.tool_call.name, id = %input.tool_call.id, "Executing tool");
        notify_tool_use_phase(input.on_phase, &input.tool_call.name);
        if !before_tool_call_allows_execution(
            input.hooks,
            input.manifest,
            input.caller_id_str,
            input.tool_call,
            &mut *input.tool_result_blocks,
        ) {
            return Ok(None);
        }

        let effective_exec_policy = input.manifest.exec_policy.as_ref();
        let (result, tool_elapsed_ms) = Box::pin(run_non_streaming_tool_call(
            input.tool_call,
            input.kernel,
            input.effective_allowlist,
            input.caller_id_str,
            input.skill_registry,
            input.mcp_connections,
            input.web_ctx,
            input.browser_ctx,
            input.hand_allowed_env,
            input.workspace_root,
            input.media_engine,
            effective_exec_policy,
            input.tts_engine,
            input.docker_config,
            input.process_manager,
            input.origin_channel,
            input.subagent_depth,
        ))
        .await;

        let streak_warning = input
            .loop_guard
            .record_tool_error(&input.tool_call.name, result.is_error);
        let verdict = crate::loop_guard::combine_verdict_with_error_streak(verdict, streak_warning);

        finish_tool_call(non_streaming_finish_input(
            &mut input,
            result,
            &verdict,
            tool_elapsed_ms,
        ))
        .await;
        Ok(None)
    })
}

async fn handle_streaming_ask_user_tool_call(
    manifest: &AgentManifest,
    tool_call: &ToolCall,
    stream_tx: &mpsc::Sender<StreamEvent>,
    user_input_rx: Option<&Arc<tokio::sync::Mutex<mpsc::Receiver<String>>>>,
    tool_calls_recorded: &mut Vec<ToolCallRecord>,
    tool_result_blocks: &mut Vec<ContentBlock>,
) -> bool {
    try_handle_ask_user_tool_call(
        &manifest.name,
        tool_call,
        stream_tx,
        user_input_rx,
        tool_calls_recorded,
        tool_result_blocks,
    )
    .await
}

struct StreamingOneToolCallInput<'a> {
    tool_call: &'a ToolCall,
    manifest: &'a AgentManifest,
    session: &'a mut Session,
    memory: &'a MemorySubstrate,
    loop_guard: &'a mut LoopGuard,
    tool_calls_recorded: &'a mut Vec<ToolCallRecord>,
    visible_tools: &'a mut Vec<ToolDefinition>,
    available_tools: &'a [ToolDefinition],
    context_budget: &'a ContextBudget,
    hand_allowed_env: &'a [String],
    kernel: Option<&'a Arc<dyn KernelHandle>>,
    stream_tx: &'a mpsc::Sender<StreamEvent>,
    user_input_rx: Option<&'a Arc<tokio::sync::Mutex<mpsc::Receiver<String>>>>,
    skill_registry: Option<&'a SkillRegistry>,
    mcp_connections: Option<&'a tokio::sync::Mutex<Vec<McpConnection>>>,
    web_ctx: Option<&'a WebToolsContext>,
    browser_ctx: Option<&'a crate::browser::BrowserManager>,
    workspace_root: Option<&'a Path>,
    on_phase: Option<&'a PhaseCallback>,
    media_engine: Option<&'a crate::media_understanding::MediaEngine>,
    tts_engine: Option<&'a crate::tts::TtsEngine>,
    docker_config: Option<&'a captain_types::config::DockerSandboxConfig>,
    hooks: Option<&'a crate::hooks::HookRegistry>,
    process_manager: Option<&'a crate::process_manager::ProcessManager>,
    origin_channel: Option<&'a String>,
    agent_id_str: &'a str,
    caller_id_str: &'a str,
    effective_allowlist: Option<&'a [String]>,
    subagent_depth: u32,
    tool_result_blocks: &'a mut Vec<ContentBlock>,
}

fn streaming_finish_input<'a>(
    input: &'a mut StreamingOneToolCallInput<'_>,
    result: ToolResult,
    verdict: &'a LoopGuardVerdict,
    tool_elapsed_ms: u64,
) -> FinishToolCallInput<'a> {
    FinishToolCallInput {
        manifest: input.manifest,
        tool_call: input.tool_call,
        result,
        verdict,
        context_budget: input.context_budget,
        available_tools: input.available_tools,
        visible_tools: &mut *input.visible_tools,
        tool_calls_recorded: &mut *input.tool_calls_recorded,
        tool_result_blocks: &mut *input.tool_result_blocks,
        kernel: input.kernel,
        hooks: input.hooks,
        caller_id_str: input.caller_id_str,
        tool_elapsed_ms,
        streaming: true,
        stream_tx: Some(input.stream_tx),
    }
}

fn run_streaming_tool_call_for_input<'a>(
    input: &'a StreamingOneToolCallInput<'_>,
) -> Pin<Box<dyn Future<Output = (ToolResult, u64)> + Send + 'a>> {
    let effective_exec_policy = input.manifest.exec_policy.as_ref();
    Box::pin(run_streaming_tool_call(
        input.tool_call,
        input.kernel,
        input.effective_allowlist,
        input.caller_id_str,
        input.skill_registry,
        input.mcp_connections,
        input.web_ctx,
        input.browser_ctx,
        input.hand_allowed_env,
        input.workspace_root,
        input.media_engine,
        effective_exec_policy,
        input.tts_engine,
        input.docker_config,
        input.process_manager,
        input.origin_channel,
        input.stream_tx,
        input.subagent_depth,
    ))
}

fn execute_one_streaming_tool_call(
    input: StreamingOneToolCallInput<'_>,
) -> ToolCallExecutionFuture<'_> {
    Box::pin(async move {
        let mut input = input;
        let verdict = input
            .loop_guard
            .check(&input.tool_call.name, &input.tool_call.input);
        match apply_loop_guard_verdict(
            input.manifest,
            &mut *input.session,
            input.memory,
            input.hooks,
            input.agent_id_str,
            input.tool_call,
            &verdict,
            &mut *input.tool_result_blocks,
            true,
        )
        .await?
        {
            ToolGuardAction::Execute => {}
            ToolGuardAction::Continue => return Ok(None),
            ToolGuardAction::Finish(result) => return Ok(Some(result)),
        }

        debug!(tool = %input.tool_call.name, id = %input.tool_call.id, "Executing tool (streaming)");
        notify_tool_use_phase(input.on_phase, &input.tool_call.name);
        if !before_tool_call_allows_execution(
            input.hooks,
            input.manifest,
            input.caller_id_str,
            input.tool_call,
            &mut *input.tool_result_blocks,
        ) {
            return Ok(None);
        }

        if handle_streaming_ask_user_tool_call(
            input.manifest,
            input.tool_call,
            input.stream_tx,
            input.user_input_rx,
            &mut *input.tool_calls_recorded,
            &mut *input.tool_result_blocks,
        )
        .await
        {
            return Ok(None);
        }

        let (result, tool_elapsed_ms) = run_streaming_tool_call_for_input(&input).await;

        let streak_warning = input
            .loop_guard
            .record_tool_error(&input.tool_call.name, result.is_error);
        let verdict = crate::loop_guard::combine_verdict_with_error_streak(verdict, streak_warning);

        finish_tool_call(streaming_finish_input(
            &mut input,
            result,
            &verdict,
            tool_elapsed_ms,
        ))
        .await;
        Ok(None)
    })
}

/// Same shape as `NonStreamingOneToolCallInput`, but for a whole
/// `ExecutionGroup::Parallel` batch: `calls` replaces the single
/// `tool_call`, and `tool_result_blocks` is the real, shared output —
/// results are appended to it in original call order once the whole group
/// is done, never mid-flight (see `execute_parallel_group`).
struct ParallelGroupInput<'a> {
    calls: &'a [ToolCall],
    manifest: &'a AgentManifest,
    session: &'a mut Session,
    memory: &'a MemorySubstrate,
    loop_guard: &'a mut LoopGuard,
    tool_calls_recorded: &'a mut Vec<ToolCallRecord>,
    visible_tools: &'a mut Vec<ToolDefinition>,
    available_tools: &'a [ToolDefinition],
    context_budget: &'a ContextBudget,
    hand_allowed_env: &'a [String],
    kernel: Option<&'a Arc<dyn KernelHandle>>,
    skill_registry: Option<&'a SkillRegistry>,
    mcp_connections: Option<&'a tokio::sync::Mutex<Vec<McpConnection>>>,
    web_ctx: Option<&'a WebToolsContext>,
    browser_ctx: Option<&'a crate::browser::BrowserManager>,
    workspace_root: Option<&'a Path>,
    on_phase: Option<&'a PhaseCallback>,
    media_engine: Option<&'a crate::media_understanding::MediaEngine>,
    tts_engine: Option<&'a crate::tts::TtsEngine>,
    docker_config: Option<&'a captain_types::config::DockerSandboxConfig>,
    hooks: Option<&'a crate::hooks::HookRegistry>,
    process_manager: Option<&'a crate::process_manager::ProcessManager>,
    origin_channel: Option<&'a String>,
    agent_id_str: &'a str,
    caller_id_str: &'a str,
    effective_allowlist: Option<&'a [String]>,
    subagent_depth: u32,
    tool_result_blocks: &'a mut Vec<ContentBlock>,
}

struct PendingExec<'a> {
    index: usize,
    verdict: LoopGuardVerdict,
    fut: Pin<Box<dyn Future<Output = (ToolResult, u64)> + Send + 'a>>,
}

/// Run a batch of side-effect-free tool calls, executing the slow I/O part
/// concurrently while keeping every state-mutating step (loop guard
/// fingerprinting, budget/result bookkeeping) exactly as sequential and
/// in-order as the plain `for` loop in `execute_tool_calls` — see the
/// module-level design note in `tool_parallelism.rs` and the Tier 2.3 plan.
///
/// Three phases, none of which overlap in time so no mutable state is ever
/// borrowed twice at once:
///   1. PRE, sequential, in order: `LoopGuard::check` mutates
///      `recent_calls` as a side effect, so this cannot run out of order or
///      concurrently without changing repeat-detection behavior. Calls
///      that pass PRE are queued (not awaited yet); blocked/hook-rejected
///      calls are already fully resolved here. A `Finish` verdict stops
///      the PRE pass immediately, leaving any later calls untouched —
///      exactly like the plain sequential loop reaching that same call.
///   2. EXEC, concurrent: only the queued calls, via `join_all`. This is
///      the only phase safe to parallelize — `run_non_streaming_tool_call`
///      takes exclusively shared `&` references.
///   3. POST, sequential, in original index order (not completion order):
///      loop guard error-streak + budget/result bookkeeping per call.
///
/// Each call's result is buffered in its own scratch `Vec<ContentBlock>`
/// slot and only flushed into the real `tool_result_blocks` at the very
/// end, in index order — otherwise a PRE-blocked call, pushed during phase
/// one, would land before an earlier executed call, pushed during phase
/// three, scrambling the order the plain sequential loop guarantees.
#[allow(clippy::too_many_arguments)]
async fn execute_parallel_group(
    input: ParallelGroupInput<'_>,
) -> CaptainResult<Option<AgentLoopResult>> {
    let ParallelGroupInput {
        calls,
        manifest,
        session,
        memory,
        loop_guard,
        tool_calls_recorded,
        visible_tools,
        available_tools,
        context_budget,
        hand_allowed_env,
        kernel,
        skill_registry,
        mcp_connections,
        web_ctx,
        browser_ctx,
        workspace_root,
        on_phase,
        media_engine,
        tts_engine,
        docker_config,
        hooks,
        process_manager,
        origin_channel,
        agent_id_str,
        caller_id_str,
        effective_allowlist,
        subagent_depth,
        tool_result_blocks,
    } = input;

    let mut per_call_blocks: Vec<Vec<ContentBlock>> = calls.iter().map(|_| Vec::new()).collect();
    let mut pending: Vec<PendingExec<'_>> = Vec::new();
    let mut finish_result: Option<AgentLoopResult> = None;
    // A PRE-phase error (e.g. circuit break) must not discard calls earlier
    // in this same group that already passed PRE: the plain sequential
    // loop would have already run those to completion before ever reaching
    // this later call, so this path still owes them EXEC+POST below before
    // the error is allowed to propagate.
    let mut pre_error: Option<captain_types::error::CaptainError> = None;
    let group_dependencies = current_dependency_frontier();
    let mut resolved_tool_ids = Vec::new();

    for (index, tool_call) in calls.iter().enumerate() {
        register_parallel_tool_dependencies(&tool_call.id, group_dependencies.clone());
        resolved_tool_ids.push(tool_call.id.clone());
        let verdict = loop_guard.check(&tool_call.name, &tool_call.input);
        let guard_action = apply_loop_guard_verdict(
            manifest,
            &mut *session,
            memory,
            hooks,
            agent_id_str,
            tool_call,
            &verdict,
            &mut per_call_blocks[index],
            false,
        )
        .await;
        match guard_action {
            Ok(ToolGuardAction::Execute) => {}
            Ok(ToolGuardAction::Continue) => continue,
            Ok(ToolGuardAction::Finish(result)) => {
                finish_result = Some(result);
                break;
            }
            Err(e) => {
                pre_error = Some(e);
                break;
            }
        }

        notify_tool_use_phase(on_phase, &tool_call.name);
        if !before_tool_call_allows_execution(
            hooks,
            manifest,
            caller_id_str,
            tool_call,
            &mut per_call_blocks[index],
        ) {
            continue;
        }

        let effective_exec_policy = manifest.exec_policy.as_ref();
        let fut = Box::pin(run_non_streaming_tool_call(
            tool_call,
            kernel,
            effective_allowlist,
            caller_id_str,
            skill_registry,
            mcp_connections,
            web_ctx,
            browser_ctx,
            hand_allowed_env,
            workspace_root,
            media_engine,
            effective_exec_policy,
            tts_engine,
            docker_config,
            process_manager,
            origin_channel,
            subagent_depth,
        ));
        pending.push(PendingExec {
            index,
            verdict,
            fut,
        });
    }

    let indices: Vec<usize> = pending.iter().map(|p| p.index).collect();
    let verdicts: Vec<LoopGuardVerdict> = pending.iter().map(|p| p.verdict.clone()).collect();
    let exec_results = futures::future::join_all(pending.into_iter().map(|p| p.fut)).await;

    for ((index, verdict), (result, tool_elapsed_ms)) in
        indices.into_iter().zip(verdicts).zip(exec_results)
    {
        let tool_call = &calls[index];
        let streak_warning = loop_guard.record_tool_error(&tool_call.name, result.is_error);
        let verdict = crate::loop_guard::combine_verdict_with_error_streak(verdict, streak_warning);

        finish_tool_call(FinishToolCallInput {
            manifest,
            tool_call,
            result,
            verdict: &verdict,
            context_budget,
            available_tools,
            visible_tools: &mut *visible_tools,
            tool_calls_recorded: &mut *tool_calls_recorded,
            tool_result_blocks: &mut per_call_blocks[index],
            kernel,
            hooks,
            caller_id_str,
            tool_elapsed_ms,
            streaming: false,
            stream_tx: None,
        })
        .await;
    }

    for blocks in per_call_blocks {
        tool_result_blocks.extend(blocks);
    }
    if !resolved_tool_ids.is_empty() {
        advance_dependency_frontier(resolved_tool_ids);
    }

    if let Some(e) = pre_error {
        return Err(e);
    }
    Ok(finish_result)
}

pub(crate) async fn execute_tool_calls(
    input: ToolExecutionInput<'_>,
) -> CaptainResult<Option<AgentLoopResult>> {
    append_tool_use_assistant_turn(input.response, input.session, input.messages);
    // Persist the assistant ToolUse message before PRE/EXEC. If Captain dies
    // inside a long foreground tool, the next process can now distinguish an
    // interrupted tool turn from a normal completed boundary.
    interim_save_tool_turn(input.session, input.memory, "tool_use").await;
    let effective_allowlist = effective_tool_policy(input.manifest);
    let caller_id_str = input.session.agent_id.to_string();
    let subagent_depth = manifest_subagent_depth(input.manifest);
    let mut tool_result_blocks = Vec::new();
    log_parallel_opportunity(&input.response.tool_calls);

    for group in partition_parallel_groups(&input.response.tool_calls) {
        let outcome = match group {
            ExecutionGroup::Sequential(tool_call) => {
                let one_call_input = NonStreamingOneToolCallInput {
                    tool_call: &tool_call,
                    manifest: input.manifest,
                    session: &mut *input.session,
                    memory: input.memory,
                    loop_guard: &mut *input.loop_guard,
                    tool_calls_recorded: &mut *input.tool_calls_recorded,
                    visible_tools: &mut *input.visible_tools,
                    available_tools: input.available_tools,
                    context_budget: input.context_budget,
                    hand_allowed_env: input.hand_allowed_env,
                    kernel: input.kernel,
                    skill_registry: input.skill_registry,
                    mcp_connections: input.mcp_connections,
                    web_ctx: input.web_ctx,
                    browser_ctx: input.browser_ctx,
                    workspace_root: input.workspace_root,
                    on_phase: input.on_phase,
                    media_engine: input.media_engine,
                    tts_engine: input.tts_engine,
                    docker_config: input.docker_config,
                    hooks: input.hooks,
                    process_manager: input.process_manager,
                    origin_channel: input.origin_channel,
                    agent_id_str: input.agent_id_str,
                    caller_id_str: &caller_id_str,
                    effective_allowlist: effective_allowlist.as_deref(),
                    subagent_depth,
                    tool_result_blocks: &mut tool_result_blocks,
                };
                execute_one_tool_call(one_call_input).await?
            }
            ExecutionGroup::Parallel(calls) => {
                execute_parallel_group(ParallelGroupInput {
                    calls: &calls,
                    manifest: input.manifest,
                    session: &mut *input.session,
                    memory: input.memory,
                    loop_guard: &mut *input.loop_guard,
                    tool_calls_recorded: &mut *input.tool_calls_recorded,
                    visible_tools: &mut *input.visible_tools,
                    available_tools: input.available_tools,
                    context_budget: input.context_budget,
                    hand_allowed_env: input.hand_allowed_env,
                    kernel: input.kernel,
                    skill_registry: input.skill_registry,
                    mcp_connections: input.mcp_connections,
                    web_ctx: input.web_ctx,
                    browser_ctx: input.browser_ctx,
                    workspace_root: input.workspace_root,
                    on_phase: input.on_phase,
                    media_engine: input.media_engine,
                    tts_engine: input.tts_engine,
                    docker_config: input.docker_config,
                    hooks: input.hooks,
                    process_manager: input.process_manager,
                    origin_channel: input.origin_channel,
                    agent_id_str: input.agent_id_str,
                    caller_id_str: &caller_id_str,
                    effective_allowlist: effective_allowlist.as_deref(),
                    subagent_depth,
                    tool_result_blocks: &mut tool_result_blocks,
                })
                .await?
            }
        };
        if let Some(result) = outcome {
            return Ok(Some(result));
        }
    }

    append_tool_result_turn(&mut tool_result_blocks, input.session, input.messages);
    interim_save_tool_turn(input.session, input.memory, "tool_results").await;
    Ok(None)
}

/// Streaming counterpart of `ParallelGroupInput` — same fields plus
/// `stream_tx`/`user_input_rx`, matching `StreamingOneToolCallInput`.
struct StreamingParallelGroupInput<'a> {
    calls: &'a [ToolCall],
    manifest: &'a AgentManifest,
    session: &'a mut Session,
    memory: &'a MemorySubstrate,
    loop_guard: &'a mut LoopGuard,
    tool_calls_recorded: &'a mut Vec<ToolCallRecord>,
    visible_tools: &'a mut Vec<ToolDefinition>,
    available_tools: &'a [ToolDefinition],
    context_budget: &'a ContextBudget,
    hand_allowed_env: &'a [String],
    kernel: Option<&'a Arc<dyn KernelHandle>>,
    stream_tx: &'a mpsc::Sender<StreamEvent>,
    user_input_rx: Option<&'a Arc<tokio::sync::Mutex<mpsc::Receiver<String>>>>,
    skill_registry: Option<&'a SkillRegistry>,
    mcp_connections: Option<&'a tokio::sync::Mutex<Vec<McpConnection>>>,
    web_ctx: Option<&'a WebToolsContext>,
    browser_ctx: Option<&'a crate::browser::BrowserManager>,
    workspace_root: Option<&'a Path>,
    on_phase: Option<&'a PhaseCallback>,
    media_engine: Option<&'a crate::media_understanding::MediaEngine>,
    tts_engine: Option<&'a crate::tts::TtsEngine>,
    docker_config: Option<&'a captain_types::config::DockerSandboxConfig>,
    hooks: Option<&'a crate::hooks::HookRegistry>,
    process_manager: Option<&'a crate::process_manager::ProcessManager>,
    origin_channel: Option<&'a String>,
    agent_id_str: &'a str,
    caller_id_str: &'a str,
    effective_allowlist: Option<&'a [String]>,
    subagent_depth: u32,
    tool_result_blocks: &'a mut Vec<ContentBlock>,
}

/// Streaming counterpart of `execute_parallel_group` — same three-phase
/// design (see its doc comment), extended with the streaming-only PRE step
/// `handle_streaming_ask_user_tool_call`. That step never actually
/// triggers here: `ask_user` is not in the reviewed parallel-safe allowlist,
/// so it can never be part of an `ExecutionGroup::Parallel` batch — kept
/// for structural parity with `execute_one_streaming_tool_call` rather
/// than because it's reachable.
///
/// `stream_tx` (an `mpsc::Sender`) is safely shared across the concurrent
/// EXEC futures: `Sender::send` takes `&self`, and progress events from
/// different calls carry their own `tool_use_id` (`ToolStreamCtx`), so
/// interleaved delivery across concurrent calls doesn't lose attribution.
#[allow(clippy::too_many_arguments)]
async fn execute_streaming_parallel_group(
    input: StreamingParallelGroupInput<'_>,
) -> CaptainResult<Option<AgentLoopResult>> {
    let StreamingParallelGroupInput {
        calls,
        manifest,
        session,
        memory,
        loop_guard,
        tool_calls_recorded,
        visible_tools,
        available_tools,
        context_budget,
        hand_allowed_env,
        kernel,
        stream_tx,
        user_input_rx,
        skill_registry,
        mcp_connections,
        web_ctx,
        browser_ctx,
        workspace_root,
        on_phase,
        media_engine,
        tts_engine,
        docker_config,
        hooks,
        process_manager,
        origin_channel,
        agent_id_str,
        caller_id_str,
        effective_allowlist,
        subagent_depth,
        tool_result_blocks,
    } = input;

    let mut per_call_blocks: Vec<Vec<ContentBlock>> = calls.iter().map(|_| Vec::new()).collect();
    let mut pending: Vec<PendingExec<'_>> = Vec::new();
    let mut finish_result: Option<AgentLoopResult> = None;
    let mut pre_error: Option<captain_types::error::CaptainError> = None;
    let group_dependencies = current_dependency_frontier();
    let mut resolved_tool_ids = Vec::new();

    for (index, tool_call) in calls.iter().enumerate() {
        register_parallel_tool_dependencies(&tool_call.id, group_dependencies.clone());
        resolved_tool_ids.push(tool_call.id.clone());
        let verdict = loop_guard.check(&tool_call.name, &tool_call.input);
        let guard_action = apply_loop_guard_verdict(
            manifest,
            &mut *session,
            memory,
            hooks,
            agent_id_str,
            tool_call,
            &verdict,
            &mut per_call_blocks[index],
            true,
        )
        .await;
        match guard_action {
            Ok(ToolGuardAction::Execute) => {}
            Ok(ToolGuardAction::Continue) => continue,
            Ok(ToolGuardAction::Finish(result)) => {
                finish_result = Some(result);
                break;
            }
            Err(e) => {
                pre_error = Some(e);
                break;
            }
        }

        notify_tool_use_phase(on_phase, &tool_call.name);
        if !before_tool_call_allows_execution(
            hooks,
            manifest,
            caller_id_str,
            tool_call,
            &mut per_call_blocks[index],
        ) {
            continue;
        }

        if handle_streaming_ask_user_tool_call(
            manifest,
            tool_call,
            stream_tx,
            user_input_rx,
            tool_calls_recorded,
            &mut per_call_blocks[index],
        )
        .await
        {
            continue;
        }

        let effective_exec_policy = manifest.exec_policy.as_ref();
        let fut = Box::pin(run_streaming_tool_call(
            tool_call,
            kernel,
            effective_allowlist,
            caller_id_str,
            skill_registry,
            mcp_connections,
            web_ctx,
            browser_ctx,
            hand_allowed_env,
            workspace_root,
            media_engine,
            effective_exec_policy,
            tts_engine,
            docker_config,
            process_manager,
            origin_channel,
            stream_tx,
            subagent_depth,
        ));
        pending.push(PendingExec {
            index,
            verdict,
            fut,
        });
    }

    let indices: Vec<usize> = pending.iter().map(|p| p.index).collect();
    let verdicts: Vec<LoopGuardVerdict> = pending.iter().map(|p| p.verdict.clone()).collect();
    let exec_results = futures::future::join_all(pending.into_iter().map(|p| p.fut)).await;

    for ((index, verdict), (result, tool_elapsed_ms)) in
        indices.into_iter().zip(verdicts).zip(exec_results)
    {
        let tool_call = &calls[index];
        let streak_warning = loop_guard.record_tool_error(&tool_call.name, result.is_error);
        let verdict = crate::loop_guard::combine_verdict_with_error_streak(verdict, streak_warning);

        finish_tool_call(FinishToolCallInput {
            manifest,
            tool_call,
            result,
            verdict: &verdict,
            context_budget,
            available_tools,
            visible_tools: &mut *visible_tools,
            tool_calls_recorded: &mut *tool_calls_recorded,
            tool_result_blocks: &mut per_call_blocks[index],
            kernel,
            hooks,
            caller_id_str,
            tool_elapsed_ms,
            streaming: true,
            stream_tx: Some(stream_tx),
        })
        .await;
    }

    for blocks in per_call_blocks {
        tool_result_blocks.extend(blocks);
    }
    if !resolved_tool_ids.is_empty() {
        advance_dependency_frontier(resolved_tool_ids);
    }

    if let Some(e) = pre_error {
        return Err(e);
    }
    Ok(finish_result)
}

pub(crate) async fn execute_tool_calls_streaming(
    input: StreamingToolExecutionInput<'_>,
) -> CaptainResult<Option<AgentLoopResult>> {
    append_tool_use_assistant_turn(input.response, input.session, input.messages);
    interim_save_tool_turn(input.session, input.memory, "tool_use").await;
    let effective_allowlist = effective_tool_policy(input.manifest);
    let caller_id_str = input.session.agent_id.to_string();
    let subagent_depth = manifest_subagent_depth(input.manifest);
    let mut tool_result_blocks = Vec::new();
    log_parallel_opportunity(&input.response.tool_calls);

    for group in partition_parallel_groups(&input.response.tool_calls) {
        let outcome = match group {
            ExecutionGroup::Sequential(tool_call) => {
                let one_call_input = StreamingOneToolCallInput {
                    tool_call: &tool_call,
                    manifest: input.manifest,
                    session: &mut *input.session,
                    memory: input.memory,
                    loop_guard: &mut *input.loop_guard,
                    tool_calls_recorded: &mut *input.tool_calls_recorded,
                    visible_tools: &mut *input.visible_tools,
                    available_tools: input.available_tools,
                    context_budget: input.context_budget,
                    hand_allowed_env: input.hand_allowed_env,
                    kernel: input.kernel,
                    stream_tx: input.stream_tx,
                    user_input_rx: input.user_input_rx,
                    skill_registry: input.skill_registry,
                    mcp_connections: input.mcp_connections,
                    web_ctx: input.web_ctx,
                    browser_ctx: input.browser_ctx,
                    workspace_root: input.workspace_root,
                    on_phase: input.on_phase,
                    media_engine: input.media_engine,
                    tts_engine: input.tts_engine,
                    docker_config: input.docker_config,
                    hooks: input.hooks,
                    process_manager: input.process_manager,
                    origin_channel: input.origin_channel,
                    agent_id_str: input.agent_id_str,
                    caller_id_str: &caller_id_str,
                    effective_allowlist: effective_allowlist.as_deref(),
                    subagent_depth,
                    tool_result_blocks: &mut tool_result_blocks,
                };
                execute_one_streaming_tool_call(one_call_input).await?
            }
            ExecutionGroup::Parallel(calls) => {
                execute_streaming_parallel_group(StreamingParallelGroupInput {
                    calls: &calls,
                    manifest: input.manifest,
                    session: &mut *input.session,
                    memory: input.memory,
                    loop_guard: &mut *input.loop_guard,
                    tool_calls_recorded: &mut *input.tool_calls_recorded,
                    visible_tools: &mut *input.visible_tools,
                    available_tools: input.available_tools,
                    context_budget: input.context_budget,
                    hand_allowed_env: input.hand_allowed_env,
                    kernel: input.kernel,
                    stream_tx: input.stream_tx,
                    user_input_rx: input.user_input_rx,
                    skill_registry: input.skill_registry,
                    mcp_connections: input.mcp_connections,
                    web_ctx: input.web_ctx,
                    browser_ctx: input.browser_ctx,
                    workspace_root: input.workspace_root,
                    on_phase: input.on_phase,
                    media_engine: input.media_engine,
                    tts_engine: input.tts_engine,
                    docker_config: input.docker_config,
                    hooks: input.hooks,
                    process_manager: input.process_manager,
                    origin_channel: input.origin_channel,
                    agent_id_str: input.agent_id_str,
                    caller_id_str: &caller_id_str,
                    effective_allowlist: effective_allowlist.as_deref(),
                    subagent_depth,
                    tool_result_blocks: &mut tool_result_blocks,
                })
                .await?
            }
        };
        if let Some(result) = outcome {
            return Ok(Some(result));
        }
    }

    append_tool_result_turn(&mut tool_result_blocks, input.session, input.messages);
    interim_save_tool_turn(input.session, input.memory, "tool_results").await;
    Ok(None)
}

fn allowed_env_vars(hand_allowed_env: &[String]) -> Option<&[String]> {
    if hand_allowed_env.is_empty() {
        None
    } else {
        Some(hand_allowed_env)
    }
}
