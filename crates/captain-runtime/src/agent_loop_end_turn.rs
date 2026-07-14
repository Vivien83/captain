use crate::agent_loop_completion::{
    decide_end_turn_response, EndTurnDecision, EndTurnDecisionInput,
};
use crate::agent_loop_finish::{finish_silent_turn, finish_successful_turn, SuccessfulTurnInput};
use crate::agent_loop_messages::assistant_message_for_response;
use crate::agent_loop_phase::PhaseCallback;
use crate::agent_loop_result::AgentLoopResult;
use crate::agent_loop_tool_flow::capability_search_nudge;
use crate::agent_loop_tool_record::ToolCallRecord;
use crate::embedding::EmbeddingDriver;
use crate::llm_driver::CompletionResponse;
use captain_memory::session::Session;
use captain_memory::MemorySubstrate;
use captain_types::agent::AgentManifest;
use captain_types::error::CaptainResult;
use captain_types::message::{Message, TokenUsage};
use captain_types::tool::ToolDefinition;

pub(crate) struct EndTurnInput<'a> {
    pub(crate) manifest: &'a AgentManifest,
    pub(crate) user_message: &'a str,
    pub(crate) response: &'a CompletionResponse,
    pub(crate) total_usage: &'a TokenUsage,
    pub(crate) messages: &'a mut Vec<Message>,
    pub(crate) iteration: u32,
    pub(crate) any_tools_executed: bool,
    pub(crate) capability_denial_watchdog_used: &'a mut bool,
    pub(crate) visible_tools: &'a [ToolDefinition],
    pub(crate) streaming: bool,
    pub(crate) phantom_action_watchdog: bool,
    pub(crate) session: &'a mut Session,
    pub(crate) memory: &'a MemorySubstrate,
    pub(crate) embedding_driver: Option<&'a (dyn EmbeddingDriver + Send + Sync)>,
    pub(crate) on_phase: Option<&'a PhaseCallback>,
    pub(crate) hooks: Option<&'a crate::hooks::HookRegistry>,
    pub(crate) agent_id_str: &'a str,
    pub(crate) tool_calls_recorded: &'a [ToolCallRecord],
}

pub(crate) async fn handle_end_turn_response(
    input: EndTurnInput<'_>,
) -> CaptainResult<Option<AgentLoopResult>> {
    let EndTurnInput {
        manifest,
        user_message,
        response,
        total_usage,
        messages,
        iteration,
        any_tools_executed,
        capability_denial_watchdog_used,
        visible_tools,
        streaming,
        phantom_action_watchdog,
        session,
        memory,
        embedding_driver,
        on_phase,
        hooks,
        agent_id_str,
        tool_calls_recorded,
    } = input;

    match decide_end_turn_response(EndTurnDecisionInput {
        agent_name: &manifest.name,
        response,
        total_usage,
        messages_len: messages.len(),
        iteration,
        any_tools_executed,
        capability_denial_watchdog_used: *capability_denial_watchdog_used,
        visible_tools,
        streaming,
        phantom_action_watchdog,
    }) {
        EndTurnDecision::Silent { directives } => finish_silent_turn(
            session,
            memory,
            *total_usage,
            iteration + 1,
            directives,
            tool_calls_recorded,
        )
        .await
        .map(Some),
        EndTurnDecision::RetryEmpty { silent_failure } => {
            if silent_failure {
                *messages = crate::session_repair::validate_and_repair(&*messages);
            }
            messages.push(Message::assistant("[no response]".to_string()));
            messages.push(Message::user("Please provide your response.".to_string()));
            Ok(None)
        }
        EndTurnDecision::RetryPhantom { text } => {
            messages.push(Message::assistant(text));
            messages.push(Message::user(
                "[System: You claimed to perform an action but did not call any tools. \
                 You must use the appropriate tool (e.g., channel_send, web_fetch, file_write) \
                 to actually perform the action. Do not claim completion without executing tools.]",
            ));
            Ok(None)
        }
        EndTurnDecision::RetryCapability { text } => {
            *capability_denial_watchdog_used = true;
            messages.push(Message::assistant(text));
            messages.push(capability_search_nudge());
            Ok(None)
        }
        EndTurnDecision::Complete { text } => {
            let assistant_message = assistant_message_for_response(response, text.clone());
            finish_successful_turn(SuccessfulTurnInput {
                manifest,
                user_message,
                final_response: text,
                assistant_message,
                completed_iterations: iteration + 1,
                session,
                memory,
                embedding_driver,
                on_phase,
                hooks,
                agent_id_str,
                total_usage: *total_usage,
                tool_calls_recorded,
                streaming,
            })
            .await
            .map(Some)
        }
    }
}

#[cfg(test)]
#[path = "agent_loop_end_turn_tests.rs"]
mod tests;
