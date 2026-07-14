use crate::agent_loop::AgentLoopResult;
use crate::agent_loop_budget::{budget_blocks_next_tool_step, finish_budget_limited_turn};
use crate::agent_loop_interjections::{drain_user_interjections, format_interjection_prompt};
use crate::agent_loop_phase::{
    notify_stream_iteration_phase, notify_thinking_phase, PhaseCallback,
};
use crate::agent_loop_request::{
    build_completion_request, prepare_request_context, strip_provider_prefix,
};
use crate::agent_loop_retry::{call_with_retry, stream_with_retry};
use crate::agent_loop_stream_events::send_context_recovery_phase_event;
use crate::agent_loop_tool_flow::codex_missing_tool_call_should_retry;
use crate::context_budget::ContextBudget;
use crate::context_overflow::RecoveryStage;
use crate::llm_driver::{CompletionResponse, LlmDriver, StreamEvent};
use captain_memory::session::Session;
use captain_memory::MemorySubstrate;
use captain_types::agent::AgentManifest;
use captain_types::error::CaptainResult;
use captain_types::message::{Message, TokenUsage};
use captain_types::tool::ToolDefinition;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

pub(crate) enum IterationCallOutcome {
    Response(CompletionResponse),
    Continue,
    Finished(AgentLoopResult),
}

pub(crate) struct CompletionIterationInput<'a> {
    pub(crate) manifest: &'a AgentManifest,
    pub(crate) session: &'a mut Session,
    pub(crate) memory: &'a MemorySubstrate,
    pub(crate) driver: &'a dyn LlmDriver,
    pub(crate) messages: &'a mut Vec<Message>,
    pub(crate) system_prompt: &'a str,
    pub(crate) visible_tools: &'a [ToolDefinition],
    pub(crate) context_budget: &'a ContextBudget,
    pub(crate) ctx_window: usize,
    pub(crate) iteration: u32,
    pub(crate) total_usage: &'a mut TokenUsage,
    pub(crate) on_phase: Option<&'a PhaseCallback>,
}

pub(crate) struct StreamingIterationInput<'a> {
    pub(crate) manifest: &'a AgentManifest,
    pub(crate) session: &'a mut Session,
    pub(crate) memory: &'a MemorySubstrate,
    pub(crate) driver: &'a dyn LlmDriver,
    pub(crate) messages: &'a mut Vec<Message>,
    pub(crate) system_prompt: &'a str,
    pub(crate) visible_tools: &'a [ToolDefinition],
    pub(crate) context_budget: &'a ContextBudget,
    pub(crate) ctx_window: usize,
    pub(crate) iteration: u32,
    pub(crate) total_usage: &'a mut TokenUsage,
    pub(crate) on_phase: Option<&'a PhaseCallback>,
    pub(crate) stream_tx: &'a mpsc::Sender<StreamEvent>,
    pub(crate) user_input_rx: &'a Option<Arc<tokio::sync::Mutex<mpsc::Receiver<String>>>>,
    pub(crate) codex_missing_tool_watchdog_used: &'a mut bool,
}

pub(crate) async fn complete_iteration(
    input: CompletionIterationInput<'_>,
) -> CaptainResult<IterationCallOutcome> {
    let provider_name = input.manifest.model.provider.as_str();
    let prepared_request_context = prepare_request_context(
        &input.manifest.name,
        input.messages,
        input.system_prompt,
        input.visible_tools,
        provider_name,
        input.context_budget,
        input.ctx_window,
        input.iteration,
        true,
        false,
    );
    if prepared_request_context.recovery == RecoveryStage::FinalError {
        warn!("Context overflow unrecoverable - suggest /reset or /compact");
    }

    let request = build_iteration_request(
        input.manifest,
        input.session,
        input.messages,
        prepared_request_context.request_tools,
        input.system_prompt,
    );

    notify_thinking_phase(input.on_phase);
    let mut response = call_with_retry(input.driver, request, Some(provider_name), None).await?;
    record_and_promote_response(
        input.total_usage,
        &mut response,
        provider_name,
        input.visible_tools,
        false,
    );

    if let Some(result) = finish_if_tool_budget_reached(FinishToolBudgetInput {
        manifest: input.manifest,
        session: input.session,
        memory: input.memory,
        total_usage: input.total_usage,
        iteration: input.iteration,
        streaming: false,
    })
    .await?
    {
        return Ok(IterationCallOutcome::Finished(result));
    }

    Ok(IterationCallOutcome::Response(response))
}

pub(crate) async fn stream_iteration(
    input: StreamingIterationInput<'_>,
) -> CaptainResult<IterationCallOutcome> {
    let provider_name = input.manifest.model.provider.as_str();
    let prepared_request_context = prepare_request_context(
        &input.manifest.name,
        input.messages,
        input.system_prompt,
        input.visible_tools,
        provider_name,
        input.context_budget,
        input.ctx_window,
        input.iteration,
        false,
        true,
    );
    send_context_recovery_phase_event(input.stream_tx, &prepared_request_context.recovery).await;

    drain_interjections(
        input.manifest,
        input.session,
        input.messages,
        input.user_input_rx,
        input.iteration,
    );

    let request = build_iteration_request(
        input.manifest,
        input.session,
        input.messages,
        prepared_request_context.request_tools,
        input.system_prompt,
    );
    let request_model_for_log = request.model.clone();

    notify_stream_iteration_phase(input.on_phase, input.iteration);
    let mut response = stream_with_retry(
        input.driver,
        request,
        input.stream_tx.clone(),
        Some(provider_name),
        None,
    )
    .await?;
    record_and_promote_response(
        input.total_usage,
        &mut response,
        provider_name,
        input.visible_tools,
        true,
    );

    if should_retry_codex_missing_tool_call(
        input.manifest,
        input.messages,
        input.codex_missing_tool_watchdog_used,
        provider_name,
        &request_model_for_log,
        input.iteration,
        &response,
        input.visible_tools,
    ) {
        return Ok(IterationCallOutcome::Continue);
    }

    if let Some(result) = finish_if_tool_budget_reached(FinishToolBudgetInput {
        manifest: input.manifest,
        session: input.session,
        memory: input.memory,
        total_usage: input.total_usage,
        iteration: input.iteration,
        streaming: true,
    })
    .await?
    {
        return Ok(IterationCallOutcome::Finished(result));
    }

    Ok(IterationCallOutcome::Response(response))
}

fn build_iteration_request(
    manifest: &AgentManifest,
    session: &Session,
    messages: &[Message],
    request_tools: Vec<ToolDefinition>,
    system_prompt: &str,
) -> crate::llm_driver::CompletionRequest {
    let api_model = strip_provider_prefix(&manifest.model.model, &manifest.model.provider);
    build_completion_request(
        manifest,
        session,
        api_model,
        messages,
        request_tools,
        system_prompt,
    )
}

fn drain_interjections(
    manifest: &AgentManifest,
    session: &mut Session,
    messages: &mut Vec<Message>,
    user_input_rx: &Option<Arc<tokio::sync::Mutex<mpsc::Receiver<String>>>>,
    iteration: u32,
) {
    let interjections = drain_user_interjections(user_input_rx);
    if interjections.is_empty() {
        return;
    }

    for msg in &interjections {
        info!(
            agent = %manifest.name,
            iteration,
            interjection = %msg,
            "user interjection drained mid-loop"
        );
        let prompt = format_interjection_prompt(msg);
        messages.push(Message::user(prompt.clone()));
        session.messages.push(Message::user(prompt));
    }
}

fn record_and_promote_response(
    total_usage: &mut TokenUsage,
    response: &mut CompletionResponse,
    provider_name: &str,
    visible_tools: &[ToolDefinition],
    streaming: bool,
) {
    crate::agent_loop_usage::record_response_usage(total_usage, response, provider_name);

    if let Some(recovered_count) =
        crate::agent_loop_tool_recovery::promote_recovered_text_tool_calls(response, visible_tools)
    {
        if streaming {
            info!(
                count = recovered_count,
                "Recovered text-based tool calls (streaming); promoting to ToolUse"
            );
        } else {
            info!(
                count = recovered_count,
                "Recovered text-based tool calls; promoting to ToolUse"
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn should_retry_codex_missing_tool_call(
    manifest: &AgentManifest,
    messages: &mut Vec<Message>,
    codex_missing_tool_watchdog_used: &mut bool,
    provider_name: &str,
    request_model_for_log: &str,
    iteration: u32,
    response: &CompletionResponse,
    visible_tools: &[ToolDefinition],
) -> bool {
    if *codex_missing_tool_watchdog_used
        || !codex_missing_tool_call_should_retry(provider_name, response, visible_tools)
    {
        return false;
    }

    *codex_missing_tool_watchdog_used = true;
    let narrated = response.text();
    warn!(
        agent = %manifest.name,
        model = %request_model_for_log,
        iteration,
        text_preview = %captain_types::truncate_str(&narrated, 240),
        "Codex narrated a tool action without emitting a tool call; retrying once with a tool-call-only nudge"
    );
    messages.push(Message::assistant(narrated));
    messages.push(Message::user(
        "Call the appropriate tool now. Do not describe the action. If no tool is needed, answer directly without saying you will use one."
            .to_string(),
    ));
    true
}

struct FinishToolBudgetInput<'a> {
    manifest: &'a AgentManifest,
    session: &'a mut Session,
    memory: &'a MemorySubstrate,
    total_usage: &'a TokenUsage,
    iteration: u32,
    streaming: bool,
}

async fn finish_if_tool_budget_reached(
    input: FinishToolBudgetInput<'_>,
) -> CaptainResult<Option<AgentLoopResult>> {
    if let Some(budget_tokens) = budget_blocks_next_tool_step(input.total_usage) {
        if input.streaming {
            warn!(
                agent = %input.manifest.name,
                used_tokens = input.total_usage.total(),
                budget_tokens,
                "Turn token budget reached before streaming tool execution"
            );
        } else {
            warn!(
                agent = %input.manifest.name,
                used_tokens = input.total_usage.total(),
                budget_tokens,
                "Turn token budget reached before tool execution"
            );
        }
        return finish_budget_limited_turn(
            input.session,
            input.memory,
            *input.total_usage,
            input.iteration + 1,
            budget_tokens,
        )
        .await
        .map(Some);
    }

    Ok(None)
}
