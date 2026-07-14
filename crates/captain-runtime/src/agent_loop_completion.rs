use crate::agent_loop_tool_flow::{capability_denial_should_retry, phantom_action_detected};
use crate::llm_driver::CompletionResponse;
use captain_types::message::{ReplyDirectives, TokenUsage};
use captain_types::tool::ToolDefinition;
use tracing::{debug, warn};

pub(crate) enum EndTurnDecision {
    Silent { directives: ReplyDirectives },
    RetryEmpty { silent_failure: bool },
    RetryPhantom { text: String },
    RetryCapability { text: String },
    Complete { text: String },
}

pub(crate) struct EndTurnDecisionInput<'a> {
    pub(crate) agent_name: &'a str,
    pub(crate) response: &'a CompletionResponse,
    pub(crate) total_usage: &'a TokenUsage,
    pub(crate) messages_len: usize,
    pub(crate) iteration: u32,
    pub(crate) any_tools_executed: bool,
    pub(crate) capability_denial_watchdog_used: bool,
    pub(crate) visible_tools: &'a [ToolDefinition],
    pub(crate) streaming: bool,
    pub(crate) phantom_action_watchdog: bool,
}

pub(crate) fn decide_end_turn_response(input: EndTurnDecisionInput<'_>) -> EndTurnDecision {
    let raw_text = input.response.text();
    let (text, parsed_directives) = crate::reply_directives::parse_directives(&raw_text);

    if text.trim() == "NO_REPLY" || parsed_directives.silent {
        if input.streaming {
            debug!(agent = %input.agent_name, "Agent chose NO_REPLY/silent (streaming) — silent completion");
        } else {
            debug!(agent = %input.agent_name, "Agent chose NO_REPLY/silent — silent completion");
        }
        return EndTurnDecision::Silent {
            directives: ReplyDirectives {
                reply_to: parsed_directives.reply_to,
                current_thread: parsed_directives.current_thread,
                silent: true,
            },
        };
    }

    if text.trim().is_empty()
        && input.response.tool_calls.is_empty()
        && !input.response.has_any_content()
    {
        let silent_failure =
            input.response.usage.input_tokens == 0 && input.response.usage.output_tokens == 0;
        if input.iteration == 0 || silent_failure {
            if input.streaming {
                warn!(
                    agent = %input.agent_name,
                    iteration = input.iteration,
                    input_tokens = input.response.usage.input_tokens,
                    output_tokens = input.response.usage.output_tokens,
                    silent_failure,
                    "Empty response (streaming), retrying once"
                );
            } else {
                warn!(
                    agent = %input.agent_name,
                    iteration = input.iteration,
                    input_tokens = input.response.usage.input_tokens,
                    output_tokens = input.response.usage.output_tokens,
                    silent_failure,
                    "Empty response, retrying once"
                );
            }
            return EndTurnDecision::RetryEmpty { silent_failure };
        }
    }

    let text = if text.trim().is_empty() {
        if input.streaming {
            warn!(
                agent = %input.agent_name,
                iteration = input.iteration,
                input_tokens = input.total_usage.input_tokens,
                output_tokens = input.total_usage.output_tokens,
                messages_count = input.messages_len,
                "Empty response from LLM (streaming) — guard activated"
            );
        } else {
            warn!(
                agent = %input.agent_name,
                iteration = input.iteration,
                input_tokens = input.total_usage.input_tokens,
                output_tokens = input.total_usage.output_tokens,
                messages_count = input.messages_len,
                "Empty response from LLM — guard activated"
            );
        }
        if input.any_tools_executed {
            "[Task completed — the agent executed tools but did not produce a text summary.]"
                .to_string()
        } else {
            "[The model returned an empty response. This usually means the model is overloaded, the context is too large, or the API key lacks credits. Try again or check /status.]"
                .to_string()
        }
    } else {
        text
    };

    if input.phantom_action_watchdog
        && !input.any_tools_executed
        && input.iteration == 0
        && phantom_action_detected(&text)
    {
        warn!(agent = %input.agent_name, "Phantom action detected — re-prompting for real tool use");
        return EndTurnDecision::RetryPhantom { text };
    }

    if !input.capability_denial_watchdog_used
        && input.iteration <= 1
        && capability_denial_should_retry(&text, input.visible_tools)
    {
        if input.streaming {
            warn!(agent = %input.agent_name, "Capability denial detected (streaming) — forcing capability_search before final answer");
        } else {
            warn!(agent = %input.agent_name, "Capability denial detected — forcing capability_search before final answer");
        }
        return EndTurnDecision::RetryCapability { text };
    }

    EndTurnDecision::Complete { text }
}

#[cfg(test)]
#[path = "agent_loop_completion_tests.rs"]
mod tests;
