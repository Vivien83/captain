use crate::agent_loop::{AgentLoopResult, ToolCallRecord};
use crate::agent_loop_messages::assistant_message_for_response;
use crate::llm_driver::CompletionResponse;
use captain_memory::session::Session;
use captain_memory::MemorySubstrate;
use captain_types::agent::AgentManifest;
use captain_types::error::{CaptainError, CaptainResult};
use captain_types::message::{Message, TokenUsage};
use tracing::warn;

/// Maximum consecutive MaxTokens/Incomplete continuations before returning a
/// partial response. Raised from 3 to 5 to allow longer-form generation.
pub(crate) const MAX_CONTINUATIONS: u32 = 5;

pub(crate) fn append_max_tokens_continuation(
    response: &CompletionResponse,
    session: &mut Session,
    messages: &mut Vec<Message>,
) {
    let text = response.text();
    let assistant_msg = assistant_message_for_response(response, text);
    session.messages.push(assistant_msg.clone());
    messages.push(assistant_msg);

    let continue_msg = Message::user("Please continue.");
    session.messages.push(continue_msg.clone());
    messages.push(continue_msg);
}

pub(crate) fn append_incomplete_continuation(
    response: &CompletionResponse,
    partial_text: String,
    provider_name: &str,
    session: &mut Session,
    messages: &mut Vec<Message>,
) {
    let assistant_msg = assistant_message_for_response(response, partial_text);
    session.messages.push(assistant_msg.clone());
    messages.push(assistant_msg);

    let nudge = incomplete_continuation_nudge(provider_name);
    let nudge_msg = Message::user(nudge);
    session.messages.push(nudge_msg.clone());
    messages.push(nudge_msg);
}

fn incomplete_continuation_nudge(provider_name: &str) -> &'static str {
    if provider_name == "codex" || provider_name == "openai-codex" {
        "Continue from the incomplete Codex response. If the next step needs a tool, call it now; otherwise finish the answer."
    } else {
        "Please continue from the incomplete response."
    }
}

pub(crate) struct MaxTokensContinuationInput<'a> {
    pub(crate) response: &'a CompletionResponse,
    pub(crate) session: &'a mut Session,
    pub(crate) memory: &'a MemorySubstrate,
    pub(crate) manifest: &'a AgentManifest,
    pub(crate) hooks: Option<&'a crate::hooks::HookRegistry>,
    pub(crate) agent_id_str: &'a str,
    pub(crate) total_usage: &'a TokenUsage,
    pub(crate) iteration: u32,
    pub(crate) consecutive_max_tokens: &'a mut u32,
    pub(crate) consecutive_incomplete: &'a mut u32,
    pub(crate) tool_calls_recorded: &'a [ToolCallRecord],
    pub(crate) streaming: bool,
    pub(crate) messages: &'a mut Vec<Message>,
}

pub(crate) async fn handle_max_tokens_continuation(
    input: MaxTokensContinuationInput<'_>,
) -> CaptainResult<Option<AgentLoopResult>> {
    *input.consecutive_max_tokens += 1;
    *input.consecutive_incomplete = 0;
    if *input.consecutive_max_tokens >= MAX_CONTINUATIONS {
        return finish_continuation_limit(FinishContinuationLimitInput {
            kind: ContinuationLimitKind::MaxTokens,
            response: input.response,
            session: input.session,
            memory: input.memory,
            manifest: input.manifest,
            hooks: input.hooks,
            agent_id_str: input.agent_id_str,
            total_usage: *input.total_usage,
            iteration: input.iteration,
            consecutive_count: *input.consecutive_max_tokens,
            tool_calls_recorded: input.tool_calls_recorded,
            streaming: input.streaming,
        })
        .await
        .map(Some);
    }

    append_max_tokens_continuation(input.response, input.session, input.messages);
    if input.streaming {
        warn!(
            iteration = input.iteration,
            "Max tokens hit (streaming), continuing"
        );
    } else {
        warn!(iteration = input.iteration, "Max tokens hit, continuing");
    }
    Ok(None)
}

pub(crate) struct IncompleteContinuationInput<'a> {
    pub(crate) response: &'a CompletionResponse,
    pub(crate) provider_name: &'a str,
    pub(crate) session: &'a mut Session,
    pub(crate) memory: &'a MemorySubstrate,
    pub(crate) manifest: &'a AgentManifest,
    pub(crate) hooks: Option<&'a crate::hooks::HookRegistry>,
    pub(crate) agent_id_str: &'a str,
    pub(crate) total_usage: &'a TokenUsage,
    pub(crate) iteration: u32,
    pub(crate) consecutive_max_tokens: &'a mut u32,
    pub(crate) consecutive_incomplete: &'a mut u32,
    pub(crate) tool_calls_recorded: &'a [ToolCallRecord],
    pub(crate) streaming: bool,
    pub(crate) messages: &'a mut Vec<Message>,
}

pub(crate) async fn handle_incomplete_continuation(
    input: IncompleteContinuationInput<'_>,
) -> CaptainResult<Option<AgentLoopResult>> {
    *input.consecutive_incomplete += 1;
    *input.consecutive_max_tokens = 0;
    let text = input.response.text();
    if *input.consecutive_incomplete >= MAX_CONTINUATIONS {
        return finish_continuation_limit(FinishContinuationLimitInput {
            kind: ContinuationLimitKind::Incomplete,
            response: input.response,
            session: input.session,
            memory: input.memory,
            manifest: input.manifest,
            hooks: input.hooks,
            agent_id_str: input.agent_id_str,
            total_usage: *input.total_usage,
            iteration: input.iteration,
            consecutive_count: *input.consecutive_incomplete,
            tool_calls_recorded: input.tool_calls_recorded,
            streaming: input.streaming,
        })
        .await
        .map(Some);
    }

    append_incomplete_continuation(
        input.response,
        text,
        input.provider_name,
        input.session,
        input.messages,
    );
    if input.streaming {
        warn!(
            iteration = input.iteration,
            "Incomplete response (streaming), continuing"
        );
    } else {
        warn!(
            iteration = input.iteration,
            "Incomplete response, continuing"
        );
    }
    Ok(None)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContinuationLimitKind {
    MaxTokens,
    Incomplete,
}

pub(crate) struct FinishContinuationLimitInput<'a> {
    pub(crate) kind: ContinuationLimitKind,
    pub(crate) response: &'a CompletionResponse,
    pub(crate) session: &'a mut Session,
    pub(crate) memory: &'a MemorySubstrate,
    pub(crate) manifest: &'a AgentManifest,
    pub(crate) hooks: Option<&'a crate::hooks::HookRegistry>,
    pub(crate) agent_id_str: &'a str,
    pub(crate) total_usage: TokenUsage,
    pub(crate) iteration: u32,
    pub(crate) consecutive_count: u32,
    pub(crate) tool_calls_recorded: &'a [ToolCallRecord],
    pub(crate) streaming: bool,
}

pub(crate) async fn finish_continuation_limit(
    input: FinishContinuationLimitInput<'_>,
) -> CaptainResult<AgentLoopResult> {
    let text = continuation_limit_text(input.kind, input.response);
    input
        .session
        .messages
        .push(assistant_message_for_response(input.response, text.clone()));

    if let Err(e) = input.memory.save_session_async(input.session).await {
        match input.kind {
            ContinuationLimitKind::MaxTokens => {
                warn!("Failed to save session on max continuations: {e}");
            }
            ContinuationLimitKind::Incomplete => {
                warn!("Failed to save session on incomplete continuations: {e}");
            }
        }
    }

    warn_continuation_limit(
        input.kind,
        input.iteration,
        input.consecutive_count,
        input.streaming,
    );

    if input.kind == ContinuationLimitKind::MaxTokens {
        fire_max_continuations_hook(
            input.hooks,
            input.manifest,
            input.agent_id_str,
            input.iteration + 1,
        );
    }

    Ok(AgentLoopResult {
        response: text,
        total_usage: input.total_usage,
        iterations: input.iteration + 1,
        cost_usd: None,
        silent: false,
        directives: Default::default(),
        tool_calls: input.tool_calls_recorded.to_vec(),
    })
}

fn continuation_limit_text(kind: ContinuationLimitKind, response: &CompletionResponse) -> String {
    let text = response.text();
    if !text.trim().is_empty() {
        return text;
    }

    match kind {
        ContinuationLimitKind::MaxTokens => {
            "[Partial response — token limit reached with no text output.]".to_string()
        }
        ContinuationLimitKind::Incomplete => {
            "[Partial response — Codex ended the turn incomplete with no text output.]".to_string()
        }
    }
}

fn warn_continuation_limit(
    kind: ContinuationLimitKind,
    iteration: u32,
    consecutive_count: u32,
    streaming: bool,
) {
    match (kind, streaming) {
        (ContinuationLimitKind::MaxTokens, false) => warn!(
            iteration,
            consecutive_max_tokens = consecutive_count,
            "Max continuations reached, returning partial response"
        ),
        (ContinuationLimitKind::MaxTokens, true) => warn!(
            iteration,
            consecutive_max_tokens = consecutive_count,
            "Max continuations reached (streaming), returning partial response"
        ),
        (ContinuationLimitKind::Incomplete, false) => warn!(
            iteration,
            consecutive_incomplete = consecutive_count,
            "Incomplete continuations reached, returning partial response"
        ),
        (ContinuationLimitKind::Incomplete, true) => warn!(
            iteration,
            consecutive_incomplete = consecutive_count,
            "Incomplete continuations reached (streaming), returning partial response"
        ),
    }
}

fn fire_max_continuations_hook(
    hooks: Option<&crate::hooks::HookRegistry>,
    manifest: &AgentManifest,
    agent_id_str: &str,
    completed_iterations: u32,
) {
    if let Some(hook_reg) = hooks {
        let ctx = crate::hooks::HookContext {
            agent_name: &manifest.name,
            agent_id: agent_id_str,
            event: captain_types::agent::HookEvent::AgentLoopEnd,
            data: serde_json::json!({
                "iterations": completed_iterations,
                "reason": "max_continuations",
            }),
        };
        let _ = hook_reg.fire(&ctx);
    }
}

pub(crate) async fn fail_max_iterations(
    manifest: &AgentManifest,
    session: &mut Session,
    memory: &MemorySubstrate,
    hooks: Option<&crate::hooks::HookRegistry>,
    agent_id_str: &str,
    max_iterations: u32,
) -> CaptainResult<AgentLoopResult> {
    if let Err(e) = memory.save_session_async(session).await {
        warn!("Failed to save session on max iterations: {e}");
    }

    if let Some(hook_reg) = hooks {
        let ctx = crate::hooks::HookContext {
            agent_name: &manifest.name,
            agent_id: agent_id_str,
            event: captain_types::agent::HookEvent::AgentLoopEnd,
            data: serde_json::json!({
                "reason": "max_iterations_exceeded",
                "iterations": max_iterations,
            }),
        };
        let _ = hook_reg.fire(&ctx);
    }

    Err(CaptainError::MaxIterationsExceeded(max_iterations))
}

#[cfg(test)]
#[path = "agent_loop_limits_tests.rs"]
mod tests;
